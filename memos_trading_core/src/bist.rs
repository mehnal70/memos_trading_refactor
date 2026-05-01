pub async fn fetch_bist_klines(
    symbol: &str,
    interval: &str,
    start_time: i64,
    end_time: i64,
    limit: usize,
) -> Result<Vec<Vec<Value>>, anyhow::Error> {
    // BIST API ve Yahoo fallback mantığı burada olmalı
    // Şimdilik sadece Yahoo Finance ile veri çekiyoruz
    fetch_bist_klines_from_yahoo(symbol, interval, start_time, end_time, limit).await.map_err(anyhow::Error::from)
}
use serde_json::Value;
use reqwest::Client;
use futures::StreamExt;
use crate::batch_config::BatchFetchConfig;
use anyhow::anyhow;
/// Sembol listesini limitli ve kademeli şekilde veri çekerek işleyen fonksiyon
/// Her batch/grup çekiminden sonra belirli bir süre bekler (ör. 2 saniye)
/// Kullanım: batch_fetch_bist_klines(&symbols, ...)
pub async fn batch_fetch_bist_klines(
    symbols: &[String],
    interval: &str,
    start_time: i64,
    end_time: i64,
    limit: usize,
    config: &BatchFetchConfig,
) -> Vec<(String, Result<Vec<Vec<Value>>, anyhow::Error>)> {
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use std::time::Instant;
    let batch_start = Instant::now();
    let results = Arc::new(Mutex::new(Vec::new()));
    futures::stream::iter(symbols.iter().cloned())
        .for_each_concurrent(config.concurrency_limit, |symbol| {
            let results = Arc::clone(&results);
            let interval = interval.to_string();
            let config = config.clone();
            async move {
                let symbol_start = Instant::now();
                let mut attempt = 0;
                let mut data_result = None;
                while attempt < config.max_retries {
                    let res = fetch_bist_klines(&symbol, &interval, start_time, end_time, limit).await;
                    match &res {
                        Ok(data) => {
                            println!("[BATCH][{}] ✅ {} kayıt çekildi (Deneme: {})", symbol, data.len(), attempt+1);
                            println!("[BATCH][{}] ⏱️ Çekim süresi: {:?}", symbol, symbol_start.elapsed());
                            data_result = Some(res);
                            break;
                        },
                        Err(e) => {
                            println!("[BATCH][{}] ❌ Hata: {} (Deneme: {})", symbol, e, attempt+1);
                            println!("[BATCH][{}] ⏱️ Hatalı deneme süresi: {:?}", symbol, symbol_start.elapsed());
                        }
                    }
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(config.wait_ms)).await;
                }
                let final_result = match data_result {
                    Some(Ok(data)) => Ok(data),
                    Some(Err(e)) => Err(anyhow!(e)),
                    None => Err(anyhow!("Tüm denemeler başarısız.")),
                };
                results.lock().await.push((symbol, final_result));
            }
        })
        .await;
    let results = Arc::try_unwrap(results).unwrap().into_inner();
    println!("[BATCH] Tüm semboller için toplam çekim süresi: {:?}", batch_start.elapsed());
    results
// ---
}

/// BIST 100 için sembol listesi çek (sync/blocking)
/// Fallback: Eğer API çalışmazsa manuel listeyi döndürür
pub async fn fetch_bist100_symbols() -> Result<Vec<String>, reqwest::Error> {
    let client = Client::new();
    let bist_url = "https://api.borsaistanbul.com/v1/stocks/bist100";
    let resp = client.get(bist_url).send().await;
    match resp {
        Ok(resp) => {
            if resp.status().is_success() {
                if let Ok(data) = resp.json::<Value>().await {
                    if let Some(symbols_array) = data["symbols"].as_array() {
                        let symbols: Vec<String> = symbols_array
                            .iter()
                            .filter_map(|v: &Value| {
                                let sym = v.as_str().or_else(|| v["symbol"].as_str())?;
                                Some(format!("{}.IS", sym))
                            })
                            .collect();
                        if !symbols.is_empty() {
                            return Ok(symbols);
                        }
                    } else if let Some(symbols_array) = data.as_array() {
                        let symbols: Vec<String> = symbols_array
                            .iter()
                            .filter_map(|v: &Value| {
                                let sym = v.as_str().or_else(|| v["symbol"].as_str())?;
                                Some(format!("{}.IS", sym))
                            })
                            .collect();
                        if !symbols.is_empty() {
                            return Ok(symbols);
                        }
                    }
                }
            }
        }
        Err(_) => {}
    }
    println!("ℹ️ BIST API çalışmadı, manuel liste kullanılıyor");
    Ok(get_bist100_symbols())
}
pub fn get_bist100_symbols() -> Vec<String> {
    vec![
        "AKBNK.IS".to_string(),
        "THYAO.IS".to_string(),
        "GARAN.IS".to_string(),
        // ... diğer semboller ...
        "ZOREN.IS".to_string(),
    ]
}
/// BIST 100 için sembol listesi çek (sync/blocking)

