// robot/data_fetcher/bist_fetcher.rs — BIST veri çekici (Yahoo Finance).
//
// DRY (çoklu-piyasa): Yahoo chart HTTP+parse çekirdeği artık [`super::yahoo::YahooFetcher`]'da
// (tek Yahoo-parse yolu, dünya piyasalarıyla paylaşılır). BistFetcher yalnız BIST kabuğu:
// sembolü `.IS` ticker'ına eşler ve YahooFetcher'a delege eder. [[project_world_markets]]

use crate::core::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::robot::data_fetcher::yahoo::YahooFetcher;
use crate::robot::data_fetcher::IsYatirimFetcher;
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
        Self {
            inner: YahooFetcher::new(),
        }
    }

    /// Yahoo'nun kısa intraday aralıklarını açıkça kullan; günlük akışın
    /// uzun-range davranışına dokunmaz.
    pub async fn fetch_intraday(
        &self,
        symbol: &str,
        interval: &str,
        range: &str,
        limit: usize,
    ) -> Result<Vec<Candle>, String> {
        let base = symbol.trim_end_matches(".IS");
        let ticker = YahooFetcher::yahoo_ticker("bist", base);
        let mut candles = self
            .inner
            .fetch_daily(&ticker, base, interval, range)
            .await?;
        if candles.len() > limit {
            candles = candles.split_off(candles.len() - limit);
        }
        Ok(candles)
    }
}

#[async_trait]
impl MarketFetcher for BistFetcher {
    fn name(&self) -> &'static str {
        "bist"
    }

    async fn fetch_latest(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>, String> {
        // Çıplak sembol (`.IS`'siz) sakla; Yahoo'ya `.IS`-ekli ticker ile git.
        let base = symbol.trim_end_matches(".IS");
        let provider = std::env::var("BIST_DATA_PROVIDER")
            .unwrap_or_else(|_| "auto".into())
            .trim()
            .to_ascii_lowercase();
        if provider == "google_sheets" || provider == "google" || provider == "auto" {
            match crate::robot::bist_trend::fetch_google_sheet_candles(symbol, interval, limit)
                .await
            {
                Ok(candles) => return Ok(candles),
                Err(error) if provider == "google_sheets" || provider == "google" => {
                    return Err(format!("Google Sheets BIST kaynağı başarısız: {error}"));
                }
                Err(_) => {}
            }
        }
        let ticker = YahooFetcher::yahoo_ticker("bist", base);
        let range = if limit > 1300 {
            "10y"
        } else if limit > 260 {
            "5y"
        } else {
            "2y"
        };
        let yahoo_result = self.inner.fetch_daily(&ticker, base, interval, range).await;
        let mut candles = match yahoo_result {
            Ok(candles) => candles,
            Err(yahoo_error)
                if interval.eq_ignore_ascii_case("1d")
                    && (provider == "isyatirim"
                        || std::env::var("BIST_ISYATIRIM_FALLBACK")
                            .map(|value| {
                                !matches!(
                                    value.trim().to_ascii_lowercase().as_str(),
                                    "0" | "false" | "off" | "no"
                                )
                            })
                            .unwrap_or(false)) =>
            {
                IsYatirimFetcher::new()
                    .fetch_daily(base, base, 2)
                    .await
                    .map_err(|isy_error| {
                        format!("Yahoo başarısız: {yahoo_error}; İş Yatırım fallback başarısız: {isy_error}")
                    })?
            }
            Err(error) => return Err(error),
        };
        if candles.len() > limit {
            candles = candles.split_off(candles.len() - limit);
        }
        Ok(candles)
    }
}
