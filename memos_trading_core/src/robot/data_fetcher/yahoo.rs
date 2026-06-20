//! `YahooFetcher` — Yahoo Finance `v8/finance/chart` genel günlük-OHLC çekici (dünya piyasaları).
//!
//! Tek endpoint tüm varlık sınıflarını verir (ücretsiz, gecikmeli/EOD, seans-bazlı):
//!   * BIST hisse  → `THYAO.IS`
//!   * Forex       → `EURUSD=X`
//!   * Emtia/altın → `GC=F` (altın), `CL=F` (petrol)
//!   * ABD hisse   → `AAPL` (çıplak)
//!
//! [[project_world_markets]] Faz A ölçüm yakıtı. Yahoo datacenter-IP'yi 429 ile throttle eder →
//! host-rotasyonu (query1/query2) + retry + tarayıcı UA ŞART. `parse_chart` saf → ağsız test edilir.
//! NOT: [`super::bist_fetcher::BistFetcher`] bu çekirdeğe delege eder (DRY; tek Yahoo-parse yolu).

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;

use crate::core::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::robot::data_fetcher::websocket::validate_ohlcv;

pub(crate) const YAHOO_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const YAHOO_HOSTS: &[&str] = &["query1.finance.yahoo.com", "query2.finance.yahoo.com"];
const MAX_RETRIES: usize = 3;

pub struct YahooFetcher {
    client: Client,
}

impl Default for YahooFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl YahooFetcher {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent(YAHOO_UA)
                .timeout(Duration::from_secs(12))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Varlık-sınıfı + çıplak sembol → Yahoo-doğal ticker (tek-kaynak ek eşlemesi). Sembol zaten
    /// Yahoo-eki taşıyorsa (`.IS`/`=X`/`=F`) dokunulmaz (idempotent). Bilinmeyen sınıf → çıplak.
    pub fn yahoo_ticker(asset_class: &str, base: &str) -> String {
        let b = base.trim();
        if b.ends_with(".IS") || b.ends_with("=X") || b.ends_with("=F") {
            return b.to_string();
        }
        match asset_class.to_lowercase().as_str() {
            "bist" | "equity_tr" => format!("{b}.IS"),
            "forex" | "fx" => format!("{b}=X"),
            "commodity" | "comm" | "gold" => format!("{b}=F"),
            // ABD hisse / endeks / ETF → çıplak ticker (AAPL, SPY, ^GSPC operatörce verilir).
            _ => b.to_string(),
        }
    }

    /// Yahoo chart yanıtını `Candle`'a parse et — SAF (ağsız test edilir). `out_symbol` üretilen
    /// mumun `symbol` alanı (çıplak sembol; Yahoo-eki taşımaz). Geçersiz/eksik bar atlanır.
    pub fn parse_chart(body: &str, out_symbol: &str, interval: &str) -> Result<Vec<Candle>, String> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|e| format!("Yahoo JSON parse: {e}"))?;
        // Hata gövdesi: {"chart":{"result":null,"error":{"code":..,"description":..}}}
        if let Some(err) = v.pointer("/chart/error") {
            if !err.is_null() {
                let desc = err.get("description").and_then(|d| d.as_str()).unwrap_or("bilinmeyen");
                return Err(format!("Yahoo hata [{out_symbol}]: {desc}"));
            }
        }
        let chart = v
            .pointer("/chart/result/0")
            .ok_or_else(|| format!("Yahoo result yok [{out_symbol}]"))?;
        let timestamps = chart
            .get("timestamp")
            .and_then(|t| t.as_array())
            .ok_or_else(|| format!("Yahoo timestamp yok [{out_symbol}]"))?;
        let quote = chart
            .pointer("/indicators/quote/0")
            .ok_or_else(|| format!("Yahoo quote yok [{out_symbol}]"))?;

        let mut candles = Vec::with_capacity(timestamps.len());
        for i in 0..timestamps.len() {
            let ts_sec = match timestamps[i].as_i64() {
                Some(t) if t > 0 => t,
                _ => continue,
            };
            let get_f = |key: &str| {
                quote.get(key)
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.get(i))
                    .and_then(|x| x.as_f64())
            };
            // Yahoo seans-içi eksik barda null koyar → herhangi biri yoksa barı atla.
            let (open, high, low, close, volume) =
                match (get_f("open"), get_f("high"), get_f("low"), get_f("close")) {
                    (Some(o), Some(h), Some(l), Some(c)) => (o, h, l, c, get_f("volume").unwrap_or(0.0)),
                    _ => continue,
                };
            if validate_ohlcv(open, high, low, close, volume).is_err() {
                continue;
            }
            if let Some(dt) = DateTime::from_timestamp(ts_sec, 0) {
                candles.push(Candle {
                    timestamp: dt.with_timezone(&Utc),
                    open,
                    high,
                    low,
                    close,
                    volume,
                    symbol: out_symbol.to_string(),
                    interval: interval.to_string(),
                });
            }
        }
        candles.sort_by_key(|c| c.timestamp);
        Ok(candles)
    }

    /// Yahoo-doğal `ticker` için günlük OHLC çek. `range` Yahoo aralık token'ı
    /// (`1mo`/`6mo`/`1y`/`2y`/`5y`/`10y`/`max`). `out_symbol` üretilen mumun çıplak sembolü.
    /// query1↔query2 host-rotasyonu + retry (429/5xx). En yeni sonda, artan sıralı döner.
    pub async fn fetch_daily(
        &self, ticker: &str, out_symbol: &str, interval: &str, range: &str,
    ) -> Result<Vec<Candle>, String> {
        let mut last_err = String::from("bilinmeyen");
        for host in YAHOO_HOSTS {
            let url = format!(
                "https://{host}/v8/finance/chart/{ticker}?interval={interval}&range={range}"
            );
            for attempt in 1..=MAX_RETRIES {
                match self.client.get(&url).send().await {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.as_u16() == 429 {
                            last_err = format!("[{host}] 429 (rate-limit) → diğer host");
                            break; // bu host'ta retry boşuna → host değiştir
                        }
                        if status.is_server_error() {
                            tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
                            continue;
                        }
                        if !status.is_success() {
                            return Err(format!("Yahoo HTTP {status} [{out_symbol}]"));
                        }
                        let body = resp.text().await.map_err(|e| e.to_string())?;
                        return Self::parse_chart(&body, out_symbol, interval);
                    }
                    Err(e) => {
                        last_err = format!("ağ: {e}");
                        tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
                    }
                }
            }
        }
        Err(format!("Yahoo çekme başarısız [{out_symbol}]: {last_err}"))
    }
}

