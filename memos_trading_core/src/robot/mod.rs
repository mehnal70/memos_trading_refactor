// --- MODÜL EXPORTLARI ---
pub use crate::robot::ml_engine::{MLModel, FeatureVector};
pub use crate::robot::monitor::{Monitor, MonitorAction};
pub use crate::robot::hyperopt::{HyperOpt, HyperOptResult};
pub use crate::robot::optimizer::AdvancedOptimizer;
pub use crate::robot::automl::AutoML;
pub use crate::robot::streaming::StreamingSource;
pub use crate::robot::dashboard::Dashboard;
pub use crate::robot::security::SecurityManager;
pub use crate::robot::user_profile::UserProfile;
pub use crate::robot::api::ApiService;
pub use crate::robot::test_orchestrator::{StrategyTestOrchestrator, PipelineConfig, PipelineResult, StageResult, StageStatus};
pub use crate::robot::config_helpers::{PositionManagementProfile, SecurityProfile, PositionConfigBuilder};

// Autonomous AI/ML Trading System
pub mod autonomous_trader;
pub mod autonomous_audit;
pub use autonomous_trader::{
    AutonomousTrader, AutonomousConfig, SymbolPerformance, 
    StrategyPerformance, GraduationDecision,
};
pub use autonomous_audit::{
    CycleRecord, StageRecord, AuditStageStatus, AutonomousAuditLogger,
    ValidationResult, ValidationCheck,
};

// Srivastava ATP Mimarisi - Order Management System
pub mod order_management;
pub use order_management::{
    OrderManager, OrderManagementSystem, Order, OrderId, OrderSide, OrderType,
    OrderStatus, SlippageInfo, SlippageLevel, PartialFillInfo, RetryPolicy,
    SlippageDetector, DefaultSlippageDetector, BaseOrderManagementSystem,
};

// Srivastava ATP Mimarisi - Data Processor
pub mod data_processor;
pub use data_processor::{DataProcessor, DataValidator, DataNormalizer, DataCleaner};

// Srivastava ATP Mimarisi - Portfolio Management (Tier 2)
pub mod portfolio_manager;
pub use portfolio_manager::{
    PortfolioManager, Position, ClosedTrade, PortfolioMetrics, CorrelationMatrix,
};

// Srivastava ATP Mimarisi - Advanced Risk Metrics (Tier 2)
pub mod advanced_risk;
pub use advanced_risk::{
    SharpeCalculator, SortinoCalculator, CalmarCalculator, OmegaCalculator,
    InformationRatio, KellyCriterion, KellyRecommendation, ValueAtRisk, VaRLimits,
};

// Srivastava ATP Mimarisi - Error Recovery (Tier 3)
pub mod error_recovery;
pub use error_recovery::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerState,
    FailoverManager, FailoverStrategy, ExecutorType,
    RecoveryStateMachine, RecoveryState, RecoveryAction,
};

// Srivastava ATP Mimarisi - Hot-Reload Engine (Tier 3)
pub mod hot_reload;
pub use hot_reload::{
    StrategyLoader, LoadedStrategy, StrategyLoadError,
    VersionManager, StrategyVersion, VersionInfo,
    ZeroDowntimeUpdateManager, UpdateState, UpdateAction, UpdateProcessInfo,
};

// Srivastava ATP Mimarisi - Advanced Monitoring (Tier 4)
pub mod advanced_monitoring;
pub use advanced_monitoring::{
    RealtimeDashboard, DashboardMetrics, MetricSnapshot,
    AlertSystem, AlertLevel, Alert, AlertChannel, AlertConfig,
    PerformanceTrendingEngine, TrendData, PerformanceTrend, TrendAnalysis,
};

// Demo ve Integration Examples
pub mod srivastava_demo;

// Advanced Integration: Dinamik Pozisyon Yönetimi + Güvenlik
pub mod integration_advanced;
pub use integration_advanced::AdvancedRoboticTrader;

// Trait importları (RiskAnalyzer pub use interfaces::* ile import olacak)
use crate::robot::strategies::MaCrossoverStrategy;
use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

// Robotik sistem için temel sağlık/anomali izleyici
pub struct RobotHealthMonitor {
	pub last_cycle_success: bool,
	pub error_count: usize,
	pub last_error: Option<String>,
}

