#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Exchange, Market};
    use std::fs;

    #[test]
    fn test_config_save_and_load() {
        let path = "/tmp/test_app_config.json";
        let config = AppConfig {
            exchange: Exchange::Binance,
            market: Market::Futures,
            interval: "1h".to_string(),
            strategy: "MA_CROSSOVER".to_string(),
            risk: Some("default".to_string()),
            extra: None,
        };
        let mgr = FileConfigManager::new(path);
        mgr.save_config(&config).unwrap();
        let loaded = mgr.load_config().unwrap();
        assert_eq!(loaded.exchange, Exchange::Binance);
        assert_eq!(loaded.market, Market::Futures);
        assert_eq!(loaded.interval, "1h");
        fs::remove_file(path).unwrap();
    }
}
// robot/config_manager.rs - Merkezi Konfigürasyon ve State Persistence interface

use crate::Result;
use crate::types::{Exchange, Market};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub exchange: Exchange,
    pub market: Market,
    pub interval: String,
    pub strategy: String,
    pub risk: Option<String>,
    pub extra: Option<serde_json::Value>,
}

pub trait ConfigManager: Send + Sync {
    fn load_config(&self) -> Result<AppConfig>;
    fn save_config(&self, config: &AppConfig) -> Result<()>;
}

/// Basit dosya tabanlı config manager (örnek, test amaçlı)
pub struct FileConfigManager {
    pub path: String,
}

impl FileConfigManager {
    pub fn new(path: &str) -> Self {
        Self { path: path.to_string() }
    }
}

impl ConfigManager for FileConfigManager {
    fn load_config(&self) -> Result<AppConfig> {
        let data = std::fs::read_to_string(&self.path)?;
        let config: AppConfig = serde_json::from_str(&data)?;
        Ok(config)
    }
    fn save_config(&self, config: &AppConfig) -> Result<()> {
        let data = serde_json::to_string_pretty(config)?;
        std::fs::write(&self.path, data)?;
        Ok(())
    }
}
