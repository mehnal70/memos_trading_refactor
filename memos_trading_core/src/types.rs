use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

/// Her açık/kapalı pozisyon için evrensel tekil kimlik.
/// String hash veya sembol+market bileşimi yerine Uuid kullanılır;
/// bu sayede aynı sembolün farklı market veya zamanda açılan pozisyonları
/// karışmaz, dedup güvenilir olur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PositionId(uuid::Uuid);

impl PositionId {
    /// Yeni rastgele kimlik üret.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for PositionId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for PositionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Funding rate noktası (Binance Futures vb. için)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingRatePoint {
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub funding_rate: f64,
    pub mark_price: Option<f64>,
}

/// OHLCV Mum verisi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub timestamp: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub symbol: String,
    pub interval: String,
}

/// Trading sinyali
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Signal {
    Buy,
    Sell,
    Hold,
}

impl Default for Signal {
    fn default() -> Self {
        Signal::Hold
    }
}

/// Ticari işlem kaydı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: Option<u64>,
    pub symbol: String,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub amount: f64,
    pub entry_time: DateTime<Utc>,
    pub exit_time: Option<DateTime<Utc>>,
    pub pnl: Option<f64>,
    pub pnl_pct: Option<f64>,
    pub strategy: String,
}

/// Risk parametreleri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskParams {
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub max_position_size_pct: Option<f64>,
    pub max_portfolio_risk_pct: Option<f64>,
    pub use_kelly_criterion: bool,
    /// Takipli stop-loss yüzdesi (None = devre dışı)
    /// Örn: Some(1.5) → fiyat en yüksek noktadan %1.5 geriye dönerse kapat
    pub trailing_stop_pct: Option<f64>,
}

/// Strateji parametreleri
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StrategyParams {
    pub fast: Option<usize>,
    pub slow: Option<usize>,
    pub period: Option<usize>,       // RSI periyodu
    pub overbought: Option<f64>,
    pub oversold: Option<f64>,
    pub fast_period: Option<usize>,  // MACD hızlı EMA
    pub slow_period: Option<usize>,  // MACD yavaş EMA
    pub signal_period: Option<usize>,// MACD sinyal EMA
    pub std_dev: Option<f64>,        // Bollinger Bands std sapması
    pub bb_period: Option<usize>,    // Bollinger Bands SMA periyodu
}

/// Exchange türü
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Exchange {
    Binance,
    Bist,
}

impl Exchange {
    pub fn as_str(&self) -> &'static str {
        match self {
            Exchange::Binance => "binance",
            Exchange::Bist => "bist",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "binance" => Some(Exchange::Binance),
            "bist" => Some(Exchange::Bist),
            _ => None,
        }
    }
}

/// Market türü
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Market {
    Spot,
    Futures,
    Coinm,
}

impl Default for Market {
    fn default() -> Self { Market::Spot }
}

impl Market {
    pub fn as_str(&self) -> &'static str {
        match self {
            Market::Spot => "spot",
            Market::Futures => "futures",
            Market::Coinm => "coinm",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "spot" => Some(Market::Spot),
            "futures" => Some(Market::Futures),
            "coinm" => Some(Market::Coinm),
            _ => None,
        }
    }
}
