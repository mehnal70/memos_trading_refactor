//! Venue katmanının paylaşılan yaprak tipleri.
//!
//! Bu dosyanın içeriği `memos_trading_core/src/core/types.rs` ve `core/model.rs`'ten
//! BİREBİR taşındı (tek satır bile yeniden yazılmadı) → davranış nötr. Çekirdek bu
//! tipleri eski yollarından `pub use` ile yeniden yayınlar, böylece 400+ çağrı yeri
//! değişmedi.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Bir venue hesabının tek, zaman damgalı görünümü.
///
/// `currency = None` bilinmeyen para birimidir ve güvenli toplama tarafından
/// özellikle dışlanır. `equity` balance'dan ayrı tutulur; desteklemeyen venue'lar
/// varsayılan adaptör kancasıyla balance'ı equity olarak kullanabilir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSnapshot {
    #[serde(default)]
    pub currency: Option<String>,
    pub balance: f64,
    #[serde(default)]
    pub equity: Option<f64>,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub quality: AccountSnapshotQuality,
    #[serde(default)]
    pub status: AccountSnapshotStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AccountSnapshotQuality {
    #[default]
    Unknown,
    Estimated,
    Reported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AccountSnapshotStatus {
    #[default]
    Unavailable,
    Fresh,
    Stale,
    Partial,
}

impl AccountSnapshot {
    pub fn from_balance(balance: f64) -> Self {
        Self {
            currency: None,
            balance,
            equity: Some(balance),
            timestamp: Utc::now(),
            quality: AccountSnapshotQuality::Estimated,
            status: AccountSnapshotStatus::Fresh,
        }
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

/// Borsa türü - Copy eklendi
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Exchange {
    Binance,
    Bist,
    Coinbase, // Yeni eklendi
    Kucoin,   // Yeni eklendi
    Mt5,      // MetaTrader 5 köprüsü (forex/emtia/endeks CFD) — VenueAdapter = Mt5Venue
    Ibkr,     // Interactive Brokers köprüsü (hazırlık katmanı) — VenueAdapter = IbkrVenue
}

impl Exchange {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Binance => "binance",
            Self::Bist => "bist",
            Self::Coinbase => "coinbase",
            Self::Kucoin => "kucoin",
            Self::Mt5 => "mt5",
            Self::Ibkr => "ibkr",
        }
    }

    /// Token'dan borsa (env listesi / config parse için). Bilinmeyen → None.
    pub fn from_token(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "binance" => Some(Self::Binance),
            "bist" => Some(Self::Bist),
            "coinbase" => Some(Self::Coinbase),
            "kucoin" => Some(Self::Kucoin),
            "mt5" | "metatrader" | "metatrader5" => Some(Self::Mt5),
            "ibkr" | "interactivebrokers" | "interactive-brokers" => Some(Self::Ibkr),
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
            // MT5: köprü (yerel MT5 terminali) ayaktayken feed gelir; soyutlama düzeyinde
            // feed-yetenekli kabul edilir (bağlantı yoksa adaptör açık Err döner, sahte değil).
            Self::Binance | Self::Coinbase | Self::Kucoin | Self::Mt5 | Self::Ibkr => true,
        }
    }

    /// Sembol adı biçiminden borsa sınıflandırması (heuristic, tek kaynak).
    /// Kripto quote ile biten / crypto prefix'li semboller → Binance. 3-6 karakterlik
    /// büyük-harf+rakam, forex/CFD eşali (EURUSD, XAUUSD, USDJPY) → MT5. Son olarak
    /// BIST equity gibi 3-6 karakterlik tek tip fiyatlar -> Bist. Yeni borsa eklenince
    /// tek noktada buraya kol eklenir.
    pub fn classify(symbol: &str) -> Self {
        let sym = symbol.trim().to_ascii_uppercase();
        if sym.is_empty() {
            return Self::Binance;
        }

        if sym.ends_with("USDT")
            || sym.ends_with("USDC")
            || sym.ends_with("BUSD")
            || sym.ends_with("FDUSD")
            || sym.ends_with("TUSD")
            || sym.ends_with("DAI")
        {
            return Self::Binance;
        }

        if forex_symbol_shape(&sym) {
            return Self::Mt5;
        }

        if bist_symbol_shape(&sym) {
            return Self::Bist;
        }

        Self::Binance
    }

    /// Bu borsanın işlediği varlık sınıfı. Edge/risk/veri-feed mantığı varlık-sınıfına
    /// göre dallanır (örn. funding-carry yalnız Crypto-perp'te anlamlı, equity'de seans/halt
    /// vardır). Yeni borsa eklerken sınıfını BURAYA bir kol olarak ekle; tek-kaynak.
    pub fn asset_class(&self) -> AssetClass {
        match self {
            Self::Binance | Self::Coinbase | Self::Kucoin => AssetClass::Crypto,
            Self::Bist => AssetClass::Equity,
            // MT5 ağırlıkla forex (24/5). XAUUSD vb. emtia CFD'leri de barındırır; per-sembol
            // emtia ayrımı edge-ölçümü gerektirirse follow-up (venue-düzeyinde kaba Forex yeter).
            Self::Mt5 | Self::Ibkr => AssetClass::Forex,
        }
    }
}

