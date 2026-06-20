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

/// Tek bir parametre adının inline (heap'siz) gösterimi. `Copy` + ömürsüz olması
/// `StrategyParams`'ın allocation-free `Copy` kalmasını sağlar (grid search kritik).
/// Param adları kısa literaller (`signal_period`=13, `funding_threshold`=17 …);
/// 24 bayt fazlasıyla yeter. Daha uzunu (olmaması beklenir) kırpılır.
#[derive(Debug, Clone, Copy)]
struct ParamName {
    bytes: [u8; 24],
    len: u8,
}

impl ParamName {
    fn from_str(s: &str) -> Self {
        let mut bytes = [0u8; 24];
        let n = s.len().min(24);
        bytes[..n].copy_from_slice(&s.as_bytes()[..n]);
        Self { bytes, len: n as u8 }
    }
    #[inline]
    fn matches(&self, key: &str) -> bool {
        let n = self.len as usize;
        key.len() == n && &self.bytes[..n] == key.as_bytes()
    }
    #[inline]
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

/// Strateji parametreleri — **açık-kelime-dağarcıklı, `Copy`, allocation-free inline KV-torba**.
///
/// Eski sabit 10-alanlı "god bag" yerine: her strateji KENDİ parametre uzayını
/// `Strategy::param_spec()` ile bildirir, değerler buraya ADIYLA yazılır/okunur
/// (`set`/`get`). Yeni bir eşik/periyot açmak için yalnız (a) `strategies::keys`'e bir
/// sabit ve (b) stratejide bir `f64_or/usize_or` okuması gerekir — struct alanı + match
/// kolu + serde üçlemesi biter. Anahtarlar `strategies::keys` modülünde tek-kaynak.
///
/// `CAP=16`: mevcut en geniş strateji ~4 param bildirir; config/store override'ları için
/// bol pay. Dolarsa `set` sessizce (warn ile) yok sayar — panik yok.
#[derive(Debug, Clone, Copy)]
pub struct StrategyParams {
    names: [ParamName; Self::CAP],
    vals: [f64; Self::CAP],
    len: usize,
}

impl StrategyParams {
    pub const CAP: usize = 16;

    #[inline]
    pub const fn new() -> Self {
        Self {
            names: [ParamName { bytes: [0u8; 24], len: 0 }; Self::CAP],
            vals: [0.0; Self::CAP],
            len: 0,
        }
    }

    /// Anahtarın ham `f64` değeri; yoksa `None`.
    #[inline]
    pub fn get(&self, key: &str) -> Option<f64> {
        self.names[..self.len]
            .iter()
            .position(|n| n.matches(key))
            .map(|i| self.vals[i])
    }

    /// Periyot/bar sayısı okuması: yoksa `default`. Ham değer `round().max(1)` ile
    /// usize'a yuvarlanır (yuvarlama politikası tek-noktada, okuma anında).
    #[inline]
    pub fn usize_or(&self, key: &str, default: usize) -> usize {
        self.get(key).map(|v| v.round().max(1.0) as usize).unwrap_or(default)
    }

    /// Sürekli eşik/çarpan okuması: yoksa `default`.
    #[inline]
    pub fn f64_or(&self, key: &str, default: f64) -> f64 {
        self.get(key).unwrap_or(default)
    }

    /// Anahtarı upsert eder. Kapasite dolu ve anahtar yeniyse sessizce (warn) atlar.
    pub fn set(&mut self, key: &str, val: f64) {
        if let Some(i) = self.names[..self.len].iter().position(|n| n.matches(key)) {
            self.vals[i] = val;
        } else if self.len < Self::CAP {
            self.names[self.len] = ParamName::from_str(key);
            self.vals[self.len] = val;
            self.len += 1;
        } else {
            log::warn!("StrategyParams kapasitesi ({}) doldu; '{}' atlandı", Self::CAP, key);
        }
    }

    /// Zincirlenebilir kurucu (test/kurulum): `StrategyParams::new().with("fast", 5.0)`.
    #[inline]
    pub fn with(mut self, key: &str, val: f64) -> Self {
        self.set(key, val);
        self
    }

    #[inline]
    pub fn is_empty(&self) -> bool { self.len == 0 }

    /// Dolu (anahtar, değer) çiftleri üzerinde yineleyici (serde/rapor için).
    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> + '_ {
        (0..self.len).map(move |i| (self.names[i].as_str(), self.vals[i]))
    }
}

impl Default for StrategyParams {
    fn default() -> Self { Self::new() }
}

