use async_trait::async_trait;
use crate::types::{Candle, Trade};
use crate::Result;
use serde::{Serialize, Deserialize};

// ML/AI destekli veri kalitesi ve anomaly detection modülü
pub struct DatabaseEngine;

impl DatabaseEngine {
    /// ML tabanlı anomaly detection (örnek: kapanış fiyatı outlier ise)
    pub fn detect_anomaly(candles: &[Candle]) -> Vec<usize> {
        if candles.is_empty() { return vec![]; }
        let mean = candles.iter().map(|c| c.close).sum::<f64>() / candles.len() as f64;
        let std = (candles.iter().map(|c| (c.close - mean).powi(2)).sum::<f64>() / candles.len() as f64).sqrt();
        candles.iter().enumerate().filter_map(|(i, c)| {
            if (c.close - mean).abs() > 3.0 * std { Some(i) } else { None }
        }).collect()
    }

    /// ML tabanlı otomatik veri tamamlama (örnek: eksik kapanışları ortalama ile doldur)
    pub fn auto_complete(candles: &mut [Candle]) {
        let mean = if candles.is_empty() { 0.0 } else { candles.iter().map(|c| c.close).sum::<f64>() / candles.len() as f64 };
        for c in candles.iter_mut() {
            if c.close == 0.0 { c.close = mean; }
        }
    }
}

/// Database hata tipi
#[derive(Debug, Serialize, Deserialize, thiserror::Error)]
pub enum DatabaseError {
    #[error("Connection error: {0}")]
    ConnectionError(String),
    
    #[error("Query error: {0}")]
    QueryError(String),
    
    #[error("Not found")]
    NotFound,
    
    #[error("Data error: {0}")]
    DataError(String),
}

/// Database operasyonlarının trait'i
/// Farklı database implementasyonları (SQLite, PostgreSQL, MongoDB vb.)
/// bu trait'i implement edebilir
#[async_trait]
pub trait Database: Send + Sync {
    /// Candle verisini kaydet
    async fn save_candle(&self, candle: &Candle) -> Result<()>;
    
    /// Candle verilerini toplu kaydet
    async fn save_candles(&self, candles: &[Candle]) -> Result<()>;
    
    /// Belirli sembol ve interval için candle'ları getir
    async fn get_candles(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>>;
    
    /// Trade kaydını kaydet
    async fn save_trade(&self, trade: &Trade) -> Result<()>;
    
    /// Trade kaydını güncelle
    async fn update_trade(&self, trade: &Trade) -> Result<()>;
    
    /// Trade kayıtlarını getir
    async fn get_trades(&self, symbol: Option<&str>, limit: usize) -> Result<Vec<Trade>>;
    
    /// Kullanıcı ayarlarını kaydet
    async fn save_setting(&self, key: &str, value: &str) -> Result<()>;
    
    /// Kullanıcı ayarlarını getir
    async fn get_setting(&self, key: &str) -> Result<Option<String>>;
    
    /// Database'i başlat (migration vb.)
    async fn init(&self) -> Result<()>;
    
    /// Health check
    async fn health_check(&self) -> Result<()>;
}

/// Basit in-memory database implementasyonu (testing için)
pub struct MemoryDatabase {
    candles: std::sync::Mutex<Vec<Candle>>,
    trades: std::sync::Mutex<Vec<Trade>>,
    settings: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl MemoryDatabase {
    pub fn new() -> Self {
        Self {
            candles: std::sync::Mutex::new(Vec::new()),
            trades: std::sync::Mutex::new(Vec::new()),
            settings: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl Database for MemoryDatabase {
    async fn save_candle(&self, candle: &Candle) -> Result<()> {
        self.candles.lock().unwrap().push(candle.clone());
        Ok(())
    }
    
    async fn save_candles(&self, candles: &[Candle]) -> Result<()> {
        self.candles.lock().unwrap().extend_from_slice(candles);
        Ok(())
    }
    
    async fn get_candles(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        let candles = self.candles.lock().unwrap();
        Ok(candles
            .iter()
            .filter(|c| c.symbol == symbol && c.interval == interval)
            .rev()
            .take(limit)
            .cloned()
            .collect())
    }
    
    async fn save_trade(&self, trade: &Trade) -> Result<()> {
        self.trades.lock().unwrap().push(trade.clone());
        Ok(())
    }
    
    async fn update_trade(&self, trade: &Trade) -> Result<()> {
        let mut trades = self.trades.lock().unwrap();
        if let Some(pos) = trades.iter().position(|t| t.id == trade.id) {
            trades[pos] = trade.clone();
        }
        Ok(())
    }
    
    async fn get_trades(&self, symbol: Option<&str>, limit: usize) -> Result<Vec<Trade>> {
        let trades = self.trades.lock().unwrap();
        Ok(trades
            .iter()
            .filter(|t| symbol.is_none() || symbol.map_or(false, |s| t.symbol == s))
            .rev()
            .take(limit)
            .cloned()
            .collect())
    }
    
    async fn save_setting(&self, key: &str, value: &str) -> Result<()> {
        self.settings.lock().unwrap().insert(key.to_string(), value.to_string());
        Ok(())
    }
    
    async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.settings.lock().unwrap().get(key).cloned())
    }
    
    async fn init(&self) -> Result<()> {
        Ok(())
    }
    
    async fn health_check(&self) -> Result<()> {
        Ok(())
    }
}

impl Default for MemoryDatabase {
    fn default() -> Self {
        Self::new()
    }
}
