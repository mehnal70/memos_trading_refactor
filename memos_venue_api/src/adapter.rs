//! `VenueAdapter` — bir borsa/piyasayı motora tek arayüzle bağlayan soyutlama.
//!
//! Bilinçli olarak iki yetenek trait'ine ayrılmıştır:
//!   * [`MarketData`]      — mum/fiyat/filtre okuma (her venue, BIST dahil, sağlayabilir).
//!   * [`OrderExecution`]  — emir gönderme/iptal/kaldıraç (yalnız işlem yapılabilen venue).
//!
//! [`VenueAdapter`] ikisini + kimliği (exchange/market/asset_class) birleştirir. Veri-only
//! bir kaynak (örn. gecikmeli BIST feed'i) yalnız `MarketData` implement edip
//! `OrderExecution`'ı `unsupported` ile geçebilir → motor `has_live_feed()`/asset_class ile
//! karar verir. Yeni borsa = bu trait'lerin bir implementasyonu; motor çağrı yerlerine dokunma.

use async_trait::async_trait;
use std::sync::Arc;

use crate::contract::{VenueCapabilities, VenueKey};
use crate::error::Result;
use crate::executor::TradeExecutor;
use crate::order::{OrderReceipt, OrderRequest, OrderSide};
use crate::types::{
    AccountSnapshot, AccountSnapshotQuality, AccountSnapshotStatus, AssetClass, Candle, Exchange,
    Market, SymbolFilters,
};

// Not: hata tipi artık `VenueError` (yalın) — core, `From<VenueError> for
// MemosTradingError` köprüsüyle tüketir; `ApiBlocked → ApiBlocked` eşlemesi
// devre-kesiciyi korur (testle mühürlü).

/// Venue'nun İŞLEM PROFİLİ: piyasa yapısına bağlı davranış sınırları.
///
/// TEK KAYNAK BURASI: eskiden `engines/master/venue_profile.rs` bu değerleri
/// `match exchange { Binance => (4, 5.0), Mt5 => (3, 2.5), ... }` ile motorda
/// tutuyordu — yeni venue eklemek motor dosyasını düzenlemek demekti. Artık her
/// venue kendi profilini taşır; motor yalnız `venue.trading_profile()` sorar.
/// (Dikey-dilim hedefi: Adım 5'te bu impl'ler piyasa crate'lerine taşınır.)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TradingProfile {
    /// Kademeli girişte (pyramiding) izin verilen azami kademe sayısı.
    pub max_pyramid_tranches: usize,
    /// Pyramid'e izin veren azami ATR% (volatilite tavanı).
    pub max_atr_pct: f64,
}

impl Default for TradingProfile {
    /// Tanımsız/veri-only venue için muhafazakar taban (eski `_ => (2, 2.0)` kolu).
    fn default() -> Self {
        Self {
            max_pyramid_tranches: 2,
            max_atr_pct: 2.0,
        }
    }
}

/// OTONOM KABİLİYET MATRİSİ: Borsaların çalışma anında taranabilen yetenek listesi.
#[derive(Debug, Clone, PartialEq)]
pub enum ExchangeCapability {
    HistoricalCandles,
    FundingHistory, // Kripto Vadeli (Binance Futures vb.) için fonlama geçmişi
    SwapRates,      // Forex/CFD (MT5 vb.) için gecelik swap/faiz oranları
    Level2OrderBook,
}

/// Piyasa-verisi okuma yeteneği (mum + en iyi alış/satış + emir filtreleri).
/// KESİN ÇÖZÜM: `#[async_trait]` makrosu burayı dyn-compatible yaptığı için,
/// asenkron indirme (download) fonksiyonları güvenle buraya taşınmıştır.
#[async_trait]
pub trait MarketData: Send + Sync {
    /// Son `limit` mum (en yeni sonda). `interval` borsa-doğal TF string'i ("1m","1h","1d").
    async fn fetch_candles(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>>;

    /// En iyi (alış, satış) — maker fiyatlama + spread kapısı için.
    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)>;

    /// Borsa-tarafı emir filtreleri (lot/tick/min-notional). Feed'i olmayan/uygulanmayan
    /// venue varsayılan (sıfır) filtre döndürebilir.
    async fn symbol_filters(&self, symbol: &str) -> Result<SymbolFilters>;

    /// Derin gap durumlarında geriye dönük mum geçmişi pagination indirmesini soyutlar.
    /// Default olarak doğrudan standart fetch_candles'a paslar; backfill destekleyen (Binance vb.) override eder.
    async fn download_candles(
        &self,
        symbol: &str,
        interval: &str,
        _start_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        return self.fetch_candles(symbol, interval, limit).await;
    }

