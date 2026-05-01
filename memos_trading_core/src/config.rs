use crate::secure_store;
impl Config {
    /// Güvenli şekilde API anahtarını getir
    pub fn get_api_key(&self) -> Option<String> {
        if let Some(ref key) = self.api_key {
            if let Some(val) = secure_store::get_secret("api_key") { return Some(val); }
            return Some(key.clone());
        }
        std::env::var("BINANCE_API_KEY").ok()
    }
    /// Güvenli şekilde secret key'i getir
    pub fn get_secret_key(&self) -> Option<String> {
        if let Some(ref key) = self.secret_key {
            if let Some(val) = secure_store::get_secret("secret_key") { return Some(val); }
            return Some(key.clone());
        }
        std::env::var("BINANCE_API_SECRET").ok()
    }
}
// Otomatik ilk kurulum ve optimum ayar modülü
use crate::types::{Exchange, Market, StrategyParams, RiskParams};
use crate::robot::ml_engine::MLModel;

pub struct AutoConfig;

impl AutoConfig {
    pub fn default_exchange() -> Exchange {
        Exchange::Binance
    }
    pub fn default_market() -> Market {
        Market::Spot
    }
    pub fn default_symbols() -> Vec<String> {
        vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()]
    }
    pub fn default_strategy_params() -> StrategyParams {
        StrategyParams {
            fast: Some(7),
            slow: Some(21),
            period: Some(14),
            overbought: Some(70.0),
            oversold: Some(30.0),
            fast_period: None,
            slow_period: None,
            signal_period: None,
            std_dev: None,
            bb_period: Some(20),
        }
    }
    pub fn default_risk_params() -> RiskParams {
        RiskParams {
            stop_loss_pct: 2.0,
            take_profit_pct: 4.0,
            max_position_size_pct: Some(10.0),
            max_portfolio_risk_pct: Some(20.0),
            use_kelly_criterion: false,
            trailing_stop_pct: None,
        }
    }
    pub fn default_model() -> MLModel {
        MLModel::new()
    }
    pub fn default_balance() -> f64 {
        10000.0
    }
    pub fn default_user_id() -> String {
        "default_user".to_string()
    }
}
use serde::{Serialize, Deserialize};

/// Trading modu
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradingMode {
    Backtest,
    Paper,
    Live,
}

/// Sembol bazlı izleme yapılandırması
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolConfig {
    pub symbol: String,
    pub exchange: String,
    pub market: String,
    pub intervals: Vec<String>,
    pub strategies: Vec<String>,
    pub enabled: bool,
    pub priority: u8,
}

impl Default for SymbolConfig {
    fn default() -> Self {
        Self {
            symbol: "BTCUSDT".to_string(),
            exchange: "binance".to_string(),
            market: "spot".to_string(),
            intervals: vec!["1m".to_string(), "5m".to_string(), "1h".to_string()],
            strategies: vec!["ma-crossover".to_string(), "rsi".to_string()],
            enabled: true,
            priority: 5,
        }
    }
}

/// Sembol filtreleme kriterleri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolFilter {
    pub exchange: Option<String>,
    pub market: Option<String>,
    pub quote_currency: Option<String>,
    pub min_rank: Option<i32>,
    pub max_rank: Option<i32>,
    pub exclude_symbols: Vec<String>,
}

impl Default for SymbolFilter {
    fn default() -> Self {
        Self {
            exchange: Some("binance".to_string()),
            market: Some("spot".to_string()),
            quote_currency: Some("USDT".to_string()),
            min_rank: Some(1),
            max_rank: Some(20),
            exclude_symbols: vec![],
        }
    }
}

/// Otomatik görev yapılandırması
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoTaskConfig {
    pub enabled: bool,
    pub interval_minutes: u64,
    pub symbol_configs: Vec<SymbolConfig>,
    pub use_filter: bool,
    pub filter: SymbolFilter,
    pub default_intervals: Vec<String>,
    pub default_strategies: Vec<String>,
    pub min_data_points: usize,
}

impl Default for AutoTaskConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 30,
            symbol_configs: vec![],
            use_filter: true,
            filter: SymbolFilter::default(),
            default_intervals: vec!["1m".to_string(), "5m".to_string(), "1h".to_string()],
            default_strategies: vec!["ma-crossover".to_string(), "rsi".to_string()],
            min_data_points: 200,
        }
    }
}

/// ML Otomatik Test Yapılandırması
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MLAutoConfig {
    pub enabled: bool,
    pub test_interval_hours: u64,
    pub min_trades_for_learning: usize,
    pub performance_threshold: f64,
    pub auto_optimize: bool,
    pub evolution_enabled: bool,
}

impl Default for MLAutoConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            test_interval_hours: 6,
            min_trades_for_learning: 10,
            performance_threshold: 0.05,
            auto_optimize: true,
            evolution_enabled: true,
        }
    }
}

/// Uygulama konfigürasyonu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub exchange: Option<String>,
    pub market: Option<String>,
    pub platform: Option<String>,
    pub api_key: Option<String>,
    pub secret_key: Option<String>,
    pub live_trading: bool,
    pub trading_mode: TradingMode,
    pub request_delay_seconds: u64,
    pub max_retries: u32,
    pub max_backoff_seconds: u64,
    pub max_symbols_per_batch: usize,
    pub http_timeout_seconds: u64,
    
    // Arka plan görevleri
    #[serde(default)]
    pub auto_task: AutoTaskConfig,
    #[serde(default)]
    pub ml_auto: MLAutoConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exchange: None,
            market: None,
            platform: None,
            api_key: None,
            secret_key: None,
            live_trading: false,
            trading_mode: TradingMode::Paper,
            request_delay_seconds: 2,
            max_retries: 5,
            max_backoff_seconds: 60,
            max_symbols_per_batch: 100,
            http_timeout_seconds: 30,
            auto_task: AutoTaskConfig::default(),
            ml_auto: MLAutoConfig::default(),
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_exchange(mut self, exchange: String) -> Self {
        self.exchange = Some(exchange);
        self
    }

    pub fn with_market(mut self, market: String) -> Self {
        self.market = Some(market);
        self
    }

    pub fn with_trading_mode(mut self, mode: TradingMode) -> Self {
        self.trading_mode = mode;
        self
    }
}
