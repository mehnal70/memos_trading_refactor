pub mod backtest_engine;
pub mod parameter_optimizer;
pub mod walk_forward;
pub mod backtest_scheduler;
pub mod live_path; // canlı-karar-yolunu taklit eden harness (#3 + risk-sizing ölçümü)
pub mod edge_scan; // DB-geneli gross-edge tarayıcı (tekrar koşulabilir araç çekirdeği)
pub mod multi_tf_ab; // çoklu-TF seed düzeneği A/B doğrulama harness'i (Single vs Multi)
pub mod xs_momentum; // kesitsel relatif-güç sinyali ölçüm harness'i (majör sepeti, pooled edge)

pub use backtest_engine::{Backtester, BacktestConfig, BacktestResult, DirectionMode, RegimeGate, SimulatedTrade};
//pub use backtest_engine::{ProfileComparisonResult, ProfilePerformance};
pub use parameter_optimizer::ParameterOptimizer;
pub use walk_forward::{WalkForwardTester, WalkForwardConfig, WalkForwardResult, WindowResult,
    evaluate_symbol_interval, evaluate_symbol_strategy, wf_cross_check, WfCrossCheck, wf_oos_windows};
pub use edge_scan::{EdgeScanConfig, EdgeScanReport, EdgeRow, GroupSummary, SeedRobustness, SeedEntry,
    run_edge_scan, run_edge_scan_with_progress, summarize_by_group, scan_one_series,
    seed_symbol_plan, seed_symbol_plan_from_file,
    seed_symbol_multi_plan, seed_symbol_multi_plan_from_file, SEED_MAX_TRACKS_DEFAULT, passes_seed_bar};
pub use multi_tf_ab::{run_multi_tf_ab, run_symbol_ab, arbitrate_single_position, arm_metrics,
    AbConfig, AbReport, SymbolAb, ArmMetrics, TradeSlot};
pub use xs_momentum::{run_xs_momentum, evaluate_xs, align_closes, XsConfig, XsResult,
    run_xs_walkforward, evaluate_xs_walkforward, XsWfConfig, XsWfResult, xs_target_book};
