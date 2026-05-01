// robot/data_fetcher/hybrid.rs - Hybrid REST + WebSocket fetcher
//
// Combines REST API (historical) with WebSocket (real-time)
// Smart switching based on use case

use crate::types::Candle;
use crate::Result;
use chrono::Utc;

/// Fetcher mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FetchMode {
    /// REST only (backtest, historical data)
    RestOnly,
    
    /// WebSocket only (live trading)
    WebSocketOnly,
    
    /// Hybrid: REST for history, WS for live
    Hybrid,
}

/// Hybrid fetcher combining REST and WebSocket
pub struct HybridBinanceFetcher {
    mode: FetchMode,
}

impl HybridBinanceFetcher {
    pub fn new(mode: FetchMode) -> Self {
        Self { mode }
    }

    /// Get historical candles from REST API
    async fn get_historical(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        let url = format!(
            "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
            symbol, interval, limit
        );

        println!("Fetching: {}", url);

        let response = reqwest::get(&url)
            .await
            .map_err(|_| "HTTP failed")?;

        let data: Vec<Vec<serde_json::Value>> = response
            .json()
            .await
            .map_err(|_| "JSON parse failed")?;

        let mut candles = vec![];
        
        for k in data {
            let ts_ms = k.get(0).and_then(|v| v.as_i64()).unwrap_or(0);
            let open = k.get(1).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let high = k.get(2).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let low = k.get(3).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let close = k.get(4).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let volume = k.get(7).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);

            match chrono::DateTime::from_timestamp(ts_ms / 1000, 0) {
                Some(t) => {
                    candles.push(Candle {
                        timestamp: t.with_timezone(&Utc),
                        open,
                        high,
                        low,
                        close,
                        volume,
                        symbol: symbol.to_string(),
                        interval: interval.to_string(),
                    });
                }
                None => {
                    // Skip invalid timestamps
                }
            }
        }

        Ok(candles)
    }

    /// WebSocket üzerinden canlı mum verisi al
    /// Binance WebSocket API ile bağlantı kurar, ilk gelen klini döndürür (örnek amaçlı, gerçek uygulamada stream/loop ile kullanılmalı)
    async fn get_live(
        &self,
        symbol: &str,
        interval: &str,
    ) -> Result<Vec<Candle>> {
        use tokio_tungstenite::connect_async;
        use futures_util::{StreamExt};
        use crate::robot::data_fetcher::websocket::{BinanceKlineUpdate, parse_kline};

        // Binance WebSocket endpoint'i
        let ws_url = format!(
            "wss://stream.binance.com:9443/ws/{}@kline_{}",
            symbol.to_lowercase(), interval
        );
        // Bağlantı kurmayı dene
        let (ws_stream, _) = connect_async(&ws_url).await.map_err(|e| format!("WebSocket bağlantı hatası: {e}"))?;
        let (mut _write, mut read) = ws_stream.split();

        // İlk kapalı klini bulana kadar dinle
        while let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| format!("WebSocket mesaj hatası: {e}"))?;
            if msg.is_text() {
                let text = msg.to_text().unwrap_or("");
                // JSON parse etmeye çalış
                if let Ok(update) = serde_json::from_str::<BinanceKlineUpdate>(text) {
                    // Sadece kapanmış (x=true) klini al
                    if update.kline.is_closed {
                        let candle = parse_kline(update)?;
                        return Ok(vec![candle]);
                    }
                }
            }
        }
        Err("WebSocket'ten canlı veri alınamadı".into())
    }

    /// Get candles (REST + WebSocket combined)
    pub async fn fetch_latest_hybrid(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        match self.mode {
            FetchMode::RestOnly => {
                self.get_historical(symbol, interval, limit).await
            }
            FetchMode::WebSocketOnly => {
                self.get_live(symbol, interval).await
            }
            FetchMode::Hybrid => {
                let mut candles = self.get_historical(symbol, interval, limit).await?;
                
                if candles.is_empty() {
                    candles = self.get_live(symbol, interval).await?;
                }
                
                Ok(candles)
            }
        }
    }
}

// impl LiveDataFetcher disabled temporarily - error type conflicts
// Will be enabled after WebSocket refactoring

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hybrid_fetcher_creation() {
        let fetcher = HybridBinanceFetcher::new(FetchMode::Hybrid);
        assert_eq!(fetcher.mode, FetchMode::Hybrid);
    }

    #[test]
    fn test_fetch_modes() {
        let rest = HybridBinanceFetcher::new(FetchMode::RestOnly);
        let ws = HybridBinanceFetcher::new(FetchMode::WebSocketOnly);
        let hybrid = HybridBinanceFetcher::new(FetchMode::Hybrid);

        assert_eq!(rest.mode, FetchMode::RestOnly);
        assert_eq!(ws.mode, FetchMode::WebSocketOnly);
        assert_eq!(hybrid.mode, FetchMode::Hybrid);
    }

    #[test]
    fn test_mode_equality() {
        assert_eq!(FetchMode::RestOnly, FetchMode::RestOnly);
        assert_ne!(FetchMode::RestOnly, FetchMode::WebSocketOnly);
    }
}
