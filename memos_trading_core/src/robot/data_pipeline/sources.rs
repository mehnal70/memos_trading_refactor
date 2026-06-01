// robot/data_pipeline/sources.rs - Çok Kaynaklı Veri Yönetimi ve Fallback Sistemi
// robot/data_pipeline/sources.rs - Çok Kanallı Veri Kaynakları

use async_trait::async_trait;
use crate::{types::Candle, Result, MemosTradingError};
use chrono::{DateTime, TimeZone, Utc};
#[cfg(not(target_arch = "wasm32"))]
use rusqlite::Connection;
use std::path::Path;
use super::FetchParams;

// --- 1. YARDIMCI ARAÇLAR ---

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(ms) = value.parse::<i64>() {
        return if ms < 9_999_999_999 {
            Utc.timestamp_opt(ms, 0).single()
        } else {
            Utc.timestamp_millis_opt(ms).single()
        };
    }
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

// --- 2. TRAIT VE ADAPTÖR ---

#[async_trait]
pub trait DataSource: Send + Sync {
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>>;
    fn source_type(&self) -> &str;
    async fn health_check(&self) -> Result<()>;
}

// --- 3. SOMUT KAYNAKLAR ---

/// CSV Kaynağı
pub struct CsvDataSource { pub base_path: String }
#[async_trait]
impl DataSource for CsvDataSource {
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>> {
        let path = Path::new(&self.base_path).join(format!("{}_{}.csv", params.symbol, params.interval));
        let content = std::fs::read_to_string(&path)?;
        let mut candles: Vec<Candle> = content.lines()
            .filter_map(|line| {
                let p: Vec<&str> = line.split(',').collect();
                Some(Candle {
                    timestamp: parse_timestamp(p[0])?,
                    open: p[1].parse().ok()?, high: p[2].parse().ok()?,
                    low: p[3].parse().ok()?, close: p[4].parse().ok()?,
                    volume: p[5].parse().ok()?,
                    symbol: params.symbol.clone(), interval: params.interval.clone(),
                })
            }).collect();
        if let Some(limit) = params.limit { candles = candles.into_iter().rev().take(limit).collect::<Vec<_>>().into_iter().rev().collect(); }
        Ok(candles)
    }
    fn source_type(&self) -> &str { "CSV" }
    async fn health_check(&self) -> Result<()> { Ok(()) }
}

/// SQLite Kaynağı
pub struct DatabaseDataSource { pub connection_string: String }
#[async_trait]
impl DataSource for DatabaseDataSource {
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>> {
        let conn = crate::persistence::open_db(&self.connection_string)?;
        let limit = params.limit.unwrap_or(500);
        let mut stmt = conn.prepare("SELECT timestamp, open, high, low, close, volume FROM candles WHERE symbol = ?1 AND interval = ?2 ORDER BY timestamp DESC LIMIT ?3")?;
        let mut candles: Vec<Candle> = stmt.query_map([&params.symbol, &params.interval, &limit.to_string()], |row| {
            let ts_raw: String = row.get(0)?;
            Ok(Candle {
                timestamp: parse_timestamp(&ts_raw).unwrap_or_else(Utc::now),
                open: row.get(1)?, high: row.get(2)?, low: row.get(3)?,
                close: row.get(4)?, volume: row.get(5)?,
                symbol: params.symbol.clone(), interval: params.interval.clone(),
            })
        })?.filter_map(|r| r.ok()).collect();
        candles.reverse();
        Ok(candles)
    }
    fn source_type(&self) -> &str { "Database" }
    async fn health_check(&self) -> Result<()> { Ok(()) }
}

/// API Kaynağı
pub struct ApiDataSource { pub base_url: String, pub api_key: Option<String> }
#[async_trait]
impl DataSource for ApiDataSource {
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>> {
        let client = reqwest::Client::new();
        let url = format!("{}/fapi/v1/klines", self.base_url.trim_end_matches('/'));
        let resp = client.get(url).query(&[("symbol", &params.symbol), ("interval", &params.interval), ("limit", &params.limit.unwrap_or(500).to_string())]).send().await?;
        let raw: Vec<Vec<serde_json::Value>> = resp.json().await?;
        Ok(raw.into_iter().filter_map(|item| {
            Some(Candle {
                timestamp: Utc.timestamp_millis_opt(item[0].as_i64()?).single()?,
                open: item[1].as_str()?.parse().ok()?, high: item[2].as_str()?.parse().ok()?,
                low: item[3].as_str()?.parse().ok()?, close: item[4].as_str()?.parse().ok()?,
                volume: item[5].as_str()?.parse().ok()?,
                symbol: params.symbol.clone(), interval: params.interval.clone(),
            })
        }).collect())
    }
    fn source_type(&self) -> &str { "API" }
    async fn health_check(&self) -> Result<()> { Ok(()) }
}

/// Hybrid Kaynağı (Fallback)
pub struct HybridDataSource { pub sources: Vec<Box<dyn DataSource>> }
#[async_trait]
impl DataSource for HybridDataSource {
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>> {
        for source in &self.sources { if let Ok(data) = source.fetch(params).await { return Ok(data); } }
        Err(MemosTradingError::Config("Tüm veri kaynakları başarısız oldu".into()).into())
    }
    fn source_type(&self) -> &str { "Hybrid" }
    async fn health_check(&self) -> Result<()> { Ok(()) }
}

pub struct DataSourceManager { pub hybrid_source: HybridDataSource }
impl DataSourceManager {
    pub fn new() -> Self { Self { hybrid_source: HybridDataSource { sources: Vec::new() } } }
    pub fn add_source(&mut self, source: Box<dyn DataSource>) { self.hybrid_source.sources.push(source); }
}



