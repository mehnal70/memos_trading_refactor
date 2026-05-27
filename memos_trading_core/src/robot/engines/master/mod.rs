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

/// `state` kilidini kısa süreliğine alıp tek bir log satırı düşürür.
/// `if let Ok(mut st) = state.lock() { st.push_log(msg); }` boilerplate'ini DRY'lar.
/// Davranış birebir: kilit poisoned ise sessizce geçer (eski blok da öyleydi).
pub(crate) fn push_state_log(state: &Arc<Mutex<AppState>>, msg: String) {
    if let Ok(mut st) = state.lock() {
        st.push_log(msg);
    }
}

/// Env değişkenini `T`'ye parse eder; eksik/geçersizse `default`. Per-call (cache yok)
/// → env-mutasyonlu testlerle uyumlu. `.ok().and_then(|s| s.parse().ok()).unwrap_or(d)`
/// kalıbını tek noktaya toplar.
pub(crate) fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// "Açık mı?" tarzı bayrak: yalnızca "1" / "true" (case-insensitive) → true; aksi
/// halde false. ALLOW_BIST, *_DISABLE, *_ENABLED gibi default-false toggle'lar için.
pub(crate) fn env_truthy(key: &str) -> bool {
    std::env::var(key).map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
}

/// Boot'ta env'den BİR KEZ okunan runtime ayar paketi. `AppState.tuning` alanında
/// `Arc` olarak tutulur; cycle hot-path'i her tur `getenv` yapmak yerine bu struct'tan
/// okur. Env hâlâ kaynaktır (operatör override eder) ama yalnızca `AppState::new`'da
/// okunur → per-cycle syscall + alloc yok. OnceLock global'inden farkı: her `AppState`
/// kendi env snapshot'ını alır → env-mutasyonlu testlerle de uyumlu.
#[derive(Debug, Clone)]
pub struct RuntimeTuning {
    /// Default'ta canlı feed'i OLMAYAN ama operatörün yine de cycle'a zorladığı
    /// borsalar (market-agnostik override). Default boş. Eligibility kararı
    /// `symbol_eligible_for_live` üzerinden tek noktada verilir → motorun hiçbir
    /// yerinde borsa-adı sabiti yok. Env: FORCE_LIVE_EXCHANGES=bist,kucoin
    /// (+ geriye uyum: ALLOW_BIST=1 → listeye Bist ekler).
    pub force_live_exchanges: Vec<crate::core::types::Exchange>,
    /// Tek emir komisyon oranı (entry+exit'te ayrı ayrı uygulanır). Default 0.001.
    /// Binance USDⓈ-M taker ≈ 0.0004 → gerçekçi live için `COMMISSION_RATE=0.0004`.
    pub commission_rate: f64,
    /// "Winner'ı koştur": açılışta sabit TP'yi çok uzağa it → kâr çıkışını ATR
    /// trailing yönetir. Env `LET_WINNERS_RUN`, default false. Backtest (bt_ab_exit_mgmt)
    /// yüksek zaman diliminde net pozitif, 1m'de marjinal gösterdi → opt-in.
    pub let_winners_run: bool,
    /// AUTO strateji seçimi: true ise rejim→tek-strateji lookup yerine adayları
    /// KENDİ resolve'lu paramlarıyla mini-backtest skoruna göre seçer (param_spec
    /// optimizasyonu seçime de girer). Volatile rejimde yine IDLE savunması. Env
    /// `STRATEGY_SELECT_EVAL`, default false (canlı seçim davranışı değişmez; opt-in
    /// + A/B). Bkz [[project_param_modularity]].
    pub strategy_select_eval: bool,
    /// StrategySignal kapanışı için min tutma süresi (sn). SL/TP/Trailing etkilenmez.
    pub min_holding_secs_strategy: i64,
    /// Açılışta entry↔candle sapma tavanı (%). 0 → price sanity guard kapalı.
    pub max_entry_price_dev_pct: f64,
    /// Candle "taze" eşiği (sn). Bundan eski candle.close fiyat referansı sayılmaz.
    pub candle_freshness_secs: i64,
    /// DataIngest empty/error log throttle penceresi (sn, sembol başına).
    pub log_dataingest_cooldown_secs: u64,
    /// Position-aligned RISK_BLOCK log throttle penceresi (sn, sembol başına).
    pub risk_block_log_cooldown_secs: u64,
    /// 🌐 Rejim bağlamı (Adım 1) cache TTL'i (sn). Cycle hot-path bu süre içinde
    /// rejimi yeniden hesaplamaz, RegimeContext cache'inden okur (seyrek tespit).
    /// 0 → her cycle yeniden hesapla (legacy per-cycle davranış). Default 900 (15 dk).
    pub regime_context_ttl_secs: u64,
}

