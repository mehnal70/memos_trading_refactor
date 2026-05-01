// robot/config/mod.rs - Merkezi konfigürasyon yönetimi

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::Result;

/// Robotik ticaret sistemi konfigürasyonu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobotConfig {
    /// Robot adı ve tanımlaması
    pub name: String,
    pub description: Option<String>,
    
    /// Ticaret ayarları
    pub trading: TradingConfig,
    
    /// Sepet konfigürasyonu
    pub basket: BasketConfig,
    
    /// Market saatleri
    pub market_hours: Vec<MarketHourConfig>,
    
    /// Strateji ayarları
    pub strategies: Vec<StrategyConfig>,
    
    /// Risk yönetimi
    pub risk: RiskConfig,
    
    /// Veri ayarları
    pub data: DataConfig,
    
    /// Reporting ayarları
    pub reporting: ReportingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    pub mode: TradingMode,
    pub exchange: String,
    pub market: String,
    pub capital: f64,
    pub max_concurrent_positions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradingMode {
    Live,
    Paper,
    Backtest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasketConfig {
    pub symbols: Vec<String>,
    pub intervals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketHourConfig {
    pub day: String,
    pub open: String,
    pub close: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    pub name: String,
    pub enabled: bool,
    pub params: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_loss_pct: f64,
    pub max_position_pct: f64,
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataConfig {
    pub cache_ttl_seconds: u64,
    pub max_cached_symbols: usize,
    pub batch_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportingConfig {
    pub output_formats: Vec<String>,
    pub report_interval_minutes: u32,
    pub save_directory: String,
}

/// Konfigürasyon yöneticisi
pub struct ConfigManager {
    config: RobotConfig,
}

impl ConfigManager {
    pub fn new(config: RobotConfig) -> Self {
        Self { config }
    }
    
    pub fn validate(&self) -> Result<()> {
        // Temel validasyonlar
        if self.config.name.is_empty() {
            return Err(crate::MemosTradingError::Config("Robot adı boş olamaz".to_string()).into());
        }
        if self.config.trading.capital <= 0.0 {
            return Err(crate::MemosTradingError::Config("Sermaye pozitif olmalı".to_string()).into());
        }
        if self.config.basket.symbols.is_empty() {
            return Err(crate::MemosTradingError::Config("En az bir sembol olmalı".to_string()).into());
        }
        Ok(())
    }
    
    pub fn get_strategy(&self, name: &str) -> Option<&StrategyConfig> {
        self.config.strategies.iter().find(|s| s.name == name)
    }
    
    pub fn get_enabled_strategies(&self) -> Vec<&StrategyConfig> {
        self.config.strategies.iter().filter(|s| s.enabled).collect()
    }
    
    pub fn config(&self) -> &RobotConfig {
        &self.config
    }
    
    pub fn config_mut(&mut self) -> &mut RobotConfig {
        &mut self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn test_config() -> RobotConfig {
        RobotConfig {
            name: "TestRobot".to_string(),
            description: None,
            trading: TradingConfig {
                mode: TradingMode::Paper,
                exchange: "BIST".to_string(),
                market: "BIST100".to_string(),
                capital: 10000.0,
                max_concurrent_positions: 5,
            },
            basket: BasketConfig {
                symbols: vec!["AKBNK".to_string(), "GARAN".to_string()],
                intervals: vec!["1h".to_string()],
            },
            market_hours: vec![],
            strategies: vec![],
            risk: RiskConfig {
                max_loss_pct: 2.0,
                max_position_pct: 10.0,
                stop_loss_pct: 2.0,
                take_profit_pct: 5.0,
            },
            data: DataConfig {
                cache_ttl_seconds: 3600,
                max_cached_symbols: 100,
                batch_size: 1000,
            },
            reporting: ReportingConfig {
                output_formats: vec!["json".to_string()],
                report_interval_minutes: 60,
                save_directory: "/tmp".to_string(),
            },
        }
    }
    
    #[test]
    fn test_config_validation() {
        let config = test_config();
        let manager = ConfigManager::new(config);
        assert!(manager.validate().is_ok());
    }
}