impl HealthCheck for RobotHealthMonitor {
	fn check_health(&self) -> HealthStatus {
		if !self.last_cycle_success {
			HealthStatus::Warning("Son işlem döngüsü başarısız".to_string())
		} else if self.error_count > 5 {
			HealthStatus::Warning(format!("Çok fazla hata: {}", self.error_count))
		} else {
			HealthStatus::Healthy
		}
	}
}

impl AnomalyDetector for RobotHealthMonitor {
	fn detect_anomaly(&self) -> Option<AnomalyType> {
		if !self.last_cycle_success {
			return Some(AnomalyType::Custom("Son işlem döngüsü başarısız".to_string()));
		}
		if self.error_count > 10 {
			return Some(AnomalyType::Custom(format!("Aşırı hata: {}", self.error_count)));
		}
		if let Some(err) = &self.last_error {
			return Some(AnomalyType::Custom(format!("Son hata: {}", err)));
		}
		None
	}
}
// ...existing code...
pub mod advanced;
pub mod indicators;
pub mod data_fetcher;
pub use data_fetcher::{HybridBinanceFetcher, FetchMode};
pub use data_fetcher::BinanceLiveAdapter;
pub mod safety;
pub use safety::{SafetyManager, SafetyRules, SafetyDrawdownMonitor, SafetyStatus, SafetyMetrics, TradingMetrics, EquityTrend, PaperTradingDashboard, DashboardData, DashboardState, OpenPosition, AlertManager, TradingAlert, TradingAlertLevel, AlertCode};
pub mod persistence;
pub use persistence::{TradeRepository, AccountStateRepository, CandleRepository, PersistenceService, TradeResponse, CandleResponse, StatsResponse};
pub mod symbol_manager;
pub use symbol_manager::{SymbolManager, SymbolState, PortfolioStats};
pub mod backtester;
pub use backtester::{Backtester, BacktestConfig, SimulatedTrade, ParameterOptimizer};
pub mod ml_engine;
pub use ml_engine::{FeatureExtractor, LinearRegressor, MLSignalPredictor, MLSignalPrediction};
pub mod file_logger;
pub use file_logger::FileLogger;
pub mod position_manager;
pub mod signal_evaluator;
pub use signal_evaluator::TradeQualityConfig;
pub mod adaptive_params;
pub use adaptive_params::AdaptiveTradeParams;
pub mod robotic_loop;
pub use robotic_loop::{RoboticLoop, SharedTradingState, TradingStateInner};
#[cfg(not(target_arch = "wasm32"))]
pub mod price_feed;
#[cfg(not(target_arch = "wasm32"))]
pub mod binance_executor;
#[cfg(not(target_arch = "wasm32"))]
pub use binance_executor::BinanceFuturesExecutor;
pub mod trade_executor;
pub mod risk_guardrails;
pub use risk_guardrails::{
    DrawdownMonitor, DrawdownStatus, LiquidityMonitor, LiquidityStatus,
    SlippageStatus // SlippageDetector is re-exported from order_management
};
pub mod logger;
pub use logger::{TradingLogger, TradeEvent};
pub mod autonomous_control;
pub use autonomous_control::{
	AutonomousState, AutonomousConfig as AutonomousControllerConfig,
	AutonomousTransition, AutonomousController,
	RiskGatePolicy, RiskInput, RiskDecision, RiskGate,
	AutonomousRecoveryAction, RecoverySupervisor,
};

// Configuration helpers for dynamic position and security management
pub mod config_helpers;
pub mod backtest_scheduler;

/// Scalp & Swing kısa-vade fırsat motoru — çakışmasız slot yönetimi ile
pub mod scalp_swing;
pub mod strategy_scorer;
pub use backtest_scheduler::{BacktestScheduler, BacktestResult, SchedulerConfig, TradingMode, CanaryStatus};

// Çoklu sembol worker orkestratörü
pub mod symbol_orchestrator;
pub use symbol_orchestrator::{SymbolOrchestrator, SymbolHandle, WorkerStatus};

// Destek / Direnç tespiti — hacim ağırlıklı swing nokta kümeleme
pub mod sr_detector;
// Telegram push bildirimleri — kritik trading olayları
pub mod telegram_notifier;
pub use sr_detector::{SrDetector, SrDetectorConfig, SrZone, SrContext, ZoneType};
#[cfg(all(test, not(target_arch = "wasm32")))]
#[cfg(all(test, not(target_arch = "wasm32")))]
mod pilot_tests {
	use super::*;
	use crate::types::{StrategyParams, Signal};
	// ...existing code...
	use chrono::Utc;

