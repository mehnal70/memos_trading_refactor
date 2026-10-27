// Evolution Module - Evrimsel Trading Sistemi
// "Kervan yolda düzülür" - Self-discovering optimal strategies

pub mod adaptive_brain;
pub mod autonomous_controller;
pub mod fitness_evaluator;
pub mod mutation_engine;
pub mod population_manager;
pub mod strategy_genome;

pub use adaptive_brain::{AdaptiveBrain, MarketRegime};
pub use autonomous_controller::{
    AutonomousController, AutonomousControllerConfig, AutonomousState,
};
pub use fitness_evaluator::{FitnessScore, PerformanceMetrics};
pub use mutation_engine::{MutationEngine, MutationType};
pub use population_manager::{PopulationManager, SelectionStrategy};
pub use strategy_genome::{GeneticParams, StrategyGenome};