    /// Kripto vadeli işlem (Futures) borsalarının otonom fonlama geçmişi indirmesini soyutlar.
    /// Desteklemeyen borsalar (MT5 vb.) doğrudan boş vec döner, böylece sistem çökmez.
    async fn download_funding_history(
        &self,
        _symbol: &str,
        _market: &str,
        _start_ms: i64,
        _max_req: u32,
    ) -> Result<Vec<(i64, f64)>> {
        return Ok(vec![]);
    }
}

/// Emir yürütme yeteneği. İşlem yapılamayan veri-only venue'lar `Err(unsupported)` döndürür.
#[async_trait]
pub trait OrderExecution: Send + Sync {
    /// Emri ilet ve normalleşmiş sonucu döndür.
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderReceipt>;

    /// Emir öncesi venue filtrelerini uygula (lot/tick/min-notional).
    async fn apply_filters(
        &self,
        _symbol: &str,
        _qty: f64,
        _price: f64,
    ) -> core::result::Result<f64, String> {
        Err("venue emir filtrelerini desteklemiyor".to_string())
    }

    async fn round_price(&self, _symbol: &str, _price: f64) -> core::result::Result<f64, String> {
        Err("venue fiyat filtrelerini desteklemiyor".to_string())
    }

    /// Maker girişini (ve venue'ya özgü re-quote/fallback politikasını) yürüt.
    async fn smart_limit_entry(
        &self,
        _symbol: &str,
        _side: OrderSide,
        _qty: f64,
        _timeout_ms: u64,
        _max_attempts: u32,
        _max_spread_bps: f64,
        _fallback_market: bool,
    ) -> Result<(OrderReceipt, bool)> {
        Err(crate::error::VenueError::Unsupported(
            "smart maker giriş desteklenmiyor".to_string(),
        ))
    }

    /// Koruma emirlerini (SL, TP) yerleştir.
    async fn place_protection_orders(
        &self,
        _symbol: &str,
        _is_long: bool,
        _qty: f64,
        _stop_loss: f64,
        _take_profit: f64,
    ) -> (Result<OrderReceipt>, Result<OrderReceipt>) {
        let err =
            || crate::error::VenueError::Unsupported("koruma emirleri desteklenmiyor".to_string());
        (Err(err()), Err(err()))
    }

    /// Tek bir venue order id'sini iptal et.
    async fn cancel_order(&self, _symbol: &str, _venue_order_id: &str) -> Result<()> {
        Err(crate::error::VenueError::Unsupported(
            "hedefli emir iptali desteklenmiyor".to_string(),
        ))
    }

    /// Açık pozisyonu venue tarafında kapat.
    async fn close_position(&self, _symbol: &str) -> Result<OrderReceipt> {
        Err(crate::error::VenueError::Unsupported(
            "pozisyon kapatma desteklenmiyor".to_string(),
        ))
    }

    /// Sembolün tüm açık emirlerini iptal et (koruma emirleri dahil).
    async fn cancel_all(&self, symbol: &str) -> Result<()>;

    /// Sembol kaldıracını ayarla (spot/kaldıraçsız venue'da no-op). Pozisyon açmadan önce.
    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()>;

    /// Hesap teminat bakiyesi (quote varlık cinsinden, örn. USDT).
    async fn balance(&self) -> Result<f64>;

    /// Hesap görünümü. Eski venue implementasyonları için geriye dönük varsayılan:
    /// balance çağrılır, equity balance kabul edilir ve para birimi bilinmiyor işaretlenir.
    async fn account_snapshot(&self) -> Result<AccountSnapshot> {
        let balance = self.balance().await?;
        let mut snapshot = AccountSnapshot::from_balance(balance);
        snapshot.quality = AccountSnapshotQuality::Estimated;
        snapshot.status = AccountSnapshotStatus::Partial;
        Ok(snapshot)
    }

    /// 🛰️ OTONOM EMİR SORGULAMA KANCASI
    /// Nesne güvenliği (dyn compatible) garantili, iki argümanlı mutlak Result sarmalı.
    async fn fetch_open_orders(&self) -> core::result::Result<Vec<serde_json::Value>, String>;

    // GEREKÇE: Her borsanın (Binance, MT5, IBKR) canlı pozisyonlarını pazar-agnostik
    // bir JSON array halinde çekerek mutabakat motoruna besler.
    async fn fetch_account_positions(&self)
        -> core::result::Result<Vec<serde_json::Value>, String>;

