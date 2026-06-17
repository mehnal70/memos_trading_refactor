//! `BybitVenue` — `VenueAdapter`'ın Bybit (v5 API) implementasyonu — gerçek 2. kripto borsa.
//!
//! Soyutlamanın Binance-DIŞI bir borsada da temiz çalıştığının kanıtı. Şu an **veri venue'su**:
//! `fetch_candles` + `book_ticker` gerçek public Bybit v5 REST'ine gider (auth/keys gerekmez);
//! yürütme (`submit_order`/...) ve `symbol_filters` açık `Err` döner (Faz 1+ yürütme katmanı).
//! Bu, sildiğimiz sahte stub'lardan farklı: ASLA sahte başarı (`Ok(0.0)`/dummy-id) dönmez —
//! ya gerçek veri ya açık hata.
//!
//! Endpoint'ler: kline `/v5/market/kline`, ticker `/v5/market/tickers` (kategori: linear=USDT
//! perp, spot, inverse=coin-margined). Kline yanıtı EN YENİ BAŞTA döner → parse ARTAN'a çevirir.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::model::SymbolFilters;
use crate::core::types::{Candle, Exchange, Market};
use crate::robot::venue::adapter::{MarketData, OrderExecution, VenueAdapter};
use crate::robot::venue::types::{OrderReceipt, OrderRequest};
use crate::Result;

const BYBIT_BASE: &str = "https://api.bybit.com";

pub struct BybitVenue {
    market: Market,
    client: reqwest::Client,
}

impl BybitVenue {
    pub fn new(market: Market) -> Self {
        Self {
            market,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Bybit v5 ürün kategorisi: futures(USDT-perp)→linear, coinm(coin-margined)→inverse, spot→spot.
    fn category(market: Market) -> &'static str {
        match market {
            Market::Futures => "linear",
            Market::Coinm => "inverse",
            Market::Spot => "spot",
        }
    }

    /// Bot TF string'ini (1m/1h/1d) Bybit interval token'ına çevir (dakika sayısı veya D/W/M).
    fn interval_token(interval: &str) -> String {
        match interval {
            "1m" => "1", "3m" => "3", "5m" => "5", "15m" => "15", "30m" => "30",
            "1h" => "60", "2h" => "120", "4h" => "240", "6h" => "360", "12h" => "720",
            "1d" => "D", "1w" => "W", "1M" => "M",
            other => other, // zaten Bybit-doğal verilmişse aynen geçir
        }
        .to_string()
    }

    /// Bybit v5 kline yanıtını `Candle`'a parse et. `result.list` AZALAN (en yeni başta) gelir →
    /// motorun beklediği ARTAN düzene (en yeni sonda) çevrilir. Saf fonksiyon → ağsız test edilir.
    fn parse_klines(symbol: &str, interval: &str, body: &str) -> Result<Vec<Candle>> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|e| format!("Bybit JSON parse: {e}"))?;
        let ret_code = v.get("retCode").and_then(|c| c.as_i64()).unwrap_or(-1);
        if ret_code != 0 {
            let msg = v.get("retMsg").and_then(|m| m.as_str()).unwrap_or("");
            return Err(format!("Bybit kline hatası (retCode {ret_code} {msg}) [{symbol}]").into());
        }
        let list = v
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(|l| l.as_array())
            .ok_or_else(|| format!("Bybit yanıtında result.list yok [{symbol}]"))?;

        let mut candles = Vec::with_capacity(list.len());
        for k in list {
            let arr = match k.as_array() {
                Some(a) if a.len() >= 6 => a,
                _ => continue,
            };
            let s = |i: usize| arr.get(i).and_then(|x| x.as_str());
            let ts_ms = match s(0).and_then(|x| x.parse::<i64>().ok()) {
                Some(t) if t > 0 => t,
                _ => continue,
            };
            let pf = |i: usize| s(i).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0);
            // Bybit kline dizi düzeni: [startMs, open, high, low, close, volume, turnover]
            let (open, high, low, close, volume) = (pf(1), pf(2), pf(3), pf(4), pf(5));
            if crate::robot::data_fetcher::validate_ohlcv(open, high, low, close, volume).is_err() {
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
        // En-yeni-başta → artan (en yeni sonda; motor son mumu "güncel" varsayar).
        candles.sort_by_key(|c| c.timestamp);
        Ok(candles)
    }

    /// `/v5/market/tickers` yanıtından (bid1Price, ask1Price). Saf → ağsız test edilir.
    fn parse_book_ticker(symbol: &str, body: &str) -> Result<(f64, f64)> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|e| format!("Bybit JSON parse: {e}"))?;
        if v.get("retCode").and_then(|c| c.as_i64()).unwrap_or(-1) != 0 {
            return Err(format!("Bybit tickers hatası [{symbol}]").into());
        }
        let item = v
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(|l| l.as_array())
            .and_then(|a| a.first())
            .ok_or_else(|| format!("Bybit ticker boş [{symbol}]"))?;
        let f = |k: &str| item.get(k).and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok());
        match (f("bid1Price"), f("ask1Price")) {
            (Some(b), Some(a)) if b > 0.0 && a > 0.0 => Ok((b, a)),
            _ => Err(format!("Bybit bid/ask alınamadı [{symbol}]").into()),
        }
    }

    /// Yürütme/filtre henüz yok — sahte değer DÖNMEZ, açık hata döner.
    fn unsupported<T>(what: &str) -> Result<T> {
        Err(format!("Bybit {what} henüz uygulanmadı (Faz 1+ yürütme katmanı) — şu an veri-only venue").into())
    }
}

