// Ortak fetcher arayüzü (trait)
use crate::core::types::Candle;
use async_trait::async_trait;

#[async_trait]
pub trait MarketFetcher: Send + Sync {
    fn name(&self) -> &'static str;
    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String>;
}