#[async_trait]
impl MarketFetcher for YahooFetcher {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    /// Genel `MarketFetcher` yüzeyi: sembol zaten Yahoo-doğal ticker varsayılır (operatör/çağıran
    /// eki vermiş olur). `limit`→`range` kabaca eşlenir (günlük için 2y bolca yeter).
    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        let range = if limit > 1300 { "10y" } else if limit > 260 { "5y" } else { "2y" };
        let mut candles = self.fetch_daily(symbol, symbol, interval, range).await?;
        if candles.len() > limit {
            candles = candles.split_off(candles.len() - limit);
        }
        Ok(candles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ticker_suffix_mapping() {
        assert_eq!(YahooFetcher::yahoo_ticker("bist", "THYAO"), "THYAO.IS");
        assert_eq!(YahooFetcher::yahoo_ticker("forex", "EURUSD"), "EURUSD=X");
        assert_eq!(YahooFetcher::yahoo_ticker("commodity", "GC"), "GC=F");
        assert_eq!(YahooFetcher::yahoo_ticker("usequity", "AAPL"), "AAPL");
        // İdempotent: zaten ekli sembol değişmez.
        assert_eq!(YahooFetcher::yahoo_ticker("bist", "GARAN.IS"), "GARAN.IS");
        assert_eq!(YahooFetcher::yahoo_ticker("forex", "GBPUSD=X"), "GBPUSD=X");
    }

    #[test]
    fn parse_chart_extracts_ascending_candles() {
        let body = json!({
            "chart": {"error": null, "result": [{
                "meta": {"symbol": "THYAO.IS", "currency": "TRY"},
                "timestamp": [1700006400, 1700092800],
                "indicators": {"quote": [{
                    "open":  [100.0, 102.0],
                    "high":  [103.0, 104.0],
                    "low":   [ 99.0, 101.0],
                    "close": [102.0, 103.5],
                    "volume":[1000.0, 1200.0]
                }]}
            }]}
        }).to_string();
        let c = YahooFetcher::parse_chart(&body, "THYAO", "1d").unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].symbol, "THYAO");
        assert!(c[0].timestamp < c[1].timestamp);
        assert_eq!(c[1].close, 103.5);
    }

    #[test]
    fn parse_chart_skips_null_bars() {
        // Yahoo seans-içi eksik barda null → o bar atlanmalı (panik yok).
        let body = json!({
            "chart": {"result": [{
                "timestamp": [1700006400, 1700092800, 1700179200],
                "indicators": {"quote": [{
                    "open":  [100.0, null, 105.0],
                    "high":  [103.0, null, 106.0],
                    "low":   [ 99.0, null, 104.0],
                    "close": [102.0, null, 105.5],
                    "volume":[1000.0, null, 900.0]
                }]}
            }]}
        }).to_string();
        let c = YahooFetcher::parse_chart(&body, "X", "1d").unwrap();
        assert_eq!(c.len(), 2, "null bar atlanır");
    }

    #[test]
    fn parse_chart_error_body_is_err() {
        let body = json!({
            "chart": {"result": null, "error": {"code": "Not Found", "description": "No data found"}}
        }).to_string();
        assert!(YahooFetcher::parse_chart(&body, "BADSYM", "1d").is_err());
    }
}
