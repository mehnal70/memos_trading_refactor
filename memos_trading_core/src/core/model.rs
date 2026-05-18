// src/core/model.rs - Memos Trading Core Library (Srivastava ATP - Master Veri Kontratı)
// Bu modül saf veri yapısıdır; hiçbir kilit (lock) veya iş parçacığı (thread) bağımlılığı taşımaz.

use serde::{Serialize, Deserialize};
use std::fmt;
use crate::core::math;
use crate::prelude::*;

// =============================================================================
// 1. YAPILANDIRMA VE BELLEK MODELLERİ (Eski config.rs'ten Tahliye Edilen Parçalar)
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TradingMode { Backtest, Paper, Live }

impl TradingMode {
    /// Env değerinden parse — case-insensitive. Bilinmeyen değer Paper'a düşer (güvenli default).
    pub fn from_env_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "live"     => TradingMode::Live,
            "backtest" => TradingMode::Backtest,
            _          => TradingMode::Paper, // "paper", "", "anything" → Paper
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            TradingMode::Live     => "Live",
            TradingMode::Paper    => "Paper",
            TradingMode::Backtest => "Backtest",
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
        Self {
            ma_fast: 5, ma_slow: 20,
            rsi_period: 14, rsi_ob: 70.0, rsi_os: 30.0,
            bb_period: 20, bb_std_dev: 2.0,
            macd_fast: 12, macd_slow: 26, macd_signal: 9,
            stoch_k: 14, stoch_ob: 80.0, stoch_os: 20.0,
            ema_fast: 5, ema_slow: 20,
            donchian_period: 20, williams_period: 14, cci_period: 20,
            stoch_rsi_period: 14, supertrend_period: 10, supertrend_mult: 3.0,
            ict_fvg_lookback: 5, smc_swing_lb: 10,
            best_strategy: None, last_updated: None,
        }
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
    pub symbol: String,
    pub interval: String,
    pub interval_secs: u64,
    pub capital: f64,
    pub db_path: String,
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

    // GÜVENLİK VE PERFORMANS ENTEGRASYONU (src/core/config.rs'ten Geldi)
    pub api_key: Option<String>,
    pub secret_key: Option<String>,
    pub trading_mode: TradingMode,
    pub request_delay_ms: u64, 
    pub max_retries: u32,
    pub http_timeout_sec: u64,
}

impl Default for RoboticLoopConfig {
    fn default() -> Self {
        Self {
            exchange: "binance".into(),
            market: "futures".into(),
            symbol: "BTCUSDT".into(),
            interval: "1m".into(),
            interval_secs: 60,
            capital: 10000.0,
            db_path: "data/trader.db".into(),
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
            
            // Performans ve Güvenlik Varsayılanları
            api_key: None,
            secret_key: None,
            trading_mode: TradingMode::Paper,
            request_delay_ms: 2000,
            max_retries: 5,
            http_timeout_sec: 30,
        }
    }
}

impl RoboticLoopConfig {
    /// Akıcı (Fluent) Builder API Entegrasyonu
    pub fn new() -> Self { Self::default() }

    pub fn with_exchange(mut self, exchange: impl Into<String>) -> Self {
        self.exchange = exchange.into();
        self
    }

    pub fn with_trading_mode(mut self, mode: TradingMode) -> Self {
        self.trading_mode = mode;
        self
    }

    /// API anahtarını hiyerarşik (Secret Store > Config > Env) ve güvenli getirir
    pub fn get_api_key(&self) -> Option<String> {
        crate::core::security::secure_store::get_secret("api_key") // secure_store kütüphanenizin yoluna göre güncelleyin
            .or_else(|| self.api_key.clone())
            .or_else(|| std::env::var("BINANCE_API_KEY").ok())
    }

