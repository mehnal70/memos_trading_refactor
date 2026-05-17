// src/robot/state/types.rs - Sistem Durum Sözleşmeleri
// Srivastava ATP - Saf Veri Yapıları Katmanı

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use crate::core::types::Market;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemStatistics {
    pub total_trades: u64,
    pub win_trades: u64,
    pub loss_trades: u64,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub average_profit: f64,
    pub average_loss: f64,
    pub profit_factor: f64,
    pub max_drawdown_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SystemStatus { Initializing, Running, Paused, Halted }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PositionStatus {
    Open,               
    Closed,             
    Pending,            
    LiquidationWarning, 
    ManualExitRequired, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub symbol: String,
    pub market: Market, // Kütüphane ana types enum'una bağlandı
    pub entry_price: f64,
    pub quantity: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub opened_at: DateTime<Utc>,
    pub status: PositionStatus,
}

/// Robotik sistem durumu (Kalıcı ve Canlı veriler bir arada)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobotState {
    pub robot_id: String,
    pub last_run: Option<DateTime<Utc>>,
    pub last_exec_times: HashMap<String, DateTime<Utc>>,
    pub current_equity: f64,
    pub cumulative_pnl: f64,
    pub peak_equity: f64,
    pub open_positions: Vec<Position>,
    pub closed_positions: Vec<Position>,
    pub statistics: SystemStatistics,
    pub status: SystemStatus,
}
