// src/robot/engines/master.rs - Master Engine Otonom İnfaz Merkezi
// Srivastava ATP - İşlevsel Çarklar Odası (Unified Master Engine - Final Safe Compilation)

use crate::prelude::*;
use super::base::{EngineConfig, TradingEngine};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::time::{sleep, Duration};

/// Anomali-tetikli ML retrain'in son fire epoch'u (saniye). 0 = hiç fire edilmedi.
/// `perform_anomaly_recovery` her cycle çağrılır ama bu cooldown sayesinde
/// ML trigger spam'i kapanır (default 300sn, `ANOMALY_ML_TRIGGER_COOLDOWN_SECS`).
static ANOMALY_ML_LAST_TRIGGER_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Sembol bazlı log throttle: (symbol, kind) → son emit epoch. Aynı sembol için
/// "DataIngest empty" log'u her cycle (500ms) tekrarlanmasın diye 300sn pencere
/// (env `LOG_DATAINGEST_COOLDOWN_SECS`). HashMap büyümesi sınırlı — sembol sayısı
/// orchestrator'la beraber tipik <100.
static LOG_THROTTLE_MAP: std::sync::OnceLock<std::sync::Mutex<
    std::collections::HashMap<(String, &'static str), u64>
>> = std::sync::OnceLock::new();

/// Phase C: TRAILING_STOP kapanışları sonrası 60sn olgunluk gözlem kuyruğu.
/// close_paper_position TrailingStop branch'ı buraya enqueue eder; periyodik
/// processor (spawn_trail_feedback_processor) olgunlaşmış kayıtları evalue edip
/// ParameterStore.record_trailing_outcome'a iletir. Static — AppState alanı eklemiyoruz.
static PENDING_TRAIL_OBS: std::sync::OnceLock<std::sync::Mutex<
    std::collections::VecDeque<crate::robot::parameters::PendingTrailObservation>
>> = std::sync::OnceLock::new();

/// Delisted sembol tespit sayacı: sembol → ardışık fetch hatası sayısı.
/// Eşik (DELISTED_DETECTION_THRESHOLD env, default 3) aşılınca
/// orchestrator'dan çıkarılır + live pozisyonu varsa force-close. Başarılı
/// fetch sayacı sıfırlar (geçici Binance hatasının yanlış pozitif olmaması için).
static DELISTED_FAIL_COUNTERS: std::sync::OnceLock<std::sync::Mutex<
    std::collections::HashMap<String, u32>
>> = std::sync::OnceLock::new();

fn delisted_counters() -> &'static std::sync::Mutex<std::collections::HashMap<String, u32>> {
    DELISTED_FAIL_COUNTERS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Ardışık fetch hatası sayacını artırır ve yeni sayıyı döner.
pub fn delisted_record_failure(symbol: &str) -> u32 {
    let mut guard = match delisted_counters().lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };
    let counter = guard.entry(symbol.to_string()).or_insert(0);
    *counter += 1;
    *counter
}

/// Başarılı fetch sonrası sayacı sıfırlar (geçici hata yanlış pozitif vermesin).
pub fn delisted_record_success(symbol: &str) {
    if let Ok(mut guard) = delisted_counters().lock() {
        guard.remove(symbol);
    }
}

/// Sayacı sorgular (test ve teşhis için).
pub fn delisted_failure_count(symbol: &str) -> u32 {
    delisted_counters().lock().ok().and_then(|g| g.get(symbol).copied()).unwrap_or(0)
}

/// Eşik (env DELISTED_DETECTION_THRESHOLD, default 3). 0 verilirse
/// auto-detect kapanır (manuel müdahale için).
pub fn delisted_detection_threshold() -> u32 {
    std::env::var("DELISTED_DETECTION_THRESHOLD")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(3)
}

fn trail_obs_queue() -> &'static std::sync::Mutex<
    std::collections::VecDeque<crate::robot::parameters::PendingTrailObservation>
> {
    PENDING_TRAIL_OBS.get_or_init(|| std::sync::Mutex::new(std::collections::VecDeque::new()))
}

pub fn enqueue_trail_observation(obs: crate::robot::parameters::PendingTrailObservation) {
    if let Ok(mut q) = trail_obs_queue().lock() {
        // Kuyruğu sınırla — pathological case'de 1000+ gözlem birikmesin.
        while q.len() >= 500 { q.pop_front(); }
        q.push_back(obs);
    }
}

