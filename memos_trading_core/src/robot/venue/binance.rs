//! `BinanceVenue` — `VenueAdapter`'ın Binance (spot + futures) implementasyonu.
//!
//! Mevcut, savaşta-denenmiş somut tipleri **sarar**, yeniden yazmaz:
//!   * mum/fiyat → [`BinanceFetcher`] (`MarketFetcher`)
//!   * emir/kaldıraç/bakiye/filtre → [`BinanceFuturesExecutor`]
//!
//! Böylece Faz 0 davranış-koruyan kalır: aynı kod, tek arayüz arkasında. Kirli `serde_json::Value`
//! yanıtları burada `OrderReceipt`'e normalleşir; motor borsa-bağımsız tipleri görür.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core::model::SymbolFilters;
use crate::core::types::{Candle, Exchange, Market};
use crate::robot::data_fetcher::BinanceFetcher;
use crate::robot::engines::binance_executor::BinanceFuturesExecutor;
use crate::robot::venue::adapter::{MarketData, OrderExecution, VenueAdapter};
use crate::robot::venue::types::{OrderKind, OrderReceipt, OrderRequest};
use crate::Result;

pub struct BinanceVenue {
    market: Market,
    fetcher: BinanceFetcher,
    executor: Arc<BinanceFuturesExecutor>,
}

impl BinanceVenue {
    /// Verilen market (Spot/Futures/Coinm) ve auth'lu executor ile bir Binance venue'su kur.
    /// Mum çekme için içeride taze bir `BinanceFetcher` tutulur (durumsuz, ucuz).
    pub fn new(market: Market, executor: Arc<BinanceFuturesExecutor>) -> Self {
        Self { market, fetcher: BinanceFetcher::new(), executor }
    }
}

#[async_trait]
impl MarketData for BinanceVenue {
    async fn fetch_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        self.fetcher
            .fetch_latest_market(symbol, interval, self.market.as_str(), limit)
            .await
            .map_err(Into::into)
    }

    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        self.executor.fetch_book_ticker(symbol).await
    }

    async fn symbol_filters(&self, symbol: &str) -> Result<SymbolFilters> {
        self.executor.ensure_filters(symbol).await
    }
}

#[async_trait]
impl OrderExecution for BinanceVenue {
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderReceipt> {
        // NOT: `reduce_only` bayrağı bu katmanda taşınıyor ama mevcut executor REST'i
        // `reduceOnly` parametresini henüz almıyor; canlı kapanış üst-katman yolundan
        // (close_paper/live_executor) yönetiliyor. Faz 1 wiring'inde executor'a
        // reduceOnly param eklenecek. Şimdilik market/limit emri olarak iletilir.
        let side = req.side.as_binance();
        let raw = match req.kind {
            OrderKind::Market => {
                self.executor.place_market_order(&req.symbol, side, req.qty).await?
            }
            OrderKind::PostOnlyLimit { price } => {
                self.executor
                    .place_post_only_limit_order(&req.symbol, side, req.qty, price)
                    .await?
            }
        };
        Ok(OrderReceipt::from_binance(raw))
    }

    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        self.executor.cancel_all_orders(symbol).await.map(|_| ())
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        self.executor.set_leverage(symbol, leverage).await.map(|_| ())
    }

    async fn balance(&self) -> Result<f64> {
        self.executor.get_balance().await
    }
}

impl VenueAdapter for BinanceVenue {
    fn exchange(&self) -> Exchange {
        Exchange::Binance
    }
    fn market(&self) -> Market {
        self.market
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::AssetClass;

    fn venue(market: Market) -> BinanceVenue {
        let exec = Arc::new(BinanceFuturesExecutor::new_for_market(
            String::new(),
            String::new(),
            true, // paper/testnet — ağ çağrısı yapılmaz (kimlik/asset_class testi)
            market.as_str(),
        ));
        BinanceVenue::new(market, exec)
    }

    #[test]
    fn identity_is_binance_crypto() {
        let v = venue(Market::Futures);
        assert_eq!(v.exchange(), Exchange::Binance);
        assert_eq!(v.market(), Market::Futures);
        assert_eq!(v.asset_class(), AssetClass::Crypto);
        assert!(v.has_live_feed());
        assert_eq!(v.name(), "binance:futures");
    }

    #[test]
    fn market_axis_is_carried() {
        assert_eq!(venue(Market::Spot).market(), Market::Spot);
        assert_eq!(venue(Market::Coinm).market(), Market::Coinm);
    }
}
