// Strategy Loader - Runtime'da Stratejileri Yükle/Kaldır
//
// Hot-reload desteği: Sistem çalışırken yeni stratejileri yükle
// Dynamic strategy management

use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// Strategy Yükleme Hatası
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StrategyLoadError {
    StrategyNotFound(String),
    InvalidConfiguration(String),
    VersionMismatch { required: String, found: String },
    DecodingError(String),
    AlreadyLoaded(String),
    InUse(String),
}

impl std::fmt::Display for StrategyLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrategyLoadError::StrategyNotFound(name) => write!(f, "Strategy not found: {}", name),
            StrategyLoadError::InvalidConfiguration(msg) => write!(f, "Invalid config: {}", msg),
            StrategyLoadError::VersionMismatch { required, found } => {
                write!(f, "Version mismatch: required {}, found {}", required, found)
            }
            StrategyLoadError::DecodingError(msg) => write!(f, "Decoding error: {}", msg),
            StrategyLoadError::AlreadyLoaded(name) => write!(f, "Already loaded: {}", name),
            StrategyLoadError::InUse(name) => write!(f, "Strategy in use: {}", name),
        }
    }
}

/// Yüklenmiş Strateji Metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedStrategy {
    /// Strateji adı
    pub name: String,
    /// Sürüm
    pub version: String,
    /// Yükleme zamanı
    pub loaded_at: DateTime<Utc>,
    /// Kullanıldığı pozisyon sayısı
    pub active_positions: u32,
    /// Son kullanıldığı zaman
    pub last_used: Option<DateTime<Utc>>,
    /// Metadata
    pub metadata: HashMap<String, String>,
}

/// Strategy Loader
pub struct StrategyLoader {
    /// Yüklenmiş stratejiler
    loaded_strategies: HashMap<String, LoadedStrategy>,
    /// Max yüklenecek strateji sayısı
    max_strategies: usize,
    /// İstatistikler
    total_loaded: u32,
    total_unloaded: u32,
}

impl StrategyLoader {
    /// Yeni Strategy Loader oluştur
    pub fn new(max_strategies: usize) -> Self {
        Self {
            loaded_strategies: HashMap::new(),
            max_strategies,
            total_loaded: 0,
            total_unloaded: 0,
        }
    }

    /// Varsayılan loader (max 50 strateji)
    pub fn default() -> Self {
        Self::new(50)
    }

    /// Stratejiyi yükle
    pub fn load_strategy(
        &mut self,
        name: String,
        version: String,
        metadata: HashMap<String, String>,
    ) -> Result<LoadedStrategy, StrategyLoadError> {
        // Kontrol: Zaten yüklü mü?
        if self.loaded_strategies.contains_key(&name) {
            return Err(StrategyLoadError::AlreadyLoaded(name));
        }

        // Kontrol: Max stratejiye ulaştı mı?
        if self.loaded_strategies.len() >= self.max_strategies {
            return Err(StrategyLoadError::InvalidConfiguration(
                format!("Max strategies ({}) reached", self.max_strategies),
            ));
        }

        // Yeni strateji oluştur
        let strategy = LoadedStrategy {
            name: name.clone(),
            version,
            loaded_at: Utc::now(),
            active_positions: 0,
            last_used: None,
            metadata,
        };

        self.loaded_strategies.insert(name, strategy.clone());
        self.total_loaded += 1;

        Ok(strategy)
    }

    /// Stratejiyi kaldır
    pub fn unload_strategy(&mut self, name: &str) -> Result<(), StrategyLoadError> {
        let strategy = self
            .loaded_strategies
            .get(name)
            .ok_or_else(|| StrategyLoadError::StrategyNotFound(name.to_string()))?;

        // Kontrol: Kullanımda mı?
        if strategy.active_positions > 0 {
            return Err(StrategyLoadError::InUse(name.to_string()));
        }

        self.loaded_strategies.remove(name);
        self.total_unloaded += 1;

        Ok(())
    }

    /// Strateji var mı?
    pub fn has_strategy(&self, name: &str) -> bool {
        self.loaded_strategies.contains_key(name)
    }

    /// Strateji metadata'sını al
    pub fn get_strategy(&self, name: &str) -> Option<&LoadedStrategy> {
        self.loaded_strategies.get(name)
    }

    /// Mutable strateji al (internal use)
    pub fn get_strategy_mut(&mut self, name: &str) -> Option<&mut LoadedStrategy> {
        self.loaded_strategies.get_mut(name)
    }

    /// Tüm stratejileri listele
    pub fn list_strategies(&self) -> Vec<String> {
        self.loaded_strategies.keys().cloned().collect()
    }

    /// Strateji sayısı
    pub fn count(&self) -> usize {
        self.loaded_strategies.len()
    }

    /// Aktif pozisyon sayısını güncelle
    pub fn update_active_positions(
        &mut self,
        name: &str,
        count: u32,
    ) -> Result<(), StrategyLoadError> {
        let strategy = self
            .get_strategy_mut(name)
            .ok_or_else(|| StrategyLoadError::StrategyNotFound(name.to_string()))?;

        strategy.active_positions = count;
        strategy.last_used = Some(Utc::now());

        Ok(())
    }

