//! `TwelveDataFetcher` — Twelve Data `time_series` REST çekici (dünya piyasaları, anahtarlı).
//!
//! [[project_world_markets]] Faz A: Yahoo/Stooq bot-kapısına takılınca sağlam keyed kaynak.
//! Ücretsiz katman 800 istek/gün, 8 istek/dk → çağıran semboller arası gecikme koymalı.
//! Tek endpoint tüm varlık sınıfları:
//!   * BIST hisse  → symbol=THYAO, exchange=BIST
//!   * Forex       → symbol=EUR/USD
//!   * Emtia/metal → symbol=XAU/USD (altın), XAG/USD (gümüş)
//!   * ABD hisse   → symbol=AAPL
//!
//! `parse_time_series` SAF (ağsız test). `values` EN YENİ BAŞTA gelir → artan'a çevrilir.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use reqwest::Client;

use crate::core::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::robot::data_fetcher::websocket::validate_ohlcv;

const TD_BASE: &str = "https://api.twelvedata.com";

pub struct TwelveDataFetcher {
    api_key: String,
    client: Client,
}

impl TwelveDataFetcher {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Bot TF'ini Twelve Data interval token'ına çevir.
    pub fn td_interval(interval: &str) -> &str {
        match interval {
            "1m" => "1min", "5m" => "5min", "15m" => "15min", "30m" => "30min",
            "1h" => "1h", "2h" => "2h", "4h" => "4h",
            "1d" => "1day", "1w" => "1week", "1M" => "1month",
            other => other,
        }
    }

    /// Varlık-sınıfı + çıplak sembol → (TD symbol, exchange?). Forex/emtia 6-harf → slash'lı
    /// (EURUSD→EUR/USD, XAUUSD→XAU/USD); zaten slash'lıysa dokunulmaz. BIST → exchange=BIST.
    pub fn td_symbol(asset_class: &str, base: &str) -> (String, Option<&'static str>) {
        let b = base.trim();
        match asset_class.to_lowercase().as_str() {
            "bist" | "equity_tr" => (b.to_string(), Some("BIST")),
            "forex" | "fx" | "commodity" | "comm" | "gold" => {
                if b.contains('/') {
                    (b.to_string(), None)
                } else if b.len() == 6 {
                    (format!("{}/{}", &b[..3], &b[3..]), None)
                } else {
                    (b.to_string(), None)
                }
            }
            _ => (b.to_string(), None), // ABD hisse / ETF / endeks → çıplak
        }
    }

    /// Twelve Data `time_series` yanıtını `Candle`'a parse et — SAF (ağsız test). `out_symbol`
    /// üretilen mumun çıplak sembolü. `values` en-yeni-başta → artan sıraya çevrilir.
    pub fn parse_time_series(body: &str, out_symbol: &str, interval: &str) -> Result<Vec<Candle>, String> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|e| format!("TwelveData JSON parse: {e}"))?;
        // Hata gövdesi: {"code":..,"message":..,"status":"error"}
        if v.get("status").and_then(|s| s.as_str()) == Some("error") {
            let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("bilinmeyen");
            return Err(format!("TwelveData hata [{out_symbol}]: {msg}"));
        }
        let values = v
            .get("values")
            .and_then(|x| x.as_array())
            .ok_or_else(|| format!("TwelveData 'values' yok [{out_symbol}]"))?;

        let mut candles = Vec::with_capacity(values.len());
        for row in values {
            let dt_str = match row.get("datetime").and_then(|x| x.as_str()) {
                Some(s) => s,
                None => continue,
            };
            // 1day → "2024-06-18", intraday → "2024-06-18 15:30:00".
            let ndt: NaiveDateTime = if let Ok(d) = NaiveDate::parse_from_str(dt_str, "%Y-%m-%d") {
                d.and_hms_opt(0, 0, 0).unwrap()
            } else if let Ok(dt) = NaiveDateTime::parse_from_str(dt_str, "%Y-%m-%d %H:%M:%S") {
                dt
            } else {
                continue;
            };
            let pf = |key: &str| row.get(key).and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok());
            let (open, high, low, close) = match (pf("open"), pf("high"), pf("low"), pf("close")) {
                (Some(o), Some(h), Some(l), Some(c)) => (o, h, l, c),
                _ => continue,
            };
            let volume = pf("volume").unwrap_or(0.0); // forex/emtia hacmi yok → 0
            if validate_ohlcv(open, high, low, close, volume).is_err() {
                continue;
            }
            candles.push(Candle {
                timestamp: Utc.from_utc_datetime(&ndt),
                open,
                high,
                low,
                close,
                volume,
                symbol: out_symbol.to_string(),
                interval: interval.to_string(),
            });
        }
        candles.sort_by_key(|c| c.timestamp);
        Ok(candles)
    }

    /// `time_series` çek. `td_symbol` TD-doğal sembol (EUR/USD vb.), `exchange` opsiyonel borsa
    /// (BIST). `out_symbol` üretilen mumun çıplak sembolü. `outputsize` ≤5000.
    pub async fn fetch_daily(
        &self, td_symbol: &str, exchange: Option<&str>, out_symbol: &str, interval: &str, outputsize: usize,
    ) -> Result<Vec<Candle>, String> {
        let mut url = format!(
            "{TD_BASE}/time_series?symbol={}&interval={}&outputsize={}&format=JSON&apikey={}",
            urlencoding(td_symbol),
            Self::td_interval(interval),
            outputsize.clamp(1, 5000),
            self.api_key,
        );
        if let Some(ex) = exchange {
            url.push_str(&format!("&exchange={ex}"));
        }
        let body = self.client.get(&url).send().await
            .map_err(|e| format!("TwelveData bağlantı [{out_symbol}]: {e}"))?
            .text().await
            .map_err(|e| format!("TwelveData gövde [{out_symbol}]: {e}"))?;
        Self::parse_time_series(&body, out_symbol, interval)
    }
}

