// robot/data_fetcher/finnhub.rs - Finnhub API ile veri çekme

use crate::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

pub struct FinnhubFetcher {
    pub api_key: String,
}

impl FinnhubFetcher {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl MarketFetcher for FinnhubFetcher {
    fn name(&self) -> &'static str {
        "finnhub"
    }

    #[allow(deprecated)]
    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        // Finnhub sembolü ör: BIMAS.IS
        let url = format!(
            "https://finnhub.io/api/v1/stock/candle?symbol={}&resolution={}&count={}&token={}",
            symbol, interval, limit, self.api_key
        );
        let client = Client::new();
        let resp = client.get(&url).send().await.map_err(|e| format!("Finnhub isteği başarısız: {}", e))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| format!("Yanıt okunamadı: {}", e))?;
        if !status.is_success() {
            return Err(format!("Finnhub HTTP hata: {} - {}", status, body));
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| format!("JSON parse hatası: {} - {}", e, body))?;
        if v.get("s").and_then(|s| s.as_str()) != Some("ok") {
            return Err(format!("Finnhub API başarısız: {}", body));
        }
        let timestamps = v.get("t").and_then(|a| a.as_array()).ok_or("timestamp alanı yok")?;
        let opens = v.get("o").and_then(|a| a.as_array()).ok_or("open alanı yok")?;
        let highs = v.get("h").and_then(|a| a.as_array()).ok_or("high alanı yok")?;
        let lows = v.get("l").and_then(|a| a.as_array()).ok_or("low alanı yok")?;
        let closes = v.get("c").and_then(|a| a.as_array()).ok_or("close alanı yok")?;
        let volumes = v.get("v").and_then(|a| a.as_array()).ok_or("volume alanı yok")?;
        use chrono::{TimeZone, Utc};
        let mut candles = Vec::new();
        for i in 0..timestamps.len() {
            let ts = timestamps.get(i).and_then(|v| v.as_i64()).unwrap_or(0);
            let timestamp = Utc.timestamp_opt(ts, 0).single().unwrap_or_else(|| Utc.timestamp(0, 0));
            let open = opens.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let high = highs.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let low = lows.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let close = closes.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let volume = volumes.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
            candles.push(Candle {
                timestamp,
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
