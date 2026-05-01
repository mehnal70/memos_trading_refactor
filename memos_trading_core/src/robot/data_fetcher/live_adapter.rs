// robot/data_fetcher/live_adapter.rs — BinanceLiveAdapter
// LiveDataFetcher trait'ini gerçek Binance REST API üzerinden implemente eder.
// stop_signal ve pause_signal ile TUI tarafından durdurulabilir/duraklatılabilir.

use crate::robot::interfaces::LiveDataFetcher;
use crate::robot::data_fetcher::validate_ohlcv;
use crate::types::{Candle, Exchange, FundingRatePoint, Market};
use crate::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Binance REST API tabanlı canlı veri çekici
pub struct BinanceLiveAdapter {
    pub stop_signal: Arc<AtomicBool>,
    pub pause_signal: Arc<AtomicBool>,
}

impl BinanceLiveAdapter {
    pub fn new(stop_signal: Arc<AtomicBool>, pause_signal: Arc<AtomicBool>) -> Self {
        Self { stop_signal, pause_signal }
    }
}

#[async_trait]
impl LiveDataFetcher for BinanceLiveAdapter {
    fn source_name(&self) -> &str {
        "binance-rest"
    }

    async fn fetch_funding_rate(
        &self,
        market: Market,
        symbol: &str,
    ) -> crate::Result<Option<FundingRatePoint>> {
        BinanceLiveAdapter::fetch_funding_rate(self, market, symbol).await
    }

    fn supported_markets(&self) -> Vec<Market> {
        vec![Market::Spot]
    }

    fn supported_symbols(&self, _market: Market) -> Vec<String> {
        vec![
            "BTCUSDT".to_string(),
            "ETHUSDT".to_string(),
            "BNBUSDT".to_string(),
            "SOLUSDT".to_string(),
        ]
    }

    async fn fetch_latest(
        &self,
        _exchange: Exchange,
        market: Market,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        // Duraklatma kontrolü — durdurulana kadar bekle
        while self.pause_signal.load(Ordering::Relaxed) {
            if self.stop_signal.load(Ordering::Relaxed) {
                return Err("RoboticLoop durduruldu".into());
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        // Durdurma kontrolü
        if self.stop_signal.load(Ordering::Relaxed) {
            return Err("RoboticLoop durduruldu".into());
        }

        // Futures için farklı endpoint
        let base_url = match market {
            Market::Futures => "https://fapi.binance.com/fapi/v1/klines",
            _ => "https://api.binance.com/api/v3/klines",
        };

        let url = format!(
            "{}?symbol={}&interval={}&limit={}",
            base_url, symbol, interval, limit
        );

        let resp = reqwest::get(&url)
            .await
            .map_err(|e| format!("Binance HTTP hatası: {}", e))?
            .json::<Vec<Vec<serde_json::Value>>>()
            .await
            .map_err(|e| format!("Binance JSON parse hatası: {}", e))?;

        let mut candles = Vec::with_capacity(resp.len());
        for k in &resp {
            // Timestamp: Binance kline array[0] = açılış zamanı (ms)
            let ts_ms = match k.get(0).and_then(|v| v.as_i64()) {
                Some(t) if t > 0 => t,
                _ => {
                    log::warn!("REST: geçersiz timestamp, candle atlandı: {:?}", k.get(0));
                    continue;
                }
            };

            // OHLCV parse: unwrap_or(0.0) yerine explicit hata — sıfır/geçersiz candle sessizce geçmez
            let open   = match k.get(1).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()) {
                Some(v) => v,
                None => { log::warn!("REST: open parse hatası, atlandı"); continue; }
            };
            let high   = match k.get(2).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()) {
                Some(v) => v,
                None => { log::warn!("REST: high parse hatası, atlandı"); continue; }
            };
            let low    = match k.get(3).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()) {
                Some(v) => v,
                None => { log::warn!("REST: low parse hatası, atlandı"); continue; }
            };
            let close  = match k.get(4).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()) {
                Some(v) => v,
                None => { log::warn!("REST: close parse hatası, atlandı"); continue; }
            };
            // Binance kline: index 5 = quote asset volume, index 7 = taker buy quote volume
            // index 5 doğru quote hacmi; talepte baseAssetVolume için index 5'i kullanıyoruz.
            let volume = match k.get(5).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()) {
                Some(v) => v,
                None => { log::warn!("REST: volume parse hatası, atlandı"); continue; }
            };

            // OHLCV bütünlük kontrolü
            if let Err(e) = validate_ohlcv(open, high, low, close, volume) {
                log::warn!("REST: OHLCV doğrulama hatası (atlandı): {e}");
                continue;
            }

            // from_timestamp_millis: WS yolu ile tutarlı, tam ms hassasiyeti
            if let Some(ts) = chrono::DateTime::from_timestamp_millis(ts_ms) {
                candles.push(Candle {
                    timestamp: ts.with_timezone(&Utc),
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

        Ok(candles)
    }
}

impl BinanceLiveAdapter {
    /// Binance Futures `/fapi/v1/premiumIndex` üzerinden anlık funding rate çeker.
    /// Yalnızca Futures/CoinM piyasası için anlamlıdır; Spot'ta `Ok(None)` döner.
    pub async fn fetch_funding_rate(
        &self,
        market: Market,
        symbol: &str,
    ) -> Result<Option<FundingRatePoint>> {
        if !matches!(market, Market::Futures | Market::Coinm) {
            return Ok(None);
        }
        let base = if matches!(market, Market::Coinm) {
            "https://dapi.binance.com/dapi/v1/premiumIndex"
        } else {
            "https://fapi.binance.com/fapi/v1/premiumIndex"
        };
        let url = format!("{}?symbol={}", base, symbol);
        let json: serde_json::Value = reqwest::get(&url)
            .await
            .map_err(|e| format!("funding rate HTTP hatası: {}", e))?
            .json()
            .await
            .map_err(|e| format!("funding rate JSON parse hatası: {}", e))?;

        let rate = json["lastFundingRate"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let mark_price = json["markPrice"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok());
        let ts = json["time"]
            .as_i64()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .unwrap_or_else(chrono::Utc::now);

        Ok(Some(FundingRatePoint {
            timestamp: ts.with_timezone(&chrono::Utc),
            symbol: symbol.to_string(),
            funding_rate: rate,
            mark_price,
        }))
    }
}
