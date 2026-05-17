// src/robot/state/state_manager.rs - Srivastava ATP Merkezi Harekat ve Hafıza Yönetimi
// Srivastava ATP - Canlı ve Operasyonel Hafıza Çekirdeği (Unified Omni-State)

use crate::prelude::*; 
use super::types::{RobotState, SystemStatistics, SystemStatus}; 
use crate::robot::logic::anomaly_detector::AnomalyDetector; 
use crate::core::types::Trade;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Instant, Duration};

// =============================================================================
// 1. ASENKRON CANLI HAFIZA PAYLAŞIM KATMANI (Eski manager.rs'ten Süzülen Zeka)
// =============================================================================

/// TradingStateInner: robotic_loop ve TUI'nin kilitlenme (deadlock) yaşamadan 
/// asenkron paylaştığı "Canlı" hafıza otobanı.
pub struct TradingStateInner {
    pub robot_state: Arc<RwLock<RobotState>>,
    pub live_price: Arc<RwLock<HashMap<String, f64>>>,
    pub sl_cooldowns: Arc<RwLock<HashMap<String, Instant>>>,
    pub dynamic_blacklist: Arc<RwLock<HashMap<String, Instant>>>,
}

#[derive(Clone)]
pub struct SharedTradingState(pub Arc<RwLock<TradingStateInner>>);

/// Global asenkron erişim için tip takma adı (Alias)
pub type SharedState = Arc<TradingStateInner>;

// =============================================================================
// 2. STANDART ARAYÜZ SÖZLEŞMESİ (ACCOUNT STORE CONTRACT)
// =============================================================================

pub trait StateManager: Send + Sync {
    fn get_symbols(&self) -> Result<Vec<String>, crate::MemosTradingError>;
    fn get_balance(&self) -> Result<f64, crate::MemosTradingError>;
    fn add_trade(&mut self, trade: Trade) -> Result<(), crate::MemosTradingError>;
}

// =============================================================================
// 3. ANA OPERASYONEL HAFIZA MOTORU (LOOP STATE)
// =============================================================================

/// ⚔️ LoopState: Robotik döngünün anlık kayma, kayıp serisi ve otonom karantina sayaçları.
pub struct LoopState {
    pub capital: f64,
    pub current_equity: f64,
    pub peak_equity: f64,
    pub cumulative_pnl: f64,
    pub day_start_equity: f64,
    
    // Performans Sayaçları
    pub loss_streak: usize,
    pub short_loss_streak: u32,
    pub session_closed: usize,
    pub session_wins: usize,
    pub session_profit: f64,
    pub session_loss: f64,
    pub win_trades: usize,
    pub total_trades: usize,

    // Pozisyon ve Karantina Yönetimi
    pub open_positions: HashMap<String, PositionModel>, // Key: Symbol
    pub sl_cooldown_map: HashMap<String, Instant>,
    pub symbol_cooldown_secs: HashMap<String, u64>,
    pub dynamic_blacklist: HashMap<String, Instant>,
    pub last_trade_time: HashMap<String, Instant>,
    pub tp_win_dir_map: HashMap<String, (bool, Instant)>, // (is_long, time)

    // Otonom Koruma Muhafızları (Circuit Breaker)
    pub api_circuit_breaker: AnomalyDetector, 
    pub hf_error_log: HashMap<String, VecDeque<Instant>>,
    pub symbol_slip_bps: HashMap<String, VecDeque<f64>>,
    pub last_health_check: Instant,
    pub stop_loop: bool,
}

impl LoopState {
    /// 🧬 PnL Kaydı ve Drawdown Güncelleme (Yön ve Rejim Duyarlı)
    pub fn record_pnl_dir(&mut self, pnl: f64, is_long: bool) {
        self.cumulative_pnl += pnl;
        self.current_equity = (self.capital + self.cumulative_pnl).max(0.0);
        
        if self.current_equity > self.peak_equity {
            self.peak_equity = self.current_equity;
        }

        self.session_closed += 1;
        self.total_trades += 1;
        
        if pnl > 0.0 {
            self.session_wins += 1;
            self.session_profit += pnl;
            self.win_trades += 1;
            self.loss_streak = 0;
            if !is_long { self.short_loss_streak = 0; }
        } else {
            self.session_loss += pnl.abs();
            self.loss_streak += 1;
            if !is_long { self.short_loss_streak += 1; }
        }
    }

    /// 📊 Slippage (Kayma) Analitiği (Bps Tabanlı)
    pub fn record_slippage_bps(&mut self, symbol: &str, expected: f64, actual: f64) {
        if expected <= 0.0 { return; }
        let bps = (actual - expected).abs() / expected * 10_000.0;
        let hist = self.symbol_slip_bps.entry(symbol.to_string()).or_insert_with(|| VecDeque::with_capacity(20));
        if hist.len() >= 20 { hist.pop_front(); }
        hist.push_back(bps);
    }

    /// 🛡️ Yüksek Frekanslı Hata Koruması ve Otonom Karantina (HF Blacklist)
    pub fn record_hf_error(&mut self, symbol: &str) -> bool {
        let now = Instant::now();
        let log = self.hf_error_log.entry(symbol.to_string()).or_insert_with(|| VecDeque::with_capacity(5));
        log.push_back(now);
        
        // Son 1 saat (3600 sn) içindeki hataları temizle (Memory Cleanup)
        log.retain(|&t| now.duration_since(t).as_secs() < 3600);
        
        // Son 1 saatte 2'den fazla donma/hata alındıysa sembolü 4 saat karantinaya al
        if log.len() >= 2 {
            self.dynamic_blacklist.insert(
                symbol.to_string(), 
                now + Duration::from_secs(14400)
            );
            return true;
        }
        false
    }
}

// =============================================================================
// 4. METOT İMPLEMENTASYONLARI VE ROBOT STATE FABRİKASI
// =============================================================================

impl RobotState {
    pub fn new(robot_id: String, initial_capital: f64) -> Self {
        Self {
            robot_id,
            last_run: None,
            last_exec_times: HashMap::new(),
            current_equity: initial_capital,
            cumulative_pnl: 0.0,
            peak_equity: initial_capital,
            open_positions: Vec::new(),
            closed_positions: Vec::new(),
            statistics: SystemStatistics::default(),
            status: SystemStatus::Initializing,
        }
    }

    /// PnL kaydı ve Canlı Bakiye güncellemesi
    pub fn record_trade_result(&mut self, pnl: f64) {
        self.cumulative_pnl += pnl;
        self.current_equity += pnl;
        if self.current_equity > self.peak_equity {
            self.peak_equity = self.current_equity;
        }
    }
}

impl StateManager for LoopState {
    fn get_symbols(&self) -> Result<Vec<String>, crate::MemosTradingError> {
        Ok(self.open_positions.keys().cloned().collect())
    }
    
    fn get_balance(&self) -> Result<f64, crate::MemosTradingError> {
        Ok(self.current_equity)
    }
    
    fn add_trade(&mut self, _trade: Trade) -> Result<(), crate::MemosTradingError> {
        Ok(())
    }
}
