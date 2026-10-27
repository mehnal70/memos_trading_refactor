pub mod combined_strategy;
pub mod decision_tree;
pub mod drift_detector;
pub mod feature_extractor;
pub mod intelligence_hub;
pub mod linear_regressor;
pub mod signal_predictor;
pub mod strategy_scorer; // Yeni: robotic_loop'taki UCB1 motoru
pub mod strategy_selector;
pub mod trade_classifier;

pub use combined_strategy::CombinedStrategy;

pub mod adaptive_params;
pub mod automl;
pub mod hyperopt;

pub use decision_tree::{
    build_training_set, gbt_grid_search, DecisionTree, GbtTuneResult, GradientBoostedTrees,
};
pub use drift_detector::DriftDetector;
pub use feature_extractor::{FeatureExtractor, FeatureVector};
pub use linear_regressor::{LinearRegressor, Prediction};
pub use signal_predictor::{FeatureImportance, MLSignalPrediction, MLSignalPredictor};
pub use strategy_scorer::StrategyScorer;
pub use trade_classifier::{ClassifierInput, TradePatternClassifier}; // robotic_loop'u hafifleten ana parça

// Backward compatibility alias
pub type MLModel = LinearRegressor;
