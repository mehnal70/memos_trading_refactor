// robot/data_pipeline/cache.rs - Yüksek Performanslı Mum Önbellek Sistemi
use crate::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock}; // Mutex yerine RwLock (Çoklu okuma desteği için)
use chrono::{DateTime, Utc};
use crate::core::types::Candle;

/// Mum Önbelleği: robotic_loop içindeki REST/DB yükünü sıfıra indirir.
pub struct CandleCache {
    /// Key: "symbol_interval" (Örn: "BTCUSDT_1h")
    data: Arc<RwLock<HashMap<String, VecDeque<Candle>>>>,
    max_size: usize,
}

impl CandleCache {
    /// Yeni bir önbellek oluşturur. limit: Per-interval tutulacak maks mum sayısı.
    pub fn new(max_size: usize) -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            max_size,
        }
    }

    /// Yeni mumları önbelleğe ekler. 
    /// Eğer aynı timestamp varsa o mumu günceller (Canlı/Kapanmamış mum desteği).
    pub fn push_bulk(&self, candles: Vec<Candle>) {
        if candles.is_empty() { return; }
        let mut storage = self.data.write().unwrap();
        
        for c in candles {
            let key = format!("{}_{}", c.symbol, c.interval);
            let entry = storage.entry(key).or_insert_with(|| VecDeque::with_capacity(self.max_size));
            
            // Son mumu kontrol et: Eğer TS aynıysa güncelle, değilse yeni ekle
            if let Some(last) = entry.back_mut() {
                if last.timestamp == c.timestamp {
                    *last = c;
                    continue;
                }
            }
            
            entry.push_back(c);
            if entry.len() > self.max_size {
                entry.pop_front();
            }
        }
    }

    /// Belirli bir sembol ve aralık için son 'limit' mumu döner.
    /// robotic_loop içindeki indikatör hesaplamaları için veriyi hazır eder.
    pub fn get_latest(&self, symbol: &str, interval: &str, limit: usize) -> Vec<Candle> {
        let storage = self.data.read().unwrap();
        let key = format!("{}_{}", symbol, interval);
        
        storage.get(&key)
            .map(|deq| {
                let skip = deq.len().saturating_sub(limit);
                deq.iter().skip(skip).cloned().collect()
            })
            .unwrap_or_default()
    }

    /// Veri tazeliği kontrolü: Son mum kaç saniye önce geldi?
    pub fn last_update_age(&self, symbol: &str, interval: &str) -> u64 {
        let storage = self.data.read().unwrap();
        let key = format!("{}_{}", symbol, interval);
        
        storage.get(&key)
            .and_then(|deq| deq.back())
            .map(|c| (Utc::now() - c.timestamp).num_seconds().max(0) as u64)
            .unwrap_or(999_999)
    }

    pub fn clear(&self) {
        self.data.write().unwrap().clear();
    }

    /// Belirtilen anahtar için kaç mum olduğunu döner.
    pub fn len(&self, symbol: &str, interval: &str) -> usize {
        let key = format!("{}_{}", symbol, interval);
        self.data.read().unwrap().get(&key).map(|d| d.len()).unwrap_or(0)
    }
}

impl Default for CandleCache {
    fn default() -> Self {
        Self::new(500) // Endüstri standardı 500 bar
    }
}
