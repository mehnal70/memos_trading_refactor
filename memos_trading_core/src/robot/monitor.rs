// robot/monitor.rs - Loglama, izleme ve fail-safe modülü (ML/AI destekli)
// Kârlılık, drawdown, başarı oranı, otomatik yeniden başlatma ve ML tabanlı anomali tespiti

#[cfg(not(target_arch = "wasm32"))]
use crate::robot::PortfolioMetrics;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub max_drawdown_pct: f64,
    pub min_win_rate: f64,
    pub critical_error_limit: usize,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Default)]
pub struct MonitorState {
    pub last_check: Option<DateTime<Utc>>,
    pub error_count: usize,
    #[cfg(not(target_arch = "wasm32"))]
    pub last_metrics: Option<PortfolioMetrics>,
    pub anomaly_detected: bool,
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Default)]
pub struct MonitorState {
    pub last_check: Option<DateTime<Utc>>,
    pub error_count: usize,
    pub anomaly_detected: bool,
}

 #[derive(Clone)]
 pub struct Monitor {
    pub config: MonitorConfig,
    pub state: MonitorState,
}

#[cfg(not(target_arch = "wasm32"))]
impl Monitor {
    pub fn new(config: MonitorConfig) -> Self {
        Self { config, state: MonitorState::default() }
    }
    /// Portföy metriklerini ve ML/AI tabanlı anomaliyi kontrol et
    pub fn check(&mut self, metrics: &PortfolioMetrics, ml_anomaly_score: Option<f64>) -> MonitorAction {
        let mut action = MonitorAction::Continue;
        // ML/AI tabanlı anomali tespiti
        if let Some(score) = ml_anomaly_score {
            if score > 0.9 {
                self.state.anomaly_detected = true;
                action = MonitorAction::Restart;
            }
        }
        // Drawdown ve winrate kontrolü (uses new PortfolioMetrics from portfolio_manager)
        if metrics.open_positions_count > 0 || metrics.closed_trades_count > 10 {
            if metrics.win_rate < self.config.min_win_rate {
                action = MonitorAction::Pause;
            }
            if metrics.total_pnl < 0.0 && metrics.max_drawdown > (self.config.max_drawdown_pct / 100.0) {
                action = MonitorAction::Pause;
            }
        }
        // Kritik hata limiti
        if self.state.error_count > self.config.critical_error_limit {
            action = MonitorAction::Stop;
        }
        self.state.last_check = Some(Utc::now());
        self.state.last_metrics = Some(metrics.clone());
        action
    }
    pub fn log_error(&mut self) {
        self.state.error_count += 1;
    }
    pub fn reset_errors(&mut self) {
        self.state.error_count = 0;
    }
}

#[cfg(target_arch = "wasm32")]
impl Monitor {
    pub fn new(config: MonitorConfig) -> Self {
        Self { config, state: MonitorState::default() }
    }
    /// WASM'da sadece ML/AI tabanlı anomaliyi kontrol et, metrik yok
    pub fn check(&mut self, ml_anomaly_score: Option<f64>) -> MonitorAction {
        let mut action = MonitorAction::Continue;
        if let Some(score) = ml_anomaly_score {
            if score > 0.9 {
                self.state.anomaly_detected = true;
                action = MonitorAction::Restart;
            }
        }
        action
    }
    pub fn log_error(&mut self) {
        self.state.error_count += 1;
    }
    pub fn reset_errors(&mut self) {
        self.state.error_count = 0;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MonitorAction {
    Continue,
    Pause,
    Restart,
    Stop,
}
