use crate::robot::interfaces::DataFetcher;
/// DataSource'u DataFetcher trait'ine adapte eden struct
pub struct DataSourceAdapter {
    pub inner: Box<dyn DataSource>,
}

impl DataSourceAdapter {
    pub fn new(inner: Box<dyn DataSource>) -> Self {
        Self { inner }
    }
}

impl DataFetcher for DataSourceAdapter {
    fn fetch(&self, symbol: &str, interval: &str, limit: usize) -> crate::Result<Vec<crate::types::Candle>> {
        use crate::robot::data_pipeline::FetchParams;
        // Not: Bu fonksiyon async değil, DataSource'un fetch'i async olduğu için block_on ile çağrılır
        let params = FetchParams {
            symbol: symbol.to_string(),
            interval: interval.to_string(),
            start_time: None,
            end_time: None,
            limit: Some(limit),
        };
        // Sadece örnek: async çağrıyı bloklar
        // tokio_test::block_on(self.inner.fetch(&params))
        // Eğer bu kod test değilse, aşağıdaki gibi tokio runtime ile çalıştırılmalı:
        #[cfg(not(target_arch = "wasm32"))]
        {
            let rt = tokio::runtime::Runtime::new().map_err(|e| crate::MemosTradingError::from(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;
            rt.block_on(self.inner.fetch(&params))
        }
        #[cfg(target_arch = "wasm32")]
            {
                return Err(crate::MemosTradingError::Other("DataSourceAdapter is not supported in WASM builds".into()));
        }
    }
    fn source_type(&self) -> &str {
        self.inner.source_type()
    }
}
// robot/data_pipeline/sources.rs - Veri kaynakları

use async_trait::async_trait;
use crate::{types::Candle, Result};
use chrono::{DateTime, TimeZone, Utc};
#[cfg(not(target_arch = "wasm32"))]
use rusqlite::Connection;
use std::path::Path;
use super::FetchParams;

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(ms) = value.parse::<i64>() {
        return Utc.timestamp_millis_opt(ms).single();
    }

    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Veri kaynağı trait'i
#[async_trait]
pub trait DataSource: Send + Sync {
    /// Veriyi getir
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>>;
    
    /// Kaynağın türü
    fn source_type(&self) -> &str;
    
    /// Bağlantı test et
    async fn health_check(&self) -> Result<()>;
}

/// CSV dosyasından veri kaynağı
#[allow(dead_code)]
pub struct CsvDataSource {
    base_path: String,
}

impl CsvDataSource {
    pub fn new(base_path: String) -> Self {
        Self { base_path }
    }
}

#[async_trait]
impl DataSource for CsvDataSource {
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>> {
        let with_interval = format!("{}_{}.csv", params.symbol, params.interval);
        let fallback = format!("{}.csv", params.symbol);

        let primary_path = Path::new(&self.base_path).join(with_interval);
        let file_path = if primary_path.exists() {
            primary_path
        } else {
            Path::new(&self.base_path).join(fallback)
        };

        if !file_path.exists() {
            return Err(crate::MemosTradingError::Config(format!(
                "CSV dosyası bulunamadı: {}",
                file_path.display()
            )));
        }

        let content = std::fs::read_to_string(&file_path)?;
        let mut candles = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let parts: Vec<&str> = trimmed.split(',').map(|p| p.trim()).collect();
            if parts.len() < 6 {
                continue;
            }

            let Some(timestamp) = parse_timestamp(parts[0]) else {
                continue;
            };
            let Ok(open) = parts[1].parse::<f64>() else {
                continue;
            };
            let Ok(high) = parts[2].parse::<f64>() else {
                continue;
            };
            let Ok(low) = parts[3].parse::<f64>() else {
                continue;
            };
            let Ok(close) = parts[4].parse::<f64>() else {
                continue;
            };
            let Ok(volume) = parts[5].parse::<f64>() else {
                continue;
            };

            candles.push(Candle {
                timestamp,
                open,
                high,
                low,
                close,
                volume,
                symbol: params.symbol.clone(),
                interval: params.interval.clone(),
            });
        }

        if let Some(limit) = params.limit {
            if candles.len() > limit {
                candles = candles[candles.len() - limit..].to_vec();
            }
        }

        Ok(candles)
    }
    
    fn source_type(&self) -> &str {
        "CSV"
    }
    
    async fn health_check(&self) -> Result<()> {
        if !Path::new(&self.base_path).exists() {
            return Err(crate::MemosTradingError::Config(format!(
                "CSV base path bulunamadı: {}",
                self.base_path
            )));
        }
        Ok(())
    }
}

/// Veritabanından veri kaynağı
#[allow(dead_code)]
pub struct DatabaseDataSource {
    connection_string: String,
}

impl DatabaseDataSource {
    pub fn new(connection_string: String) -> Self {
        Self { connection_string }
    }
}

#[async_trait]
impl DataSource for DatabaseDataSource {
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>> {
        let conn = Connection::open(&self.connection_string)?;
        let limit = params.limit.unwrap_or(500) as i64;

        let mut stmt = conn.prepare(
            "SELECT timestamp, open, high, low, close, volume, symbol, interval
             FROM candles
             WHERE symbol = ?1 AND interval = ?2
             ORDER BY timestamp DESC
             LIMIT ?3"
        )?;

        let rows = stmt.query_map(
            rusqlite::params![params.symbol, params.interval, limit],
            |row| {
                let timestamp_raw: String = row.get(0)?;
                let timestamp = parse_timestamp(&timestamp_raw).unwrap_or_else(Utc::now);
                Ok(Candle {
                    timestamp,
                    open: row.get(1)?,
                    high: row.get(2)?,
                    low: row.get(3)?,
                    close: row.get(4)?,
                    volume: row.get(5)?,
                    symbol: row.get(6)?,
                    interval: row.get(7)?,
                })
            },
        )?;

        let mut candles: Vec<Candle> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        candles.reverse();
        Ok(candles)
    }
    