/// İşlem gören varlık sınıfı — borsa-üstü, davranış dallanmasının tek-kaynağı.
/// (Crypto: 7/24, perp'te funding; Equity: seans/halt/temettü; Commodity: futures/spot
/// emtia; Forex: 24/5 döviz.) Yeni sınıf gerektiğinde buraya bir kol ekle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AssetClass {
    Crypto,
    Equity,
    Commodity,
    Forex,
}

impl AssetClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Crypto => "crypto",
            Self::Equity => "equity",
            Self::Commodity => "commodity",
            Self::Forex => "forex",
        }
    }

    /// 7/24 işlem görür mü? (Crypto/Forex süreklilik varsayımı; Equity/Commodity seanslı.)
    /// Stale-feed kapısı + işlem-takvimi mantığı bunu temel alır.
    pub fn is_continuous(&self) -> bool {
        return matches!(self, Self::Crypto);
    }
}

/// Venue hesabının çalışma profili.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum VenueProfile {
    #[default]
    Default,
    Paper,
    Demo,
    Live,
}

impl VenueProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Paper => "paper",
            Self::Demo => "demo",
            Self::Live => "live",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" | "primary" => Some(Self::Default),
            "paper" => Some(Self::Paper),
            "demo" | "testnet" => Some(Self::Demo),
            "live" | "production" | "prod" => Some(Self::Live),
            _ => None,
        }
    }
}

/// Bir venue kimliği (borsa + market + profil). Operatör config'i
/// (`VENUES` env → RoboticLoopConfig.venues)
/// aktif venue'ları bunlarla listeler; `VenueRegistry` bunlardan adaptörleri kurar. [[venue]]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VenueSpec {
    pub exchange: Exchange,
    pub market: Market,
    #[serde(default)]
    pub profile: VenueProfile,
}

impl VenueSpec {
    pub fn new(exchange: Exchange, market: Market) -> Self {
        Self {
            exchange,
            market,
            profile: VenueProfile::Default,
        }
    }

    pub fn with_profile(exchange: Exchange, market: Market, profile: VenueProfile) -> Self {
        Self {
            exchange,
            market,
            profile,
        }
    }

    /// "binance:futures[:demo]" token'ından parse. Borsa bilinmiyorsa `None`;
    /// market kısmı yoksa Spot (tek-kaynak `Exchange::from_token` + `Market::from_label`).
    pub fn parse_token(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let mut parts = s.split(':');
        let ex_str = parts.next()?;
        let mk_str = parts.next().unwrap_or("spot");
        let profile = parts
            .next()
            .map(VenueProfile::from_token)
            .unwrap_or(Some(VenueProfile::Default))?;
        Some(Self {
            exchange: Exchange::from_token(ex_str)?,
            market: Market::from_label(mk_str),
            profile,
        })
    }

    /// "exchange:market[:profile]" token'ı (parse_token ile round-trip).
    pub fn token(&self) -> String {
        if self.profile == VenueProfile::Default {
            format!("{}:{}", self.exchange.as_str(), self.market.as_str())
        } else {
            format!(
                "{}:{}:{}",
                self.exchange.as_str(),
                self.market.as_str(),
                self.profile.as_str()
            )
        }
    }
}

