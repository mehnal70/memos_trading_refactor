/// Yahoo Finance üzerinden BIST veri çekme (async/await, User-Agent)
pub async fn download_bist_yahoo(
    symbol: &str,
    interval: &str,
    start_time: i64,
    end_time: i64,
    _limit: usize,
) -> Result<Vec<Vec<serde_json::Value>>, reqwest::Error> {
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
    let data: serde_json::Value = resp.json().await?;
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
                    .unwrap_or(0) * 1000;
                let open = opens.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let high = highs.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let low = lows.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let close = closes.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let volume = volumes.and_then(|arr| arr.get(i))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                klines.push(vec![
                    serde_json::Value::Number(timestamp_ms.into()),
                    serde_json::Value::Number(serde_json::Number::from_f64(open).unwrap_or(0.into())),
                    serde_json::Value::Number(serde_json::Number::from_f64(high).unwrap_or(0.into())),
                    serde_json::Value::Number(serde_json::Number::from_f64(low).unwrap_or(0.into())),
                    serde_json::Value::Number(serde_json::Number::from_f64(close).unwrap_or(0.into())),
                    serde_json::Value::Number(serde_json::Number::from_f64(volume).unwrap_or(0.into())),
                ]);
            }
            Ok(klines)
        } else {
            Ok(vec![])
        }
    } else {
        Ok(vec![])
    }
}
use crate::database_writer::DBWriter;
use serde_json::json;
use reqwest::blocking::Client;
use std::{thread, time::Duration};

#[allow(dead_code)]
pub enum ReportTypeArg { AvgClose, AvgVolume, MinClose, Volatility }
#[allow(dead_code)]
pub enum OutputType { Console, Json, File }

fn interval_to_ms(interval: &str) -> i64 {
    match interval {
        "1m"  => 60_000,
        "3m"  => 180_000,
        "5m"  => 300_000,
        "15m" => 900_000,
        "30m" => 1_800_000,
        "1h"  => 3_600_000,
        "4h"  => 14_400_000,
        "1d"  => 86_400_000,
        "1w"  => 604_800_000,
        _     => 60_000,
    }
}

fn parse_candle(candle: &[serde_json::Value]) -> Option<(i64, f64, f64, f64, f64, f64)> {
    let ts          = candle.get(0)?.as_i64()?;
    let open:   f64 = candle.get(1)?.as_str()?.parse().ok()?;
    let high:   f64 = candle.get(2)?.as_str()?.parse().ok()?;
    let low:    f64 = candle.get(3)?.as_str()?.parse().ok()?;
    let close:  f64 = candle.get(4)?.as_str()?.parse().ok()?;
    let vol:    f64 = candle.get(7)?.as_str()?.parse().ok()?;
    // Fiziksel tutarlılık: bozuk OHLCV (high=0, high<open, vs.) mum atlanır.
    // Aksi halde aşağı akışta sürekli "K4⚠ fiziksel ihlal" log'u oluşur ve
    // hatalı sinyal/risk hesabı tetiklenebilir.
    if !(high > 0.0 && low > 0.0 && open > 0.0 && close > 0.0) { return None; }
    if high < open.max(close) || low > open.min(close) { return None; }
    Some((ts, open, high, low, close, vol))
}

// DBWriter işçi thread mantığı database_writer.rs içinde tanımlı, burada tekrar tanımlamaya gerek yok.


pub fn insert_candle(
    conn: &rusqlite::Connection,
    exchange: &str,
    market: &str,
    symbol: &str,
    interval: &str,
    k: &Vec<serde_json::Value>,
) -> Result<bool, rusqlite::Error> {
    // Binance kline array'ini Candle'a parse et
    let k_json = serde_json::Value::Array(k.clone());
    if let Some(candle) = crate::database_writer::parse_binance_kline(&k_json, symbol, interval) {
        // save_candle Result<bool, MemosTradingError> -> Result<bool, rusqlite::Error>
        match crate::database_writer::save_candle(conn, exchange, market, &candle) {
            Ok(val) => Ok(val),
            Err(_) => Ok(false),
        }
    } else {
        Ok(false)
    }
}

