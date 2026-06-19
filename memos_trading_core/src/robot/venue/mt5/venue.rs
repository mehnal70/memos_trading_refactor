//! `Mt5Venue` — `VenueAdapter`'ın MetaTrader 5 implementasyonu (forex/emtia/endeks CFD).
//!
//! **Faz 1 = veri venue'su:** `fetch_candles`/`book_ticker`/`symbol_filters` yerel MT5 EA
//! köprüsüne ([`Mt5Bridge`]) gider — yeni piyasada önce izole edge ölçümü duvarı
//! ([[project_world_markets]]). Yürütme (`submit_order`/...) **Faz 2**'dir: edge doğrulanınca
//! açılır. Şimdilik sahte başarı DÖNMEZ — açık `Err` döner (Bybit veri-venue deseni).
//!
//! Sembol biçimi (EURUSD/XAUUSD) BIST equity heuristic'iyle çakışır → MT5 sembolleri
//! `Exchange::classify` ile OTO-sınıflanmaz; explicit routing (`SYM@mt5`) ile kullanılır.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core::model::SymbolFilters;
use crate::core::types::{Candle, Exchange, Market};
use crate::robot::venue::adapter::{MarketData, OrderExecution, VenueAdapter};
use crate::robot::venue::mt5::bridge::Mt5Bridge;
use crate::robot::venue::mt5::protocol;
use crate::robot::venue::types::{OrderReceipt, OrderRequest};
use crate::Result;

pub struct Mt5Venue {
    market: Market,
    bridge: Arc<Mt5Bridge>,
}

impl Mt5Venue {
    /// Verilen köprü üzerinden MT5 venue (market = futures/spot; CFD'ler çoğunlukla "spot"
    /// muamelesi görür, short ayrımı için Futures verilebilir).
    pub fn new(market: Market, bridge: Arc<Mt5Bridge>) -> Self {
        Self { market, bridge }
    }

    /// Köprüye erişim (paylaşımlı; aynı EA bağlantısını tüm semboller kullanır).
    pub fn bridge(&self) -> &Arc<Mt5Bridge> {
        &self.bridge
    }

    /// Yürütme henüz yok (Faz 2) — sahte değer DÖNMEZ, açık hata döner.
    fn unsupported<T>(what: &str) -> Result<T> {
        Err(format!(
            "MT5 {what} henüz uygulanmadı (Faz 2 yürütme katmanı) — şu an veri-only venue"
        )
        .into())
    }
}

#[async_trait]
impl MarketData for Mt5Venue {
    async fn fetch_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        let id = self.bridge.next_id();
        let line = protocol::req_candles(id, symbol, interval, limit.clamp(1, 5000));
        let resp = self.bridge.request(&line).await?;
        protocol::parse_candles(symbol, interval, &resp)
    }

    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        let id = self.bridge.next_id();
        let line = protocol::req_tick(id, symbol);
        let resp = self.bridge.request(&line).await?;
        protocol::parse_tick(symbol, &resp)
    }

    async fn symbol_filters(&self, symbol: &str) -> Result<SymbolFilters> {
        let id = self.bridge.next_id();
        let line = protocol::req_filters(id, symbol);
        let resp = self.bridge.request(&line).await?;
        protocol::parse_filters(symbol, &resp)
    }
}

#[async_trait]
impl OrderExecution for Mt5Venue {
    async fn submit_order(&self, _req: &OrderRequest) -> Result<OrderReceipt> {
        Self::unsupported("submit_order")
    }
    async fn cancel_all(&self, _symbol: &str) -> Result<()> {
        Self::unsupported("cancel_all")
    }
    async fn set_leverage(&self, _symbol: &str, _leverage: u32) -> Result<()> {
        Self::unsupported("set_leverage")
    }
    async fn balance(&self) -> Result<f64> {
        Self::unsupported("balance")
    }
}

impl VenueAdapter for Mt5Venue {
    fn exchange(&self) -> Exchange {
        Exchange::Mt5
    }
    fn market(&self) -> Market {
        self.market
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::AssetClass;

    #[test]
    fn identity_is_mt5_forex() {
        let v = Mt5Venue::new(Market::Spot, Arc::new(Mt5Bridge::with_defaults(None)));
        assert_eq!(v.exchange(), Exchange::Mt5);
        assert_eq!(v.asset_class(), AssetClass::Forex);
        assert!(v.has_live_feed());
        assert_eq!(v.name(), "mt5:spot");
    }

    #[tokio::test]
    async fn execution_is_explicit_error_not_fake_success() {
        let v = Mt5Venue::new(Market::Spot, Arc::new(Mt5Bridge::with_defaults(None)));
        assert!(v.balance().await.is_err());
        assert!(v.cancel_all("EURUSD").await.is_err());
        assert!(v
            .submit_order(&OrderRequest::market(
                "EURUSD",
                crate::robot::venue::types::OrderSide::Buy,
                0.1
            ))
            .await
            .is_err());
    }
}