/// Bilinen forex / CFD ana çiftleri (EURUSD, USDJPY, XAUUSD, NAS100, ...)
/// `Exchange::classify` ile aynı tek-kaynak/tanımlayıcı üzerinde kullanılır.
pub fn forex_symbol_shape(sym: &str) -> bool {
    let sym = sym.trim().to_ascii_uppercase();
    if sym.is_empty() || sym.len() != 6 {
        return false;
    }
    if !sym.chars().all(|c| c.is_ascii_uppercase()) {
        return false;
    }

    const CURRENCY_QUOTES: &[&str] = &[
        "USD", "JPY", "EUR", "GBP", "CHF", "CAD", "AUD", "NZD", "SEK", "NOK", "DKK", "PLN", "CZK",
        "HUF", "TRY", "RUB", "ZAR", "MXN", "BRL", "CNY", "HKD", "SGD", "INR", "SAR", "AED",
    ];
    const CRYPTO_PREFIXES: &[&str] = &[
        "BTC", "ETH", "SOL", "XRP", "ADA", "DOGE", "BNB", "LINK", "AVAX", "DOT", "MATIC", "ATOM",
        "LTC", "UNI", "FIL", "APT", "ARB", "NEAR", "OP", "RNDR", "SUI", "PEPE", "BONK", "FET",
        "GRT", "EGLD", "ICP", "FLOW", "KAS", "XLM", "ALGO", "HBAR", "VET", "THETA", "EOS", "TRX",
        "BCH", "XMR", "IOTA", "ZEC", "DASH", "XTZ", "CRO", "OKB",
    ];

    if CRYPTO_PREFIXES.iter().any(|p| sym.starts_with(p)) {
        return false;
    }

    let quote = &sym[3..6];
    if !CURRENCY_QUOTES.iter().any(|q| *q == quote) {
        return false;
    }

    // Tüm 3 haneli ilk kısım da en az bir harf içeriyor; böylece BIST equity'ler
    // (THYAO, GARAN, A1CAP) forex sanitizer'ine düşmez.
    sym[..3].chars().any(|c| c.is_ascii_uppercase())
}

/// BIST equity sembol biçimi heuristic'i: 3-6 büyük-harf/rakam, kripto quote'suz.
/// (THYAO, GARAN, A1CAP ✓ · BTCUSDT ✗). Exchange::classify ve master katmanındaki
/// looks_like_bist_symbol tek bu fonksiyondan beslenir.
pub fn bist_symbol_shape(sym: &str) -> bool {
    let sym = sym.trim().to_ascii_uppercase();
    if sym.len() < 3 || sym.len() > 6 {
        return false;
    }
    if !sym
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return false;
    }
    const CRYPTO_QUOTES: &[&str] = &["USDT", "USDC", "BUSD", "FDUSD", "TUSD", "DAI"];
    for q in CRYPTO_QUOTES {
        if sym.ends_with(q) {
            return false;
        }
    }
    !forex_symbol_shape(&sym)
}

/// Market türü - Copy eklendi
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum Market {
    #[default]
    Spot,
    Futures,
    Coinm,
    /// CFD piyasası (forex/emtia/endeks — örn. MT5 venue). Türev-benzeri: short serbest,
    /// kaldıraçlı. Crypto `Futures`'tan AYRI tutulur → izole DB-namespace (market="mt5") +
    /// kripto-futures evrenine karışmaz. [[project_venue_multimarket]]
    Cfd,
}

impl Market {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Futures => "futures",
            Self::Coinm => "coinm",
            Self::Cfd => "cfd",
        }
    }

    /// config.market gibi serbest etiket string'inden Market'e (case-insensitive).
    /// Tanınmayan → Spot (en kısıtlı/güvenli varsayım). String karşılaştırmasını
    /// koda serpmek yerine tek-nokta. MT5/forex/emtia → Cfd (short serbest; izole namespace).
    pub fn from_label(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "futures" | "fut" | "perp" => Self::Futures,
            "coinm" | "coin-m" => Self::Coinm,
            "cfd" | "mt5" | "forex" | "commodity" => Self::Cfd,
            _ => Self::Spot,
        }
    }

    /// Bu piyasada short (açığa satış) mekanik olarak mümkün mü? Spot'ta borrow
    /// yoktur → long-only. Futures/coinm/CFD türevdir → short serbest.
    pub fn allows_short(&self) -> bool {
        !matches!(self, Self::Spot)
    }
}