    /// Tüm stratejileri temizle
    pub fn clear(&mut self) -> Result<u32, StrategyLoadError> {
        // Aktif pozisyon olan stratejiler var mı?
        let active: Vec<_> = self
            .loaded_strategies
            .values()
            .filter(|s| s.active_positions > 0)
            .map(|s| s.name.clone())
            .collect();

        if !active.is_empty() {
            return Err(StrategyLoadError::InUse(format!(
                "{} strategies have active positions",
                active.len()
            )));
        }

        let count = self.loaded_strategies.len() as u32;
        self.loaded_strategies.clear();
        self.total_unloaded += count;

        Ok(count)
    }

    /// İstatistikleri al
    pub fn stats(&self) -> (u32, u32, usize) {
        (self.total_loaded, self.total_unloaded, self.loaded_strategies.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loader_creation() {
        let loader = StrategyLoader::new(10);
        assert_eq!(loader.count(), 0);
    }

    #[test]
    fn test_load_strategy() {
        let mut loader = StrategyLoader::new(10);
        let mut meta = HashMap::new();
        meta.insert("type".to_string(), "moving_average".to_string());

        let result = loader.load_strategy("ma_cross".to_string(), "1.0.0".to_string(), meta);
        assert!(result.is_ok());
        assert!(loader.has_strategy("ma_cross"));
        assert_eq!(loader.count(), 1);
    }

    #[test]
    fn test_cannot_load_duplicate() {
        let mut loader = StrategyLoader::new(10);
        let meta = HashMap::new();

        let _ = loader.load_strategy("ma_cross".to_string(), "1.0.0".to_string(), meta.clone());
        let result = loader.load_strategy("ma_cross".to_string(), "1.0.0".to_string(), meta);

        match result {
            Err(StrategyLoadError::AlreadyLoaded(_)) => assert!(true),
            _ => panic!("Expected AlreadyLoaded error"),
        }
    }

    #[test]
    fn test_unload_strategy() {
        let mut loader = StrategyLoader::new(10);
        let meta = HashMap::new();

        loader
            .load_strategy("ma_cross".to_string(), "1.0.0".to_string(), meta)
            .unwrap();

        let result = loader.unload_strategy("ma_cross");
        assert!(result.is_ok());
        assert!(!loader.has_strategy("ma_cross"));
    }

    #[test]
    fn test_cannot_unload_in_use() {
        let mut loader = StrategyLoader::new(10);
        let meta = HashMap::new();

        loader
            .load_strategy("ma_cross".to_string(), "1.0.0".to_string(), meta)
            .unwrap();

        // Aktif pozisyon ekle
        loader
            .update_active_positions("ma_cross", 5)
            .unwrap();

        let result = loader.unload_strategy("ma_cross");

        match result {
            Err(StrategyLoadError::InUse(_)) => assert!(true),
            _ => panic!("Expected InUse error"),
        }
    }

    #[test]
    fn test_max_strategies_limit() {
        let mut loader = StrategyLoader::new(2);
        let meta = HashMap::new();

        loader
            .load_strategy("ma_cross".to_string(), "1.0.0".to_string(), meta.clone())
            .unwrap();
        loader
            .load_strategy("rsi".to_string(), "1.0.0".to_string(), meta.clone())
            .unwrap();

        let result = loader.load_strategy("macd".to_string(), "1.0.0".to_string(), meta);

        match result {
            Err(StrategyLoadError::InvalidConfiguration(_)) => assert!(true),
            _ => panic!("Expected InvalidConfiguration error"),
        }
    }

    #[test]
    fn test_list_strategies() {
        let mut loader = StrategyLoader::new(10);
        let meta = HashMap::new();

        loader
            .load_strategy("ma_cross".to_string(), "1.0.0".to_string(), meta.clone())
            .unwrap();
        loader
            .load_strategy("rsi".to_string(), "1.0.0".to_string(), meta)
            .unwrap();

        let list = loader.list_strategies();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&"ma_cross".to_string()));
        assert!(list.contains(&"rsi".to_string()));
    }

    #[test]
    fn test_stats() {
        let mut loader = StrategyLoader::new(10);
        let meta = HashMap::new();

        loader
            .load_strategy("ma_cross".to_string(), "1.0.0".to_string(), meta)
            .unwrap();

        let (loaded, unloaded, current) = loader.stats();
        assert_eq!(loaded, 1);
        assert_eq!(unloaded, 0);
        assert_eq!(current, 1);
    }

    #[test]
    fn test_clear_strategies() {
        let mut loader = StrategyLoader::new(10);
        let meta = HashMap::new();

        loader
            .load_strategy("ma_cross".to_string(), "1.0.0".to_string(), meta.clone())
            .unwrap();
        loader
            .load_strategy("rsi".to_string(), "1.0.0".to_string(), meta)
            .unwrap();

        let result = loader.clear();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2);
        assert_eq!(loader.count(), 0);
    }
}
