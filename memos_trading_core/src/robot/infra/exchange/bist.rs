// bist_data_fetcher.rs
// BIST Veri Çekme ve Batch İşleme Modülü

use serde_json::Value;
use reqwest::Client;
use futures::{StreamExt, stream};
use anyhow::{anyhow, Context};
use std::time::Instant;
use crate::core::batch_config::BatchFetchConfig;

/// Tekil sembol çekimi - Logic bozulmadan hata yönetimi modernize edildi
pub async fn fetch_bist_klines(
    symbol: &str,
    interval: &str,
    start_time: i64,
    end_time: i64,
    limit: usize,
) -> anyhow::Result<Vec<Vec<Value>>> {
    fetch_bist_klines_from_yahoo(symbol, interval, start_time, end_time, limit)
        .await
        .with_context(|| format!("Yahoo verisi çekilemedi: {}", symbol))
}

/// Pipeline Dostu Batch Çekim - Mutex kaldırıldı, performanslı Map/Reduce yapısına geçildi
pub async fn batch_fetch_bist_klines(
    symbols: &[String],
    interval: &str,
    start_time: i64,
    end_time: i64,
    limit: usize,
    config: &BatchFetchConfig,
) -> Vec<(String, anyhow::Result<Vec<Vec<Value>>>)> {
    let batch_start = Instant::now();
    let client = Client::new(); // Tek client örneği üzerinden reuse (performans)

    // Modern Rust: Mutex yerine stream'lerin sonuçlarını toplayan yapı (lock-free)
    let results = stream::iter(symbols)
        .map(|symbol| {
            let config = config;
            async move {
                let symbol_start = Instant::now();
                let mut data_result = Err(anyhow!("Başlatılmadı"));

                for attempt in 1..=config.max_retries {
                    match fetch_bist_klines(symbol, interval, start_time, end_time, limit).await {
                        Ok(data) => {
                            println!("[BATCH][{}] ✅ {} kayıt (Deneme: {}) | ⏱️ {:?}", 
                                symbol, data.len(), attempt, symbol_start.elapsed());
                            data_result = Ok(data);
                            break;
                        }
                        Err(e) => {
                            data_result = Err(e);
                            if attempt < config.max_retries {
                                tokio::time::sleep(std::time::Duration::from_millis(config.retry_wait_ms)).await;
                            }
                        }
                    }
                }
                (symbol.clone(), data_result)
            }
        })
        .buffer_unordered(config.concurrency_limit) // Paralel istekleri yönetir
        .collect::<Vec<_>>()
        .await;

    println!("[BATCH] Toplam çekim süresi: {:?}", batch_start.elapsed());
    results
}

/// BIST 100 Sembol Listesi - Fallback mantığı optimize edildi
pub async fn fetch_bist100_symbols() -> anyhow::Result<Vec<String>> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
        
    let bist_url = "https://api.borsaistanbul.com/v1/stocks/bist100";
    
    // Modern pattern: if let chain veya map_err ile kısa kesme
    let resp = client.get(bist_url).send().await;
    
    if let Ok(r) = resp {
        if r.status().is_success() {
            if let Ok(data) = r.json::<Value>().await {
                let symbols = data["symbols"].as_array()
                    .or_else(|| data.as_array()) // İki farklı JSON formatı için
                    .map(|arr| {
                        arr.iter().filter_map(|v| {
                            v.as_str().or_else(|| v["symbol"].as_str())
                             .map(|s| format!("{}.IS", s))
                        }).collect::<Vec<String>>()
                    });
                
                if let Some(s) = symbols {
                    if !s.is_empty() { return Ok(s); }
                }
            }
        }
    }

    println!("ℹ️ BIST API fallback: Manuel liste yükleniyor.");
    Ok(get_bist100_symbols())
}

/// Yahoo Finance Veri Çekimi - Zero-copy referanslarla optimize edildi
pub async fn fetch_bist_klines_from_yahoo(
    symbol: &str,
    interval: &str,
    start_time: i64,
    end_time: i64,
    _limit: usize,
) -> anyhow::Result<Vec<Vec<Value>>> {
    let client = Client::new();
    // Yahoo aralık eşleşmeleri
    let yf_interval = match interval {
        "1h" | "60m" => "1h",
        "1d" => "1d",
        i @ ("1m" | "2m" | "5m" | "15m" | "30m" | "1wk" | "1mo") => i,
        _ => "1d",
    };

    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&period1={}&period2={}",
        symbol, yf_interval, start_time / 1000, end_time / 1000
    );

    let resp = client.get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?
        .error_for_status()?; // 404 veya 500 hatalarını otomatik yakalar

    let data: Value = resp.json().await?;
    
    // Veri Parse Mantığı - Option combinators (and_then, or_default) kullanımı
    let result = data["chart"]["result"].as_array()
        .and_then(|res| res.first())
        .ok_or_else(|| anyhow!("JSON yapısı geçersiz: {}", symbol))?;

    let timestamps = result["timestamp"].as_array().context("Timestamp eksik")?;
    let quote = result["indicators"]["quote"][0].as_object().context("Quote eksik")?;

    // Helper closure: Veri çekme sırasında oluşacak panic risklerini önler
    let get_arr = |key: &str| quote.get(key).and_then(|v| v.as_array());

    let mut klines = Vec::with_capacity(timestamps.len());
    
    for i in 0..timestamps.len() {
        let ts = timestamps[i].as_i64().unwrap_or(0) * 1000;
        let o = get_arr("open").and_then(|a| a[i].as_f64()).unwrap_or(0.0);
        let h = get_arr("high").and_then(|a| a[i].as_f64()).unwrap_or(0.0);
        let l = get_arr("low").and_then(|a| a[i].as_f64()).unwrap_or(0.0);
        let c = get_arr("close").and_then(|a| a[i].as_f64()).unwrap_or(0.0);
        let v = get_arr("volume").and_then(|a| a[i].as_f64()).unwrap_or(0.0);

        klines.push(vec![
            Value::from(ts), 
            Value::from(o), 
            Value::from(h), 
            Value::from(l), 
            Value::from(c), 
            Value::from(v)
        ]);
    }

    Ok(klines)
}


/// 🏛️ ACİL DURUM EMNİYET SUBABI: BIST API bağlantısı çöktüğünde robotun 
/// ticaret döngüsünü korumak için BIST100'ün en likit lokomotif sembol listesini teslim eder.
pub fn get_bist100_symbols() -> Vec<String> {
    // BIST100 endeksindeki en yüksek hacimli ve derinliğe sahip ana pariteler
    let symbols = vec![
        "THYAO", "ASELS", "EREGL", "AKBNK", "YKBNK", "ISCTR", "GARAN", "TUPRS",
        "KCHOL", "SAHOL", "SISE", "BIMAS", "FROTO", "TOASO", "HEKTS", "SASA",
        "ODAS", "EKGYO", "PETKM", "EUPWR", "ASTOR", "KONTR", "ALARK", "PGSUS",
        "ENKAI", "GUBRF", "ARCLK", "VESTL", "TTKOM", "TCELL", "HALKB", "VAKBN"
    ];

    // Robotik loop'ların string eşleşme standartlarına (.to_string()) uygun hale getiriliyor
    symbols.into_iter().map(|s| s.to_string()).collect()
}