// robot/data_fetcher/binance.rs - Binance REST API data fetcher - SIMPLE VERSION

use crate::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use async_trait::async_trait;


pub struct BinanceFetcher;

impl BinanceFetcher {
    pub fn new() -> Self {
        BinanceFetcher
    }
}

#[async_trait]
impl MarketFetcher for BinanceFetcher {
    fn name(&self) -> &'static str { "binance" }
    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        let url = format!(
            "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
            symbol, interval, limit
        );
        println!("Fetching: {}", url);
        let resp = reqwest::get(&url)
            .await
            .map_err(|_| "HTTP request failed")?
            .json::<Vec<Vec<serde_json::Value>>>()
            .await
            .map_err(|_| "JSON parse failed")?;
        let mut candles = vec![];
        for k in resp {
            let ts_ms = match k.get(0).and_then(|v| v.as_i64()) {
                Some(ts) => ts,
                None => continue,
            };
            let ts = if let Some(t) = chrono::DateTime::from_timestamp(ts_ms / 1000, 0) {
                t.with_timezone(&chrono::Utc)
            } else {
                continue;
            };
            let open:   f64 = k.get(1).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let high:   f64 = k.get(2).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let low:    f64 = k.get(3).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let close:  f64 = k.get(4).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let volume: f64 = k.get(7).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            // Bozuk OHLCV (high=0, high<open, vb.) atlanır — aşağı akışta sahte
            // sinyal/risk hesabı tetiklemesin.
            if !(open > 0.0 && high > 0.0 && low > 0.0 && close > 0.0) { continue; }
            if high < open.max(close) || low > open.min(close) || high < low { continue; }
            candles.push(Candle {
                timestamp: ts,
                open,
                high,
                low,
                close,
                volume,
                symbol: symbol.to_string(),
                interval: interval.to_string(),
            });
        }
        Ok(candles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetcher_creation() {
        let _ = BinanceFetcher::new();
        assert!(true);
    }
}
