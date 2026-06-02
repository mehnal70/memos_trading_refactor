// robot/data_fetcher/binance.rs - Binance REST API Veri Çekici (Modernize Edilmiş)

use crate::core::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::robot::data_fetcher::websocket::validate_ohlcv; 
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::time::Duration;

pub struct BinanceFetcher {
    client: reqwest::Client,
}

impl Default for BinanceFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl BinanceFetcher {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15)) // Ağ gecikmelerine karşı tolerans artırıldı
                .build()
                .unwrap_or_default(),
        }
    }

    /// Market-farkında klines endpoint'i: futures → fapi.binance.com/fapi/v1,
    /// diğer (spot) → api.binance.com/api/v3. Eskiden fetcher SABİT spot endpoint'ine
    /// vuruyordu → futures botu spot veriyle karar veriyordu (Faz 2 correctness).
    fn klines_base(market: &str) -> &'static str {
        if market.eq_ignore_ascii_case("futures") {
            "https://fapi.binance.com/fapi/v1/klines"
        } else {
            "https://api.binance.com/api/v3/klines"
        }
    }

    /// Market-farkında son N mum. `fetch_latest` bunun spot kısayoludur (geriye-uyum).
    pub async fn fetch_latest_market(
        &self, symbol: &str, interval: &str, market: &str, limit: usize,
    ) -> Result<Vec<Candle>, String> {
        let url = format!(
            "{}?symbol={}&interval={}&limit={}",
            Self::klines_base(market), symbol, interval, limit
        );
        self.fetch_klines(&url, symbol, interval).await
    }

    /// Ortak klines parse çekirdeği (spot/futures aynı payload formatı).
    async fn fetch_klines(&self, url: &str, symbol: &str, interval: &str) -> Result<Vec<Candle>, String> {
        let resp = self.client.get(url)
            .send()
            .await
            .map_err(|e| format!("Binance Bağlantı Hatası: {}", e))?
            .json::<Vec<Vec<serde_json::Value>>>()
            .await
            .map_err(|e| format!("Binance Veri Format Hatası: {}", e))?;

        let mut candles = Vec::with_capacity(resp.len());
        
        for k in resp {
            // 1. Zaman Damgası Kontrolü (i64 ms)
            let ts_ms = match k.first().and_then(|v| v.as_i64()) {
                Some(ts) if ts > 0 => ts,
                _ => continue,
            };

            // 2. Sayısal Verilerin Güvenli Parse Edilmesi
            // Binance verileri string döner, bu yüzden as_str() üzerinden parse ediyoruz.
            let parse_f = |idx: usize| {
                k.get(idx)
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
            };

            let open   = parse_f(1).unwrap_or(0.0);
            let high   = parse_f(2).unwrap_or(0.0);
            let low    = parse_f(3).unwrap_or(0.0);
            let close  = parse_f(4).unwrap_or(0.0);
            
            // §12.3: Taker Buy Quote Asset Volume (Index 7) 
            // Bu değer, piyasa alıcılarının (agresif işlemler) gerçek hacmini gösterir.
            let volume = parse_f(7).unwrap_or(0.0);

            // 3. Otonom Veri Doğrulama (validate_ohlcv)
            // Sadece matematiksel olarak tutarlı mumlar boru hattına girebilir.
            if validate_ohlcv(open, high, low, close, volume).is_err() {
                continue;
            }

            if let Some(dt) = DateTime::from_timestamp_millis(ts_ms) {
                candles.push(Candle {
                    timestamp: dt.with_timezone(&Utc),
                    open,
                    high,
                    low,
                    close,
                    volume,
                    symbol: symbol.to_string(),
                    interval: interval.to_string(),
                });
            }
        }

        if candles.is_empty() {
            return Err(format!("{} sembolü için geçerli mum verisi alınamadı", symbol));
        }

        Ok(candles)
    }
}

#[async_trait]
impl MarketFetcher for BinanceFetcher {
    fn name(&self) -> &'static str { "binance" }

    /// Trait yolu spot kısayolu (geriye-uyum). Market-farkında çağrılar
    /// `fetch_latest_market` kullanmalı (download job Faz 2'de geçti).
    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        self.fetch_latest_market(symbol, interval, "spot", limit).await
    }
}
