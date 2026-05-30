// robot/infra/monitor.rs - Srivastava ATP Otonom Sağlık ve Fail-Safe Denetçisi
//
// Modernizasyon Notları:
// 1. Match-Guard ve Pattern Matching ile karar hiyerarşisi
// 2. Platform bağımsız (Native/WASM) birleştirilmiş durum yönetimi
// 3. Fonksiyonel anomali ve metrik değerlendirme
// 4. Kapsüllü hata yönetimi ve Reset lojiği

use chrono::{DateTime, Utc};
#[cfg(not(target_arch = "wasm32"))]
use crate::robot::logic::portfolio::PortfolioMetrics;

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub max_drawdown_pct: f64,
    pub min_win_rate: f64,
    pub critical_error_limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct MonitorState {
    pub last_check: Option<DateTime<Utc>>,
    pub error_count: usize,
    pub anomaly_detected: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub last_metrics: Option<PortfolioMetrics>,
}

#[derive(Clone)]
pub struct Monitor {
    pub config: MonitorConfig,
    pub state: MonitorState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MonitorAction { Continue, Pause, Restart, Stop }

impl Monitor {
    pub fn new(config: MonitorConfig) -> Self {
        Self { config, state: MonitorState::default() }
    }

    /// §84.5: Otonom Denetim Döngüsü - Karar Matrisi
    #[cfg(not(target_arch = "wasm32"))]
    pub fn check(&mut self, metrics: &PortfolioMetrics, ml_score: Option<f64>) -> MonitorAction {
        self.state.last_check = Some(Utc::now());
        self.state.last_metrics = Some(metrics.clone());

        // Hiyerarşik Karar Mekanizması (if/else yerine Match-Guard)
        
        match () {
            // 1. Kritik Hata Freni (En yüksek öncelik)
            _ if self.state.error_count > self.config.critical_error_limit => MonitorAction::Stop,

            // 2. ML Anomali Tespiti
            _ if ml_score.unwrap_or(0.0) > 0.9 => {
                self.state.anomaly_detected = true;
                MonitorAction::Restart
            },

            // 3. Performans Koridor Denetimi
            _ if metrics.closed_trades_count > 10 || metrics.open_positions_count > 0 => {
                self.evaluate_performance(metrics)
            },

            _ => MonitorAction::Continue,
        }
    }

    /// WASM uyumlu hafif denetim
    #[cfg(target_arch = "wasm32")]
    pub fn check(&mut self, ml_score: Option<f64>) -> MonitorAction {
        match ml_score {
            Some(s) if s > 0.9 => {
                self.state.anomaly_detected = true;
                MonitorAction::Restart
            },
            _ => MonitorAction::Continue
        }
    }

    /// Performans metriklerini otonom süzgeçten geçirir
    #[cfg(not(target_arch = "wasm32"))]
    fn evaluate_performance(&self, m: &PortfolioMetrics) -> MonitorAction {
        let dd_limit = self.config.max_drawdown_pct / 100.0;
        
        match () {
            _ if m.win_rate < self.config.min_win_rate => MonitorAction::Pause,
            _ if m.total_pnl < 0.0 && m.max_drawdown > dd_limit => MonitorAction::Pause,
            _ => MonitorAction::Continue,
        }
    }

    pub fn log_error(&mut self)    { self.state.error_count += 1; }
    pub fn reset_errors(&mut self) { self.state.error_count = 0; }
}
