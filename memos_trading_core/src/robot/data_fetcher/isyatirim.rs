//! `IsYatirimFetcher` — İş Yatırım (isyatirim.com.tr) BIST günlük-OHLC çekici (ücretsiz, TR-yerli).
//!
//! [[project_world_markets]] Faz A: TD-ücretsiz BIST'i kilitliyor, Yahoo 429 → İş Yatırım'ın
//! `HisseTekno` endpoint'i (yaygın `isyatirimhisse` paketinin kullandığı kamuya-açık veri yolu).
//! Günlük TL OHLC; yıl-yıl pagine eder (endpoint aralık-cap'i olabilir). `parse_hisse_tekno` SAF
//! → ağsız test. NOT: endpoint TR residential IP ister (datacenter/yurt-dışı IP 401); canlı çağrı
//! kullanıcı makinesinde doğrulanır. Header'lar (Referer/X-Requested-With) gerekli.

use std::time::Duration;

use chrono::{Datelike, NaiveDate, TimeZone, Utc};

use crate::core::types::Candle;
use crate::robot::data_fetcher::websocket::validate_ohlcv;

const ISY_BASE: &str =
    "https://www.isyatirim.com.tr/_Layouts/15/IsYatirim.Website/Common/Data.aspx/HisseTekno";
const ISY_REFERER: &str =
    "https://www.isyatirim.com.tr/tr-tr/analiz/hisse/Sayfalar/default.aspx";
const ISY_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

pub struct IsYatirimFetcher {
    client: reqwest::Client,
}