impl Default for RuntimeTuning {
    fn default() -> Self {
        Self {
            force_live_exchanges: Vec::new(),
            commission_rate: 0.001,
            let_winners_run: false,
            strategy_select_eval: false,
            min_holding_secs_strategy: 30,
            max_entry_price_dev_pct: 5.0,
            candle_freshness_secs: 300,
            log_dataingest_cooldown_secs: 300,
            risk_block_log_cooldown_secs: 60,
            regime_context_ttl_secs: 900,
        }
    }
}

impl RuntimeTuning {
    /// Env'den okur; eksik/geçersiz değerlerde `Default` kullanılır. AppState::new'da
    /// bir kez çağrılır. Eski inline env okumalarıyla birebir aynı default'lar.
    pub fn from_env() -> Self {
        let d = Self::default();
        let commission_rate = {
            let v = env_parse("COMMISSION_RATE", d.commission_rate);
            if v.is_finite() && v >= 0.0 { v } else { d.commission_rate }
        };
        Self {
            force_live_exchanges: Self::parse_force_live_exchanges(),
            commission_rate,
            let_winners_run: env_truthy("LET_WINNERS_RUN"),
            strategy_select_eval: env_truthy("STRATEGY_SELECT_EVAL"),
            min_holding_secs_strategy: env_parse("MIN_HOLDING_SECS_STRATEGY", d.min_holding_secs_strategy),
            max_entry_price_dev_pct: env_parse("MAX_ENTRY_PRICE_DEVIATION_PCT", d.max_entry_price_dev_pct),
            candle_freshness_secs: env_parse("CANDLE_FRESHNESS_SECS", d.candle_freshness_secs),
            log_dataingest_cooldown_secs: env_parse("LOG_DATAINGEST_COOLDOWN_SECS", d.log_dataingest_cooldown_secs),
            risk_block_log_cooldown_secs: env_parse("RISK_BLOCK_LOG_COOLDOWN_SECS", d.risk_block_log_cooldown_secs),
            regime_context_ttl_secs: env_parse("REGIME_CONTEXT_TTL_SECS", d.regime_context_ttl_secs),
        }
    }

    /// FORCE_LIVE_EXCHANGES env'ini (virgül/boşluk ayraçlı) parse eder; bilinmeyen
    /// token'lar yok sayılır. Geriye uyum: ALLOW_BIST=1 → listeye Bist eklenir.
    fn parse_force_live_exchanges() -> Vec<crate::core::types::Exchange> {
        use crate::core::types::Exchange;
        let mut out: Vec<Exchange> = std::env::var("FORCE_LIVE_EXCHANGES")
            .unwrap_or_default()
            .split(|c| c == ',' || c == ' ' || c == ';')
            .filter_map(Exchange::from_token)
            .collect();
        if env_truthy("ALLOW_BIST") && !out.contains(&Exchange::Bist) {
            out.push(Exchange::Bist); // legacy alias
        }
        out
    }

    /// Sembol canlı cycle'a (DataIngest + trade) uygun mu? Borsasının canlı feed'i
    /// varsa ya da operatör force ettiyse true. **Tüm market-bazlı dışlama tek nokta** —
    /// motorun başka hiçbir yerinde borsa-adı sabiti olmamalı.
    pub fn symbol_eligible_for_live(&self, symbol: &str) -> bool {
        let ex = crate::core::types::Exchange::classify(symbol);
        ex.has_live_feed() || self.force_live_exchanges.contains(&ex)
    }
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
    candle_is_fresh_within(candle_ts, env_parse("CANDLE_FRESHNESS_SECS", 300))
}

/// Saf freshness kontrolü: 0 <= (now - ts) <= max_age_secs. `candle_is_fresh` bunun
/// env-default sarmalayıcısı (testler default'la çağırıyor); hot-path RuntimeTuning'den
/// gelen cached eşikle bu saf sürümü çağırır → per-call getenv yok.
pub fn candle_is_fresh_within(candle_ts: &chrono::DateTime<chrono::Utc>, max_age_secs: i64) -> bool {
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
