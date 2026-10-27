pub mod backtest_engine;
pub mod backtest_scheduler;
pub mod bb_pool; // "1d-BB havuzlanmış" hipotez harness'i (edge_scan per-sembol ipucunun pooled testi)
pub mod edge_scan; // DB-geneli gross-edge tarayıcı (tekrar koşulabilir araç çekirdeği)
pub mod funding_carry;
pub mod live_path; // canlı-karar-yolunu taklit eden harness (#3 + risk-sizing ölçümü)
pub mod multi_tf_ab; // çoklu-TF seed düzeneği A/B doğrulama harness'i (Single vs Multi)
pub mod parameter_optimizer;
pub mod walk_forward;
pub mod xs_momentum; // kesitsel relatif-güç sinyali ölçüm harness'i (majör sepeti, pooled edge) // kesitsel funding-carry harness'i (fiyat-dışı taşıma getirisi; dik eksen)

pub use backtest_engine::{
    BacktestConfig, BacktestResult, Backtester, DirectionMode, RegimeGate, SimulatedTrade,
};
//pub use backtest_engine::{ProfileComparisonResult, ProfilePerformance};
pub use bb_pool::{
    bb_pool_returns, evaluate_bb_pool, evaluate_bb_pool_walkforward, run_bb_pool,
    run_bb_pool_walkforward, BbPoolConfig, BbWfConfig, BbWfResult,
};
pub use edge_scan::{
    passes_seed_bar, run_edge_scan, run_edge_scan_with_progress, scan_one_series,
    seed_symbol_multi_plan, seed_symbol_multi_plan_from_file, seed_symbol_plan,
    seed_symbol_plan_from_file, summarize_by_group, EdgeRow, EdgeScanConfig, EdgeScanReport,
    GroupSummary, SeedEntry, SeedRobustness, SEED_MAX_TRACKS_DEFAULT,
};
pub use funding_carry::{
    align_closes_and_funding, evaluate_funding_carry, evaluate_funding_carry_walkforward,
    funding_carry_returns, run_funding_carry, run_funding_carry_returns,
    run_funding_carry_walkforward, FcWfConfig, FcWfResult, FundingCarryConfig,
};
pub use multi_tf_ab::{
    arbitrate_single_position, arm_metrics, run_multi_tf_ab, run_symbol_ab, AbConfig, AbReport,
    ArmMetrics, SymbolAb, TradeSlot,
};
pub use parameter_optimizer::ParameterOptimizer;
pub use walk_forward::{
    evaluate_symbol_interval, evaluate_symbol_strategy, wf_cross_check, wf_oos_windows,
    WalkForwardConfig, WalkForwardResult, WalkForwardTester, WfCrossCheck, WindowResult,
};
pub use xs_momentum::{
    align_closes, evaluate_xs, evaluate_xs_walkforward, run_xs_momentum, run_xs_returns,
    run_xs_walkforward, series_metrics, xs_target_book, XsConfig, XsResult, XsSignal, XsWfConfig,
    XsWfResult,
};