	#[test]
	fn test_ma_crossover_pipeline_trade() {
		// ...existing code...
		use crate::types::Candle;
		// ...existing code...
		// 1. Dummy veri ve pipeline
		let candles = (0..30).map(|i| Candle {
			timestamp: Utc::now(),
			open: i as f64,
			high: i as f64,
			low: i as f64,
			close: i as f64,
			volume: 1.0,
			symbol: "BTCUSDT".to_string(),
			interval: "1h".to_string(),
		}).collect::<Vec<_>>();
		let strat = MaCrossoverStrategy;
		let params = StrategyParams { fast: Some(5), slow: Some(20), period: None, overbought: None, oversold: None, fast_period: None, slow_period: None, signal_period: None, std_dev: None, bb_period: None };
		let sig = strat.generate_signal(&candles, &params, None, None).unwrap();
		assert!(matches!(sig, Signal::Buy | Signal::Sell | Signal::Hold));
		// 2. State, executor, reporter
		let mut state = InMemoryStateManager::new();
		state.set_symbols(vec!["BTCUSDT".to_string()]).unwrap();
		state.set_balance(10000.0).unwrap();
		let executor = DummyTradeExecutor;
		let reporter = UniversalReporter;
		let robot = RoboticTradeExecutor::with_state(&executor, &state, Some((0, 24)));
		let trades = robot.execute_basket(sig.clone(), 0.1);
		for t in &trades {
			if let Ok(trade) = t {
				reporter.report_trade(trade).unwrap();
			}
		}
		assert!(trades.len() == 1);
	}
}
/* 			if let Ok(trade) = t {
				reporter.report_trade(trade).unwrap();
			}
		}
		assert!(trades.len() == 1);
	}
} */
#[cfg(test)]
mod config_state_tests {
	use super::*;
	use crate::robot::{FileConfigManager, InMemoryStateManager};
	use crate::types::{Exchange, Market, Candle};
	use chrono::Utc;
	use std::fs;

	#[test]
	fn test_config_and_state_manager_integration() {
		// Config dosyasına yaz ve oku
		let path = "/tmp/test_app_config2.json";
		let config = AppConfig {
			exchange: Exchange::Bist,
			market: Market::Spot,
			interval: "5m".to_string(),
			strategy: "RSI".to_string(),
			risk: Some("low".to_string()),
			extra: None,
		};

		let mgr = FileConfigManager::new(path);
		mgr.save_config(&config).unwrap();
		let loaded = mgr.load_config().unwrap();
		assert_eq!(loaded.exchange, Exchange::Bist);
		assert_eq!(loaded.market, Market::Spot);
		fs::remove_file(path).unwrap();

		// StateManager ile sembol ve bakiye yönetimi
		let mut state = InMemoryStateManager::new();
		state.set_symbols(vec!["GARAN".to_string(), "AKBNK".to_string()]).unwrap();
		state.set_balance(5000.0).unwrap();
		let syms = state.get_symbols().unwrap();
		let bal = state.get_balance().unwrap();
		assert_eq!(syms, vec!["GARAN", "AKBNK"]);
		assert_eq!(bal, 5000.0);
	}
// ...existing code...

	#[test]
	fn test_ma_crossover_pipeline_trade() {
		// 1. Dummy veri ve pipeline
		let candles = (0..30).map(|i| Candle {
			timestamp: Utc::now(),
			open: i as f64,
			high: i as f64,
			low: i as f64,
			close: i as f64,
			symbol: "BTCUSDT".to_string(),
			interval: "1m".to_string(),
			volume: 0.0,
		}).collect::<Vec<_>>();

		let strat = MaCrossoverStrategy;
		let params = StrategyParams { fast: Some(5), slow: Some(20), period: None, overbought: None, oversold: None, fast_period: None, slow_period: None, signal_period: None, std_dev: None, bb_period: None };
		let sig = strat.generate_signal(&candles, &params, None, None).unwrap();
		assert!(matches!(sig, Signal::Buy | Signal::Sell | Signal::Hold));

		// 2. State, executor, reporter
		let mut state = InMemoryStateManager::new();
		state.set_symbols(vec!["BTCUSDT".to_string()]).unwrap();
		state.set_balance(10000.0).unwrap();
		let executor = DummyTradeExecutor;
		let reporter = UniversalReporter;
		let robot = RoboticTradeExecutor::with_state(&executor, &state, Some((0, 24)));
		let trades = robot.execute_basket(sig.clone(), 0.1);
		for t in &trades {
			if let Ok(trade) = t {
				reporter.report_trade(trade).unwrap();
			}
		}
		assert!(trades.len() == 1);
	}
}
// robot/mod.rs - Robotik Trading Sistemi (Ana modül)

