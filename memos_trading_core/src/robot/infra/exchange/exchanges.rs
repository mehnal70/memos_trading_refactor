// data_loader.rs - Veri İndirme, Senkronizasyon ve Raporlama Modülü

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::{Value, json, Map};
use std::time::Duration;
use std::{thread, fs::File, io::Write};
use anyhow::{Result, anyhow, Context};
use crate::database_writer::{DBWriter, parse_binance_kline_ref, save_candle};
use crate::types::Candle;

// --- 1. YARDIMCI YAPILAR VE FONKSİYONLAR ---

#[allow(dead_code)]
pub enum ReportTypeArg { AvgClose, AvgVolume, MinClose, Volatility }
#[allow(dead_code)]
pub enum OutputType { Console, Json, File }

fn interval_to_ms(interval: &str) -> i64 {
    match interval {
        "1m" => 60_000, "5m" => 300_000, "15m" => 900_000, "1h" => 3_600_000,
        "4h" => 14_400_000, "1d" => 86_400_000, _ => 60_000,
    }
}

pub fn parse_candle(candle: &[Value]) -> Option<(i64, f64, f64, f64, f64, f64)> {
    let ts = candle.get(0)?.as_i64()?;
    let get_f64 = |idx: usize| candle.get(idx).and_then(|v| {
        v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    });

    let (open, high, low, close, vol) = (get_f64(1)?, get_f64(2)?, get_f64(3)?, get_f64(4)?, get_f64(5)?);
    
    // K4: Fiziksel tutarlılık kontrolü
    if high <= 0.0 || low <= 0.0 || high < open.max(close) || low > open.min(close) { return None; }
    Some((ts, open, high, low, close, vol))
}

// --- 2. VERİ İNDİRME VE SENKRONİZASYON (REST) ---

pub async fn download_bist_yahoo(
    symbol: &str,
    interval: &str,
    start_time: i64,
    end_time: i64,
    _limit: usize,
) -> Result<Vec<Vec<Value>>> {
    let client = Client::new();
    let yf_interval = match interval {
        "1h" | "60m" => "1h", "1w" => "1wk", "1M" => "1mo",
        i @ ("1m" | "2m" | "5m" | "15m" | "30m" | "1d") => i,
        _ => "1d",
    };

    let url = format!(
        "https://yahoo.com{}?interval={}&period1={}&period2={}",
        symbol, yf_interval, start_time / 1000, end_time / 1000
    );

    let resp = client.get(&url).header("User-Agent", "Mozilla/5.0").send().await?;
    if !resp.status().is_success() { return Ok(vec![]); }

    let data: Value = resp.json().await?;
    let result = data["chart"]["result"].as_array().and_then(|r| r.first()).ok_or(anyhow!("Veri yok"))?;
    let (ts_arr, quote) = (result["timestamp"].as_array().context("TS eksik")?, result["indicators"]["quote"].as_array().and_then(|q| q.first()).context("Quote eksik")?);

    let mut klines = Vec::with_capacity(ts_arr.len());
    let get_val = |key: &str, i: usize| quote[key].get(i).and_then(|v| v.as_f64()).and_then(Value::from_f64).unwrap_or(Value::from(0));

    for i in 0..ts_arr.len() {
        klines.push(vec![
            Value::from(ts_arr[i].as_i64().unwrap_or(0) * 1000),
            get_val("open", i), get_val("high", i), get_val("low", i), get_val("close", i), get_val("volume", i)
        ]);
    }
    Ok(klines)
}

pub async fn run_realtime_sync(db_writer: &DBWriter, exchange: &str, market: &str, symbol: &str) -> Result<()> {
    let base_url = match (exchange, market) {
        ("binance", "spot") => "https://binance.com",
        ("binance", "futures") => "https://binance.com",
        _ => return Err(anyhow!("Desteklenmeyen market")),
    };

    let url = format!("{}/klines?symbol={}&interval=1m&limit=1", base_url, symbol);
    let data: Vec<Vec<Value>> = Client::new().get(&url).send().await?.json().await?;

    if let Some(kline) = data.first() {
        if let Some(candle) = parse_binance_kline_ref(kline, symbol, "1m") {
            db_writer.write_candle(exchange, market, candle);
            return Ok(());
        }
    }
    Err(anyhow!("Sync başarısız"))
}

// --- 3. RAPORLAMA VE ANALİZ ---

pub fn generate_reports(
    conn: &rusqlite::Connection,
    symbol: &str,
    report_types: Vec<ReportTypeArg>,
    output: OutputType,
    file_path: Option<String>,
) -> Result<()> {
    let mut results = Map::new();
    results.insert("symbol".to_owned(), json!(symbol));

    for rt in report_types {
        let (key, sql) = match rt {
            ReportTypeArg::AvgClose => ("avg_close", "SELECT AVG(close) FROM candles WHERE symbol = ?1"),
            ReportTypeArg::Volatility => ("volatility", "SELECT (MAX(close) - MIN(close)) / NULLIF(AVG(close), 0) FROM candles WHERE symbol = ?1"),
            _ => continue,
        };
        let val: f64 = conn.query_row(sql, [symbol], |r| r.get(0)).unwrap_or(0.0);
        results.insert(key.to_owned(), json!(val));
    }

    let report_str = serde_json::to_string_pretty(&Value::Object(results))?;
    match output {
        OutputType::Console | OutputType::Json => println!("{}", report_str),
        OutputType::File => {
            if let Some(p) = file_path {
                let mut f = File::create(p)?;
                f.write_all(report_str.as_bytes())?;
            }
        }
    }
    Ok(())
}

pub fn analyze_existing_data_for_strategy(conn: &rusqlite::Connection, symbol: &str, interval: &str) -> Result<()> {
    let mut stmt = conn.prepare("SELECT close FROM candles WHERE symbol = ?1 AND interval = ?2 ORDER BY timestamp DESC LIMIT 100")?;
    let mut closes: Vec<f64> = stmt.query_map([symbol, interval], |r| r.get(0))?.filter_map(Result::ok).collect();
    
    if closes.len() < 30 { return Ok(()); }
    closes.reverse();
    let (first, second) = closes.split_at(closes.len() / 2);
    let (m1, m2) = (first.iter().sum::<f64>() / first.len() as f64, second.iter().sum::<f64>() / second.len() as f64);
    
    let trend = if m2 > m1 * 1.02 { "📈 YÜKSELİŞ" } else if m2 < m1 * 0.98 { "📉 DÜŞÜŞ" } else { "➡️ YATAY" };
    println!("   {} Trend: {}", symbol, trend);
    Ok(())
}

pub fn clear(conn: &rusqlite::Connection) -> Result<()> {
    let tables = ["candles", "ticks", "signals", "portfolio", "trades"];
    for t in &tables {
        let _ = conn.execute(&format!("DELETE FROM {}", t), []);
    }
    println!("🧹 Tablolar temizlendi.");
    Ok(())
}