#[async_trait]
impl MarketData for BybitVenue {
    async fn fetch_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        let url = format!(
            "{BYBIT_BASE}/v5/market/kline?category={}&symbol={}&interval={}&limit={}",
            Self::category(self.market),
            symbol,
            Self::interval_token(interval),
            limit.clamp(1, 1000),
        );
        let body = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Bybit bağlantı: {e}"))?
            .text()
            .await
            .map_err(|e| format!("Bybit gövde: {e}"))?;
        Self::parse_klines(symbol, interval, &body)
    }

    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        let url = format!(
            "{BYBIT_BASE}/v5/market/tickers?category={}&symbol={}",
            Self::category(self.market),
            symbol,
        );
        let body = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Bybit bağlantı: {e}"))?
            .text()
            .await
            .map_err(|e| format!("Bybit gövde: {e}"))?;
        Self::parse_book_ticker(symbol, &body)
    }

    async fn symbol_filters(&self, _symbol: &str) -> Result<SymbolFilters> {
        Self::unsupported("symbol_filters")
    }
}

#[async_trait]
impl OrderExecution for BybitVenue {
    async fn submit_order(&self, _req: &OrderRequest) -> Result<OrderReceipt> {
        Self::unsupported("submit_order")
    }
    async fn cancel_all(&self, _symbol: &str) -> Result<()> {
        Self::unsupported("cancel_all")
    }
    async fn set_leverage(&self, _symbol: &str, _leverage: u32) -> Result<()> {
        Self::unsupported("set_leverage")
    }
    async fn balance(&self) -> Result<f64> {
        Self::unsupported("balance")
    }
}

impl VenueAdapter for BybitVenue {
    fn exchange(&self) -> Exchange {
        Exchange::Bybit
    }
    fn market(&self) -> Market {
        self.market
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::AssetClass;
    use serde_json::json;

    #[test]
    fn identity_is_bybit_crypto() {
        let v = BybitVenue::new(Market::Futures);
        assert_eq!(v.exchange(), Exchange::Bybit);
        assert_eq!(v.market(), Market::Futures);
        assert_eq!(v.asset_class(), AssetClass::Crypto);
        assert!(v.has_live_feed());
        assert_eq!(v.name(), "bybit:futures");
    }

    #[test]
    fn category_and_interval_mapping() {
        assert_eq!(BybitVenue::category(Market::Futures), "linear");
        assert_eq!(BybitVenue::category(Market::Spot), "spot");
        assert_eq!(BybitVenue::category(Market::Coinm), "inverse");
        assert_eq!(BybitVenue::interval_token("1m"), "1");
        assert_eq!(BybitVenue::interval_token("4h"), "240");
        assert_eq!(BybitVenue::interval_token("1d"), "D");
    }

    #[test]
    fn parse_klines_reverses_to_ascending() {
        // Bybit en-yeni-başta döner; parse artan sıraya (en yeni sonda) çevirmeli.
        let body = json!({
            "retCode": 0, "retMsg": "OK",
            "result": {"symbol":"BTCUSDT","category":"linear","list":[
                ["1700000060000","101","102","100","101.5","10","x"],
                ["1700000000000","100","101","99","100.5","12","x"]
            ]}
        })
        .to_string();
        let c = BybitVenue::parse_klines("BTCUSDT", "1m", &body).unwrap();
        assert_eq!(c.len(), 2);
        assert!(c[0].timestamp < c[1].timestamp, "artan sıra (en yeni sonda)");
        assert_eq!(c[0].close, 100.5);
        assert_eq!(c[1].close, 101.5);
    }

    #[test]
    fn parse_klines_api_error_is_err() {
        let body = json!({"retCode": 10001, "retMsg": "params error", "result": {}}).to_string();
        assert!(BybitVenue::parse_klines("BTCUSDT", "1m", &body).is_err());
    }

    #[test]
    fn parse_book_ticker_extracts_bid_ask() {
        let body = json!({"retCode":0,"result":{"list":[
            {"symbol":"BTCUSDT","bid1Price":"100.0","ask1Price":"100.2"}
        ]}})
        .to_string();
        let (b, a) = BybitVenue::parse_book_ticker("BTCUSDT", &body).unwrap();
        assert_eq!(b, 100.0);
        assert_eq!(a, 100.2);
    }

    #[tokio::test]
    async fn execution_and_filters_explicitly_unsupported() {
        let v = BybitVenue::new(Market::Futures);
        assert!(v.balance().await.is_err());
        assert!(v.symbol_filters("BTCUSDT").await.is_err());
        let req = OrderRequest::market("BTCUSDT", crate::robot::venue::types::OrderSide::Buy, 1.0);
        assert!(v.submit_order(&req).await.is_err());
    }
}
