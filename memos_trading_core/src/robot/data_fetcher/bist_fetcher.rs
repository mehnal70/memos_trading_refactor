// robot/data_fetcher/bist_fetcher.rs — BIST veri çekici (Yahoo Finance).
//
// DRY (çoklu-piyasa): Yahoo chart HTTP+parse çekirdeği artık [`super::yahoo::YahooFetcher`]'da
// (tek Yahoo-parse yolu, dünya piyasalarıyla paylaşılır). BistFetcher yalnız BIST kabuğu:
// sembolü `.IS` ticker'ına eşler ve YahooFetcher'a delege eder. [[project_world_markets]]

use crate::core::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::robot::data_fetcher::yahoo::YahooFetcher;
use async_trait::async_trait;

pub struct BistFetcher {
    inner: YahooFetcher,
}

impl Default for BistFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl BistFetcher {
    pub fn new() -> Self {
        Self { inner: YahooFetcher::new() }
    }
}

#[async_trait]
impl MarketFetcher for BistFetcher {
    fn name(&self) -> &'static str {
        "bist"
    }

    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        // Çıplak sembol (`.IS`'siz) sakla; Yahoo'ya `.IS`-ekli ticker ile git.
        let base = symbol.trim_end_matches(".IS");
        let ticker = YahooFetcher::yahoo_ticker("bist", base);
        let range = if limit > 1300 { "10y" } else if limit > 260 { "5y" } else { "2y" };
        let mut candles = self.inner.fetch_daily(&ticker, base, interval, range).await?;
        if candles.len() > limit {
            candles = candles.split_off(candles.len() - limit);
        }
        Ok(candles)
    }
}
