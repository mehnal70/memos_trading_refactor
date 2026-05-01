// robot/state/mod.rs - Robotik sistem durum yönetimi

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// Robotik sistem durumu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobotState {
    /// Robot kimliği
    pub robot_id: String,
    
    /// Son çalıştırma zamanı
    pub last_run: Option<DateTime<Utc>>,
    
    /// Sembol bazında son işlem zamanları
    pub last_exec_times: HashMap<String, DateTime<Utc>>,
    
    /// Açık pozisyonlar
    pub open_positions: Vec<Position>,
    
    /// Kapalı pozisyonlar (geçmiş)
    pub closed_positions: Vec<Position>,
    
    /// RL (Reinforcement Learning) state'leri
    pub rl_states: HashMap<String, RLState>,
    
    /// Sistem istatistikleri
    pub statistics: SystemStatistics,
    
    /// Sistem durumu
    pub status: SystemStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub symbol: String,
    pub entry_price: f64,
    pub quantity: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub status: PositionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PositionStatus {
    Open,
    Closed,
    ClosedWithProfit,
    ClosedWithLoss,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RLState {
    pub symbol: String,
    pub strategy: String,
    pub q_table: HashMap<String, f64>,
    pub last_action: Option<String>,
    pub total_reward: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStatistics {
    pub total_trades: u32,
    pub winning_trades: u32,
    pub losing_trades: u32,
    pub total_profit: f64,
    pub total_loss: f64,
    pub win_rate: f64,
    pub average_trade_time_hours: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SystemStatus {
    Initializing,
    Running,
    Paused,
    Stopped,
    Error,
}

impl RobotState {
    pub fn new(robot_id: String) -> Self {
        Self {
            robot_id,
            last_run: None,
            last_exec_times: HashMap::new(),
            open_positions: Vec::new(),
            closed_positions: Vec::new(),
            rl_states: HashMap::new(),
            statistics: SystemStatistics {
                total_trades: 0,
                winning_trades: 0,
                losing_trades: 0,
                total_profit: 0.0,
                total_loss: 0.0,
                win_rate: 0.0,
                average_trade_time_hours: 0.0,
            },
            status: SystemStatus::Initializing,
        }
    }
    
    /// Son işlem zamanını güncelle
    pub fn update_last_exec_time(&mut self, symbol: &str, time: DateTime<Utc>) {
        self.last_exec_times.insert(symbol.to_string(), time);
    }
    
    /// Sembol için son işlem zamanını al
    pub fn get_last_exec_time(&self, symbol: &str) -> Option<DateTime<Utc>> {
        self.last_exec_times.get(symbol).copied()
    }
    
    /// Pozisyon ekle
    pub fn add_position(&mut self, position: Position) {
        self.open_positions.push(position);
        self.statistics.total_trades += 1;
    }
    
    /// Pozisyonu kapat
    pub fn close_position(&mut self, position_id: &str, close_price: f64) {
        if let Some(pos) = self.open_positions.iter_mut().find(|p| p.id == position_id) {
            let profit = (close_price - pos.entry_price) * pos.quantity;
            
            if profit > 0.0 {
                self.statistics.total_profit += profit;
                self.statistics.winning_trades += 1;
                pos.status = PositionStatus::ClosedWithProfit;
            } else {
                self.statistics.total_loss += profit.abs();
                self.statistics.losing_trades += 1;
                pos.status = PositionStatus::ClosedWithLoss;
            }
            
            pos.closed_at = Some(Utc::now());
        }
    }
    
    /// Açık pozisyon sayısı
    pub fn open_positions_count(&self) -> usize {
        self.open_positions.len()
    }
    
    /// RL state'i al veya oluştur
    pub fn get_or_create_rl_state(&mut self, symbol: &str, strategy: &str) -> &mut RLState {
        let key = format!("{}_{}", symbol, strategy);
        self.rl_states.entry(key).or_insert_with(|| RLState {
            symbol: symbol.to_string(),
            strategy: strategy.to_string(),
            q_table: HashMap::new(),
            last_action: None,
            total_reward: 0.0,
        })
    }
    
    /// İstatistikleri güncelle
    pub fn update_statistics(&mut self) {
        let total = self.statistics.total_trades;
        if total > 0 {
            self.statistics.win_rate = (self.statistics.winning_trades as f64 / total as f64) * 100.0;
        }
    }
}

/// Durum yöneticisi
pub struct StateManager {
    state: RobotState,
}

impl StateManager {
    pub fn new(robot_id: String) -> Self {
        Self {
            state: RobotState::new(robot_id),
        }
    }
    
    pub fn state(&self) -> &RobotState {
        &self.state
    }
    
    pub fn state_mut(&mut self) -> &mut RobotState {
        &mut self.state
    }
    
    pub fn save_to_json(&self, path: &str) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.state)?;
        std::fs::write(path, json)?;
        Ok(())
    }
    
    pub fn load_from_json(path: &str) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let state = serde_json::from_str(&json)?;
        Ok(Self { state })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_robot_state_creation() {
        let state = RobotState::new("robot1".to_string());
        assert_eq!(state.robot_id, "robot1");
        assert_eq!(state.status, SystemStatus::Initializing);
    }
    
    #[test]
    fn test_update_exec_time() {
        let mut state = RobotState::new("robot1".to_string());
        let now = Utc::now();
        state.update_last_exec_time("AKBNK", now);
        assert_eq!(state.get_last_exec_time("AKBNK"), Some(now));
    }
    
    #[test]
    fn test_position_management() {
        let mut state = RobotState::new("robot1".to_string());
        let position = Position {
            id: "pos1".to_string(),
            symbol: "AKBNK".to_string(),
            entry_price: 100.0,
            quantity: 10.0,
            stop_loss: 95.0,
            take_profit: 110.0,
            opened_at: Utc::now(),
            closed_at: None,
            status: PositionStatus::Open,
        };
        
        state.add_position(position);
        assert_eq!(state.open_positions_count(), 1);
        assert_eq!(state.statistics.total_trades, 1);
    }
}
