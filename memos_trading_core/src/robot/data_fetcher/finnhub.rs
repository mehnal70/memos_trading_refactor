// robot/data_fetcher/finnhub.rs - Finnhub API Veri Çekici (Modernize Edilmiş)

use crate::core::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::robot::data_fetcher::websocket::validate_ohlcv; // Merkezi validasyon
use async_trait::async_trait;
use chrono::{DateTime, Utc, TimeZone};
use reqwest::Client;
use serde_json::Value;

pub struct FinnhubFetcher {
    pub api_key: String,
    client: Client,
}

impl FinnhubFetcher {
    pub fn new(api_key: String) -> Self {
        Self { 
            api_key,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl MarketFetcher for FinnhubFetcher {
    fn name(&self) -> &'static str { "finnhub" }

    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        // Finnhub resolution eşlemesi (Gerekiyorsa buraya eklenebilir, örn: "1h" -> "60")
        let resolution = match interval {
            "1m" => "1",
            "5m" => "5",
            "15m" => "15",
            "30m" => "30",
            "1h" => "60",
            "1d" => "D",
            _ => interval,
        };

        let url = format!(
            "https://finnhub.io/api/v1/stock/candle?symbol={}&resolution={}&count={}&token={}",
            symbol, resolution, limit, self.api_key
        );

        let resp = self.client.get(&url).send().await
            .map_err(|e| format!("Finnhub isteği başarısız: {}", e))?;
        
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("Finnhub HTTP hata: {}", status));
        }

        let v: Value = resp.json().await.map_err(|e| format!("JSON parse hatası: {}", e))?;
        
        // Finnhub "s" (status) kontrolü
        if v.get("s").and_then(|s| s.as_str()) != Some("ok") {
            return Ok(Vec::new()); // Veri yoksa boş dön, hata fırlatma (graceful)
        }

        let t = v["t"].as_array().ok_or("timestamp eksik")?;
        let o = v["o"].as_array().ok_or("open eksik")?;
        let h = v["h"].as_array().ok_or("high eksik")?;
        let l = v["l"].as_array().ok_or("low eksik")?;
        let c = v["c"].as_array().ok_or("close eksik")?;
        let v_arr = v["v"].as_array().ok_or("volume eksik")?;

        let mut candles = Vec::with_capacity(t.len());
        
        for i in 0..t.len() {
            let ts_sec = t[i].as_i64().unwrap_or(0);
            let open   = o[i].as_f64().unwrap_or(0.0);
            let high   = h[i].as_f64().unwrap_or(0.0);
            let low    = l[i].as_f64().unwrap_or(0.0);
            let close  = c[i].as_f64().unwrap_or(0.0);
            let volume = v_arr[i].as_f64().unwrap_or(0.0);

            // Otonom Kalite Kontrolü
            if validate_ohlcv(open, high, low, close, volume).is_err() { continue; }

            let timestamp = Utc.timestamp_opt(ts_sec, 0)
                    .single()
                    .unwrap_or_else(|| Utc::now()); // || operatörü bir fonksiyon tanımlar

            candles.push(Candle {
                timestamp,
                open, high, low, close, volume,
                symbol: symbol.to_string(),
                interval: interval.to_string(),
            });
        }

        Ok(candles)
    }
}