pub mod data_pipeline;
pub mod calculations;
pub mod config;
pub mod state;
pub mod strategies;
pub use crate::strategies::Strategy;
pub mod interfaces;
pub mod reporter;
pub mod risk_adapter;
pub mod pipeline;
pub mod reporting;
pub mod executor;
pub mod state_manager;
pub mod error;
pub mod config_manager;
pub mod monitor;
pub mod hyperopt;
pub mod test_orchestrator;  // 📍 YENİ: Pipeline Orchestrator - katmanlı test yönetimi
pub mod optimizer;
pub mod pattern_matcher;
pub mod automl;
pub mod streaming;
pub mod dashboard;
pub mod security;
pub mod user_profile;
pub mod api;

pub use data_pipeline::DataPipeline;
pub use calculations::IndicatorEngine;
// ...existing code...
pub use interfaces::*;
pub use reporter::StdoutReporter;
pub use risk_adapter::SimpleRiskAnalyzer;
pub use trade_executor::{DummyTradeExecutor, BinanceTradeExecutor};
pub use pipeline::DataPipelineModular;
pub use reporting::UniversalReporter;
pub use executor::RoboticTradeExecutor;
pub use state_manager::{StateManager, InMemoryStateManager};
pub use error::{ErrorLogger, StdoutLogger};
#[cfg(not(target_arch = "wasm32"))]
pub use crate::portfolio::{Portfolio, Position as LegacyPosition, PortfolioMetrics as LegacyPortfolioMetrics};
pub use config_manager::{AppConfig, ConfigManager, FileConfigManager};

/// Yeni mimari modüllerinin versiyonu
pub const ROBOT_VERSION: &str = "2.0.0";

/// Global constants
pub const DEFAULT_CACHE_TTL_SECONDS: u64 = 3600;
pub const DEFAULT_MAX_CACHED_SYMBOLS: usize = 100;
pub const DEFAULT_BATCH_SIZE: usize = 1000;

use crate::types::{StrategyParams, Signal};

/// Örnek modüler pipeline: DataPipeline + Strategy entegrasyonu
/// (Asenkron, test amaçlı)
#[allow(dead_code)]
pub async fn run_ma_crossover_pipeline(
	pipeline: &DataPipeline,
	symbol: &str,
	interval: &str,
	fast: usize,
	slow: usize,
) -> crate::Result<Signal> {
	let params = crate::robot::data_pipeline::FetchParams {
		symbol: symbol.to_string(),
		interval: interval.to_string(),
		start_time: None,
		end_time: None,
		limit: Some((slow + 5).max(30)),
	};
	
	// Pipeline'dan candle verisi çek
	let candles = pipeline.process(params).await?;
	
	#[cfg(not(target_arch = "wasm32"))]
	{
		let strat = MaCrossoverStrategy;
		let strat_params = StrategyParams {
			fast: Some(fast),
			slow: Some(slow),
			..Default::default()
		};
		strat.generate_signal(&candles, &strat_params, None, None)
	}
	#[cfg(target_arch = "wasm32")]
	Ok(Signal::Hold)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::robot::data_pipeline::{FetchParams, sources::DataSource};
	use async_trait::async_trait;
	use tokio_test::block_on;

	/// Test için boş veri döndüren mock kaynak
	struct EmptyDataSource;

	#[async_trait]
	impl DataSource for EmptyDataSource {
		async fn fetch(&self, _p: &FetchParams) -> crate::Result<Vec<crate::types::Candle>> {
			Ok(vec![])
		}
		fn source_type(&self) -> &str { "mock" }
		async fn health_check(&self) -> crate::Result<()> { Ok(()) }
	}

	#[test]
	fn test_run_ma_crossover_pipeline() {
		let pipeline = DataPipeline::new(Box::new(EmptyDataSource));
		// Veri yok → strategy Hold döner
		let sig = block_on(run_ma_crossover_pipeline(&pipeline, "TEST", "1m", 5, 20)).unwrap();
		assert_eq!(sig, Signal::Hold);
	}
}