pub fn download(
    conn: &rusqlite::Connection,
    symbol: &str,
    interval: &str,
    exchange: &str,
    market: &str,
) -> Result<(), String> {
    // Market tipine göre endpoint seçimi
    let base_url = match (exchange, market) {
        ("binance", "spot") => "https://api.binance.com/api/v3",
        ("binance", "futures") => "https://fapi.binance.com/fapi/v1",
        ("binance", "coinm") => "https://dapi.binance.com/dapi/v1",
        ("bist", "bist100") | ("bist", "stocks") => {
            // BIST için özel işlem
            return Err("BIST için ayrı fonksiyon kullanılmalı".to_string());
        }
        _ => {
            println!("⚠️ Desteklenmeyen borsa/market: {} {}", exchange, market);
            return Ok(());
        }
    };


    let url = format!("{}/klines?symbol={}&interval={}&limit=1000", base_url, symbol, interval);
    println!("📡 {} ({}) için veri çekiliyor...", symbol, market);
    println!("   URL: {}", url);
    let client = Client::new();
    let mut retries = 0;
    let max_retries = 5;
    let mut last_err = None;
    let mut data: Vec<Vec<serde_json::Value>> = Vec::new();
    while retries < max_retries {
        let resp = client.get(&url).send();
        match resp {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    match resp.json::<Vec<Vec<serde_json::Value>>>() {
                        Ok(d) => {
                            data = d;
                            println!("   📊 API'den {} kayıt alındı", data.len());
                            thread::sleep(Duration::from_millis(1200));
                            break;
                        },
                        Err(e) => {
                            last_err = Some(format!("JSON parse hatası: {} - URL: {}", e, url));
                        }
                    }
                } else if status.as_u16() == 429 {
                    let wait = 1000 * (retries + 1);
                    println!("⚠️ 429 Too Many Requests, {} ms bekleniyor...", wait);
                    thread::sleep(Duration::from_millis(wait as u64));
                    retries += 1;
                    continue;
                } else {
                    let error_text = resp.text().unwrap_or_else(|_| "Bilinmeyen hata".to_string());
                    last_err = Some(format!("{} API hatası ({} {}): {} - URL: {}", exchange, status.as_u16(), status.as_str(), error_text, url));
                }
            },
            Err(e) => {
                last_err = Some(format!("HTTP istek hatası: {}", e));
            }
        }
        thread::sleep(Duration::from_millis(1000));
        retries += 1;
    }
    if data.is_empty() {
        if let Some(err) = last_err {
            return Err(err);
        } else {
            println!("⚠️ {} için veri bulunamadı ({} {}).", symbol, exchange, market);
            return Ok(());
        }
    }

        let mut saved_count = 0;
        let mut skipped_count = 0;
        let mut error_count = 0;

        for k in &data {
            match insert_candle(conn, exchange, market, symbol, interval, k) {
                Ok(was_inserted) => {
                    if was_inserted {
                        saved_count += 1;
                    } else {
                        skipped_count += 1; // Duplicate kayıt
                    }
                },
                Err(e) => {
                    error_count += 1;
                    println!("   ⚠️ Kayıt hatası: {}", e);
                }
            }
            // Her kayıt sonrası kısa bekleme (rate limit koruması)
            thread::sleep(Duration::from_millis(200));
        }
        // Gerçekten kaç kayıt eklendiğini kontrol et
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM candles WHERE symbol = ?1 AND interval = ?2").map_err(|e| e.to_string())?;
        let actual_count: i64 = stmt.query_row([symbol, interval], |row| row.get(0)).unwrap_or(0);
        println!("   💾 Veritabanında toplam {} kayıt var", actual_count);
        println!("✅ {} için {} yeni kayıt eklendi, {} duplicate atlandı ({} {}).", 
                 symbol, saved_count, skipped_count, exchange, market);
        if error_count > 0 {
            println!("   ⚠️ {} kayıt kaydedilemedi", error_count);
        }
        if saved_count == 0 && skipped_count > 0 {
            println!("   ℹ️  Tüm kayıtlar zaten mevcut (duplicate)");
        }

        Ok(())
    }


// BIST için özel download fonksiyonu şimdilik devre dışı (eksik modül)
// fn download_bist(...) { /* ... */ }


