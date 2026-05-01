// Evolution Module - Evrimsel Trading Sistemi
// "Kervan yolda düzülür" - Self-discovering optimal strategies

pub mod strategy_genome;
pub mod fitness_evaluator;
pub mod mutation_engine;
pub mod population_manager;
pub mod adaptive_brain;

pub use strategy_genome::{StrategyGenome, GeneticParams};
pub use fitness_evaluator::{FitnessScore, PerformanceMetrics};
pub use mutation_engine::{MutationEngine, MutationType};
pub use population_manager::{PopulationManager, SelectionStrategy};
pub use adaptive_brain::{AdaptiveBrain, MarketRegime};