impl Default for IsYatirimFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl IsYatirimFetcher {
    pub fn new() -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::ACCEPT, "application/json, text/plain, */*".parse().unwrap());
        headers.insert(reqwest::header::REFERER, ISY_REFERER.parse().unwrap());
        headers.insert("X-Requested-With", "XMLHttpRequest".parse().unwrap());
        Self {
            client: reqwest::Client::builder()
                .user_agent(ISY_UA)
                .default_headers(headers)
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Bir satır alanını f64 oku — Number ya da String (TR ondalık virgülü → nokta).
    fn num(row: &serde_json::Value, key: &str) -> Option<f64> {
        match row.get(key) {
            Some(serde_json::Value::Number(n)) => n.as_f64(),
            Some(serde_json::Value::String(s)) => s.trim().replace(',', ".").parse::<f64>().ok(),
            _ => None,
        }
    }

    /// `HGDG_TARIH` tarihini parse et (gözlenen biçimler: "DD-MM-YYYY", "DD.MM.YYYY", "YYYY-MM-DD").
    fn parse_date(s: &str) -> Option<NaiveDate> {
        let s = s.trim();
        NaiveDate::parse_from_str(s, "%d-%m-%Y")
            .or_else(|_| NaiveDate::parse_from_str(s, "%d.%m.%Y"))
            .or_else(|_| NaiveDate::parse_from_str(s, "%Y-%m-%d"))
            .ok()
    }

    /// `HisseTekno` yanıtını (`{"value":[...]}`) `Candle`'a parse et — SAF (ağsız test). Alanlar:
    /// HGDG_ACILIS(open)/HGDG_MAX(high)/HGDG_MIN(low)/HGDG_KAPANIS(close)/HGDG_HACIM(volume)/HGDG_TARIH.
    pub fn parse_hisse_tekno(body: &str, out_symbol: &str, interval: &str) -> Result<Vec<Candle>, String> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|e| format!("İşYatırım JSON parse [{out_symbol}]: {e}"))?;
        let rows = v
            .get("value")
            .and_then(|x| x.as_array())
            .ok_or_else(|| format!("İşYatırım 'value' yok [{out_symbol}] (401/boş olabilir)"))?;

        let mut candles = Vec::with_capacity(rows.len());
        for row in rows {
            let date = match row.get("HGDG_TARIH").and_then(|x| x.as_str()).and_then(Self::parse_date) {
                Some(d) => d,
                None => continue,
            };
            let (open, high, low, close) = match (
                Self::num(row, "HGDG_ACILIS"),
                Self::num(row, "HGDG_MAX"),
                Self::num(row, "HGDG_MIN"),
                Self::num(row, "HGDG_KAPANIS"),
            ) {
                (Some(o), Some(h), Some(l), Some(c)) => (o, h, l, c),
                _ => continue,
            };
            let volume = Self::num(row, "HGDG_HACIM").unwrap_or(0.0).max(0.0);
            if validate_ohlcv(open, high, low, close, volume).is_err() {
                continue;
            }
            if let Some(ndt) = date.and_hms_opt(0, 0, 0) {
                candles.push(Candle {
                    timestamp: Utc.from_utc_datetime(&ndt),
                    open, high, low, close, volume,
                    symbol: out_symbol.to_string(),
                    interval: interval.to_string(),
                });
            }
        }
        candles.sort_by_key(|c| c.timestamp);
        candles.dedup_by_key(|c| c.timestamp);
        Ok(candles)
    }

    /// Tek tarih-penceresi ham gövde çek (DD-MM-YYYY).
    async fn fetch_window(&self, symbol: &str, start: &str, end: &str) -> Result<String, String> {
        let url = format!("{ISY_BASE}?hisse={symbol}&startdate={start}&enddate={end}");
        let resp = self.client.get(&url).send().await
            .map_err(|e| format!("İşYatırım bağlantı [{symbol}]: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("İşYatırım HTTP {status} [{symbol}] (401 → TR-IP/header gerekebilir)"));
        }
        resp.text().await.map_err(|e| format!("İşYatırım gövde [{symbol}]: {e}"))
    }

    /// `years_back` yıl geriye GÜNLÜK OHLC. Endpoint aralık-cap'ine karşı YIL-YIL pagine eder,
    /// birleştirir (dedup+artan). `out_symbol` üretilen mumun sembolü (çıplak, ör. THYAO).
    pub async fn fetch_daily(&self, symbol: &str, out_symbol: &str, years_back: i32) -> Result<Vec<Candle>, String> {
        let now = Utc::now();
        let cur_year = now.year();
        let start_year = cur_year - years_back.max(1) + 1;
        let mut out: Vec<Candle> = Vec::new();
        let mut last_err: Option<String> = None;

        for year in start_year..=cur_year {
            let start = format!("01-01-{year}");
            let end = if year == cur_year {
                now.format("%d-%m-%Y").to_string()
            } else {
                format!("31-12-{year}")
            };
            match self.fetch_window(symbol, &start, &end).await {
                Ok(body) => match Self::parse_hisse_tekno(&body, out_symbol, "1d") {
                    Ok(mut c) => out.append(&mut c),
                    Err(e) => last_err = Some(e),
                },
                Err(e) => last_err = Some(e),
            }
            tokio::time::sleep(Duration::from_millis(300)).await; // nezaket
        }

        if out.is_empty() {
            return Err(last_err.unwrap_or_else(|| format!("İşYatırım: veri yok [{out_symbol}]")));
        }
        out.sort_by_key(|c| c.timestamp);
        out.dedup_by_key(|c| c.timestamp);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_dates_multi_format() {
        assert_eq!(IsYatirimFetcher::parse_date("18-06-2024"), NaiveDate::from_ymd_opt(2024, 6, 18));
        assert_eq!(IsYatirimFetcher::parse_date("18.06.2024"), NaiveDate::from_ymd_opt(2024, 6, 18));
        assert_eq!(IsYatirimFetcher::parse_date("2024-06-18"), NaiveDate::from_ymd_opt(2024, 6, 18));
        assert_eq!(IsYatirimFetcher::parse_date("saçma"), None);
    }

    #[test]
    fn num_reads_number_and_comma_string() {
        let row = json!({"a": 12.5, "b": "10,25", "c": "x"});
        assert_eq!(IsYatirimFetcher::num(&row, "a"), Some(12.5));
        assert_eq!(IsYatirimFetcher::num(&row, "b"), Some(10.25));
        assert_eq!(IsYatirimFetcher::num(&row, "c"), None);
    }

    #[test]
    fn parse_hisse_tekno_builds_candles() {
        let body = json!({"value":[
            {"HGDG_HS_KODU":"THYAO","HGDG_TARIH":"17-06-2024","HGDG_ACILIS":300.0,"HGDG_MAX":305.0,"HGDG_MIN":298.0,"HGDG_KAPANIS":303.5,"HGDG_HACIM":1000000.0},
            {"HGDG_HS_KODU":"THYAO","HGDG_TARIH":"18-06-2024","HGDG_ACILIS":303.5,"HGDG_MAX":308.0,"HGDG_MIN":302.0,"HGDG_KAPANIS":307.0,"HGDG_HACIM":1200000.0}
        ]}).to_string();
        let c = IsYatirimFetcher::parse_hisse_tekno(&body, "THYAO", "1d").unwrap();
        assert_eq!(c.len(), 2);
        assert!(c[0].timestamp < c[1].timestamp, "artan sıra");
        assert_eq!(c[0].close, 303.5);
        assert_eq!(c[1].symbol, "THYAO");
    }

    #[test]
    fn parse_hisse_tekno_skips_bad_rows() {
        let body = json!({"value":[
            {"HGDG_TARIH":"17-06-2024","HGDG_ACILIS":300.0,"HGDG_MAX":305.0,"HGDG_MIN":298.0,"HGDG_KAPANIS":303.5},
            {"HGDG_TARIH":"bad","HGDG_ACILIS":1.0,"HGDG_MAX":1.0,"HGDG_MIN":1.0,"HGDG_KAPANIS":1.0},
            {"HGDG_TARIH":"18-06-2024","HGDG_ACILIS":0.0,"HGDG_MAX":0.0,"HGDG_MIN":0.0,"HGDG_KAPANIS":0.0}
        ]}).to_string();
        let c = IsYatirimFetcher::parse_hisse_tekno(&body, "X", "1d").unwrap();
        assert_eq!(c.len(), 1, "kötü-tarih + sıfır-fiyat atlanır");
    }

    #[test]
    fn parse_missing_value_is_err() {
        let body = json!({"baska":"alan"}).to_string();
        assert!(IsYatirimFetcher::parse_hisse_tekno(&body, "X", "1d").is_err());
    }
}
