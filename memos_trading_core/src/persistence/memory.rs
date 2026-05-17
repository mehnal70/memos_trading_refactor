// persistence/memory.rs
// Modernize edilmiş bellek içi veri katmanı ve AI veri kalitesi motoru

use async_trait::async_trait;
use crate::core::types::{Candle, Trade};
use crate::Result;
use serde::{Serialize, Deserialize};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;

// --- 1. ML/AI VERİ KALİTESİ MOTORU (DatabaseEngine) ---

pub struct DatabaseEngine;

impl DatabaseEngine {
    /// ML tabanlı anomali tespiti (Z-Score temelli Outlier tespiti)
    /// Performans: Allocation yapmadan iterator üzerinden hesaplama yapar.
    pub fn detect_anomaly(candles: &[Candle]) -> Vec<usize> {
        let n = candles.len();
        if n < 5 { return vec![]; } // Güvenilir istatistik için min örneklem

        let sum: f64 = candles.iter().map(|c| c.close).sum();
        let mean = sum / n as f64;
        
        let variance: f64 = candles.iter()
            .map(|c| (c.close - mean).powi(2))
            .sum::<f64>() / n as f64;
        let std = variance.sqrt();

        // 3-Sigma kuralı (Verinin %99.7'sini kapsar, dışı anomalidir)
        candles.iter().enumerate().filter_map(|(i, c)| {
            if (c.close - mean).abs() > 3.0 * std { Some(i) } else { None }
        }).collect()
    }

    /// ML tabanlı veri tamamlama (İnterpolasyon hazırlığı)
    pub fn auto_complete(candles: &mut [Candle]) {
        if candles.is_empty() { return; }
        
        let sum: f64 = candles.iter().map(|c| c.close).sum();
        let mean = sum / candles.len() as f64;
        
        for c in candles.iter_mut() {
            if c.close <= 0.0 { c.close = mean; }
        }
    }
}

// --- 2. HATA VE TRAIT TANIMLARI ---

#[derive(Debug, Serialize, Deserialize, thiserror::Error)]
pub enum DatabaseError {
    #[error("Veritabanı bağlantı hatası: {0}")]
    Connection(String),
    #[error("Sorgu hatası: {0}")]
    Query(String),
    #[error("Kayıt bulunamadı")]
    NotFound,
    #[error("Veri bütünlüğü hatası: {0}")]
    DataCorruption(String),
}

#[async_trait]
pub trait Database: Send + Sync {
    async fn save_candle(&self, candle: &Candle) -> Result<()>;
    async fn save_candles(&self, candles: &[Candle]) -> Result<()>;
    async fn get_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>>;
    async fn save_trade(&self, trade: &Trade) -> Result<()>;
    async fn update_trade(&self, trade: &Trade) -> Result<()>;
    async fn get_trades(&self, symbol: Option<&str>, limit: usize) -> Result<Vec<Trade>>;
    async fn save_setting(&self, key: &str, value: &str) -> Result<()>;
    async fn get_setting(&self, key: &str) -> Result<Option<String>>;
    async fn init(&self) -> Result<()>;
    async fn health_check(&self) -> Result<()>;
}

// --- 3. OPTİMİZE EDİLMİŞ MEMORY DATABASE ---

pub struct MemoryDatabase {
    // Performans: Mutex yerine RwLock kullanarak paralel okuma desteği sağladık.
    candles: Arc<RwLock<Vec<Candle>>>,
    trades: Arc<RwLock<Vec<Trade>>>,
    settings: Arc<RwLock<HashMap<String, String>>>,
}

impl MemoryDatabase {
    pub fn new() -> Self {
        Self {
            // Başlangıç kapasitelerini belirleyerek re-allocation maliyetini düşürdük.
            candles: Arc::new(RwLock::new(Vec::with_capacity(10000))),
            trades: Arc::new(RwLock::new(Vec::with_capacity(1000))),
            settings: Arc::new(RwLock::new(HashMap::with_capacity(50))),
        }
    }
}

#[async_trait]
impl Database for MemoryDatabase {
    async fn save_candle(&self, candle: &Candle) -> Result<()> {
        let mut lock = self.candles.write().map_err(|e| DatabaseError::Query(e.to_string()))?;
        lock.push(candle.clone());
        Ok(())
    }

    async fn save_candles(&self, candles: &[Candle]) -> Result<()> {
        let mut lock = self.candles.write().map_err(|e| DatabaseError::Query(e.to_string()))?;
        lock.extend_from_slice(candles);
        Ok(())
    }

    async fn get_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        let lock = self.candles.read().map_err(|e| DatabaseError::Query(e.to_string()))?;
        // Filtreleme ve ters çevirme (Zero-copy referanslarla başla, sadece sonuçları kopyala)
        Ok(lock.iter()
            .filter(|c| c.symbol == symbol && c.interval == interval)
            .rev()
            .take(limit)
            .cloned()
            .collect())
    }

    async fn save_trade(&self, trade: &Trade) -> Result<()> {
        let mut lock = self.trades.write().map_err(|e| DatabaseError::Query(e.to_string()))?;
        lock.push(trade.clone());
        Ok(())
    }

    async fn update_trade(&self, trade: &Trade) -> Result<()> {
        let mut lock = self.trades.write().map_err(|e| DatabaseError::Query(e.to_string()))?;
        if let Some(pos) = lock.iter().position(|t| t.id == trade.id) {
            lock[pos] = trade.clone();
        }
        Ok(())
    }

    async fn get_trades(&self, symbol: Option<&str>, limit: usize) -> Result<Vec<Trade>> {
        let lock = self.trades.read().map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(lock.iter()
            .filter(|t| symbol.is_none() || symbol.map_or(false, |s| t.symbol == s))
            .rev()
            .take(limit)
            .cloned()
            .collect())
    }

    async fn save_setting(&self, key: &str, value: &str) -> Result<()> {
        let mut lock = self.settings.write().map_err(|e| DatabaseError::Query(e.to_string()))?;
        lock.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let lock = self.settings.read().map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(lock.get(key).cloned())
    }

    async fn init(&self) -> Result<()> { Ok(()) }
    async fn health_check(&self) -> Result<()> { Ok(()) }
}

impl Default for MemoryDatabase {
    fn default() -> Self { Self::new() }
}
