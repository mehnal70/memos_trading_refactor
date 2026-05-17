// types.rs - Merkezi Veri Tipleri ve Tanımlamalar

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::fmt;

/// Her pozisyon için evrensel tekil kimlik (UUID tabanlı)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PositionId(uuid::Uuid);

impl PositionId {
    #[inline]
    pub fn new() -> Self { Self(uuid::Uuid::new_v4()) }
}

impl Default for PositionId {
    fn default() -> Self { Self::new() }
}

impl fmt::Display for PositionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Funding rate noktası - Futures piyasalar için kritik veri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingRatePoint {
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub funding_rate: f64,
    pub mark_price: Option<f64>,
}

/// OHLCV Mum verisi - Performans için Copy desteği verilebilir ancak String/String (symbol/interval) bunu engeller.
#[derive(Debug, Clone, Serialize, Deserialize,Default)]
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

/// Trading sinyali - Copy eklendi (CPU register düzeyinde taşınır)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Signal {
    Buy,
    Sell,
    #[default]
    Hold,
}

/// Ticari işlem kaydı (Audit ve PnL takibi için)
#[derive(Debug, Clone, Serialize, Deserialize, Default)] // <-- Default buraya eklendi
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

/// Risk parametreleri - Copy eklendi
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct RiskParams {
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub max_position_size_pct: Option<f64>,
    pub max_portfolio_risk_pct: Option<f64>,
    pub use_kelly_criterion: bool,
    pub trailing_stop_pct: Option<f64>,
}

/// Strateji parametreleri - Copy eklendi (Allocation-free grid search için kritik)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct StrategyParams {
    pub fast: Option<usize>,
    pub slow: Option<usize>,
    pub period: Option<usize>,
    pub overbought: Option<f64>,
    pub oversold: Option<f64>,
    pub fast_period: Option<usize>,
    pub slow_period: Option<usize>,
    pub signal_period: Option<usize>,
    pub std_dev: Option<f64>,
    pub bb_period: Option<usize>,
}

/// Borsa türü - Copy eklendi
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Exchange {
    Binance,
    Bist,
    Coinbase, // Yeni eklendi
    Kucoin,   // Yeni eklendi
}

impl Exchange {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Binance => "binance",
            Self::Bist => "bist",
            Self::Coinbase => "coinbase",
            Self::Kucoin => "kucoin",
        }
    }
}

/// Market türü - Copy eklendi
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum Market {
    #[default]
    Spot,
    Futures,
    Coinm,
}

impl Market {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Futures => "futures",
            Self::Coinm => "coinm",
        }
    }
}
