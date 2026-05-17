// robot/data_fetcher/bist_fetcher.rs — BIST Yahoo Finance Veri Çekici (Modernize Edilmiş)

use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::robot::data_fetcher::websocket::validate_ohlcv; // Ortak validasyon
use crate::core::types::Candle;
use crate::Result as MemosResult;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const YAHOO_HOSTS: &[&str] = &["query1.finance.yahoo.com", "query2.finance.yahoo.com"];
const MAX_RETRIES: usize = 3;

pub struct BistFetcher {
    client: Client,
}

impl BistFetcher {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent(USER_AGENT)
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl MarketFetcher for BistFetcher {
    fn name(&self) -> &'static str { "bist" }

    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        let mut last_err = String::new();

        // Akıllı Host Rotasyonu (query1 patlarsa query2'ye geçer)
        'outer: for host in YAHOO_HOSTS {
            let url = self.build_url(host, symbol, interval, limit);
            
            for attempt in 1..=MAX_RETRIES {
                match self.client.get(&url).send().await {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.as_u16() == 429 {
                            last_err = format!("[{host}] 429 Hızı Aşma — Diğer hosta geçiliyor");
                            continue 'outer;
                        }
                        if status.is_server_error() {
                            tokio::time::sleep(Duration::seconds(attempt as i64).to_std().unwrap()).await;
                            continue;
                        }
                        if !status.is_success() {
                            return Err(format!("BIST HTTP Hatası: {}", status));
                        }

                        let body = resp.text().await.map_err(|e| e.to_string())?;
                        return self.parse_response(&body, symbol, interval, limit);
                    }
                    Err(e) => {
                        last_err = format!("Ağ Hatası: {}", e);
                        tokio::time::sleep(Duration::milliseconds(500 * attempt as i64).to_std().unwrap()).await;
                    }
                }
            }
        }
        Err(format!("BIST Veri Çekme Başarısız: {}", last_err))
    }
}

impl BistFetcher {
    fn build_url(&self, host: &str, symbol: &str, interval: &str, limit: usize) -> String {
        let period2 = Utc::now().timestamp();
        let candle_secs = self.interval_to_secs(interval);
        // %20 pay bırakarak Yahoo'nun eksik bar riskini minimize ediyoruz
        let period1 = period2 - (candle_secs * (limit as i64) * 12 / 10);
        let base_symbol = symbol.trim_end_matches(".IS");
        
        format!(
            "https://{}/v8/finance/chart/{}.IS?interval={}&period1={}&period2={}",
            host, base_symbol, interval, period1, period2
        )
    }

    fn parse_response(&self, body: &str, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
        let chart = v.pointer("/chart/result/0").ok_or("Yahoo Format Hatası")?;
        
        let timestamps = chart["timestamp"].as_array().ok_or("TS eksik")?;
        let quote = chart.pointer("/indicators/quote/0").ok_or("Veri eksik")?;

        let mut candles = Vec::with_capacity(timestamps.len());
        
        for i in 0..timestamps.len() {
            let ts_sec = timestamps[i].as_i64().unwrap_or(0);
            
            let get_f = |key: &str| quote[key].as_array().and_then(|a| a.get(i)).and_then(|v| v.as_f64()).unwrap_or(0.0);
            
            let open   = get_f("open");
            let high   = get_f("high");
            let low    = get_f("low");
            let close  = get_f("close");
            let volume = get_f("volume");

            // Otonom Kalite Kontrolü (Binance verisiyle aynı süzgeç)
            if validate_ohlcv(open, high, low, close, volume).is_err() { continue; }

            if let Some(dt) = DateTime::from_timestamp(ts_sec, 0) {
                candles.push(Candle {
                    timestamp: dt.with_timezone(&Utc),
                    open, high, low, close, volume,
                    symbol: symbol.trim_end_matches(".IS").to_string(),
                    interval: interval.to_string(),
                });
            }
        }

        if candles.len() > limit {
            Ok(candles[candles.len() - limit..].to_vec())
        } else {
            Ok(candles)
        }
    }

    fn interval_to_secs(&self, interval: &str) -> i64 {
        match interval {
            "1m" => 60, "5m" => 300, "15m" => 900, "1h" => 3600, "1d" => 86400, _ => 86400,
        }
    }
}