#[cfg(test)]
mod exchange_and_market_tests {
    use super::{Exchange, Market};

    #[test]
    fn classify_forex_symbols_as_mt5() {
        assert_eq!(Exchange::classify("EURUSD"), Exchange::Mt5);
        assert_eq!(Exchange::classify("USDJPY"), Exchange::Mt5);
        assert_eq!(Exchange::classify("XAUUSD"), Exchange::Mt5);
    }

    #[test]
    fn classify_crypto_and_equity_symbols_without_false_mt5() {
        assert_eq!(Exchange::classify("BTCUSDT"), Exchange::Binance);
        assert_eq!(Exchange::classify("THYAO"), Exchange::Bist);
    }

    #[test]
    fn from_label_case_insensitive() {
        assert_eq!(Market::from_label("futures"), Market::Futures);
        assert_eq!(Market::from_label("  FUTURES "), Market::Futures);
        assert_eq!(Market::from_label("coinm"), Market::Coinm);
        assert_eq!(Market::from_label("spot"), Market::Spot);
        // Tanınmayan/boş → en güvenli (long-only) varsayım.
        assert_eq!(Market::from_label("garip"), Market::Spot);
        assert_eq!(Market::from_label(""), Market::Spot);
        // MT5/forex/emtia → Cfd (izole namespace + short serbest).
        assert_eq!(Market::from_label("mt5"), Market::Cfd);
        assert_eq!(Market::from_label("FOREX"), Market::Cfd);
        assert_eq!(Market::from_label("cfd"), Market::Cfd);
    }

    #[test]
    fn only_spot_blocks_short() {
        assert!(!Market::Spot.allows_short(), "spot = long-only");
        assert!(Market::Futures.allows_short());
        assert!(Market::Coinm.allows_short());
        assert!(
            Market::Cfd.allows_short(),
            "CFD (forex/emtia) = short serbest"
        );
    }
}

// --- core/model.rs:740-781'den taşındı (emir filtreleri) ---
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolFilters {
    pub step_size: f64,
    pub min_qty: f64,
    pub tick_size: f64,
    pub min_notional: f64,
}

impl Default for SymbolFilters {
    fn default() -> Self {
        return Self {
            step_size: 0.0,
            min_qty: 0.0,
            tick_size: 0.0,
            min_notional: 0.0,
        };
    }
}
impl SymbolFilters {
    /// Qty'yi stepSize'a aşağı yuvarlar. step_size 0 ise qty'yi aynen döndürür.
    pub fn round_qty_down(&self, qty: f64) -> f64 {
        if self.step_size <= 0.0 {
            return qty;
        }
        return (qty / self.step_size).floor() * self.step_size;
    }

    /// Fiyatı tickSize'a yuvarlar (en yakın). tick_size 0 ise price'ı aynen döndürür.
    pub fn round_price(&self, price: f64) -> f64 {
        if self.tick_size <= 0.0 {
            return price;
        }
        return (price / self.tick_size).round() * self.tick_size;
    }

    /// Emirin filtrelerden geçip geçmediğini sınar. Dönüş: yuvarlanmış qty veya Err(sebep).
    pub fn validate(&self, qty: f64, price: f64) -> Result<f64, String> {
        let rounded = self.round_qty_down(qty);
        if rounded <= 0.0 {
            return Err(format!(
                "stepSize sonrası qty=0 (input={:.8}, step={:.8})",
                qty, self.step_size
            ));
        }
        if self.min_qty > 0.0 && rounded < self.min_qty {
            return Err(format!("qty {:.8} < minQty {:.8}", rounded, self.min_qty));
        }
        let notional = rounded * price;
        if self.min_notional > 0.0 && notional < self.min_notional {
            return Err(format!(
                "notional ${:.4} < minNotional ${:.4}",
                notional, self.min_notional
            ));
        }
        return Ok(rounded);
    }
}
