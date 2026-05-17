// robot/data_fetcher/hybrid.rs - Hibrit REST + WebSocket Veri Motoru

use crate::core::types::{Candle, Exchange, Market, FundingRatePoint};
use crate::robot::infra::interfaces::LiveDataFetcher;
use crate::robot::data_fetcher::binance::BinanceFetcher;
use crate::robot::data_fetcher::websocket::{parse_kline, validate_ohlcv, BinanceKlineUpdate};
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use std::sync::Arc;
use tokio_tungstenite::connect_async;

/// Veri çekme modu: Otonom karar mekanizması
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FetchMode {
    RestOnly,      // Sadece Geçmiş (Backtest)
    WebSocketOnly, // Sadece Canlı (Hızlı Scalping)
    Hybrid,        // Hibrit: REST ile doldur, WS ile güncelle (Default)
}

/// HybridBinanceFetcher: REST ve WebSocket güçlerini birleştirir
pub struct HybridBinanceFetcher {
    mode: FetchMode,
    rest_fetcher: BinanceFetcher,
    client: reqwest::Client,
}

impl HybridBinanceFetcher {
    pub fn new(mode: FetchMode) -> Self {
        Self {
            mode,
            rest_fetcher: BinanceFetcher::new(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl LiveDataFetcher for HybridBinanceFetcher {
    fn source_name(&self) -> &str { "binance-hybrid-engine" }

    fn supported_markets(&self) -> Vec<Market> {
        vec![Market::Spot, Market::Futures, Market::Coinm]
    }

    fn supported_symbols(&self, _market: Market) -> Vec<String> {
        vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()]
    }

    /// Funding rate çekme (REST üzerinden)
    async fn fetch_funding_rate(&self, market: Market, symbol: &str) -> Result<Option<FundingRatePoint>> {
        // BinanceFetcher içindeki mantığı kullanıyoruz (DRY)
        // Not: BinanceFetcher'a ileride FundingRate desteği eklendiğinde buradan direkt çağrılacak.
        // Şimdilik hibrit katman kendi REST client'ını kullanıyor.
        let url = match market {
            Market::Coinm => format!("https://binance.com{}", symbol),
            _ => format!("https://binance.com{}", symbol),
        };

        if !matches!(market, Market::Futures | Market::Coinm) { return Ok(None); }

        let json: serde_json::Value = self.client.get(&url).send().await?.json().await?;
        let rate = json["lastFundingRate"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        
        Ok(Some(FundingRatePoint {
            timestamp: Utc::now(),
            symbol: symbol.to_string(),
            funding_rate: rate,
            mark_price: json["markPrice"].as_str().and_then(|s| s.parse::<f64>().ok()),
        }))
    }

    /// Hibrit Veri Çekme: Mode'a göre en hızlı ve en doğru kaynağı seçer
    async fn fetch_latest(
        &self,
        _exchange: Exchange,
        market: Market,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        match self.mode {
            FetchMode::RestOnly => {
                let res = self.rest_fetcher.fetch_latest(symbol, interval, limit).await
                    .map_err(|e| format!("Hybrid-REST Hatası: {}", e))?;
                Ok(res)
            }
            FetchMode::WebSocketOnly => self.get_live_one_shot(symbol, interval).await,
            FetchMode::Hybrid => {
                // Hibrit Stratejisi: Önce REST'ten geçmişi al, eğer boşsa WS'e düş
                let candles = self.rest_fetcher.fetch_latest(symbol, interval, limit).await;
                match candles {
                    Ok(c) if !c.is_empty() => Ok(c),
                    _ => self.get_live_one_shot(symbol, interval).await,
                }
            }
        }
    }
}

impl HybridBinanceFetcher {
    /// WebSocket One-Shot: Tek bir taze kapanmış mumu bekler ve döner
    async fn get_live_one_shot(&self, symbol: &str, interval: &str) -> Result<Vec<Candle>> {
        let ws_url = format!(
            "wss://stream.binance.com:9443/ws/{}@kline_{}",
            symbol.to_lowercase(), interval
        );

        let (ws_stream, _) = connect_async(&ws_url).await
            .map_err(|e| format!("WS Bağlantı Hatası: {e}"))?;
        
        let (_, mut read) = ws_stream.split();

        // Maksimum 30 saniye boyunca taze mum bekle (Timeout koruması)
        let timeout = tokio::time::sleep(std::time::Duration::from_secs(30));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                msg = read.next() => {
                    if let Some(Ok(msg)) = msg {
                        if let Ok(text) = msg.to_text() {
                            if let Ok(update) = serde_json::from_str::<BinanceKlineUpdate>(text) {
                                if update.kline.is_closed {
                                    return Ok(vec![parse_kline(update)?]);
                                }
                            }
                        }
                    } else {
                        return Err("WebSocket akışı kesildi".into());
                    }
                }
                _ = &mut timeout => {
                    return Err("WebSocket veri zaman aşımı (30sn)".into());
                }
            }
        }
    }
}
