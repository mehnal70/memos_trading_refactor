// robot/data_fetcher/bist_fetcher.rs — BIST Yahoo Finance veri çekici
//
// İyileştirmeler:
//   - period1/period2 timestamp bazlı URL (range= yerine) → Yahoo gerçek aralıkları destekler
//   - User-Agent başlığı eklendi (Yahoo scraping koruması için)
//   - query1 → query2 fallback (429 / 5xx durumunda)
//   - 3x retry + exponential backoff
//   - Null değerler atlanır, kısmi veri kabul edilir

use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::types::Candle;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use reqwest::Client;
use serde_json::Value;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

const YAHOO_HOSTS: &[&str] = &["query1.finance.yahoo.com", "query2.finance.yahoo.com"];
const MAX_RETRIES: usize = 3;

pub struct BistFetcher;

/// Interval stringini Duration'a çevirir (kaç saniyede bir mum kapanır)
fn interval_to_duration(interval: &str) -> Duration {
    match interval {
        "1m"         => Duration::minutes(1),
        "2m"         => Duration::minutes(2),
        "5m"         => Duration::minutes(5),
        "15m"        => Duration::minutes(15),
        "30m"        => Duration::minutes(30),
        "60m" | "1h" => Duration::hours(1),
        "4h"         => Duration::hours(4),
        "1d"         => Duration::days(1),
        "1wk" | "1w" => Duration::weeks(1),
        "1mo" | "1M" => Duration::days(30),
        _            => Duration::days(1),
    }
}

/// Yahoo Finance v8 endpoint URL'sini oluşturur.
/// limit adet mumu kapsayacak period1/period2 timestamp hesaplar.
fn build_url(host: &str, symbol: &str, interval: &str, limit: usize) -> String {
    let period2 = Utc::now();
    let candle_dur = interval_to_duration(interval);
    // %10 fazlasıyla geri gidiyoruz; Yahoo bazen boş bar gönderir
    let lookback_secs = candle_dur.num_seconds() * (limit as i64) * 11 / 10;
    let period1 = period2 - Duration::seconds(lookback_secs);
    let base_symbol = symbol.trim_end_matches(".IS");
    format!(
        "https://{}/v8/finance/chart/{}.IS?interval={}&period1={}&period2={}",
        host, base_symbol, interval, period1.timestamp(), period2.timestamp(),
    )
}

/// JSON yanıtını Vec<Candle>'a dönüştürür.
fn parse_response(body: &str, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| format!("JSON parse hatası: {e}"))?;

    let chart = v.pointer("/chart/result/0")
        .ok_or("Yahoo yanıtında chart.result[0] yok")?;

    let timestamps = chart["timestamp"]
        .as_array()
        .ok_or("timestamp alanı yok")?;

    let quote = chart.pointer("/indicators/quote/0")
        .ok_or("indicators.quote[0] yok")?;

    let opens   = quote["open"]  .as_array().ok_or("open alanı yok")?;
    let highs   = quote["high"]  .as_array().ok_or("high alanı yok")?;
    let lows    = quote["low"]   .as_array().ok_or("low alanı yok")?;
    let closes  = quote["close"] .as_array().ok_or("close alanı yok")?;
    let volumes = quote["volume"].as_array().ok_or("volume alanı yok")?;

    let mut candles = Vec::with_capacity(timestamps.len());
    for i in 0..timestamps.len() {
        let ts_sec = match timestamps[i].as_i64() {
            Some(t) => t,
            None => continue,
        };
        let open  = opens .get(i).and_then(|v| v.as_f64());
        let close = closes.get(i).and_then(|v| v.as_f64());
        // Her ikisi de null ise bu bar anlamsız — atla
        if open.is_none() && close.is_none() { continue; }

        let ts = chrono::DateTime::from_timestamp(ts_sec, 0)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        candles.push(Candle {
            timestamp: ts,
            open:   open .unwrap_or(0.0),
            high:   highs  .get(i).and_then(|v| v.as_f64()).unwrap_or(0.0),
            low:    lows   .get(i).and_then(|v| v.as_f64()).unwrap_or(0.0),
            close:  close.unwrap_or(0.0),
            volume: volumes.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0),
            symbol:   symbol.trim_end_matches(".IS").to_string(),
            interval: interval.to_string(),
        });
    }

    if candles.len() > limit {
        candles = candles[candles.len() - limit..].to_vec();
    }
    Ok(candles)
}

#[async_trait]
impl MarketFetcher for BistFetcher {
    fn name(&self) -> &'static str { "bist" }

    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| format!("HTTP client hatası: {e}"))?;

        let mut last_err = String::new();

        // query1 → query2 host fallback; her host için birkaç retry
        'outer: for host in YAHOO_HOSTS {
            let url = build_url(host, symbol, interval, limit);
            for attempt in 1..=MAX_RETRIES {
                match client.get(&url).send().await {
                    Err(e) => {
                        last_err = format!("[{host}] İstek hatası (deneme {attempt}): {e}");
                        log::warn!("[BIST] {}", last_err);
                        tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        if status.as_u16() == 429 {
                            last_err = format!("[{host}] 429 Too Many Requests — diğer host'a geçiliyor");
                            log::warn!("[BIST] {}", last_err);
                            continue 'outer;
                        }
                        if status.is_server_error() {
                            last_err = format!("[{host}] Sunucu hatası {status} (deneme {attempt})");
                            log::warn!("[BIST] {}", last_err);
                            tokio::time::sleep(std::time::Duration::from_millis(1000 * attempt as u64)).await;
                            continue;
                        }
                        if status.is_client_error() {
                            return Err(format!("[{host}] HTTP {status} — istek geçersiz (URL: {url})"));
                        }
                        let body = resp.text().await
                            .map_err(|e| format!("Yanıt okunamadı: {e}"))?;
                        if body.trim().is_empty() {
                            last_err = format!("[{host}] Boş yanıt (deneme {attempt})");
                            log::warn!("[BIST] {}", last_err);
                            continue;
                        }
                        return parse_response(&body, symbol, interval, limit)
                            .map_err(|e| format!("[{host}] {e}"));
                    }
                }
            }
        }
        Err(format!("[BIST] Tüm denemeler başarısız. Son hata: {last_err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_duration_correct() {
        assert_eq!(interval_to_duration("1d").num_seconds(), 86400);
        assert_eq!(interval_to_duration("1h").num_seconds(), 3600);
        assert_eq!(interval_to_duration("5m").num_seconds(), 300);
    }

    #[test]
    fn build_url_contains_symbol_and_timestamps() {
        let url = build_url("query1.finance.yahoo.com", "AKBNK", "1d", 60);
        assert!(url.contains("AKBNK.IS"));
        assert!(url.contains("interval=1d"));
        assert!(url.contains("period1="));
        assert!(url.contains("period2="));
        assert!(!url.contains("range="));
    }

    #[test]
    fn build_url_strips_is_suffix_correctly() {
        let url = build_url("query1.finance.yahoo.com", "THYAO.IS", "1d", 10);
        assert!(!url.contains(".IS.IS"));
        assert!(url.contains("THYAO.IS"));
    }
}
