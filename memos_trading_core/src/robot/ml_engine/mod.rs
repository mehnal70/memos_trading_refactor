pub mod feature_extractor;
pub mod linear_regressor;
pub mod signal_predictor;
pub mod decision_tree;
pub mod drift_detector;
pub mod trade_classifier;
pub mod strategy_scorer;    // Yeni: robotic_loop'taki UCB1 motoru
pub mod strategy_selector;
pub mod combined_strategy;
pub mod intelligence_hub;

pub use combined_strategy::CombinedStrategy;

pub mod automl;
pub mod hyperopt;
pub mod adaptive_params;

pub use feature_extractor::{FeatureExtractor, FeatureVector};
pub use linear_regressor::{LinearRegressor, Prediction};
pub use signal_predictor::{MLSignalPredictor, MLSignalPrediction, FeatureImportance};
pub use decision_tree::{DecisionTree, GradientBoostedTrees, build_training_set, gbt_grid_search, GbtTuneResult};
pub use drift_detector::DriftDetector;
pub use trade_classifier::{TradePatternClassifier, ClassifierInput};
pub use strategy_scorer::StrategyScorer; // robotic_loop'u hafifleten ana parça

// Backward compatibility alias
pub type MLModel = LinearRegressor;
