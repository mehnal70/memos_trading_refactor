//! BIST veri-only venue adapter.
//!
//! Yahoo gecikmeli veri sağlar; execution bilinçli olarak kapalıdır. Gerçek
//! BIST aracı kurum entegrasyonu daha sonra aynı adapter sözleşmesini uygular.

use async_trait::async_trait;

use crate::robot::data_fetcher::{BistFetcher, MarketFetcher};
use crate::robot::venue::adapter::{MarketData, OrderExecution, VenueAdapter};
use memos_venue_api::{
    Exchange, ExchangeCapability, Market, OrderReceipt, OrderRequest, Result, SymbolFilters,
    TradingProfile, VenueCapabilities, VenueError, VenueProfile, VenueSpec,
};

pub struct BistVenue {
    fetcher: BistFetcher,
    profile: VenueProfile,
}

impl BistVenue {
    pub fn new(profile: VenueProfile) -> Self {
        Self {
            fetcher: BistFetcher::new(),
            profile,
        }
    }

    fn unsupported<T>(operation: &str) -> Result<T> {
        Err(VenueError::Unsupported(format!(
            "BIST veri-only adapter'ı '{}' desteklemiyor",
            operation
        )))
    }
}

#[async_trait]
impl MarketData for BistVenue {
    async fn fetch_candles(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<memos_venue_api::Candle>> {
        self.fetcher
            .fetch_latest(symbol, interval, limit)
            .await
            .map_err(VenueError::Network)
    }

    async fn book_ticker(&self, _symbol: &str) -> Result<(f64, f64)> {
        Self::unsupported("book_ticker")
    }

    async fn symbol_filters(&self, _symbol: &str) -> Result<SymbolFilters> {
        Ok(SymbolFilters::default())
    }
}

#[async_trait]
impl OrderExecution for BistVenue {
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

    async fn fetch_open_orders(&self) -> core::result::Result<Vec<serde_json::Value>, String> {
        Ok(Vec::new())
    }

    async fn fetch_account_positions(
        &self,
    ) -> core::result::Result<Vec<serde_json::Value>, String> {
        Ok(Vec::new())
    }
}

impl VenueAdapter for BistVenue {
    fn exchange(&self) -> Exchange {
        Exchange::Bist
    }

    fn market(&self) -> Market {
        Market::Spot
    }

    fn profile(&self) -> VenueProfile {
        self.profile
    }

    fn name(&self) -> String {
        VenueSpec::with_profile(self.exchange(), self.market(), self.profile).token()
    }

    fn has_live_feed(&self) -> bool {
        false
    }

    fn capabilities(&self) -> Vec<ExchangeCapability> {
        vec![ExchangeCapability::HistoricalCandles]
    }

    fn venue_capabilities(&self) -> VenueCapabilities {
        VenueCapabilities::data_only()
    }

    fn is_delisting_error(&self, error_msg: &str) -> bool {
        let message = error_msg.to_ascii_lowercase();
        message.contains("not found") || message.contains("no data")
    }

    fn execution_engine(&self) -> Option<std::sync::Arc<dyn memos_venue_api::TradeExecutor>> {
        None
    }

    fn trading_profile(&self) -> TradingProfile {
        TradingProfile {
            max_pyramid_tranches: 1,
            max_atr_pct: 3.0,
        }
    }
}