/// Yahoo Finance'tan BIST kline/candle verisi çek (sync/blocking)
pub async fn fetch_bist_klines_from_yahoo(
    symbol: &str,
    interval: &str,
    start_time: i64,
    end_time: i64,
    _limit: usize,
) -> Result<Vec<Vec<Value>>, reqwest::Error> {
    let client = reqwest::Client::new();
    let yf_interval = match interval {
        "1m" => "1m",
        "2m" => "2m",
        "5m" => "5m",
        "15m" => "15m",
        "30m" => "30m",
        "60m" | "1h" => "1h",
        "90m" => "90m",
        "1d" => "1d",
        "5d" => "5d",
        "1wk" | "1w" => "1wk",
        "1mo" | "1M" => "1mo",
        "3mo" => "3mo",
        _ => "1d",
    };
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&period1={}&period2={}",
        symbol, yf_interval, start_time / 1000, end_time / 1000
    );
    let resp = client.get(&url)
        .header("User-Agent", "Mozilla/5.0 (compatible; trading-bot/1.0)")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }
    let data: Value = resp.json().await?;
    let result = data["chart"]["result"].as_array()
        .and_then(|results| results.first());
    if let Some(result) = result {
        let timestamps = result["timestamp"].as_array();
        let quote = result["indicators"]["quote"].as_array()
            .and_then(|quotes| quotes.first());
        if let (Some(timestamps), Some(quote)) = (timestamps, quote) {
            let opens = quote["open"].as_array();
            let highs = quote["high"].as_array();
            let lows = quote["low"].as_array();
            let closes = quote["close"].as_array();
            let volumes = quote["volume"].as_array();
            let mut klines = Vec::new();
            for i in 0..timestamps.len() {
                let timestamp_ms = timestamps.get(i)
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| {
                        eprintln!("[YAHOO] timestamp alınamadı, default 0");
                        0
                    }) * 1000;
                let open = opens.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        eprintln!("[YAHOO] open alınamadı, default 0.0");
                        0.0
                    });
                let high = highs.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        eprintln!("[YAHOO] high alınamadı, default 0.0");
                        0.0
                    });
                let low = lows.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        eprintln!("[YAHOO] low alınamadı, default 0.0");
                        0.0
                    });
                let close = closes.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        eprintln!("[YAHOO] close alınamadı, default 0.0");
                        0.0
                    });
                let volume = volumes.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        eprintln!("[YAHOO] volume alınamadı, default 0.0");
                        0.0
                    });
                klines.push(vec![
                    Value::Number(timestamp_ms.into()),
                    Value::Number(serde_json::Number::from_f64(open).unwrap_or_else(|| {
                        eprintln!("[YAHOO] open f64->number dönüşüm hatası, default 0");
                        0.into()
                    })),
                    Value::Number(serde_json::Number::from_f64(high).unwrap_or_else(|| {
                        eprintln!("[YAHOO] high f64->number dönüşüm hatası, default 0");
                        0.into()
                    })),
                    Value::Number(serde_json::Number::from_f64(low).unwrap_or_else(|| {
                        eprintln!("[YAHOO] low f64->number dönüşüm hatası, default 0");
                        0.into()
                    })),
                    Value::Number(serde_json::Number::from_f64(close).unwrap_or_else(|| {
                        eprintln!("[YAHOO] close f64->number dönüşüm hatası, default 0");
                        0.into()
                    })),
                    Value::Number(serde_json::Number::from_f64(volume).unwrap_or_else(|| {
                        eprintln!("[YAHOO] volume f64->number dönüşüm hatası, default 0");
                        0.into()
                    })),
                ]);
            }
            Ok(klines)
        } else {
            Ok(vec![])
        }
    } else {
        Ok(vec![])
    }
    // Fonksiyonun kalan kısmı (BIST API fallback) dışarıda kalmalı
}
// ---

/// Interval'ı BIST API formatına çevir
#[allow(dead_code)]
fn map_interval_to_bist(interval: &str) -> &str {
    match interval {
        "1m" => "1min",
        "3m" => "3min",
        "5m" => "5min",
        "15m" => "15min",
        "30m" => "30min",
        "1h" => "1hour",
        "2h" => "2hour",
        "4h" => "4hour",
        "6h" => "6hour",
        "8h" => "8hour",
        "12h" => "12hour",
        "1d" => "1day",
        "3d" => "3day",
        "1w" => "1week",
        "1M" => "1month",
        _ => interval,
    }
}

/// BIST 100 için base URL
pub fn get_bist_base_url() -> &'static str {
    "https://api.borsaistanbul.com/v1"
}