    /// Yakın geçmişte BORSA TARAFINDA kapanmış işlemler (SL/TP/likidasyon/manuel) —
    /// kapalı-işlem mutabakatının beslemesi. Beklenen alanlar: `symbol`, `pnl`,
    /// `commission`, `qty`, `side`, ve dedup için `ticket` (+ `time`, `price`).
    /// Default BOŞ: canlı fill'leri stream'den alan (Binance userdata) ya da geçmiş
    /// API'si henüz bağlanmamış venue'lar için no-op — sistem çökmez, mutabakat
    /// yalnız destekleyen venue'lardan beslenir. (ESKİ durum: bu yetenek trait'te
    /// yoktu; mutabakat MT5'e özel + kendi İKİNCİ köprüsünü kurarak yapılıyordu →
    /// port çekişmesi ve hiç yanıt alamama.)
    async fn fetch_closed_trades(&self) -> core::result::Result<Vec<serde_json::Value>, String> {
        return Ok(vec![]);
    }
}

/// Borsa/piyasayı motora bağlayan tam adaptör: kimlik + veri + (varsa) yürütme.
/// KESİN ÇÖZÜM: `dyn VenueAdapter` pointer'ının kırılmaması için bu ana trait
/// içerisinde kesinlikle hiçbir `async fn` metodu barındırılmamaktadır.
pub trait VenueAdapter: MarketData + OrderExecution {
    fn exchange(&self) -> Exchange;
    fn market(&self) -> Market;
    fn profile(&self) -> crate::types::VenueProfile {
        crate::types::VenueProfile::Default
    }

    /// Venue kimliğinin tek, typed gösterimi. Core bu anahtarı sembol yerine
    /// reconcile ve portföy namespace'lerinde kullanmalıdır.
    fn venue_key(&self) -> VenueKey {
        VenueKey::new(self.exchange(), self.market(), self.profile())
    }

    /// İnsan-okunur ad (log/teşhis). Default: `exchange:market`.
    fn name(&self) -> String {
        return format!("{}:{}", self.exchange().as_str(), self.market().as_str());
    }

    /// İşlenen varlık sınıfı — `Exchange::asset_class()`'ten türer (tek-kaynak).
    fn asset_class(&self) -> AssetClass {
        return self.exchange().asset_class();
    }

    /// Bu venue'nun bu kurulumda canlı feed'i var mı (`Exchange::has_live_feed()` tek-kaynak).
    fn has_live_feed(&self) -> bool {
        return self.exchange().has_live_feed();
    }

    /// 🗄️ KANONİK DB NAMESPACE'i: `(exchange, market)`. Mum tablosunun UNIQUE index'i
    /// `(exchange, market, symbol, interval, timestamp)` olduğu için bu çift bir
    /// enstrümanın hangi kovaya yazılacağını/okunacağını belirler.
    ///
    /// ⚠️ TEK KAYNAK OLMAK ZORUNDA: eskiden yazma yolu (`jobs_download`) MT5 için
    /// `"mt5"`, okuma yolu (`loop_core`) ise `Mt5|Ibkr` için `"cfd"` üretiyordu; ayrıca
    /// diğer okuma yolları global `config.market`'i kullanıyordu. Sonuç: aynı enstrümanın
    /// geçmişi BEŞ ayrı kovaya parçalanmıştı (ölçüldü: AUDUSD 4 kovada, 15m'in 3508 barı
    /// canlı döngünün okuduğu kovanın DIŞINDA kalıyordu).
    ///
    /// Özel-duruma gerek yok: `exchange` mt5 ile ibkr'ı zaten ayırdığı için ikisi de
    /// `Market::Cfd` ("cfd") olabilir.
    fn storage_namespace(&self) -> (&'static str, &'static str) {
        return (self.exchange().as_str(), self.market().as_str());
    }

    /// Gelen hata mesajının borsadan delist edilme / sembol yok hatası olup olmadığını doğrular.
    fn is_delisting_error(&self, error_msg: &str) -> bool;

    /// Borsanın canlı/paper sipariş iletim ve emir yönetim motorunu döner.
    /// Eğer borsa salt veri çekme amaçlıysa (örn: TradingView, BIST veri yayını vb.) None dönebilir.
    fn execution_engine(&self) -> Option<Arc<dyn TradeExecutor>>;

    /// İndirme motorunun (`jobs_download.rs`) veya stratejilerin borsa türünü otonom keşfetmesini sağlar.
    fn capabilities(&self) -> Vec<ExchangeCapability> {
        return vec![ExchangeCapability::HistoricalCandles];
    }

    /// Core'un venue'ya özgü davranışı sorgulaması için typed capability özeti.
    fn venue_capabilities(&self) -> VenueCapabilities {
        VenueCapabilities {
            market_data: self.has_live_feed(),
            order_execution: self.execution_engine().is_some(),
            open_positions: false,
            open_orders: false,
            closed_fills: false,
            native_protection_orders: false,
            supports_short: self.market().allows_short(),
        }
    }

    /// Venue'nun işlem profili (pyramid/ATR sınırları). Default muhafazakar taban;
    /// her gerçek venue kendi değerini override eder.
    fn trading_profile(&self) -> TradingProfile {
        return TradingProfile::default();
    }
}
