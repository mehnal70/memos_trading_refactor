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

use crate::core::model::SymbolFilters;
use crate::core::types::{AssetClass, Candle, Exchange, Market};
use crate::robot::venue::types::{OrderReceipt, OrderRequest};
use crate::Result;

/// Piyasa-verisi okuma yeteneği (mum + en iyi alış/satış + emir filtreleri).
#[async_trait]
pub trait MarketData: Send + Sync {
    /// Son `limit` mum (en yeni sonda). `interval` borsa-doğal TF string'i ("1m","1h","1d").
    async fn fetch_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>>;

    /// En iyi (alış, satış) — maker fiyatlama + spread kapısı için.
    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)>;

    /// Borsa-tarafı emir filtreleri (lot/tick/min-notional). Feed'i olmayan/uygulanmayan
    /// venue varsayılan (sıfır) filtre döndürebilir.
    async fn symbol_filters(&self, symbol: &str) -> Result<SymbolFilters>;
}

/// Emir yürütme yeteneği. İşlem yapılamayan veri-only venue'lar `Err(unsupported)` döndürür.
#[async_trait]
pub trait OrderExecution: Send + Sync {
    /// Emri ilet ve normalleşmiş sonucu döndür.
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderReceipt>;

    /// Sembolün tüm açık emirlerini iptal et (koruma emirleri dahil).
    async fn cancel_all(&self, symbol: &str) -> Result<()>;

    /// Sembol kaldıracını ayarla (spot/kaldıraçsız venue'da no-op). Pozisyon açmadan önce.
    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()>;

    /// Hesap teminat bakiyesi (quote varlık cinsinden, örn. USDT).
    async fn balance(&self) -> Result<f64>;
}

/// Borsa/piyasayı motora bağlayan tam adaptör: kimlik + veri + (varsa) yürütme.
pub trait VenueAdapter: MarketData + OrderExecution {
    fn exchange(&self) -> Exchange;
    fn market(&self) -> Market;

    /// İnsan-okunur ad (log/teşhis). Default: `exchange:market`.
    fn name(&self) -> String {
        format!("{}:{}", self.exchange().as_str(), self.market().as_str())
    }

    /// İşlenen varlık sınıfı — `Exchange::asset_class()`'ten türer (tek-kaynak).
    fn asset_class(&self) -> AssetClass {
        self.exchange().asset_class()
    }

    /// Bu venue'nun bu kurulumda canlı feed'i var mı (`Exchange::has_live_feed()` tek-kaynak).
    fn has_live_feed(&self) -> bool {
        self.exchange().has_live_feed()
    }
}