    fn source_type(&self) -> &str {
        "Database"
    }
    
    async fn health_check(&self) -> Result<()> {
        let conn = Connection::open(&self.connection_string)?;
        conn.execute("SELECT 1", [])?;
        Ok(())
    }
}

/// API'den veri kaynağı
#[allow(dead_code)]
pub struct ApiDataSource {
    base_url: String,
    api_key: Option<String>,
}

impl ApiDataSource {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self { base_url, api_key }
    }
}

#[async_trait]
impl DataSource for ApiDataSource {
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>> {
        let limit = params.limit.unwrap_or(500).min(1500);
        let endpoint = format!("{}/fapi/v1/klines", self.base_url.trim_end_matches('/'));

        let client = reqwest::Client::new();
        let mut request = client
            .get(endpoint)
            .query(&[
                ("symbol", params.symbol.as_str()),
                ("interval", params.interval.as_str()),
                ("limit", &limit.to_string()),
            ]);

        if let Some(api_key) = &self.api_key {
            request = request.header("X-MBX-APIKEY", api_key);
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(crate::MemosTradingError::Api(format!(
                "API veri çekme hatası: {}",
                body
            )));
        }

        let raw = response.json::<Vec<Vec<serde_json::Value>>>().await?;
        let mut candles = Vec::with_capacity(raw.len());

        for item in raw {
            if item.len() < 6 {
                continue;
            }

            let Some(ts) = item[0].as_i64() else {
                continue;
            };
            let Some(timestamp) = Utc.timestamp_millis_opt(ts).single() else {
                continue;
            };

            let open = item[1].as_str().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            let high = item[2].as_str().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            let low = item[3].as_str().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            let close = item[4].as_str().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            let volume = item[5].as_str().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);

            candles.push(Candle {
                timestamp,
                open,
                high,
                low,
                close,
                volume,
                symbol: params.symbol.clone(),
                interval: params.interval.clone(),
            });
        }

        Ok(candles)
    }
    
    fn source_type(&self) -> &str {
        "API"
    }
    
    async fn health_check(&self) -> Result<()> {
        let endpoint = format!("{}/fapi/v1/ping", self.base_url.trim_end_matches('/'));
        let response = reqwest::Client::new().get(endpoint).send().await?;
        if !response.status().is_success() {
            return Err(crate::MemosTradingError::Api("API health_check başarısız".to_string()));
        }
        Ok(())
    }
}

/// Hibrit veri kaynağı (fallback ile)
pub struct HybridDataSource {
    sources: Vec<Box<dyn DataSource>>,
}

impl HybridDataSource {
    pub fn new(sources: Vec<Box<dyn DataSource>>) -> Self {
        Self { sources }
    }
}

#[async_trait]
impl DataSource for HybridDataSource {
    async fn fetch(&self, params: &FetchParams) -> Result<Vec<Candle>> {
        // Kaynakları sırayla dene
        for source in &self.sources {
            match source.fetch(params).await {
                Ok(candles) => return Ok(candles),
                Err(_) => continue,
            }
        }
        Err(crate::MemosTradingError::Config("Tüm kaynaklar başarısız".to_string()).into())
    }
    
    fn source_type(&self) -> &str {
        "Hybrid"
    }
    
    async fn health_check(&self) -> Result<()> {
        // En az bir kaynağın çalışıyor olması yeterli
        for source in &self.sources {
            if source.health_check().await.is_ok() {
                return Ok(());
            }
        }
        Err(crate::MemosTradingError::Config("Hiçbir kaynakta bağlantı kurulamadı".to_string()).into())
    }
}