/// Çoklu sembol için SENKRON veri çekme ve DBWriter ile yazma
pub fn bulk_download_sync(
    db_writer: &DBWriter,
    exchange: &str,
    market: &str,
    interval: &str,
    from_ms: i64,
    to_ms: i64,
    symbols: Option<Vec<String>>,
) -> Result<(), String> {
    let list = symbols.unwrap_or_else(|| vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()]);
    let base_url = match (exchange, market) {
        ("binance", "spot") => "https://api.binance.com/api/v3",
        ("binance", "futures") => "https://fapi.binance.com/fapi/v1",
        ("binance", "coinm") => "https://dapi.binance.com/dapi/v1",
        ("bist", "bist100") | ("bist", "stocks") => {
            println!("⚠️ BIST için bulk_download henüz desteklenmiyor. Lütfen download fonksiyonunu kullanın.");
            return Ok(());
        }
        _ => {
            println!("⚠️ Desteklenmeyen borsa/market: {} {}", exchange, market);
            return Ok(());
        }
    };
    let interval_ms = interval_to_ms(interval);
    let max_limit = 1000;
    let total_range = (to_ms - from_ms) / interval_ms;
    for sym in list {
        if total_range <= max_limit as i64 {
            let url = format!(
                "{}/klines?symbol={}&interval={}&startTime={}&endTime={}&limit={}",
                base_url, sym, interval, from_ms, to_ms, max_limit
            );
            let client = Client::new();
            let resp = client.get(&url).send().map_err(|e| format!("HTTP istek hatası: {} - URL: {}", e, url))?;
            let status = resp.status();
            if !status.is_success() {
                let error_text = resp.text().unwrap_or_else(|_| "Bilinmeyen hata".to_string());
                println!("⚠️ {} için API hatası ({} {}): {}", sym, status.as_u16(), status.as_str(), error_text);
                continue;
            }
            let data: Vec<Vec<serde_json::Value>> = resp.json().map_err(|e| format!("JSON parse hatası: {} - URL: {}", e, url))?;
            for k in &data {
                let k_json: serde_json::Value = serde_json::Value::Array(k.clone());
                if let Some(candle) = crate::database_writer::parse_binance_kline(&k_json, &sym, interval) {
                    db_writer.write_candle(exchange, market, candle);
                }
            }
        } else {
            let mut current_start = from_ms;
            while current_start < to_ms {
                let current_end = (current_start + (max_limit as i64 * interval_ms)).min(to_ms);
                let url = format!(
                    "{}/klines?symbol={}&interval={}&startTime={}&endTime={}&limit={}",
                    base_url, sym, interval, current_start, current_end, max_limit
                );
                let client = Client::new();
                let resp = client.get(&url).send().map_err(|e| format!("HTTP istek hatası: {} - URL: {}", e, url))?;
                let status = resp.status();
                if !status.is_success() {
                    let error_text = resp.text().unwrap_or_else(|_| "Bilinmeyen hata".to_string());
                    println!("⚠️ {} için API hatası ({} {}): {}", sym, status.as_u16(), status.as_str(), error_text);
                    current_start = current_end + interval_ms;
                    continue;
                }
                let data: Vec<Vec<serde_json::Value>> = resp.json().map_err(|e| format!("JSON parse hatası: {} - URL: {}", e, url))?;
                for k in &data {
                    let k_json: serde_json::Value = serde_json::Value::Array(k.clone());
                    if let Some(candle) = crate::database_writer::parse_binance_kline(&k_json, &sym, interval) {
                        db_writer.write_candle(exchange, market, candle);
                    }
                }
                if let Some(last_candle) = data.last() {
                    if let Some((last_ts, _, _, _, _, _)) = parse_candle(last_candle) {
                        current_start = last_ts + interval_ms;
                    } else {
                        current_start = current_end + interval_ms;
                    }
                } else {
                    current_start = current_end + interval_ms;
                }
                // API rate limit için kısa bekleme
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        println!("✅ {} indirildi ({} {}).", sym, exchange, market);
    }
    Ok(())
}

/// Senkron gerçek-zamanlı senkronizasyon: son kline verisini çekip DB'ye yazar
pub fn run_realtime_sync(db_writer: &DBWriter, exchange: &str, market: &str, symbol: &str) -> Result<(), String> {
    let base_url = match (exchange, market) {
        ("binance", "spot") => "https://api.binance.com/api/v3",
        ("binance", "futures") => "https://fapi.binance.com/fapi/v1",
        ("binance", "coinm") => "https://dapi.binance.com/dapi/v1",
        _ => return Err(format!("Desteklenmeyen borsa/market: {} {}", exchange, market)),
    };

    let url = format!("{}/klines?symbol={}&interval=1m&limit=1", base_url, symbol);
    let client = Client::new();
    let response = client
        .get(&url)
        .send()
        .map_err(|e| format!("Realtime sync HTTP hatası: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_else(|_| "Bilinmeyen hata".to_string());
        return Err(format!("Realtime sync API hatası ({}): {}", status, body));
    }

    let data: Vec<Vec<serde_json::Value>> = response
        .json()
        .map_err(|e| format!("Realtime sync JSON parse hatası: {}", e))?;

    if let Some(kline) = data.first() {
        let k_json = serde_json::Value::Array(kline.clone());
        if let Some(candle) = crate::database_writer::parse_binance_kline(&k_json, symbol, "1m") {
            db_writer.write_candle(exchange, market, candle);
            return Ok(());
        }
        return Err("Realtime sync: kline parse edilemedi".to_string());
    }

    Err("Realtime sync: boş veri döndü".to_string())
}

/// Sadece rusqlite ile rapor üretir (senkron)
pub fn generate_reports(
    conn: &rusqlite::Connection,
    symbol: &str,
    report_types: Vec<ReportTypeArg>,
    output: OutputType,
    file: Option<String>,
) -> Result<(), rusqlite::Error> {
    let mut results = serde_json::Map::new();
    results.insert("symbol".to_string(), json!(symbol));
    for report_type in report_types {
        match report_type {
            ReportTypeArg::AvgClose => {
                let mut stmt = conn.prepare("SELECT AVG(close) AS value FROM candles WHERE symbol = ?1")?;
                let value: f64 = stmt.query_row([symbol], |row| row.get(0)).unwrap_or(0.0);
                results.insert("avg_close".to_string(), json!(value));
            }
            ReportTypeArg::AvgVolume => {
                let mut stmt = conn.prepare("SELECT AVG(volume) AS value FROM candles WHERE symbol = ?1")?;
                let value: f64 = stmt.query_row([symbol], |row| row.get(0)).unwrap_or(0.0);
                results.insert("avg_volume".to_string(), json!(value));
            }
            ReportTypeArg::MinClose => {
                let mut stmt = conn.prepare("SELECT MIN(close) AS value FROM candles WHERE symbol = ?1")?;
                let value: f64 = stmt.query_row([symbol], |row| row.get(0)).unwrap_or(0.0);
                results.insert("min_close".to_string(), json!(value));
            }
            ReportTypeArg::Volatility => {
                let mut stmt = conn.prepare("SELECT (MAX(close) - MIN(close)) / AVG(close) AS value FROM candles WHERE symbol = ?1")?;
                let value: f64 = stmt.query_row([symbol], |row| row.get(0)).unwrap_or(0.0);
                results.insert("volatility".to_string(), json!(value));
            }
        }
    }
    let obj = serde_json::Value::Object(results);
    match output {
        OutputType::Console => {
            println!("📊 Rapor: {}", obj);
        }
        OutputType::Json => {
            println!("{}", serde_json::to_string_pretty(&obj).unwrap());
        }
        OutputType::File => {
            if let Some(path) = file {
                use std::fs::File;
                use std::io::Write;
                let mut f = File::create(path).expect("dosya açılamadı");
                f.write_all(serde_json::to_string_pretty(&obj).unwrap().as_bytes()).expect("yazılamadı");
                println!("✅ Rapor dosyaya yazıldı.");
            } else {
                println!("ℹ️ Dosya yolu gerekli (--file).");
            }
        }
    }
    Ok(())
}


/// Mevcut veriler üzerinde strateji analizi yapar (AI fonksiyonları için)
#[allow(dead_code)]
#[allow(dead_code)]
fn analyze_existing_data_for_strategy(
    conn: &rusqlite::Connection,
    symbol: &str,
    interval: &str,
) -> Result<(), rusqlite::Error> {
    // Son 100 candle'ı kontrol et
    let mut stmt = conn.prepare(
        "SELECT close FROM candles WHERE symbol = ?1 AND interval = ?2 ORDER BY timestamp DESC LIMIT 100"
    )?;
    let closes_iter = stmt.query_map(&[symbol, interval], |row| row.get::<_, f64>(0))?;
    let mut closes: Vec<f64> = closes_iter.filter_map(Result::ok).collect();
    closes.reverse(); // En eski başa gelsin
    if closes.len() < 30 {
        // Yetersiz veri
        return Ok(());
    }
    // Basit trend analizi (örnek)
    let first_half: f64 = closes[..15].iter().sum::<f64>() / 15.0;
    let second_half: f64 = closes[15..].iter().sum::<f64>() / 15.0;
    let trend = if second_half > first_half * 1.02 {
        "YÜKSELİŞ"
    } else if second_half < first_half * 0.98 {
        "DÜŞÜŞ"
    } else {
        "YATAY"
    };
    println!("   📈 {} ({}): Trend = {}", symbol, interval, trend);
    Ok(())
}

pub fn clear(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    for table in &["candles", "ticks", "signals", "consensus_signals", "portfolio", "trades"] {
        let q = format!("DELETE FROM {}", table);
        conn.execute(&q, []).ok();
    }
    println!("🧹 Tüm tablolar temizlendi.");
    Ok(())
}
