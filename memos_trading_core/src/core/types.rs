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
    /// Var olan UUID'den inşa et — track_trade ↔ learn_from_exit eşlemesi için.
    #[inline]
    pub fn from_uuid(u: uuid::Uuid) -> Self { Self(u) }
    /// String'ten parse et; hata sessizce yeni bir UUID üretir (test/serde uyumu).
    #[inline]
    pub fn from_str_or_new(s: &str) -> Self {
        uuid::Uuid::parse_str(s).map(Self).unwrap_or_else(|_| Self::new())
    }
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

    /// Token'dan borsa (env listesi / config parse için). Bilinmeyen → None.
    pub fn from_token(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "binance" => Some(Self::Binance),
            "bist"    => Some(Self::Bist),
            "coinbase" => Some(Self::Coinbase),
            "kucoin"  => Some(Self::Kucoin),
            _ => None,
        }
    }

    /// Bu borsanın bu kurulumda gerçek-zamanlı veri/fiyat feed'i var mı?
    /// Feed'i olmayan borsa sembolleri canlı cycle'a alınmaz (fiyatsız satırlar
    /// DataIngest/PriceFetch Failed → anomaly birikimi yapar). Yeni borsa eklerken
    /// feed durumunu BURAYA bir kol olarak ekle; motor çağrı yerlerine dokunma.
    pub fn has_live_feed(&self) -> bool {
        match self {
            // BIST: bu dağıtımda canlı feed yok (manuel/gecikmeli liste). Operatör
            // RuntimeTuning.force_live_exchanges ile yine de zorlayabilir.
            Self::Bist => false,
            Self::Binance | Self::Coinbase | Self::Kucoin => true,
        }
    }

    /// Sembol adı biçiminden borsa sınıflandırması (heuristic, tek kaynak).
    /// Kripto quote ile biten / format dışı → Binance (kripto). 3-6 karakterlik
    /// büyük-harf+rakam (kripto quote'suz) → Bist (BIST equity). Yeni borsanın
    /// sembol biçimi farklıysa buraya bir kol ekle.
    pub fn classify(symbol: &str) -> Self {
        if bist_symbol_shape(symbol) { Self::Bist } else { Self::Binance }
    }
}

/// BIST equity sembol biçimi heuristic'i: 3-6 büyük-harf/rakam, kripto quote'suz.
/// (THYAO, GARAN, A1CAP ✓ · BTCUSDT ✗). Exchange::classify ve master katmanındaki
/// looks_like_bist_symbol tek bu fonksiyondan beslenir.
pub fn bist_symbol_shape(sym: &str) -> bool {
    if sym.len() < 3 || sym.len() > 6 { return false; }
    if !sym.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
        return false;
    }
    const CRYPTO_QUOTES: &[&str] = &["USDT", "USDC", "BUSD", "FDUSD", "TUSD", "DAI"];
    for q in CRYPTO_QUOTES {
        if sym.ends_with(q) { return false; }
    }
    true
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

    /// config.market gibi serbest etiket string'inden Market'e (case-insensitive).
    /// Tanınmayan → Spot (en kısıtlı/güvenli varsayım). String karşılaştırmasını
    /// koda serpmek yerine tek-nokta.
    pub fn from_label(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "futures" | "fut" | "perp" => Self::Futures,
            "coinm" | "coin-m" => Self::Coinm,
            _ => Self::Spot,
        }
    }

    /// Bu piyasada short (açığa satış) mekanik olarak mümkün mü? Spot'ta borrow
    /// yoktur → long-only. Futures/coinm türevdir → short serbest.
    pub fn allows_short(&self) -> bool {
        !matches!(self, Self::Spot)
    }
}

#[cfg(test)]
mod market_tests {
    use super::Market;

    #[test]
    fn from_label_case_insensitive() {
        assert_eq!(Market::from_label("futures"), Market::Futures);
        assert_eq!(Market::from_label("  FUTURES "), Market::Futures);
        assert_eq!(Market::from_label("coinm"), Market::Coinm);
        assert_eq!(Market::from_label("spot"), Market::Spot);
        // Tanınmayan/boş → en güvenli (long-only) varsayım.
        assert_eq!(Market::from_label("garip"), Market::Spot);
        assert_eq!(Market::from_label(""), Market::Spot);
    }

    #[test]
    fn only_spot_blocks_short() {
        assert!(!Market::Spot.allows_short(), "spot = long-only");
        assert!(Market::Futures.allows_short());
        assert!(Market::Coinm.allows_short());
    }
}
