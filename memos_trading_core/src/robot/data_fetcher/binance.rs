//! Backwards-compatible Binance fetcher re-exports.
pub use memos_market_binance::{fetch_error_is_delisting, BinanceFetcher, DELISTING_ERR_PREFIX};

use crate::core::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;

#[async_trait::async_trait]
impl MarketFetcher for memos_market_binance::BinanceFetcher {
    fn name(&self) -> &'static str {
        "binance"
    }

    async fn fetch_latest(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>, String> {
        memos_market_binance::BinanceFetcher::fetch_latest(self, symbol, interval, limit).await
    }
}