    /// Secret key'i hiyerarşik ve güvenli getirir
    pub fn get_secret_key(&self) -> Option<String> {
        crate::core::security::secure_store::get_secret("secret_key")
            .or_else(|| self.secret_key.clone())
            .or_else(|| std::env::var("BINANCE_API_SECRET").ok())
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
}

impl FinanceSnapshot {
    pub fn net_pnl(&self) -> f64 {
        self.realize_pnl + self.open_pnl - self.total_fees
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

/// 💱 Live mode'da bir sembol için açık olan borsa emirlerinin referansı.
/// AppState.finance.live_orders[symbol] altında saklanır. Açılışta entry/SL/TP
/// order_id'leri yazılır; kapatma anında cancel_all_orders yerine bu hedefli
/// id'ler kullanılır → paralel sembollerin emirleri etkilenmez.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LiveOrderRefs {
    /// Açılış market emrinin id'si (audit trail için, cancel edilmez)
    #[serde(default)]
    pub entry_order_id: Option<String>,
    /// Stop-loss emrinin id'si
    #[serde(default)]
    pub sl_order_id: Option<String>,
    /// Take-profit emrinin id'si
    #[serde(default)]
    pub tp_order_id: Option<String>,
    /// Mühürlendiği zaman (RFC3339)
    #[serde(default)]
    pub placed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionModel {
    /// Pozisyonun tekil kimliği. IntelligenceHub.track_trade ↔ learn_from_exit eşlemesi için.
    /// UUID v4 string. Eski serialize'lı pozisyonlarda boş gelir (serde default).
    #[serde(default)]
    pub pos_id: String,
    pub symbol: String,
    pub entry_price: f64,
    pub current_price: f64,
    pub qty: f64,
    pub leverage: f64,
    pub is_long: bool,
    pub trade_type: String,
    pub opened_at: String,

    // === Pozisyon Yönetimi (Risk Exit Çekirdeği) ===
    /// Statik stop loss seviyesi (long için entry'nin altı, short için üstü).
    /// 0.0 ise SL devre dışı (geriye uyumluluk: eski serialize'lı pozisyonlar).
    #[serde(default)]
    pub stop_loss: f64,
    /// Statik take profit seviyesi.
    #[serde(default)]
    pub take_profit: f64,
    /// Trailing stop seviyesi — ATR × atr_trail_mult uzaklıkta, en uygun fiyatı kovalar.
    /// Long: max_favorable_price - delta; Short: min_favorable_price + delta.
    #[serde(default)]
    pub trailing_stop: f64,
    /// Pozisyon açıldıktan sonra ulaşılan en uygun (long için en yüksek, short için en düşük) fiyat.
    /// Trailing stop ve breakeven kararı bu rakama göre verilir.
    #[serde(default)]
    pub max_favorable_price: f64,
    /// Breakeven aktif mi? Aktif olduktan sonra SL = entry_price'a sabitlenir.
    #[serde(default)]
    pub breakeven_activated: bool,
}

impl PositionModel {
    pub fn calculate_pnl(&self) -> f64 {
        math::calculate_pnl(self.entry_price, self.current_price, self.qty, self.is_long)
    }

    pub fn roe(&self) -> f64 {
        math::calculate_roe(self.entry_price, self.current_price, self.leverage, self.is_long)
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

    // === IntelligenceHub canlı verileri (AI Center paneli) ===
    /// Aktif strateji adı (live_strategy). Backtest job otonom değiştirir.
    #[serde(default)]
    pub live_strategy: String,
    /// AutonomousController state: Observe/Optimize/Trade/SafeMode/Halted
    #[serde(default)]
    pub controller_state: String,
    /// AutonomousController cycle sayacı (her IntelligenceHub tick'inde +1)
    #[serde(default)]
    pub controller_cycle: u64,
    /// Ardışık kayıp işlem sayısı (5+ ⇒ SafeMode tetikleyicisi)
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Hub.pending_trades — açık olup hub.learn_from_exit beklenen pozisyon sayısı
    #[serde(default)]
    pub pending_trades: usize,
    /// Drift skor tarihçesi (son 60 nokta) — AI Center sparkline için
    #[serde(default)]
    pub drift_series: Vec<f64>,
    /// brain.best_params'tan kritik 3'lü: TP / SL / Position Size
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
    pub current_price: f64,
    pub change_24h: f64,
    pub zones: Vec<SrZoneModel>,
    pub nearest_support: Option<f64>,
    pub nearest_resistance: Option<f64>,
}

/// 🛡️ Srivastava ATP - Destek ve Direnç Bölgesi Veri Modeli
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SrZoneModel {
    pub zone_type: String,     // "SUPPORT" | "RESISTANCE"
    pub price_low: f64,        // Bölgenin taban fiyatı
    pub price_high: f64,       // Bölgenin tavan fiyatı
    pub strength: f64,         // Bölgenin hacimsel/istatistiksel gücü
    pub touch_count: u32,      // Fiyatın bu bölgeye kaç kez temas ettiği
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
    /// Equity tarihçesi (en eski → en yeni). Sparkline'da çizilir.
    #[serde(default)]
    pub equity_series: Vec<f64>,
    /// Anlık drawdown yüzdesi (zirve ⇒ şu an), 0..100.
    #[serde(default)]
    pub current_drawdown_pct: f64,
    /// Zirve equity (rapor için).
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

// =============================================================================
// 3. MERKEZİ ÜST YAPI (MISSION CONTROL SNAPSHOT)
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionControl {
    pub finance: FinanceSnapshot,
    pub positions: Vec<PositionModel>,
    pub fleet: Vec<WorkerModel>,
    /// Ana döngünün anlık fazı: Booting/Scanning/Executing/Recovering/Stopped/Idle.
    #[serde(default)]
    pub phase: String,
    pub pipeline_steps: Vec<PipelineStep>,
    pub ai_brain: AiBrainSnapshot,
    pub market_fleet: Vec<MarketAnalysisModel>,
    pub logs: Vec<LogEntry>,
    pub trade_history: Vec<ClosedTradeModel>,
    pub charts: ChartSnapshot,
    pub anomalies: Vec<AnomalyModel>,
    pub repair_log: Vec<String>,
    pub scalp_stats: TradeTypeStats,
    pub swing_stats: TradeTypeStats,
    pub active_anomalies: usize,
}

// =============================================================================
// 4. EMİR YAPILARI VE TÜR GÜVENLİĞİ (OMS TABANI)
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrderId(pub u64);

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "{}", self.0) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide { #[serde(rename = "BUY")] Buy, #[serde(rename = "SELL")] Sell }

impl fmt::Display for OrderSide {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self { OrderSide::Buy => write!(f, "BUY"), OrderSide::Sell => write!(f, "SELL") }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    #[serde(rename = "NEW")] New,
    #[serde(rename = "PARTIALLY_FILLED")] PartiallyFilled,
    #[serde(rename = "FILLED")] Filled,
    #[serde(rename = "CANCELED")] Canceled,
    #[serde(rename = "REJECTED")] Rejected,
    #[serde(rename = "EXPIRED")] Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    #[serde(rename = "LIMIT")] Limit,
    #[serde(rename = "MARKET")] Market,
    #[serde(rename = "STOP_LOSS")] StopLoss,
    #[serde(rename = "TAKE_PROFIT")] TakeProfit,
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
        Self {
            id: None, symbol: String::new(), side: OrderSide::Buy, order_type: OrderType::Market,
            quantity: 0.0, price: None, stop_price: None, filled_quantity: 0.0,
            status: OrderStatus::New, average_price: 0.0, created_at: None, raw_data: None,
        }
    }
}

impl Order {
    pub fn market(symbol: String, side: OrderSide, quantity: f64) -> Self {
        Self { symbol, side, order_type: OrderType::Market, quantity, ..Default::default() }
    }
    
    pub fn limit(symbol: String, side: OrderSide, quantity: f64, price: f64) -> Self {
        Self { symbol, side, order_type: OrderType::Limit, quantity, price: Some(price), ..Default::default() }
    }
    
    pub fn stop_loss(symbol: String, quantity: f64, stop_price: f64) -> Self {
        Self { symbol, side: OrderSide::Sell, order_type: OrderType::StopLoss, quantity, stop_price: Some(stop_price), ..Default::default() }
    }
    
    pub fn take_profit(symbol: String, quantity: f64, price: f64) -> Self {
        Self { symbol, side: OrderSide::Sell, order_type: OrderType::TakeProfit, quantity, price: Some(price), ..Default::default() }
    }
    
    pub fn remaining_quantity(&self) -> f64 { 
        self.quantity - self.filled_quantity 
    }
    
    pub fn is_fully_filled(&self) -> bool { 
        (self.quantity - self.filled_quantity).abs() < f64::EPSILON 
    }
}

// =============================================================================
// 6. BORSA SEMBOL DETAYLARI (SYMBOL INFO)
// =============================================================================

/// 📊 Srivastava ATP - Borsa Sembolü Teknik ve Regülatif Kuralları
/// Borsadan (Binance/BIST) çekilen emir hassasiyetleri ve kısıtlamaları.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub symbol: String,
    pub status: String,            // "TRADING" | "BREAK"
    pub base_asset: String,        // Örn: "BTC"
    pub quote_asset: String,       // Örn: "USDT"
    pub price_precision: u8,       // Fiyat virgülden sonra kaç basamak? (Örn: 2)
    pub qty_precision: u8,         // Miktar virgülden sonra kaç basamak? (Örn: 5)
    pub min_notional: f64,         // Minimum işlem dolar hacmi barajı (Örn: 5.0 USDT)
    pub max_leverage: Option<u32>, // Kaldıraçlı pazar için maksimum kaldıraç sınırı
}

impl Default for SymbolInfo {
    fn default() -> Self {
        Self {
            symbol: "BTCUSDT".to_string(),
            status: "TRADING".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USDT".to_string(),
            price_precision: 2,
            qty_precision: 5,
            min_notional: 5.0,
            max_leverage: Some(125),
        }
    }
}

// =============================================================================
// 7. SİMÜLASYON VE KAĞIT TİCARETİ ÖZETİ (PAPER TRADING RESULT)
// =============================================================================

/// 🔬 Srivastava ATP - Backtest ve Simülasyon Raporlama Modeli
/// Robotun geçmiş testlerden veya sanal işlemlerden elde ettiği adli performans karnesi.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperTradingResult {
    pub symbol: String,
    pub interval: String,
    pub total_trades: u32,
    pub win_trades: u32,
    pub loss_trades: u32,
    pub win_rate: f64,             // 0.0 - 1.0 arası başarı oranı
    pub profit_factor: f64,        // Toplam Kazanç / Toplam Kayıp rasyosu
    pub total_pnl_usd: f64,        // Net kazanılan/kaybedilen dolar miktarı
    pub max_drawdown_pct: f64,     // Kasada görülen en büyük tepe-dip düşüş yüzdesi
    pub sharpe_ratio: f64,         // Riski ayarlanmış getiri rasyosu
    pub tested_at: String,         // Testin yapıldığı zaman damgası "YYYY-MM-DD HH:MM"
}

impl PaperTradingResult {
    /// Simülasyonun istatistiksel olarak başarılı (güvenli) olup olmadığını doğrular
    pub fn is_edge_confirmed(&self) -> bool {
        self.win_rate >= 0.45 && self.profit_factor > 1.25 && self.max_drawdown_pct < 15.0
    }
}