pub fn log_throttle_should_emit(symbol: &str, kind: &'static str, cooldown_secs: u64) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let map = LOG_THROTTLE_MAP.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = match map.lock() {
        Ok(g) => g,
        Err(_) => return true, // poisoned ise log'a izin ver
    };
    let key = (symbol.to_string(), kind);
    if let Some(last) = guard.get(&key) {
        if now.saturating_sub(*last) < cooldown_secs {
            return false;
        }
    }
    guard.insert(key, now);
    true
}

/// `MAX_ENTRY_PRICE_DEVIATION_PCT` env'ini parse eder; geçersiz/eksikse 5.0 döner.
/// 0 verilirse price sanity guard kapanır (test ve operatör override için).
pub fn price_deviation_threshold_from_env() -> f64 {
    std::env::var("MAX_ENTRY_PRICE_DEVIATION_PCT")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(5.0)
}

/// Entry ile reference (DB son mum) arasındaki yüzde sapma. Reference <= 0 ise 0 döner.
pub fn price_deviation_pct(entry: f64, reference: f64) -> f64 {
    if reference > 0.0 {
        ((entry - reference).abs() / reference) * 100.0
    } else { 0.0 }
}

/// Price sanity sınırı aşıldı mı? `max_dev_pct <= 0` ise guard kapalı (false döner).
pub fn price_deviation_exceeds(entry: f64, reference: f64, max_dev_pct: f64) -> bool {
    if max_dev_pct <= 0.0 || reference <= 0.0 { return false; }
    price_deviation_pct(entry, reference) > max_dev_pct
}

/// Candle'ın "fresh" olduğunu doğrular: now - candle.timestamp <= eşik.
/// Eşik env `CANDLE_FRESHNESS_SECS` (default 300sn). DB candles günlerce
/// eski olabilir (BIST veya pasif sembol) → bu durumda candle.close referans
/// olarak güvenilmez; price sanity guard pas geçer.
pub fn candle_is_fresh(candle_ts: &chrono::DateTime<chrono::Utc>) -> bool {
    let max_age_secs: i64 = std::env::var("CANDLE_FRESHNESS_SECS")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let age_secs = (chrono::Utc::now() - *candle_ts).num_seconds();
    age_secs >= 0 && age_secs <= max_age_secs
}

// Projedeki gerçek tiplerin ve trait'lerin bağlanması için ön hazırlık (Agnostik Katman)
pub struct MLModel;
pub struct Monitor;
pub trait MarketRegimeDetector {}
pub trait StrategyLifecycleManager {}

pub struct Engine {
    pub config: EngineConfig,
    pub ml_model: Option<MLModel>,
    pub monitor: Option<Monitor>,
    pub last_cycle_at: std::time::Instant,
    pub regime_detector: Box<dyn MarketRegimeDetector + Send>,
    pub strategy_manager: Box<dyn StrategyLifecycleManager + Send>,
}

/// Bir pozisyonun kapanış sebebi — `ClosedTradeModel.exit_reason` string'i bu enum'dan üretilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    StopLoss,
    TakeProfit,
    TrailingStop,
    Breakeven,
    StrategySignal,
}

impl ExitReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ExitReason::StopLoss        => "STOP_LOSS",
            ExitReason::TakeProfit      => "TAKE_PROFIT",
            ExitReason::TrailingStop    => "TRAILING_STOP",
            ExitReason::Breakeven       => "BREAKEVEN",
            ExitReason::StrategySignal  => "STRATEGY_SIGNAL",
        }
    }
    pub fn emoji(self) -> &'static str {
        match self {
            ExitReason::StopLoss        => "🔻",
            ExitReason::TakeProfit      => "🎯",
            ExitReason::TrailingStop    => "🪤",
            ExitReason::Breakeven       => "⚖️",
            ExitReason::StrategySignal  => "🏁",
        }
    }
}


// ── Faz 1: impl Engine sorumluluk modüllerine bölündü (davranış birebir) ──
mod loop_core;
mod infra_fleet;
mod userdata;
mod positions;
mod jobs;
mod persistence;
