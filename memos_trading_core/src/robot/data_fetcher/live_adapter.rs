// robot/data_fetcher/live_adapter.rs — BinanceLiveAdapter (Modernize Edilmiş)

// robot/data_fetcher/live_adapter.rs - Gelişmiş Canlı Veri Adaptörü (Srivastava ATP)

use crate::robot::infra::interfaces::LiveDataFetcher;
use crate::robot::data_fetcher::websocket::validate_ohlcv;
use crate::core::types::{Candle, Exchange, FundingRatePoint, Market};
use crate::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Binance REST API tabanlı canlı veri çekici - Çok Kanallı ve Hata Bağışıklıklı
pub struct BinanceLiveAdapter {
    pub stop_signal: Arc<AtomicBool>,
    pub pause_signal: Arc<AtomicBool>,
    client: reqwest::Client,
}

impl BinanceLiveAdapter {
    pub fn new(stop_signal: Arc<AtomicBool>, pause_signal: Arc<AtomicBool>) -> Self {
        Self { 
            stop_signal, 
            pause_signal,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(12))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl LiveDataFetcher for BinanceLiveAdapter {
    fn source_name(&self) -> &str { "binance-rest-v2" }

    async fn fetch_funding_rate(&self, market: Market, symbol: &str) -> crate::Result<Option<FundingRatePoint>> {
        self.internal_fetch_funding_rate(market, symbol).await
    }

    fn supported_markets(&self) -> Vec<Market> {
        vec![Market::Spot, Market::Futures, Market::Coinm]
    }

    fn supported_symbols(&self, _market: Market) -> Vec<String> {
        vec!["BTCUSDT".to_string(), "ETHUSDT".to_string(), "SOLUSDT".to_string()]
    }

    async fn fetch_latest(&self, _exchange: Exchange, market: Market, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        if self.should_halt().await? { return Err("Fetcher durduruldu".into()); }

        let base_url = match market {
            Market::Futures => "https://binance.com",
            Market::Coinm   => "https://binance.com",
            _               => "https://binance.com",
        };

        let url = format!("{}?symbol={}&interval={}&limit={}", base_url, symbol, interval, limit);
        let resp = self.client.get(&url).send().await.map_err(|e| format!("Binance Bağlantı Hatası: {}", e))?;
        
        let status = resp.status();
        if !status.is_success() {
            let err_txt = resp.text().await.unwrap_or_default();
            return Err(format!("Binance API Hatası ({}): {}", status, err_txt).into());
        }

        let raw_data = resp.json::<Vec<Vec<serde_json::Value>>>().await?;
        Ok(raw_data.into_iter().filter_map(|k| self.parse_single_kline(k, symbol, interval)).collect())
    }
}

impl BinanceLiveAdapter {
    async fn should_halt(&self) -> Result<bool> {
        while self.pause_signal.load(Ordering::Relaxed) {
            if self.stop_signal.load(Ordering::Relaxed) { return Ok(true); }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Ok(self.stop_signal.load(Ordering::Relaxed))
    }

    /// Tekil bir kline verisini Srivastava ATP standartlarında doğrular ve mühürler.
    fn parse_single_kline(&self, k: Vec<serde_json::Value>, symbol: &str, interval: &str) -> Option<Candle> {
        let ts_ms = k.first()?.as_i64()?;
        
        let parse_f64 = |idx: usize| {
            k.get(idx)?.as_str()?.parse::<f64>().ok()
        };

        let (open, high, low, close, volume) = (
            parse_f64(1)?, parse_f64(2)?, parse_f64(3)?, parse_f64(4)?, parse_f64(5)?
        );

        // Otonom OHLCV Doğrulaması
        if validate_ohlcv(open, high, low, close, volume).is_err() { return None; }

        chrono::DateTime::from_timestamp_millis(ts_ms).map(|ts| Candle {
            timestamp: ts.with_timezone(&Utc),
            open, high, low, close, volume,
            symbol: symbol.to_string(),
            interval: interval.to_string(),
        })
    }

    /// Futures piyasası için Funding Rate ve Mark Price verilerini çeker.
    async fn internal_fetch_funding_rate(&self, market: Market, symbol: &str) -> Result<Option<FundingRatePoint>> {
        if !matches!(market, Market::Futures | Market::Coinm) { return Ok(None); }
        
        let url = match market {
            Market::Futures => format!("https://binance.com{}", symbol),
            Market::Coinm   => format!("https://binance.com{}", symbol),
            _ => return Ok(None),
        };

        let json: serde_json::Value = self.client.get(&url).send().await?.json().await?;

        let rate = json["lastFundingRate"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        let mark = json["markPrice"].as_str().and_then(|s| s.parse::<f64>().ok());
        let ts_ms = json["time"].as_i64().unwrap_or_else(|| Utc::now().timestamp_millis());

        Ok(Some(FundingRatePoint {
            timestamp: chrono::DateTime::from_timestamp_millis(ts_ms).unwrap_or_else(Utc::now).with_timezone(&Utc),
            symbol: symbol.to_string(),
            funding_rate: rate,
            mark_price: mark,
        }))
    }
}
