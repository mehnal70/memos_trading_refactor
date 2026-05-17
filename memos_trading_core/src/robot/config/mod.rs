// robot/config/mod.rs - Merkezi Konfigürasyon ve Anayasal Denetim

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::Result;
use crate::core::types::Market; // Merkezi Market tipini kullanıyoruz

/// §84.1: RoboticLoopConfig - robotic_loop'un beklediği otonom parametreler.
/// Not: RobotConfig içindeki veriler buraya eşlenir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoboticLoopConfig {
    pub symbol: String,
    pub market: Market,
    pub interval: String,
    pub capital: f64,
    pub candle_limit: usize,
    pub autonomous_enabled: bool,
    pub max_spread_bps: Option<f64>,
    pub trade_amount: Option<f64>,
    pub risk_params: crate::core::types::RiskParams,
    pub scalp_swing: Option<crate::robot::scalp_swing::ScalpSwingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobotConfig {
    pub name: String,
    pub description: Option<String>,
    pub trading: TradingConfig,
    pub basket: BasketConfig,
    pub market_hours: Vec<MarketHourConfig>,
    pub strategies: Vec<StrategyConfig>,
    pub risk: RiskConfig,
    pub data: DataConfig,
    pub reporting: ReportingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    pub mode: TradingMode,
    pub exchange: String,
    pub market: Market, // String yerine merkezi Market enum'u
    pub capital: f64,
    pub max_concurrent_positions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradingMode { Live, Paper, Backtest }

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

pub struct ConfigManager { config: RobotConfig }

impl ConfigManager {
    pub fn new(config: RobotConfig) -> Self { Self { config } }
    
    pub fn validate(&self) -> Result<()> {
        if self.config.name.is_empty() {
            return Err(crate::MemosTradingError::Config("Robot adı boş olamaz".into()).into());
        }
        if self.config.trading.capital <= 0.0 {
            return Err(crate::MemosTradingError::Config("Sermaye pozitif olmalı".into()).into());
        }
        Ok(())
    }

    /// RobotConfig verilerini RoboticLoop'un anlayacağı dile otonom çevirir.
    pub fn to_loop_config(&self, symbol: &str, interval: &str) -> RoboticLoopConfig {
        RoboticLoopConfig {
            symbol: symbol.to_string(),
            market: self.config.trading.market,
            interval: interval.to_string(),
            capital: self.config.trading.capital,
            candle_limit: 500,
            autonomous_enabled: true,
            max_spread_bps: Some(10.0),
            trade_amount: None,
            risk_params: crate::core::types::RiskParams {
                stop_loss_pct: self.config.risk.stop_loss_pct,
                take_profit_pct: self.config.risk.take_profit_pct,
                max_position_size_pct: Some(self.config.risk.max_position_pct),
                ..Default::default()
            },
            scalp_swing: None,
        }
    }
}
