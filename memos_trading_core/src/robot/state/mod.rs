// src/robot/state/mod.rs - Robot Durum Garnizon Kapısı
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod position_manager; // src/robot/state/position_manager.rs (Mevcut alt dosyanız)
pub mod state_manager; // src/robot/state/state_manager.rs (Mevcut alt dosyanız)
pub mod symbol_orchestrator;
pub mod types; // Sistem istatistikleri, Position ve RobotState kontratları // Sembol-başına worker filosu (FleetCommand altına bağlanır)

// Kütüphane geneline (prelude / lib.rs) kolay erişim için re-export mühürleri
pub use state_manager::{SharedState, SharedTradingState, TradingStateInner};
pub use symbol_orchestrator::{SymbolHandle, SymbolOrchestrator, WorkerStatus};
pub use types::{
    LivePositionMap, LivePriceData, Position, PositionStatus, RobotState, SystemStatistics,
    SystemStatus,
};