/// Minimal URL-encode (yalnız '/' ve boşluk; TD sembollerinde yeterli — EUR/USD).
fn urlencoding(s: &str) -> String {
    s.replace('/', "%2F").replace(' ', "%20")
}

#[async_trait]
impl MarketFetcher for TwelveDataFetcher {
    fn name(&self) -> &'static str {
        "twelvedata"
    }

    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        // Genel yüzey: symbol TD-doğal varsayılır, exchange yok.
        self.fetch_daily(symbol, None, symbol, interval, limit.clamp(1, 5000)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn interval_and_symbol_mapping() {
        assert_eq!(TwelveDataFetcher::td_interval("1d"), "1day");
        assert_eq!(TwelveDataFetcher::td_interval("1h"), "1h");
        assert_eq!(TwelveDataFetcher::td_symbol("bist", "THYAO"), ("THYAO".into(), Some("BIST")));
        assert_eq!(TwelveDataFetcher::td_symbol("forex", "EURUSD"), ("EUR/USD".into(), None));
        assert_eq!(TwelveDataFetcher::td_symbol("commodity", "XAUUSD"), ("XAU/USD".into(), None));
        assert_eq!(TwelveDataFetcher::td_symbol("forex", "EUR/USD"), ("EUR/USD".into(), None));
        assert_eq!(TwelveDataFetcher::td_symbol("usequity", "AAPL"), ("AAPL".into(), None));
    }

    #[test]
    fn parse_descending_values_to_ascending() {
        let body = json!({
            "meta": {"symbol":"THYAO","interval":"1day","exchange":"BIST"},
            "values": [
                {"datetime":"2024-06-19","open":"102","high":"104","low":"101","close":"103.5","volume":"1200"},
                {"datetime":"2024-06-18","open":"100","high":"103","low":"99","close":"102","volume":"1000"}
            ],
            "status": "ok"
        }).to_string();
        let c = TwelveDataFetcher::parse_time_series(&body, "THYAO", "1d").unwrap();
        assert_eq!(c.len(), 2);
        assert!(c[0].timestamp < c[1].timestamp, "artan sıra");
        assert_eq!(c[0].close, 102.0);
        assert_eq!(c[1].close, 103.5);
    }

    #[test]
    fn parse_forex_zero_volume_ok() {
        let body = json!({
            "values": [{"datetime":"2024-06-18","open":"1.07","high":"1.08","low":"1.06","close":"1.075"}],
            "status": "ok"
        }).to_string();
        let c = TwelveDataFetcher::parse_time_series(&body, "EURUSD", "1d").unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].volume, 0.0);
        assert_eq!(c[0].close, 1.075);
    }

    #[test]
    fn parse_error_status_is_err() {
        let body = json!({"code":429,"message":"API limit reached","status":"error"}).to_string();
        assert!(TwelveDataFetcher::parse_time_series(&body, "X", "1d").is_err());
    }

    #[test]
    fn parse_skips_malformed_rows() {
        let body = json!({
            "values": [
                {"datetime":"2024-06-18","open":"100","high":"103","low":"99","close":"102","volume":"1000"},
                {"datetime":"bad-date","open":"1","high":"1","low":"1","close":"1","volume":"1"},
                {"datetime":"2024-06-17","open":"x","high":"103","low":"99","close":"102"}
            ],
            "status":"ok"
        }).to_string();
        let c = TwelveDataFetcher::parse_time_series(&body, "X", "1d").unwrap();
        assert_eq!(c.len(), 1, "yalnız tam-geçerli satır");
    }
}
