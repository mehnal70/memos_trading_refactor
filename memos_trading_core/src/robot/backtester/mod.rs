pub mod engine;
pub mod parameter_optimizer;
pub mod walk_forward;

pub use engine::{Backtester, BacktestConfig, BacktestResult, SimulatedTrade, ProfileComparisonResult, ProfilePerformance};
pub use parameter_optimizer::ParameterOptimizer;
pub use walk_forward::{WalkForwardTester, WalkForwardConfig, WalkForwardResult, WindowResult};
