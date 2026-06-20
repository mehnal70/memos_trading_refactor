//! `BinanceVenue` — `VenueAdapter`'ın Binance (spot + futures) implementasyonu.
//!
//! Mevcut, savaşta-denenmiş somut tipleri **sarar**, yeniden yazmaz:
//!   * mum/fiyat → [`BinanceFetcher`] (`MarketFetcher`)
//!   * emir/kaldıraç/bakiye/filtre → [`BinanceFuturesExecutor`]
//!
//! Böylece Faz 0 davranış-koruyan kalır: aynı kod, tek arayüz arkasında. Kirli `serde_json::Value`
//! yanıtları burada `OrderReceipt`'e normalleşir; motor borsa-bağımsız tipleri görür.
//!
//! İki kuruluş modu: [`BinanceVenue::data_only`] (auth'suz — yalnız public veri: mum) ve
//! [`BinanceVenue::with_executor`] (auth'lu — veri + yürütme). Mum çekme (`fetch_candles`)
//! executor'a bağlı değildir (public klines); book/filtre/emir executor ister.

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
    /// Auth'lu executor — yürütme + book/filtre için. `None` = data-only venue (yalnız mum).
    executor: Option<Arc<BinanceFuturesExecutor>>,
}

impl BinanceVenue {
    /// Auth'lu venue: veri + yürütme. `executor` market'iyle (Spot/Futures) tutarlı olmalı.
    pub fn with_executor(market: Market, executor: Arc<BinanceFuturesExecutor>) -> Self {
        Self { market, fetcher: BinanceFetcher::new(), executor: Some(executor) }
    }

    /// Data-only venue: yalnız public mum çekme (auth/keys gerekmez). Book/filtre/emir
    /// çağrıldığında açık hata döner. Fiyat-poll/screener gibi salt-veri yolları için.
    pub fn data_only(market: Market) -> Self {
        Self { market, fetcher: BinanceFetcher::new(), executor: None }
    }

    /// Executor'a erişim — data-only venue'da açık hata.
    fn exec(&self) -> Result<&BinanceFuturesExecutor> {
        self.executor
            .as_deref()
            .ok_or_else(|| "BinanceVenue data-only: executor (auth) gerekiyor".to_string().into())
    }
}

#[async_trait]
impl MarketData for BinanceVenue {
    async fn fetch_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        // Public klines — executor'a bağlı değil; data-only venue'da da çalışır.
        self.fetcher
            .fetch_latest_market(symbol, interval, self.market.as_str(), limit)
            .await
            .map_err(Into::into)
    }

    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        self.exec()?.fetch_book_ticker(symbol).await
    }

    async fn symbol_filters(&self, symbol: &str) -> Result<SymbolFilters> {
        self.exec()?.ensure_filters(symbol).await
    }
}

#[async_trait]
impl OrderExecution for BinanceVenue {
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderReceipt> {
        // NOT: `reduce_only` bayrağı bu katmanda taşınıyor ama mevcut executor REST'i
        // `reduceOnly` parametresini henüz almıyor; canlı kapanış üst-katman yolundan
        // (close_paper/live_executor) yönetiliyor. Faz 1 wiring'inde executor'a
        // reduceOnly param eklenecek. Şimdilik market/limit emri olarak iletilir.
        let exec = self.exec()?;
        let side = req.side.as_binance();
        let raw = match req.kind {
            OrderKind::Market => exec.place_market_order(&req.symbol, side, req.qty).await?,
            OrderKind::PostOnlyLimit { price } => {
                exec.place_post_only_limit_order(&req.symbol, side, req.qty, price).await?
            }
        };
        Ok(OrderReceipt::from_binance(raw))
    }

    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        self.exec()?.cancel_all_orders(symbol).await.map(|_| ())
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        self.exec()?.set_leverage(symbol, leverage).await.map(|_| ())
    }

    async fn balance(&self) -> Result<f64> {
        self.exec()?.get_balance().await
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

    fn authed(market: Market) -> BinanceVenue {
        let exec = Arc::new(BinanceFuturesExecutor::new_for_market(
            String::new(),
            String::new(),
            true, // paper/testnet — ağ çağrısı yapılmaz (kimlik/asset_class testi)
            market.as_str(),
        ));
        BinanceVenue::with_executor(market, exec)
    }

    #[test]
    fn identity_is_binance_crypto() {
        let v = authed(Market::Futures);
        assert_eq!(v.exchange(), Exchange::Binance);
        assert_eq!(v.market(), Market::Futures);
        assert_eq!(v.asset_class(), AssetClass::Crypto);
        assert!(v.has_live_feed());
        assert_eq!(v.name(), "binance:futures");
    }

    #[test]
    fn market_axis_is_carried() {
        assert_eq!(authed(Market::Spot).market(), Market::Spot);
        assert_eq!(authed(Market::Coinm).market(), Market::Coinm);
    }

    #[tokio::test]
    async fn data_only_rejects_execution_but_keeps_identity() {
        let v = BinanceVenue::data_only(Market::Futures);
        assert_eq!(v.exchange(), Exchange::Binance);
        assert_eq!(v.market(), Market::Futures);
        // Yürütme/book/filtre → açık hata (ağ çağrısı yapılmaz, executor None).
        assert!(v.balance().await.is_err());
        assert!(v.book_ticker("BTCUSDT").await.is_err());
        assert!(v.set_leverage("BTCUSDT", 5).await.is_err());
    }
}
