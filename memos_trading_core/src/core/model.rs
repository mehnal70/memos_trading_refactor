// src/core/model.rs - Memos Trading Core Library (Srivastava ATP - Master Veri Kontratı)
// Bu modül saf veri yapısıdır; hiçbir kilit (lock) veya iş parçacığı (thread) bağımlılığı taşımaz.

use crate::core::math;
use crate::prelude::*;
use serde::{Deserialize, Serialize};
use std::{fmt, println};

// =============================================================================
// 1. YAPILANDIRMA VE BELLEK MODELLERİ (Eski config.rs'ten Tahliye Edilen Parçalar)
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TradingMode {
    Backtest,
    Paper,
    Live,
}

impl TradingMode {
    /// Env değerinden parse — case-insensitive. Bilinmeyen değer Paper'a düşer (güvenli default).
    pub fn from_env_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "live" => return TradingMode::Live,
            "backtest" => return TradingMode::Backtest,
            _ => return TradingMode::Paper,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            TradingMode::Live => return "Live",
            TradingMode::Paper => return "Paper",
            TradingMode::Backtest => return "Backtest",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizedParamsCache {
    pub ma_fast: usize,
    pub ma_slow: usize,
    pub rsi_period: usize,
    pub rsi_ob: f64,
    pub rsi_os: f64,
    pub bb_period: usize,
    pub bb_std_dev: f64,
    pub macd_fast: usize,
    pub macd_slow: usize,
    pub macd_signal: usize,
    pub stoch_k: usize,
    pub stoch_ob: f64,
    pub stoch_os: f64,
    pub ema_fast: usize,
    pub ema_slow: usize,
    pub donchian_period: usize,
    pub williams_period: usize,
    pub cci_period: usize,
    pub stoch_rsi_period: usize,
    pub supertrend_period: usize,
    pub supertrend_mult: f64,
    pub ict_fvg_lookback: usize,
    pub smc_swing_lb: usize,
    pub best_strategy: Option<String>,
    pub last_updated: Option<String>,
}

impl Default for OptimizedParamsCache {
    fn default() -> Self {
        return Self {
            ma_fast: 5,
            ma_slow: 20,
            rsi_period: 14,
            rsi_ob: 70.0,
            rsi_os: 30.0,
            bb_period: 20,
            bb_std_dev: 2.0,
            macd_fast: 12,
            macd_slow: 26,
            macd_signal: 9,
            stoch_k: 14,
            stoch_ob: 80.0,
            stoch_os: 20.0,
            ema_fast: 5,
            ema_slow: 20,
            donchian_period: 20,
            williams_period: 14,
            cci_period: 20,
            stoch_rsi_period: 14,
            supertrend_period: 10,
            supertrend_mult: 3.0,
            ict_fvg_lookback: 5,
            smc_swing_lb: 10,
            best_strategy: None,
            last_updated: None,
        };
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionFilterConfig {
    pub enabled: bool,
    pub allowed_hours: Vec<u8>,
    pub blocked_hours: Vec<u8>,
    pub long_preferred_hours: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoboticLoopConfig {
    pub exchange: String,
    pub market: String,
    pub venue_profile: crate::core::types::VenueProfile,
    pub venues: Vec<crate::core::types::VenueSpec>,
    pub symbol: String,
    pub interval: String,
    pub interval_secs: u64,
    pub capital: f64,
    pub db_path: String,
    pub state_db_path: String,
    pub trade_amount: f64,
    pub download_enabled: bool,
    pub download_every_mins: u64,
    pub download_candle_limit: usize,
    pub download_top_n: usize,
    pub auto_export_every_mins: u64,
    pub auto_export_keep: usize,
    pub trade_quality_config_path: String,
    pub adaptive_params_path: String,
    pub robotic_profiles_path: String,
    pub evolution_state_path: String,
    pub fsm_state_path: String,
    pub app_snapshot_path: String,
    pub leverage_base: f64,
    pub leverage_max: f64,
    pub pipeline_enabled: bool,
    pub pipeline_every_mins: u64,
    pub pipeline_p5_top_n: usize,
    pub auto_interval: bool,
    pub blocked_symbols: Vec<String>,
    pub pinned_symbols: Vec<String>,
    pub optimized_params: OptimizedParamsCache,
    pub session_filter: SessionFilterConfig,
    pub api_key: Option<String>,
    pub secret_key: Option<String>,
    pub trading_mode: TradingMode,
    pub request_delay_ms: u64,
    pub max_retries: u32,
    pub http_timeout_sec: u64,
}

impl Default for RoboticLoopConfig {
    fn default() -> Self {
        return Self {
            exchange: "binance".into(),
            market: "futures".into(),
            venue_profile: crate::core::types::VenueProfile::Default,
            venues: vec![
                crate::core::types::VenueSpec::new(
                    crate::core::types::Exchange::Binance,
                    crate::core::types::Market::Futures,
                ),
                crate::core::types::VenueSpec::new(
                    crate::core::types::Exchange::Mt5,
                    crate::core::types::Market::Spot,
                ),
            ],
            symbol: "BTCUSDT".into(),
            interval: "1m".into(),
            interval_secs: 60,
            capital: 10000.0,
            db_path: "data/trader.db".into(),
            state_db_path: "data/trader.db".into(),
            trade_amount: 0.01,
            download_enabled: true,
            download_every_mins: 15,
            download_candle_limit: 500,
            download_top_n: 3,
            auto_export_every_mins: 30,
            auto_export_keep: 24,
            trade_quality_config_path: "config/trade_quality.json".into(),
            adaptive_params_path: "config/adaptive_params.json".into(),
            robotic_profiles_path: "config/robotic_profiles.json".into(),
            evolution_state_path: "config/evolution_state.json".into(),
            fsm_state_path: "config/fsm_state.json".into(),
            app_snapshot_path: "config/app_snapshot.json".into(),
            leverage_base: 7.0,
            leverage_max: 10.0,
            pipeline_enabled: true,
            pipeline_every_mins: 120,
            pipeline_p5_top_n: 3,
            auto_interval: false,
            blocked_symbols: Vec::new(),
            pinned_symbols: vec!["BTCUSDT".into(), "ETHUSDT".into()],
            optimized_params: OptimizedParamsCache::default(),
            session_filter: SessionFilterConfig::default(),
            api_key: None,
            secret_key: None,
            trading_mode: TradingMode::Paper,
            request_delay_ms: 2000,
            max_retries: 5,
            http_timeout_sec: 30,
        };
    }
}
impl RoboticLoopConfig {
    pub fn new() -> Self {
        return Self::default();
    }

    fn parse_symbol_csv(raw: &str) -> Vec<String> {
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn fallback_pinned_symbols_from_env() -> Vec<String> {
        let mut out = Vec::new();
        for key in [
            "AUTO_BINANCE_SYMBOLS",
            "AUTO_MT5_SYMBOLS",
            "AUTO_IBKR_SYMBOLS",
            "AUTO_BIST_SYMBOLS",
        ] {
            if let Ok(raw) = std::env::var(key) {
                out.extend(Self::parse_symbol_csv(&raw));
            }
        }
        out
    }

    pub fn from_env() -> Self {
        let trading_mode = if let Ok(v) = std::env::var("TRADING_MODE") {
            TradingMode::from_env_str(&v)
        } else if std::env::var("BINANCE_PAPER_MODE")
            .map(|v| v == "false")
            .unwrap_or(false)
        {
            TradingMode::Live
        } else {
            TradingMode::Paper
        };
        let d = Self::default();
        let capital = crate::core::env::env_parse::<f64>("STARTING_CAPITAL")
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(d.capital);
        let venue_profile = std::env::var("TRADE_PROFILE")
            .ok()
            .and_then(|value| crate::core::types::VenueProfile::from_token(&value))
            .unwrap_or(d.venue_profile);

        return Self {
            symbol: crate::core::env::env_or("TRADE_SYMBOL", "BTCUSDT"),
            exchange: crate::core::env::env_or("TRADE_EXCHANGE", &d.exchange),
            market: crate::core::env::env_or("TRADE_MARKET", "spot"),
            venue_profile,
            interval: crate::core::env::env_or("TRADE_INTERVAL", "1m"),
            db_path: crate::core::env::env_or("DB_PATH", "data/trader.db"),
            state_db_path: std::env::var("STATE_DB_PATH")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| crate::core::env::env_or("DB_PATH", "data/trader.db")),
            download_enabled: crate::core::env::env_bool("DOWNLOAD_ENABLED")
                .unwrap_or(d.download_enabled),
            download_every_mins: crate::core::env::env_parse_or(
                "DOWNLOAD_EVERY_MINS",
                d.download_every_mins,
            )
            .max(1),
            trading_mode,
            capital,
            api_key: crate::core::env::env_opt("BINANCE_API_KEY"),
            secret_key: crate::core::env::env_opt("BINANCE_API_SECRET"),
            venues: {
                let raw = crate::core::env::env_or("VENUES", "");
                let parsed: Vec<crate::core::types::VenueSpec> = raw
                    .split(',')
                    .filter_map(crate::core::types::VenueSpec::parse_token)
                    .collect();
                if parsed.is_empty() {
                    // Başlangıçta MT5 pinli sembolleri de route edebilmek için varsayılan
                    // venue seti Binance + MT5 olarak korunur. VENUES override yoksa bu
                    // değer, sembolün explicit @mt5 etiketini registry üzerinden çözebilmelidir.
                    d.venues
                        .iter()
                        .map(|spec| {
                            crate::core::types::VenueSpec::with_profile(
                                spec.exchange,
                                spec.market,
                                venue_profile,
                            )
                        })
                        .collect()
                } else {
                    parsed
                }
            },
            pinned_symbols: {
                let raw = crate::core::env::env_or("PINNED_SYMBOLS", "");
                let parsed = Self::parse_symbol_csv(&raw);
                if parsed.is_empty() {
                    let fallback = Self::fallback_pinned_symbols_from_env();
                    if fallback.is_empty() {
                        d.pinned_symbols.clone()
                    } else {
                        fallback
                    }
                } else {
                    parsed
                }
            },
            ..d
        };
    }

    pub fn with_exchange(mut self, exchange: impl Into<String>) -> Self {
        self.exchange = exchange.into();
        return self;
    }

    pub fn with_trading_mode(mut self, mode: TradingMode) -> Self {
        self.trading_mode = mode;
        return self;
    }

    pub fn get_api_key(&self) -> Option<String> {
        return crate::core::security::secure_store::get_secret("api_key")
            .or_else(|| self.api_key.clone())
            .or_else(|| crate::core::env::env_opt("BINANCE_API_KEY"));
    }

    pub fn get_secret_key(&self) -> Option<String> {
        return crate::core::security::secure_store::get_secret("secret_key")
            .or_else(|| self.secret_key.clone())
            .or_else(|| crate::core::env::env_opt("BINANCE_API_SECRET"));
    }
}
// =============================================================================
// 2. ATOMİK VERİ SNAPSHOT MODELLERİ (TUI & Android İletişim Dilimleri)
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinanceSnapshot {
    pub total_equity: f64,
    pub realize_pnl: f64,
    pub open_pnl: f64,
    pub starting_capital: f64,
    pub total_fees: f64,
    #[serde(default)]
    pub balance: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub status: memos_venue_api::AccountSnapshotStatus,
    #[serde(default)]
    pub venue_accounts: std::collections::BTreeMap<String, memos_venue_api::AccountSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VenuePortfolioSnapshot {
    pub venue: String,
    pub account_keys: Vec<String>,
    pub balance: Option<f64>,
    pub equity: Option<f64>,
    pub open_pnl: f64,
    pub realized_pnl: f64,
    pub total_fees: f64,
    pub position_count: usize,
    pub closed_trade_count: usize,
}

impl FinanceSnapshot {
    pub fn net_pnl(&self) -> f64 {
        return self.realize_pnl + self.open_pnl - self.total_fees;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerModel {
    pub symbol: String,
    pub market: String,
    pub interval: String,
    pub price: f64,
    pub change_pct: f64,
    pub uptime_secs: u64,
    pub is_paused: bool,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    pub label: String,
    pub status: String,
    pub last_run_age_secs: u64,
    pub overdue_secs: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LiveOrderRefs {
    #[serde(default)]
    pub entry_order_id: Option<String>,
    #[serde(default)]
    pub sl_order_id: Option<String>,
    #[serde(default)]
    pub tp_order_id: Option<String>,
    #[serde(default)]
    pub placed_at: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub side: String,
    #[serde(default)]
    pub quantity: f64,
    #[serde(default)]
    pub filled_quantity: f64,
    #[serde(default)]
    pub remaining_quantity: f64,
    #[serde(default)]
    pub limit_price: Option<f64>,
    #[serde(default)]
    pub account: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UncertainOrderState {
    pub symbol: String,
    pub side: String,
    pub requested_quantity: f64,
    pub filled_quantity: f64,
    pub broker_order_id: Option<String>,
    pub status: String,
    pub reason: String,
    pub created_at: String,
    pub last_checked_at: String,
    pub resolved_at: Option<String>,
    #[serde(default)]
    pub healthy_reconcile_count: u32,
}

fn default_position_market() -> String {
    return "spot".to_string();
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionModel {
    #[serde(default)]
    pub pos_id: String,
    pub symbol: String,
    #[serde(default)]
    pub venue: Option<memos_venue_api::VenueKey>,
    pub entry_price: f64,
    pub current_price: f64,
    pub qty: f64,
    pub leverage: f64,
    #[serde(default = "default_position_market")]
    pub market: String,
    #[serde(default)]
    pub interval: String,
    pub is_long: bool,
    pub trade_type: String,
    pub opened_at: String,
    #[serde(default)]
    pub stop_loss: f64,
    #[serde(default)]
    pub take_profit: f64,
    #[serde(default)]
    pub trailing_stop: f64,
    #[serde(default)]
    pub max_favorable_price: f64,
    #[serde(default)]
    pub breakeven_activated: bool,
    #[serde(default)]
    pub kind: Option<crate::robot::scalp_swing::TradeType>,
    #[serde(default)]
    pub entry_commission: f64,
}

impl PositionModel {
    pub fn canonical_key(&self) -> Option<memos_venue_api::CanonicalPositionKey> {
        self.venue
            .map(|venue| memos_venue_api::CanonicalPositionKey::new(venue, self.symbol.clone()))
    }

    pub fn calculate_pnl(&self) -> f64 {
        return math::calculate_pnl(self.entry_price, self.current_price, self.qty, self.is_long);
    }

    pub fn roe(&self) -> f64 {
        return math::calculate_roe(
            self.entry_price,
            self.current_price,
            self.leverage,
            self.is_long,
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiBrainSnapshot {
    pub genome_id: String,
    pub fitness: f64,
    pub win_rate: f64,
    pub trade_count: usize,
    pub gbt_score: Option<f64>,
    pub exploration_rate: f64,
    pub drift_score: f64,
    pub mc_ruin_prob: f64,
    pub is_evolution_active: bool,
    pub next_evolution_secs: u64,
    #[serde(default)]
    pub live_strategy: String,
    #[serde(default)]
    pub controller_state: String,
    #[serde(default)]
    pub controller_cycle: u64,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub pending_trades: usize,
    #[serde(default)]
    pub drift_series: Vec<f64>,
    #[serde(default)]
    pub best_tp_pct: f64,
    #[serde(default)]
    pub best_sl_pct: f64,
    #[serde(default)]
    pub best_position_size: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketAnalysisModel {
    pub symbol: String,
    #[serde(default)]
    pub market: String,
    pub current_price: f64,
    pub change_24h: f64,
    pub zones: Vec<SrZoneModel>,
    pub nearest_support: Option<f64>,
    pub nearest_resistance: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VenueSymbolSnapshot {
    pub venue: String,
    pub symbol: String,
    pub market: String,
    pub status: String,
    pub tradeable: bool,
    pub current_price: f64,
    pub change_24h: f64,
    pub last_candle_at: Option<chrono::DateTime<chrono::Utc>>,
    pub candle_status: String,
    pub filters_status: String,
    pub source: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SrZoneModel {
    pub zone_type: String,
    pub price_low: f64,
    pub price_high: f64,
    pub strength: f64,
    pub touch_count: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub message: String,
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedTradeModel {
    pub symbol: String,
    pub is_long: bool,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub exit_reason: String,
    pub closed_at: String,
    #[serde(default)]
    pub opened_at: String,
    #[serde(default = "default_leverage_one")]
    pub leverage: f64,
    #[serde(default)]
    pub net_pnl: f64,
    #[serde(default)]
    pub net_pnl_pct: f64,
    #[serde(default)]
    pub commission: f64,
    #[serde(default)]
    pub entry_price: f64,
    #[serde(default)]
    pub exit_price: f64,
    #[serde(default)]
    pub strategy: String,
    #[serde(default)]
    pub venue: String,
    #[serde(default)]
    pub pyramid_tranches: u8,
    /// Borsa tarafı işlem referansı (MT5 deal ticket / Binance orderId). Mutabakatın
    /// DEDUP anahtarı: boş değilse (venue, exchange_ref) çifti tekildir. Eski kayıtlar
    /// için serde default "" (geriye-uyum).
    #[serde(default)]
    pub exchange_ref: String,
}

impl ClosedTradeModel {
    /// Returns whether the record is a local trailing-stop settlement created for
    /// an adopted broker position before broker confirmation existed.
    pub fn is_unconfirmed_adopt_close(&self) -> bool {
        self.strategy == "MANUEL_ADOPT"
            && self.exchange_ref.starts_with("adopt_")
            && self.exit_reason == "TRAILING_STOP"
    }

    /// Returns the settled PnL used by all performance views.
    /// Legacy records may have a zero `net_pnl` while carrying a gross `pnl`.
    pub fn effective_net_pnl(&self) -> f64 {
        if self.net_pnl == 0.0 && self.pnl != 0.0 && self.commission == 0.0 {
            self.pnl
        } else {
            self.net_pnl
        }
    }

    /// Returns whether two records represent the same settled position.
    ///
    /// Broker references and local position IDs are authoritative. Legacy records
    /// without a reference use a conservative field-based fallback.
    pub fn same_identity(&self, other: &Self) -> bool {
        if !self.exchange_ref.is_empty() && !other.exchange_ref.is_empty() {
            return self.venue == other.venue && self.exchange_ref == other.exchange_ref;
        }
        self.venue == other.venue
            && self.symbol == other.symbol
            && self.opened_at == other.opened_at
            && (self.entry_price - other.entry_price).abs() < 1e-8
            && (self.exit_price - other.exit_price).abs() < 1e-8
            && self.exit_reason == other.exit_reason
    }

    /// Removes repeated settlement records while preserving their first arrival order.
    pub fn deduplicate(trades: &mut Vec<Self>) -> usize {
        if trades.len() < 2 {
            return 0;
        }
        let original_len = trades.len();
        let mut unique: Vec<Self> = Vec::with_capacity(original_len);
        for trade in trades.drain(..) {
            if !unique.iter().any(|existing| existing.same_identity(&trade)) {
                unique.push(trade);
            }
        }
        let removed = original_len - unique.len();
        *trades = unique;
        removed
    }
}

fn default_leverage_one() -> f64 {
    return 1.0;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeDistribution {
    pub symbol: String,
    pub pnl: f64,
    pub trade_count: u32,
    pub win_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartSnapshot {
    pub distributions: Vec<TradeDistribution>,
    pub total_closed_pnl: f64,
    pub total_trade_count: usize,
    #[serde(default)]
    pub equity_series: Vec<f64>,
    #[serde(default)]
    pub current_drawdown_pct: f64,
    #[serde(default)]
    pub peak_equity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyModel {
    pub severity: String,
    pub kind: String,
    pub message: String,
    pub fix_hint: String,
    pub auto_fixed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeTypeStats {
    pub label: String,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub current_streak: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunitySnapshotModel {
    pub symbol: String,
    pub interval: String,
    pub bars: usize,
    pub span_minutes: usize,
    pub opportunity_score: f64,
    pub phase: String,
    pub rsi_14: f64,
    pub momentum_pct: f64,
    pub volatility_pct: f64,
    pub trend_bias: f64,
    pub scalp_bias: f64,
    pub swing_bias: f64,
    pub last_ts: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataCoverageModel {
    pub expected_symbols: usize,
    pub evaluated_symbols: usize,
    pub fresh_symbols: usize,
    pub stale_symbols: usize,
    pub missing_symbols: usize,
    pub max_age_secs: Option<i64>,
    pub freshness_budget_secs: i64,
    pub status: String,
    pub priority: String,
    pub stale_symbol_names: Vec<String>,
    pub missing_symbol_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DownloadProgressModel {
    pub active: bool,
    pub total_symbols: usize,
    pub completed_symbols: usize,
    pub current_symbol: String,
    pub current_interval: String,
    pub current_phase: String,
    pub last_completed_symbol: String,
    pub fetched_candles: usize,
    pub failed_symbols: usize,
    #[serde(default)]
    pub failed_symbol_names: Vec<String>,
    pub last_error: String,
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BistDataStatusModel {
    pub data_source: String,
    pub source_sync_status: String,
    pub latest_candle: Option<String>,
    pub latest_age_secs: Option<i64>,
    pub expected_symbols: usize,
    pub fresh_symbols: usize,
    pub stale_symbols: usize,
    pub rejected_rows: usize,
    pub market_state: String,
    pub daily_candle_expectation: String,
    pub soak_health: String,
    pub readiness_status: String,
    pub execution_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionTraceModel {
    pub timestamp: String,
    pub symbol: String,
    pub outcome: String,
    pub strategy: String,
    pub regime: String,
    pub edge: Option<f64>,
    pub pyramid_limit: usize,
    pub max_atr_pct: f64,
    pub reason: String,
    #[serde(default)]
    pub shadow_signal: String,
    #[serde(default)]
    pub shadow_status: String,
    #[serde(default)]
    pub shadow_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VenueHealthModel {
    pub venue: String,
    pub watched_symbols: usize,
    pub priced_symbols: usize,
    pub data_status: String,
    pub bridge_status: String,
    pub oldest_price_age_secs: Option<i64>,
    #[serde(default)]
    pub connectivity_status: String,
    #[serde(default)]
    pub account_status: String,
    #[serde(default)]
    pub protection_status: String,
    #[serde(default)]
    pub reconciliation_status: String,
    #[serde(default)]
    pub exposure_status: String,
    #[serde(default)]
    pub overall_status: String,
    #[serde(default)]
    pub execution_status: String,
    #[serde(default)]
    pub readiness_status: String,
    #[serde(default)]
    pub readiness_blockers: Vec<String>,
    #[serde(default)]
    pub symbol_filters_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StrategyPerformanceModel {
    pub venue: String,
    pub strategy: String,
    pub trades: u32,
    pub net_pnl: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PyramidPerformanceModel {
    pub tranches: u8,
    pub trades: u32,
    pub net_pnl: f64,
    pub win_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VenueRuntimeSnapshot {
    pub scheduler_phase: String,
    pub scheduler_cycle_started_epoch: i64,
    pub scheduler_cycle_completed_epoch: i64,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeLockDiagnosticSnapshot {
    pub owner: String,
    pub phase: String,
    pub started_epoch: i64,
}
// =============================================================================
// 3. MERKEZİ ÜST YAPI (MISSION CONTROL SNAPSHOT)
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionControl {
    pub finance: FinanceSnapshot,
    pub positions: Vec<PositionModel>,
    pub fleet: Vec<WorkerModel>,
    #[serde(default)]
    pub phase: String,
    pub pipeline_steps: Vec<PipelineStep>,
    #[serde(default)]
    pub decision_pipeline_steps: Vec<PipelineStep>,
    #[serde(default)]
    pub runtime_pipeline_steps: Vec<PipelineStep>,
    pub ai_brain: AiBrainSnapshot,
    pub market_fleet: Vec<MarketAnalysisModel>,
    pub logs: Vec<LogEntry>,
    pub trade_history: Vec<ClosedTradeModel>,
    pub charts: ChartSnapshot,
    pub anomalies: Vec<AnomalyModel>,
    pub repair_log: Vec<String>,
    pub scalp_stats: TradeTypeStats,
    pub swing_stats: TradeTypeStats,
    #[serde(default)]
    pub opportunity_snapshots: Vec<OpportunitySnapshotModel>,
    #[serde(default)]
    pub data_coverage: DataCoverageModel,
    #[serde(default)]
    pub download_progress: DownloadProgressModel,
    #[serde(default)]
    pub bist_data_status: BistDataStatusModel,
    #[serde(default)]
    pub decision_trace: Vec<DecisionTraceModel>,
    #[serde(default)]
    pub venue_health: Vec<VenueHealthModel>,
    #[serde(default)]
    pub strategy_performance: Vec<StrategyPerformanceModel>,
    #[serde(default)]
    pub pyramid_performance: Vec<PyramidPerformanceModel>,
    pub active_anomalies: usize,
    #[serde(default)]
    pub anomalies_by_kind: std::collections::BTreeMap<String, usize>,
    #[serde(default)]
    pub autonomy_counters: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    pub indicator_health: std::collections::BTreeMap<String, f64>,
    #[serde(default)]
    pub symbolic_health: std::collections::BTreeMap<String, f64>,
    #[serde(default)]
    pub mt5_bridge_healthy: bool,
    #[serde(default)]
    pub mt5_bridge_last_error: Option<String>,
    #[serde(default)]
    pub venue_accounts: std::collections::BTreeMap<String, memos_venue_api::AccountSnapshot>,
    #[serde(default)]
    pub venue_runtime: std::collections::BTreeMap<String, VenueRuntimeSnapshot>,
    #[serde(default)]
    pub runtime_lock_diagnostic: RuntimeLockDiagnosticSnapshot,
    #[serde(default)]
    pub venue_portfolios: Vec<VenuePortfolioSnapshot>,
    #[serde(default)]
    pub venue_symbols: Vec<VenueSymbolSnapshot>,
}

pub fn normalize_strategy_label(raw: &str) -> String {
    if raw.eq_ignore_ascii_case("default") || raw.eq_ignore_ascii_case("auto") || raw.is_empty() {
        return "Otonom (rejime göre)".to_string();
    } else {
        return raw.to_string();
    }
}

// =============================================================================
// 4. EMİR YAPILARI VE TÜR GÜVENLİĞİ (OMS TABANI)
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrderId(pub u64);

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        return write!(f, "{}", self.0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderSide {
    #[serde(rename = "BUY")]
    Buy,
    #[serde(rename = "SELL")]
    Sell,
}

impl fmt::Display for OrderSide {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            OrderSide::Buy => return write!(f, "BUY"),
            OrderSide::Sell => return write!(f, "SELL"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    #[serde(rename = "NEW")]
    New,
    #[serde(rename = "PARTIALLY_FILLED")]
    PartiallyFilled,
    #[serde(rename = "FILLED")]
    Filled,
    #[serde(rename = "CANCELED")]
    Canceled,
    #[serde(rename = "REJECTED")]
    Rejected,
    #[serde(rename = "EXPIRED")]
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    #[serde(rename = "LIMIT")]
    Limit,
    #[serde(rename = "MARKET")]
    Market,
    #[serde(rename = "STOP_LOSS")]
    StopLoss,
    #[serde(rename = "TAKE_PROFIT")]
    TakeProfit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: Option<OrderId>,
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: f64,
    pub price: Option<f64>,
    pub stop_price: Option<f64>,
    pub filled_quantity: f64,
    pub status: OrderStatus,
    pub average_price: f64,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub raw_data: Option<String>,
}
impl Default for Order {
    fn default() -> Self {
        return Self {
            id: None,
            symbol: String::new(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: 0.0,
            price: None,
            stop_price: None,
            filled_quantity: 0.0,
            status: OrderStatus::New,
            average_price: 0.0,
            created_at: None,
            raw_data: None,
        };
    }
}

impl Order {
    pub fn market(symbol: String, side: OrderSide, quantity: f64) -> Self {
        return Self {
            symbol,
            side,
            order_type: OrderType::Market,
            quantity,
            ..Default::default()
        };
    }

    pub fn limit(symbol: String, side: OrderSide, quantity: f64, price: f64) -> Self {
        return Self {
            symbol,
            side,
            order_type: OrderType::Limit,
            quantity,
            price: Some(price),
            ..Default::default()
        };
    }

    pub fn stop_loss(symbol: String, quantity: f64, stop_price: f64) -> Self {
        return Self {
            symbol,
            side: OrderSide::Sell,
            order_type: OrderType::StopLoss,
            quantity,
            stop_price: Some(stop_price),
            ..Default::default()
        };
    }

    pub fn take_profit(symbol: String, quantity: f64, price: f64) -> Self {
        return Self {
            symbol,
            side: OrderSide::Sell,
            order_type: OrderType::TakeProfit,
            quantity,
            price: Some(price),
            ..Default::default()
        };
    }

    pub fn remaining_quantity(&self) -> f64 {
        return self.quantity - self.filled_quantity;
    }

    pub fn is_fully_filled(&self) -> bool {
        return (self.quantity - self.filled_quantity).abs() < f64::EPSILON;
    }
}

// =============================================================================
// 6. BORSA SEMBOL DETAYLARI (SYMBOL INFO)
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub symbol: String,
    pub status: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub price_precision: u8,
    pub qty_precision: u8,
    pub min_notional: f64,
    pub max_leverage: Option<u32>,
}

impl Default for SymbolInfo {
    fn default() -> Self {
        return Self {
            symbol: "BTCUSDT".to_string(),
            status: "TRADING".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USDT".to_string(),
            price_precision: 2,
            qty_precision: 5,
            min_notional: 5.0,
            max_leverage: Some(125),
        };
    }
}

// =============================================================================
// 7. SİMÜLASYON VE KAĞIT TİCARETİ ÖZETİ (PAPER TRADING RESULT)
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperTradingResult {
    pub symbol: String,
    pub interval: String,
    pub total_trades: u32,
    pub win_trades: u32,
    pub loss_trades: u32,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub total_pnl_usd: f64,
    pub max_drawdown_pct: f64,
    pub sharpe_ratio: f64,
    pub tested_at: String,
}

impl PaperTradingResult {
    pub fn is_edge_confirmed(&self) -> bool {
        return self.win_rate >= 0.45 && self.profit_factor > 1.25 && self.max_drawdown_pct < 15.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::venue::VenueRegistry;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn normalize_strategy_label_handles_sentinels() {
        assert_eq!(normalize_strategy_label("AUTO"), "Otonom (rejime göre)");
        assert_eq!(normalize_strategy_label("auto"), "Otonom (rejime göre)");
        assert_eq!(normalize_strategy_label("Default"), "Otonom (rejime göre)");
        assert_eq!(normalize_strategy_label("default"), "Otonom (rejime göre)");
        assert_eq!(normalize_strategy_label(""), "Otonom (rejime göre)");
    }

    #[test]
    fn normalize_strategy_label_preserves_real_names() {
        assert_eq!(normalize_strategy_label("MA_CROSSOVER"), "MA_CROSSOVER");
        assert_eq!(normalize_strategy_label("SUPERTREND"), "SUPERTREND");
        assert_eq!(normalize_strategy_label("Bollinger"), "Bollinger");
    }

    #[test]
    fn from_env_keeps_mt5_venue_when_no_venues_override_is_set() {
        let _guard = env_lock().lock().unwrap();
        let prev_venues = std::env::var("VENUES").ok();
        std::env::remove_var("VENUES");
        let config = RoboticLoopConfig::from_env();
        let registry = VenueRegistry::from_specs(&config.venues, None);
        let routed = registry.route("EURUSD@mt5");

        assert!(
            routed.is_some(),
            "MT5 etiketli sembol registry üzerinden yönlendirilemedi"
        );
        assert_eq!(
            routed.unwrap().0.exchange(),
            crate::core::types::Exchange::Mt5
        );

        match prev_venues {
            Some(v) => std::env::set_var("VENUES", v),
            None => std::env::remove_var("VENUES"),
        }
    }
}
// SymbolFilters venue contract'ına taşındı (Adım 2). Eski yol korunur:
// `crate::core::model::SymbolFilters` → `memos_venue_api::types::SymbolFilters`.
pub use memos_venue_api::types::SymbolFilters;
