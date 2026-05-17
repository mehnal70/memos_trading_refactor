pub mod portfolio;
pub mod strategy_lifecycle;
pub mod risk; // Eski src/risk.rs buraya taşınmış olmalı
pub mod anomaly_detector;
pub mod autonomous_trader;
pub mod autonomous_control;
pub mod market_regime;
pub mod optimizer;
pub mod pattern_matcher;
pub mod pipeline_supervisor;
pub mod profitability_manager;
pub mod config_helpers;
// strategies/ modülü robot/strategies/ altına taşındı (tek source-of-truth).
pub mod symbol_watch_manager;