// Serde: harita ({"fast": 5.0, ...}) formu. Diske persist edilip geri okunmuyor
// (ParameterStore from_env tek-kaynak), ama rapor/test için temiz round-trip.
impl Serialize for StrategyParams {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = ser.serialize_map(Some(self.len))?;
        for (k, v) in self.iter() {
            map.serialize_entry(k, &v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for StrategyParams {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        let m = std::collections::BTreeMap::<String, f64>::deserialize(de)?;
        let mut p = StrategyParams::new();
        for (k, v) in m {
            p.set(&k, v);
        }
        Ok(p)
    }
}

/// Borsa türü - Copy eklendi
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Exchange {
    Binance,
    Bist,
    Coinbase, // Yeni eklendi
    Kucoin,   // Yeni eklendi
    Bybit,    // Çoklu-piyasa Faz 1: gerçek 2. kripto borsa (VenueAdapter = BybitVenue)
    Mt5,      // MetaTrader 5 köprüsü (forex/emtia/endeks CFD) — VenueAdapter = Mt5Venue
}

impl Exchange {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Binance => "binance",
            Self::Bist => "bist",
            Self::Coinbase => "coinbase",
            Self::Kucoin => "kucoin",
            Self::Bybit => "bybit",
            Self::Mt5 => "mt5",
        }
    }

    /// Token'dan borsa (env listesi / config parse için). Bilinmeyen → None.
    pub fn from_token(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "binance" => Some(Self::Binance),
            "bist"    => Some(Self::Bist),
            "coinbase" => Some(Self::Coinbase),
            "kucoin"  => Some(Self::Kucoin),
            "bybit"   => Some(Self::Bybit),
            "mt5" | "metatrader" | "metatrader5" => Some(Self::Mt5),
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
            Self::Binance | Self::Coinbase | Self::Kucoin | Self::Bybit | Self::Mt5 => true,
        }
    }

    /// Sembol adı biçiminden borsa sınıflandırması (heuristic, tek kaynak).
    /// Kripto quote ile biten / format dışı → Binance (kripto). 3-6 karakterlik
    /// büyük-harf+rakam (kripto quote'suz) → Bist (BIST equity). Yeni borsanın
    /// sembol biçimi farklıysa buraya bir kol ekle.
    pub fn classify(symbol: &str) -> Self {
        if bist_symbol_shape(symbol) { Self::Bist } else { Self::Binance }
    }

    /// Bu borsanın işlediği varlık sınıfı. Edge/risk/veri-feed mantığı varlık-sınıfına
    /// göre dallanır (örn. funding-carry yalnız Crypto-perp'te anlamlı, equity'de seans/halt
    /// vardır). Yeni borsa eklerken sınıfını BURAYA bir kol olarak ekle; tek-kaynak.
    pub fn asset_class(&self) -> AssetClass {
        match self {
            Self::Binance | Self::Coinbase | Self::Kucoin | Self::Bybit => AssetClass::Crypto,
            Self::Bist => AssetClass::Equity,
            // MT5 ağırlıkla forex (24/5). XAUUSD vb. emtia CFD'leri de barındırır; per-sembol
            // emtia ayrımı edge-ölçümü gerektirirse follow-up (venue-düzeyinde kaba Forex yeter).
            Self::Mt5 => AssetClass::Forex,
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
        matches!(self, Self::Crypto)
    }
}

/// Bir venue kimliği (borsa + market). Operatör config'i (`VENUES` env → RoboticLoopConfig.venues)
/// aktif venue'ları bunlarla listeler; `VenueRegistry` bunlardan adaptörleri kurar. [[venue]]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VenueSpec {
    pub exchange: Exchange,
    pub market: Market,
}

impl VenueSpec {
    pub fn new(exchange: Exchange, market: Market) -> Self {
        Self { exchange, market }
    }

    /// "binance:futures" / "bybit:spot" token'ından parse. Borsa bilinmiyorsa `None`;
    /// market kısmı yoksa Spot (tek-kaynak `Exchange::from_token` + `Market::from_label`).
    pub fn parse_token(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let (ex_str, mk_str) = match s.split_once(':') {
            Some((e, m)) => (e, m),
            None => (s, "spot"),
        };
        Some(Self {
            exchange: Exchange::from_token(ex_str)?,
            market: Market::from_label(mk_str),
        })
    }

    /// "exchange:market" token'ı (parse_token ile round-trip).
    pub fn token(&self) -> String {
        format!("{}:{}", self.exchange.as_str(), self.market.as_str())
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
        assert!(Market::Cfd.allows_short(), "CFD (forex/emtia) = short serbest");
    }
}
