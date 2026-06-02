pub mod backtest_engine;
pub mod parameter_optimizer;
pub mod walk_forward;
pub mod backtest_scheduler;
pub mod live_path; // canlı-karar-yolunu taklit eden harness (#3 + risk-sizing ölçümü)
pub mod edge_scan; // DB-geneli gross-edge tarayıcı (tekrar koşulabilir araç çekirdeği)

pub use backtest_engine::{Backtester, BacktestConfig, BacktestResult, DirectionMode, RegimeGate, SimulatedTrade};
//pub use backtest_engine::{ProfileComparisonResult, ProfilePerformance};
pub use parameter_optimizer::ParameterOptimizer;
pub use walk_forward::{WalkForwardTester, WalkForwardConfig, WalkForwardResult, WindowResult};
pub use edge_scan::{EdgeScanConfig, EdgeScanReport, EdgeRow, GroupSummary,
    run_edge_scan, run_edge_scan_with_progress, summarize_by_group, scan_one_series};
