// src/robot/robotic_loop.rs - Srivastava ATP Reaktif Mimari Çekirdeği
// 4 Büyük Krallık (Bakanlık) Düzeni
use crate::core::model::{ClosedTradeModel, PositionModel, RoboticLoopConfig};
use crate::prelude::*;
use crate::robot::data_pipeline::PipelineStatus;
use crate::robot::risk::RiskGate;
use crate::robot::sr_detector::SrZone;
use crate::robot::state::symbol_orchestrator::SymbolOrchestrator;
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, RwLock,
};
use tokio::time::Instant;

/// Tüm venue'ler için ortak canlı emir anahtarı.
///
/// Venue capability'si bu iki aşamalı operatör onayının yerine geçmez.
pub fn live_execution_is_armed() -> bool {
    let armed = std::env::var("LIVE_EXECUTION_ARMED")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    let go = std::env::var("LIVE_GO_NO_GO")
        .map(|value| value.trim().eq_ignore_ascii_case("go"))
        .unwrap_or(false);
    armed && go
}

/// 💸 Yürütme maliyetleri (komisyon + slipaj) izlemesi.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutionCosts {
    pub total_cost_usd: f64,
    pub commission_usd: f64,
    pub slippage_usd: f64,
    pub trade_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct VenueRuntimeState {
    pub account_snapshot: Option<memos_venue_api::AccountSnapshot>,
    pub reconciliation_status: String,
    pub last_error: Option<String>,
    pub last_sync_epoch: i64,
    pub scheduler_phase: String,
    pub scheduler_cycle_started_epoch: i64,
    pub scheduler_cycle_completed_epoch: i64,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeLockDiagnostic {
    pub phase: String,
    pub started_epoch: i64,
    pub owner: String,
}

/// 🧬 Srivastava ATP - Otonom Eşik Muhafazası
#[derive(Debug, Clone, Copy, Default)]
pub struct AdaptiveThresholds {
    pub drift_baseline: f64,
    pub volatility_regime: f64,
}

// =============================================================================
// 1. BAKANLIKLAR (ALT BİRİMLER)
// =============================================================================

/// 🏦 FİNANS BAKANLIĞI: Cüzdan, Bakiyeler ve Açık Pozisyonlar
#[derive(Debug, Clone, Copy)]
pub struct GradedPosState {
    pub tranches_filled: u8,
    pub target_capital: f64,
}

pub struct FinanceVault {
    pub equity: f64,
    pub starting_capital: f64,
    pub peak_equity: f64,
    pub live_positions: Arc<RwLock<HashMap<String, PositionModel>>>,
    /// Venue + symbol anahtarlı canonical pozisyon kitabı.
    pub canonical_positions:
        Arc<RwLock<HashMap<memos_venue_api::CanonicalPositionKey, PositionModel>>>,
    pub live_closed_trades: Arc<RwLock<Vec<ClosedTradeModel>>>,
    pub live_execution_costs: Arc<RwLock<ExecutionCosts>>,
    pub equity_history: Arc<RwLock<VecDeque<f64>>>,
    pub live_orders: Arc<RwLock<HashMap<String, crate::core::model::LiveOrderRefs>>>,
    pub uncertain_orders: Arc<RwLock<HashMap<String, crate::core::model::UncertainOrderState>>>,
    pub uncertain_reconcile_streak: Arc<RwLock<HashMap<String, u8>>>,
    pub open_order_symbols: Arc<RwLock<std::collections::HashSet<String>>>,
    pub open_orders_reconcile_healthy: Arc<std::sync::atomic::AtomicBool>,
    /// Broker emri kabul edildi ancak pozisyonun kaybolduğu henüz doğrulanmadı.
    pub pending_closes: Arc<RwLock<HashMap<memos_venue_api::CanonicalPositionKey, String>>>,
    pub closed_trades_total: Arc<AtomicUsize>,
    pub last_close_at: Arc<RwLock<HashMap<String, std::time::Instant>>>,
    pub last_signal_bar: Arc<RwLock<HashMap<String, chrono::DateTime<chrono::Utc>>>>,
    pub xs_circuit_breaker_until: Arc<RwLock<Option<std::time::Instant>>>,
    pub xs_last_rebalance_bar: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    pub carry_circuit_breaker_until: Arc<RwLock<Option<std::time::Instant>>>,
    pub carry_last_rebalance_bar: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    pub blend_circuit_breaker_until: Arc<RwLock<Option<std::time::Instant>>>,
    pub blend_last_rebalance_bar: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    pub graded_tranches: Arc<RwLock<HashMap<String, GradedPosState>>>,
    pub pending_open_long: Arc<std::sync::atomic::AtomicU32>,
    pub pending_open_short: Arc<std::sync::atomic::AtomicU32>,
}
fn claim_bar_in(
    map: &RwLock<HashMap<String, chrono::DateTime<chrono::Utc>>>,
    symbol: &str,
    bar_ts: chrono::DateTime<chrono::Utc>,
) -> bool {
    match map.write() {
        Ok(m) if m.get(symbol).copied() == Some(bar_ts) => return false,
        Ok(mut m) => {
            m.insert(symbol.to_string(), bar_ts);
            return true;
        }
        Err(_) => return true,
    }
}

impl FinanceVault {
    pub fn claim_signal_bar(&self, symbol: &str, bar_ts: chrono::DateTime<chrono::Utc>) -> bool {
        return claim_bar_in(&self.last_signal_bar, symbol, bar_ts);
    }

    pub fn calculate_risk_appetite(&self) -> f64 {
        if self.starting_capital <= 0.0 {
            return 1.0;
        }
        return (self.equity / self.starting_capital).clamp(0.5, 2.0);
    }
}

/// 🧠 İSTİHBARAT BAŞKANLIĞI: ML Tahminleri, Sinyaller ve HyperOpt Parametreleri
pub struct BrainBox {
    pub ml_signal: String,
    pub ml_confidence: f64,
    pub hyperopt_score: f64,
    pub best_params: HashMap<String, f64>,
    pub live_strategy: Arc<RwLock<String>>,
    pub thresholds: AdaptiveThresholds,
    pub drift_history: std::collections::VecDeque<f64>,
    pub intelligence_hub: Arc<RwLock<crate::robot::ml_engine::intelligence_hub::IntelligenceHub>>,
    pub parameters: Arc<RwLock<crate::robot::parameters::ParameterStore>>,
    pub scalp_swing_config: Arc<RwLock<crate::robot::scalp_swing::ScalpSwingConfig>>,
    pub scalp_swing_stats: Arc<RwLock<crate::robot::scalp_swing::ScalpSwingStatsTable>>,
    pub regime_context: Arc<RwLock<crate::robot::logic::regime_context::RegimeCache>>,
}

impl BrainBox {
    pub fn should_retrain(&self, current_drift: f64) -> bool {
        let history = &self.drift_history;
        let n = history.len();

        if n < 10 {
            return false;
        }

        let sum: f64 = history.iter().sum();
        let mean = sum / n as f64;

        let variance_sum: f64 = history.iter().map(|&d| (d - mean).powi(2)).sum();
        let std_dev = (variance_sum / n as f64).sqrt();

        let dynamic_threshold = mean + (2.0 * std_dev);

        return current_drift > dynamic_threshold;
    }
}
/// ⚔️ HAREKÂT VE FİLO KOMUTANLIĞI: İş Akışı, Semboller ve Tetikleyiciler
pub struct FleetCommand {
    pub phase: String,
    pub download_active: bool,
    pub download_progress:
        std::sync::Arc<std::sync::RwLock<crate::core::model::DownloadProgressModel>>,
    pub triggers: std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub autonomy_counters: std::collections::BTreeMap<String, u64>,
    pub live_price: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, f64>>>,
    pub live_price_updated_at: std::sync::Arc<
        std::sync::RwLock<std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>>,
    >,
    pub live_sr_zones:
        std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, Vec<SrZone>>>>,
    pub live_ref_price_24h:
        std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, f64>>>,
    pub opportunity_cache: Arc<crate::robot::data_pipeline::CandleCache>,
    pub opportunity_tick5m: Arc<RwLock<HashMap<String, Tick5mAccumulator>>>,
    pub opportunity_tick15m: Arc<RwLock<HashMap<String, Tick15mAccumulator>>>,
    pub symbol_orchestrator: std::sync::Arc<std::sync::RwLock<SymbolOrchestrator>>,
    pub last_loop_tick: Arc<AtomicU64>,
    pub last_execution_epoch: Arc<AtomicU64>,
    pub latest_decisions: Arc<RwLock<std::collections::HashMap<String, DecisionAudit>>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionAudit {
    pub timestamp: String,
    pub symbol: String,
    pub strategy: String,
    pub signal: String,
    pub regime: String,
    pub edge: f64,
    pub edge_threshold: f64,
    pub ml_confidence: f64,
    pub stage: String,
    pub outcome: String,
    pub reason: String,
    pub bar_timestamp: Option<String>,
    #[serde(default)]
    pub signal_interval: String,
    #[serde(default)]
    pub regime_interval: String,
    #[serde(default)]
    pub setup_interval: String,
    #[serde(default)]
    pub trigger_interval: String,
    #[serde(default)]
    pub decision: String,
    #[serde(default)]
    pub reason_code: String,
    #[serde(default)]
    pub data_quality: String,
    #[serde(default)]
    pub risk_status: String,
    #[serde(default)]
    pub execution_status: String,
    #[serde(default)]
    pub shadow_signal: String,
    #[serde(default)]
    pub shadow_status: String,
    #[serde(default)]
    pub shadow_reason: String,
}

impl FleetCommand {
    pub fn calculate_current_volatility(&self) -> f64 {
        let guard = match self.live_price.read() {
            Ok(g) => g,
            Err(_) => return 0.0,
        };
        let prices: Vec<f64> = guard.values().copied().filter(|p| *p > 0.0).collect();
        let n = prices.len();
        if n < 2 {
            return 0.0;
        }
        let mean = prices.iter().sum::<f64>() / n as f64;
        if mean == 0.0 {
            return 0.0;
        }
        let variance = prices.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / n as f64;
        return (variance.sqrt() / mean).min(1.0);
    }
}

/// 🛡️ ADLİ VE SAĞLIK MUHAFIZLIĞI: Risk Kontrolü ve Hata Kayıtları
pub struct GuardianShield {
    pub log: VecDeque<String>,
    pub diag_alerts: VecDeque<String>,
    pub risk_gate: RiskGate,
    pub anomaly_count: u32,
    pub db_conn: Option<Connection>,
    pub live_pipeline: Arc<RwLock<PipelineStatus>>,
    pub repair_log: VecDeque<String>,
    pub live_block_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Risk kapısı HALT mührü. `Some(until)` ve `now < until` → YENİ pozisyon açılmaz
    /// (paper+canlı). Mevcut pozisyonlar normal yönetilmeye devam eder (SL/TP/trailing).
    /// `live_block_until`'den AYRI tutulur: o borsa sistemik reddi içindir ve başarılı
    /// emirde kendini temizler — drawdown halt'ı öyle temizlenmemeli.
    pub risk_halt_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Risk kapısı SAFE-MODE mührü. Aktifken tahsisat `safe_mode_alloc_mult` ile kısılır
    /// (işlem durmaz, risk azalır).
    pub risk_safe_mode_until: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone)]
pub struct TickBarAccumulator {
    pub bucket_start_epoch: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

pub type Tick5mAccumulator = TickBarAccumulator;
pub type Tick15mAccumulator = TickBarAccumulator;

impl GuardianShield {
    pub fn dynamic_safety_barrier(&self) -> f64 {
        let extra = (self.anomaly_count as f64 * 0.05).min(0.4);
        return 0.5 + extra;
    }
}
// =============================================================================
// 2. ANA MERKEZ (APP STATE)
// =============================================================================

/// Srivastava ATP - Otonom Sistem Merkezi (Orkestratör)
pub struct AppState {
    pub config: RoboticLoopConfig,

    // Bakanlıklar
    pub finance: FinanceVault,
    pub brain: BrainBox,
    pub fleet: FleetCommand,
    pub guardian: GuardianShield,

    /// 💱 LIVE Mode Köprüsü (config.trading_mode == Live + API key varsa)
    /// None ise tüm pozisyon açış/kapanış paper-mode'da kalır.
    pub live_executor: Option<Arc<crate::robot::engines::binance_executor::BinanceFuturesExecutor>>,
    /// Venue-agnostic read-only lifecycle (balance/account reconciliation).
    pub live_venue_lifecycle: Option<Arc<crate::robot::venue::VenueLifecycleService>>,
    /// LIVE_DRY_RUN=true ise executor olsa bile gerçek emir gönderilmez,
    /// sadece "şu emir gönderilecekti" log'u atılır. Paper akışı çalışır.
    pub live_dry_run: bool,
    /// Tek emir için sert notional tavan (USD). Env LIVE_MAX_NOTIONAL_USD,
    /// default $100. RiskGate'in üstünde bir bariyer.
    pub live_max_notional_usd: f64,

    /// 📣 Telegram push kanalı (kritik olaylar için). TELEGRAM_BOT_TOKEN +
    /// TELEGRAM_CHAT_ID set değilse None — kod sessizce çalışmaya devam eder.
    pub notifier: Option<Arc<crate::robot::infra::telegram_notifier::TelegramNotifier>>,

    /// 📝 Periyodik dosya logu (logs/robotic_trading.log + logs/trades.jsonl).
    /// `TRADING_LOGGER_DISABLE=1` set ise None; aksi halde her zaman aktif.
    /// SIGNAL/TRADE_OPEN/TRADE_CLOSE/RISK_BLOCK/ERROR olayları için kullanılır.
    pub trading_logger: Option<Arc<crate::robot::infra::logger::TradingLogger>>,

    /// 🔁 Kanonik iç event akışı (broadcast + replay JSONL).
    /// `EVENT_BUS_DISABLE=1` ise None; aksi halde event'ler logs/events_replay.jsonl
    /// dosyasına append edilir ve canlı subscriber'lara yayınlanır.
    pub event_bus: Option<Arc<crate::robot::infra::event_bus::EventBus>>,

    /// 🩺 MT5 bridge canlılık bayrağı. Otonom loop buna göre degrade mode'a geçer;
    /// UI bunu günlük/alert için okur ama karar verme merkezi core loop'ta kalır.
    pub mt5_bridge_healthy: Arc<AtomicBool>,
    pub mt5_bridge_last_error: Arc<RwLock<Option<String>>>,
    /// IBKR bağlantısı ve emir/veri için gerekli farm'ların birleşik sağlığı.
    pub ibkr_farm_healthy: Arc<AtomicBool>,
    pub ibkr_farm_last_error: Arc<RwLock<Option<String>>>,
    pub ibkr_health_bridge: Option<Arc<memos_market_ibkr::IbkrBridge>>,

    // Global Durdurma Sinyalleri
    pub app_stop_signal: Arc<AtomicBool>,
    pub pause_signal: Arc<AtomicBool>,

    /// ⚙️ Boot'ta env'den bir kez okunan runtime ayarları (komisyon oranı,
    /// freshness/sapma eşikleri, log cooldown'ları, BIST toggle). Cycle hot-path'i
    /// per-tick getenv yapmak yerine bu Arc'tan okur. Bkz. master::RuntimeTuning.
    pub tuning: Arc<crate::robot::engines::master::RuntimeTuning>,

    /// 🏛️ Venue kayıt defteri (config.venues'tan bir kez kurulur). Sembol→venue yönlendirme
    /// (explicit `@borsa` etiketi + şekil-tabanlı default) tek-kaynak. price_poll/download/
    /// execution call-site'ları bu paylaşılan registry'yi kullanır. [[project_venue_multimarket]]
    pub venue_registry: Arc<crate::robot::venue::VenueRegistry>,
    /// Venue-local gözlem durumu; scheduler güncellemeleri için global AppState'ten ayrıdır.
    pub venue_runtime: Arc<RwLock<BTreeMap<String, VenueRuntimeState>>>,
    pub runtime_lock_diagnostic: Arc<RwLock<RuntimeLockDiagnostic>>,
}

impl AppState {
    pub fn set_runtime_lock_diagnostic(&self, owner: &str, phase: &str) {
        if let Ok(mut diagnostic) = self.runtime_lock_diagnostic.write() {
            diagnostic.owner = owner.to_string();
            diagnostic.phase = phase.to_string();
            diagnostic.started_epoch = crate::core::time::now_epoch_secs() as i64;
        }
    }

    pub fn insert_live_position(&self, legacy_key: String, position: PositionModel) {
        if let Ok(mut positions) = self.finance.live_positions.write() {
            positions.insert(legacy_key, position.clone());
        }
        if let Some(key) = position.canonical_key() {
            if let Ok(mut canonical) = self.finance.canonical_positions.write() {
                canonical.insert(key, position);
            }
        }
    }

    pub fn remove_live_position(&self, legacy_key: &str) -> Option<PositionModel> {
        let removed = self
            .finance
            .live_positions
            .write()
            .ok()
            .and_then(|mut positions| positions.remove(legacy_key));
        if let Some(position) = removed.as_ref() {
            if let Some(key) = position.canonical_key() {
                if let Ok(mut canonical) = self.finance.canonical_positions.write() {
                    canonical.remove(&key);
                }
            }
        }
        removed
    }

    pub fn refresh_canonical_positions(&self) {
        let Ok(positions) = self.finance.live_positions.read() else {
            return;
        };
        let Ok(mut canonical) = self.finance.canonical_positions.write() else {
            return;
        };
        canonical.clear();
        for position in positions.values() {
            if let Some(key) = position.canonical_key() {
                canonical.insert(key, position.clone());
            }
        }
    }

    /// Yeni bir otonom organizma oluşturur (Srivastava ATP - F1 Mühürlü)
    pub fn new(mut config: RoboticLoopConfig) -> Self {
        let stop_sig = Arc::new(AtomicBool::new(false));

        let tuning = Arc::new(crate::robot::engines::master::RuntimeTuning::from_env());

        let live_positions = Arc::new(RwLock::new(HashMap::new()));

        let mut initial_history = VecDeque::with_capacity(120);
        initial_history.push_back(config.capital);
        let finance = FinanceVault {
            equity: config.capital,
            starting_capital: config.capital,
            peak_equity: config.capital,
            live_positions: Arc::clone(&live_positions),
            canonical_positions: Arc::new(RwLock::new(HashMap::new())),
            live_closed_trades: Arc::new(RwLock::new(Vec::new())),
            live_execution_costs: Arc::new(RwLock::new(ExecutionCosts::default())),
            equity_history: Arc::new(RwLock::new(initial_history)),
            live_orders: Arc::new(RwLock::new(HashMap::new())),
            uncertain_orders: Arc::new(RwLock::new(HashMap::new())),
            uncertain_reconcile_streak: Arc::new(RwLock::new(HashMap::new())),
            open_order_symbols: Arc::new(RwLock::new(std::collections::HashSet::new())),
            open_orders_reconcile_healthy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pending_closes: Arc::new(RwLock::new(HashMap::new())),
            closed_trades_total: Arc::new(AtomicUsize::new(0)),
            last_close_at: Arc::new(RwLock::new(HashMap::new())),
            last_signal_bar: Arc::new(RwLock::new(HashMap::new())),
            xs_circuit_breaker_until: Arc::new(RwLock::new(None)),
            xs_last_rebalance_bar: Arc::new(RwLock::new(None)),
            carry_circuit_breaker_until: Arc::new(RwLock::new(None)),
            carry_last_rebalance_bar: Arc::new(RwLock::new(None)),
            blend_circuit_breaker_until: Arc::new(RwLock::new(None)),
            blend_last_rebalance_bar: Arc::new(RwLock::new(None)),
            graded_tranches: Arc::new(RwLock::new(HashMap::new())),
            pending_open_long: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            pending_open_short: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        };

        let intelligence_hub = crate::robot::ml_engine::intelligence_hub::IntelligenceHub::new(
            crate::evolution::AutonomousController::new(
                crate::evolution::AutonomousControllerConfig::default(),
            ),
        );
        let mut scalp_swing_cfg = crate::robot::scalp_swing::ScalpSwingConfig::default();
        if let Ok(v) = std::env::var("SCALP_SWING_ENABLE") {
            let off = matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            );
            if off {
                scalp_swing_cfg.scalp_enabled = false;
                scalp_swing_cfg.swing_enabled = false;
            }
        }
        let brain = BrainBox {
            ml_signal: "HOLD".to_string(),
            ml_confidence: 0.0,
            hyperopt_score: 0.0,
            best_params: HashMap::new(),
            live_strategy: Arc::new(RwLock::new(
                std::env::var("LIVE_STRATEGY")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "AUTO".to_string()),
            )),
            thresholds: AdaptiveThresholds {
                drift_baseline: 0.15,
                volatility_regime: 1.0,
            },
            drift_history: VecDeque::with_capacity(100),
            intelligence_hub: Arc::new(RwLock::new(intelligence_hub)),
            parameters: Arc::new(RwLock::new(
                crate::robot::parameters::ParameterStore::from_env(),
            )),
            scalp_swing_config: Arc::new(RwLock::new(scalp_swing_cfg)),
            scalp_swing_stats: Arc::new(RwLock::new(
                crate::robot::scalp_swing::ScalpSwingStatsTable::default(),
            )),
            regime_context: Arc::new(RwLock::new(
                crate::robot::logic::regime_context::RegimeCache::new(),
            )),
        };

        if let Ok(ps) = brain.parameters.read() {
            for sym in ps.symbol_strategy.keys() {
                if crate::robot::config_engine::OtonomConfigEngine::symbol_allowed(sym)
                    && !config.pinned_symbols.iter().any(|p| p == sym)
                {
                    config.pinned_symbols.push(sym.clone());
                }
            }
        }

        let mut symbol_orchestrator = SymbolOrchestrator::new(16, Arc::clone(&live_positions));

        for sym in &config.pinned_symbols {
            println!("Registering symbol: {}", sym);
            symbol_orchestrator.register(sym, &config.market, &config.interval);
        }

        let elite_harvest_enabled = !matches!(
            std::env::var("ELITE_SYMBOL_HARVEST_ENABLED")
                .ok()
                .as_deref(),
            Some("0") | Some("false") | Some("off")
        );
        if elite_harvest_enabled {
            if let Ok(elite_symbols) = crate::persistence::reader::list_symbols(&config.db_path) {
                for sym in elite_symbols {
                    if !crate::robot::config_engine::OtonomConfigEngine::symbol_allowed(&sym) {
                        continue;
                    }
                    if !tuning.symbol_eligible_for_live(&sym) {
                        continue;
                    }
                    if !config.pinned_symbols.contains(&sym) {
                        symbol_orchestrator.register(&sym, &config.market, &config.interval);
                    }
                }
            }
        }

        let fleet = FleetCommand {
            phase: "Idle".to_string(),
            download_active: false,
            download_progress: Arc::new(RwLock::new(Default::default())),
            triggers: Self::init_default_triggers(),
            autonomy_counters: std::collections::BTreeMap::new(),
            live_price: Arc::new(RwLock::new(HashMap::new())),
            live_price_updated_at: Arc::new(RwLock::new(HashMap::new())),
            live_sr_zones: Arc::new(RwLock::new(HashMap::new())),
            live_ref_price_24h: Arc::new(RwLock::new(HashMap::new())),
            opportunity_cache: Arc::new(crate::robot::data_pipeline::CandleCache::new(2000)),
            opportunity_tick5m: Arc::new(RwLock::new(HashMap::new())),
            opportunity_tick15m: Arc::new(RwLock::new(HashMap::new())),
            symbol_orchestrator: Arc::new(RwLock::new(symbol_orchestrator)),
            last_loop_tick: Arc::new(AtomicU64::new(0)),
            last_execution_epoch: Arc::new(AtomicU64::new(0)),
            latest_decisions: Arc::new(RwLock::new(HashMap::new())),
        };

        let guardian = GuardianShield {
            log: VecDeque::with_capacity(300),
            diag_alerts: VecDeque::with_capacity(20),
            risk_gate: RiskGate::default(),
            anomaly_count: 0,
            db_conn: crate::persistence::open_db(&config.db_path).ok(),
            live_pipeline: Arc::new(RwLock::new(PipelineStatus::new())),
            repair_log: VecDeque::with_capacity(100),
            live_block_until: None,
            risk_halt_until: None,
            risk_safe_mode_until: None,
        };
        log::info!(target:"STATE_INIT",
            "F1: Otonom filo, pinned ve SQLite geçmiş sembolleriyle donatılarak ayağa kaldırıldı. Seviye INFO"
        );

        let live_execution_armed = std::env::var("LIVE_EXECUTION_ARMED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
        let live_go_no_go = std::env::var("LIVE_GO_NO_GO")
            .map(|v| v.eq_ignore_ascii_case("go"))
            .unwrap_or(false);
        let live_dry_run = std::env::var("LIVE_DRY_RUN")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let live_executor = if matches!(config.trading_mode, crate::core::model::TradingMode::Live)
            && (live_dry_run || (live_execution_armed && live_go_no_go))
        {
            match (config.get_api_key(), config.get_secret_key()) {
                (Some(k), Some(s)) if !k.is_empty() && !s.is_empty() => {
                    let use_testnet = std::env::var("BINANCE_TESTNET")
                        .map(|v| v == "true" || v == "1")
                        .unwrap_or(false);
                    log::info!(target:"STATE_INIT",
                        "💱 Live mode aktif: BinanceFuturesExecutor kuruldu (market={}, {})",
                        config.market, if use_testnet { "TESTNET" } else { "CANLI-BORSA" });
                    Some(Arc::new(
                        crate::robot::engines::binance_executor::BinanceFuturesExecutor::new_for_market(
                            k, s, use_testnet, &config.market,
                        )
                    ))
                }
                _ => {
                    log::warn!(target:"STATE_INIT",
                        "⚠️ TradingMode::Live seçildi ama API key/secret yok → Paper-fallback");
                    None
                }
            }
        } else {
            if matches!(config.trading_mode, crate::core::model::TradingMode::Live)
                && (!live_execution_armed || !live_go_no_go)
            {
                log::warn!(
                    target:"STATE_INIT",
                    "⚠️ Canlı executor kilitli: LIVE_EXECUTION_ARMED=true ve LIVE_GO_NO_GO=GO zorunlu → Paper-fallback"
                );
            }
            None
        };

        let venue_registry = Arc::new(crate::robot::venue::VenueRegistry::from_specs(
            &config.venues,
            live_executor.clone(),
        ));
        log::info!(target:"STATE_INIT", "🏛️ Venue registry: {} venue ({})",
            venue_registry.len(),
            config.venues.iter().map(|v| v.token()).collect::<Vec<_>>().join(","));

        let live_max_notional_usd = std::env::var("LIVE_MAX_NOTIONAL_USD")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(100.0)
            .max(0.0);

        let live_exchange = crate::core::types::Exchange::from_token(&config.exchange)
            .unwrap_or(crate::core::types::Exchange::Binance);
        let active_spec = crate::core::types::VenueSpec::with_profile(
            live_exchange,
            crate::core::types::Market::from_label(&config.market),
            config.venue_profile,
        );
        let live_venue_lifecycle = venue_registry
            .get_spec(active_spec)
            .or_else(|| venue_registry.get(live_exchange))
            .map(|venue| {
                let binance_user_data = if live_exchange == crate::core::types::Exchange::Binance {
                    live_executor.as_ref().map(Arc::clone)
                } else {
                    None
                };
                Arc::new(crate::robot::venue::VenueLifecycleService::new(
                    venue.clone(),
                    binance_user_data,
                ))
            });

        let notifier =
            crate::robot::infra::telegram_notifier::TelegramNotifier::from_env().map(Arc::new);
        if notifier.is_some() {
            log::info!(target:"STATE_INIT", "📣 Telegram notifier aktif (TELEGRAM_BOT_TOKEN/CHAT_ID set)");
        } else {
            log::info!(target:"STATE_INIT", "📣 Telegram notifier devre dışı (env yok)");
        }

        let mt5_bridge_healthy = Arc::new(AtomicBool::new(false));
        let mt5_bridge_last_error = Arc::new(RwLock::new(None));
        let ibkr_farm_healthy = Arc::new(AtomicBool::new(false));
        let ibkr_farm_last_error = Arc::new(RwLock::new(None));
        let ibkr_health_bridge = config
            .venues
            .iter()
            .any(|spec| spec.exchange == crate::core::types::Exchange::Ibkr)
            .then(|| {
                Arc::new(memos_market_ibkr::IbkrBridge::with_defaults(
                    std::env::var("IBKR_BRIDGE_ADDR").ok(),
                ))
            });

        let trading_logger_disabled = std::env::var("TRADING_LOGGER_DISABLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let trading_logger = if trading_logger_disabled {
            log::info!(target:"STATE_INIT", "📝 TradingLogger devre dışı (TRADING_LOGGER_DISABLE)");
            None
        } else {
            let log_path = std::env::var("TRADING_LOG_PATH")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "logs/robotic_trading.log".into());
            let trades_path = std::env::var("TRADES_LOG_PATH")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "logs/trades.jsonl".into());
            if let Some(parent) = std::path::Path::new(&log_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match crate::robot::infra::logger::TradingLogger::new(&log_path, &trades_path) {
                Ok(lg) => {
                    log::info!(target:"STATE_INIT",
                        "📝 TradingLogger aktif: {} + {}", log_path, trades_path);
                    Some(Arc::new(lg))
                }
                Err(e) => {
                    log::warn!(target:"STATE_INIT",
                        "📝 TradingLogger kurulamadı ({:?}) — dosya logu devre dışı", e);
                    None
                }
            }
        };

        let event_bus_disabled = std::env::var("EVENT_BUS_DISABLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let event_bus = if event_bus_disabled {
            log::info!(target:"STATE_INIT", "🔁 EventBus devre dışı (EVENT_BUS_DISABLE)");
            None
        } else {
            match crate::robot::infra::event_bus::EventBus::new_from_env() {
                Ok(bus) => {
                    let replay_path = std::env::var("EVENT_REPLAY_LOG_PATH")
                        .ok()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "logs/events_replay.jsonl".to_string());
                    log::info!(target:"STATE_INIT", "🔁 EventBus aktif: replay={}", replay_path);
                    Some(Arc::new(bus))
                }
                Err(e) => {
                    log::warn!(target:"STATE_INIT", "🔁 EventBus kurulamadı ({:?})", e);
                    None
                }
            }
        };

        let venue_runtime = Arc::new(RwLock::new(
            config
                .venues
                .iter()
                .map(|spec| {
                    (
                        spec.token(),
                        VenueRuntimeState {
                            scheduler_phase: "NotStarted".to_string(),
                            ..VenueRuntimeState::default()
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        ));

        return Self {
            config,
            finance,
            brain,
            fleet,
            guardian,
            live_executor,
            live_venue_lifecycle,
            live_dry_run,
            live_max_notional_usd,
            notifier,
            trading_logger,
            event_bus,
            mt5_bridge_healthy,
            mt5_bridge_last_error,
            ibkr_farm_healthy,
            ibkr_farm_last_error,
            ibkr_health_bridge,
            app_stop_signal: stop_sig,
            pause_signal: Arc::new(AtomicBool::new(false)),
            tuning,
            venue_registry,
            venue_runtime,
            runtime_lock_diagnostic: Arc::new(RwLock::new(RuntimeLockDiagnostic::default())),
        };
    }
    /// Hem `push_log`'a yazar hem (notifier varsa) Telegram'a yollar.
    /// `key`: throttle anahtarı (örn. "BALANCE-AUTOFIX"). Aynı key cooldown
    /// süresince tekrar yollanmaz; UI log'una her zaman düşer.
    pub fn push_alert(
        &mut self,
        key: &str,
        severity: crate::robot::infra::telegram_notifier::Severity,
        msg: String,
    ) {
        let formatted = crate::robot::infra::telegram_notifier::format_message(severity, &msg);
        self.push_log(formatted);
        if let Some(n) = self.notifier.as_ref() {
            n.notify(key, severity, &msg);
        }
    }

    /// Tüm otonom tetikleyicileri (m, b, d, screener) tek seferde mühürler
    fn init_default_triggers() -> HashMap<String, Arc<AtomicBool>> {
        let mut t = HashMap::new();
        t.insert("ml".to_string(), Arc::new(AtomicBool::new(false)));
        t.insert("backtest".to_string(), Arc::new(AtomicBool::new(false)));
        t.insert("download".to_string(), Arc::new(AtomicBool::new(false)));
        t.insert("screener".to_string(), Arc::new(AtomicBool::new(false)));
        return t;
    }

    /// Adli Log Kaydı: guardian üzerinden mühürlenir (TUI panel).
    pub fn push_log(&mut self, msg: String) {
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        self.guardian.log.push_back(format!("[{}] {}", ts, msg));
        if self.guardian.log.len() > 300 {
            self.guardian.log.pop_front();
        }
    }

    /// Hem TUI panel buffer'a hem stderr'a (log::info!) yansıt. Headless modda
    /// operatör panel olmadan da boot/recovery/şema mesajlarını görebilsin diye.
    pub fn push_log_mirror(&mut self, msg: String) {
        log::info!("{}", msg);
        self.push_log(msg);
    }

    /// Otonom Karar Verici: Bakanlıklar arası dengeyi gözeterek aksiyon alır.
    pub fn orchestrate_autonomy(&mut self) {
        let _regime_volatility = self.fleet.calculate_current_volatility();
        let risk_multiplier = self.finance.calculate_risk_appetite();

        if self.brain.ml_confidence * risk_multiplier > self.guardian.dynamic_safety_barrier() {
            if let Some(t) = self.fleet.triggers.get("execution") {
                t.store(true, Ordering::Relaxed);
            }
            self.push_log("🚀 Otonom Karar: Koşullar optimal, harekât başlatıldı.".into());
        }
    }
}
