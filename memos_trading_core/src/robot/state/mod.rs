// src/robot/state/mod.rs - Robot Durum Garnizon Kapısı
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod types;            // Sistem istatistikleri, Position ve RobotState kontratları
pub mod state_manager;    // src/robot/state/state_manager.rs (Mevcut alt dosyanız)
pub mod position_manager; // src/robot/state/position_manager.rs (Mevcut alt dosyanız)
pub mod symbol_orchestrator; // Sembol-başına worker filosu (FleetCommand altına bağlanır)

// Kütüphane geneline (prelude / lib.rs) kolay erişim için re-export mühürleri
pub use types::{SystemStatistics, RobotState, SystemStatus, PositionStatus, Position,
                LivePositionMap, LivePriceData};
pub use state_manager::{TradingStateInner, SharedTradingState, SharedState};
pub use symbol_orchestrator::{SymbolOrchestrator, SymbolHandle, WorkerStatus};
