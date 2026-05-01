// robot/data_pipeline/cache.rs - Veri caching sistemi

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc, Duration};
use crate::types::Candle;

/// Cache entry
struct CacheEntry {
    data: Vec<Candle>,
    timestamp: DateTime<Utc>,
    ttl_seconds: u64,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        Utc::now() > self.timestamp + Duration::seconds(self.ttl_seconds as i64)
    }
}

/// Veri cache sistemi
pub struct DataCache {
    storage: Arc<Mutex<HashMap<String, CacheEntry>>>,
    ttl_seconds: u64,
    max_entries: usize,
}

impl DataCache {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(Mutex::new(HashMap::new())),
            ttl_seconds: 3600, // 1 saat
            max_entries: 100,
        }
    }
    
    pub fn with_ttl(ttl_seconds: u64) -> Self {
        Self {
            storage: Arc::new(Mutex::new(HashMap::new())),
            ttl_seconds,
            max_entries: 100,
        }
    }
    
    pub fn with_max_entries(max_entries: usize) -> Self {
        Self {
            storage: Arc::new(Mutex::new(HashMap::new())),
            ttl_seconds: 3600,
            max_entries,
        }
    }
    
    /// Cache'e veri kaydet
    pub fn set(&self, key: &str, data: Vec<Candle>) {
        if let Ok(mut storage) = self.storage.lock() {
            // Eğer cache dolu ise, en eski entry'i sil
            if storage.len() >= self.max_entries {
                // En eski entry'i bul
                if let Some(oldest_key) = storage
                    .iter()
                    .min_by_key(|(_, entry)| entry.timestamp)
                    .map(|(k, _)| k.clone())
                {
                    storage.remove(&oldest_key);
                }
            }
            
            storage.insert(
                key.to_string(),
                CacheEntry {
                    data,
                    timestamp: Utc::now(),
                    ttl_seconds: self.ttl_seconds,
                },
            );
        }
    }
    
    /// Cache'den veri al
    pub fn get(&self, key: &str) -> Option<Vec<Candle>> {
        if let Ok(mut storage) = self.storage.lock() {
            if let Some(entry) = storage.get(key) {
                if entry.is_expired() {
                    // Süresi dolmuş, sil
                    storage.remove(key);
                    None
                } else {
                    Some(entry.data.clone())
                }
            } else {
                None
            }
        } else {
            None
        }
    }
    
    /// Cache'i temizle
    pub fn clear(&self) {
        if let Ok(mut storage) = self.storage.lock() {
            storage.clear();
        }
    }
    
    /// Süresi dolmuş entry'leri temizle
    pub fn cleanup_expired(&self) {
        if let Ok(mut storage) = self.storage.lock() {
            storage.retain(|_, entry| !entry.is_expired());
        }
    }
    
    /// Cache istatistikleri
    pub fn stats(&self) -> CacheStats {
        if let Ok(storage) = self.storage.lock() {
            let total_entries = storage.len();
            let expired_entries = storage.iter().filter(|(_, entry)| entry.is_expired()).count();
            let active_entries = total_entries - expired_entries;
            
            CacheStats {
                total_entries,
                active_entries,
                expired_entries,
                max_capacity: self.max_entries,
            }
        } else {
            CacheStats {
                total_entries: 0,
                active_entries: 0,
                expired_entries: 0,
                max_capacity: self.max_entries,
            }
        }
    }
}

impl Default for DataCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache istatistikleri
#[derive(Debug)]
pub struct CacheStats {
    pub total_entries: usize,
    pub active_entries: usize,
    pub expired_entries: usize,
    pub max_capacity: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cache_set_and_get() {
        let cache = DataCache::new();
        let candle = vec![Candle {
            symbol: "TEST".to_string(),
            timestamp: Utc::now(),
            open: 100.0,
            high: 102.0,
            low: 99.0,
            close: 101.0,
            volume: 1000.0,
            interval: "1h".to_string(),
        }];
        
        cache.set("test_key", candle.clone());
        let retrieved = cache.get("test_key");
        assert!(retrieved.is_some());
    }
    
    #[test]
    fn test_cache_expiry() {
        let cache = DataCache::with_ttl(1); // 1 saniye TTL
        let candle = vec![Candle {
            symbol: "TEST".to_string(),
            timestamp: Utc::now(),
            open: 100.0,
            high: 102.0,
            low: 99.0,
            close: 101.0,
            volume: 1000.0,
            interval: "1h".to_string(),
        }];
        
        cache.set("test_key", candle);
        
        // Hemen al - başarılı olmalı
        assert!(cache.get("test_key").is_some());
        
        // 2 saniye bekle
        std::thread::sleep(std::time::Duration::from_secs(2));
        
        // Şimdi süresi dolmuş olmalı
        assert!(cache.get("test_key").is_none());
    }
    
    #[test]
    fn test_cache_cleanup() {
        let cache = DataCache::new();
        let candle = vec![Candle {
            symbol: "TEST".to_string(),
            timestamp: Utc::now(),
            open: 100.0,
            high: 102.0,
            low: 99.0,
            close: 101.0,
            volume: 1000.0,
            interval: "1h".to_string(),
        }];
        
        cache.set("key1", candle.clone());
        cache.set("key2", candle);
        
        cache.cleanup_expired();
        let stats = cache.stats();
        assert_eq!(stats.total_entries, 2);
    }
}
