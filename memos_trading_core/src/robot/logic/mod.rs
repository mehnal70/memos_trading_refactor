pub mod anomaly_detector;
pub mod autonomous_control;
pub mod autonomous_trader;
pub mod config_helpers;
pub mod market_regime;
pub mod optimizer;
pub mod pattern_matcher;
pub mod pipeline_supervisor;
pub mod portfolio;
pub mod profitability_manager;
pub mod regime_context;
pub mod risk; // Eski src/risk.rs buraya taşınmış olmalı
pub mod strategy_lifecycle;
// strategies/ modülü robot/strategies/ altına taşındı (tek source-of-truth).
pub mod symbol_watch_manager;
