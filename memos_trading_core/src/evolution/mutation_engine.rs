// mutation_engine.rs - Akıllı ve Adaptif Mutasyon Motoru

// evolution/mutation_engine.rs - Akıllı ve Adaptif Mutasyon Motoru
use crate::prelude::*; 

use crate::evolution::StrategyGenome;
use serde::{Deserialize, Serialize};
use rand::Rng;
use rand_distr::{Distribution, Normal};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MutationType {
    Random,
    Adaptive,
    Gaussian { mean: f64, std_dev: f64 },
    Directed { direction: f64 },
    Hybrid,
}

#[derive(Debug, Clone, Default)]
pub struct MutationStats {
    pub total_mutations: u64,
    pub beneficial_mutations: u64,
    pub neutral_mutations: u64,
    pub harmful_mutations: u64,
}

pub struct MutationEngine {
    pub default_mutation_type: MutationType,
    pub mutation_stats: MutationStats,
}

impl MutationEngine {
    pub fn new(mutation_type: MutationType) -> Self {
        Self {
            default_mutation_type: mutation_type,
            mutation_stats: MutationStats::default(),
        }
    }

    /// Ana Mutasyon Girişi: Belirlenen türe göre işlemi gerçekleştirir.
    pub fn mutate(&mut self, genome: &mut StrategyGenome, rate: f64, strength: f64) {
        match self.default_mutation_type {
            MutationType::Adaptive => self.mutate_adaptive(genome, rate, strength),
            MutationType::Random => self.mutate_random(genome, rate, strength),
            MutationType::Gaussian { mean, std_dev } => self.mutate_gaussian(genome, rate, mean, std_dev),
            MutationType::Directed { direction } => self.mutate_directed(genome, rate, direction),
            MutationType::Hybrid => {
                // Hibrit mod: %70 Adaptif, %30 Directed
                if rand::thread_rng().gen_bool(0.7) {
                    self.mutate_adaptive(genome, rate, strength);
                } else {
                    self.mutate_directed(genome, rate, 1.0);
                }
            }
        }
    }

    pub fn mutate_adaptive(&mut self, genome: &mut StrategyGenome, base_rate: f64, base_strength: f64) {
        let fitness = genome.fitness;
        let factor = match fitness {
            f if f < 50.0  => 2.0,
            f if f < 100.0 => 1.0,
            _              => 0.5,
        };

        let adj_rate = base_rate * factor;
        let adj_strength = (base_strength * factor).max(0.01);
        
        let mut rng = rand::thread_rng();
        let normal = Normal::new(0.0, adj_strength).unwrap_or_else(|_| Normal::new(0.0, 0.01).unwrap());

        for (key, value) in genome.genes.iter_mut() {
            if rng.gen::<f64>() < adj_rate {
                let delta = normal.sample(&mut rng);
                let old_val = *value;
                *value += delta;
                self.apply_constraints(key, value);
                genome.mutation_history.push(format!("{}:{:.2}->{:.2}", key, old_val, *value));
                self.mutation_stats.total_mutations += 1;
            }
        }
    }

    fn mutate_random(&mut self, genome: &mut StrategyGenome, rate: f64, strength: f64) {
        let mut rng = rand::thread_rng();
        for (key, value) in genome.genes.iter_mut() {
            if rng.gen::<f64>() < rate {
                let delta = rng.gen_range(-strength..strength);
                *value += delta;
                self.apply_constraints(key, value);
                self.mutation_stats.total_mutations += 1;
            }
        }
    }

    fn mutate_gaussian(&mut self, genome: &mut StrategyGenome, rate: f64, mean: f64, std_dev: f64) {
        let mut rng = rand::thread_rng();
        let normal = Normal::new(mean, std_dev).unwrap_or_else(|_| Normal::new(0.0, 0.01).unwrap());
        for (key, value) in genome.genes.iter_mut() {
            if rng.gen::<f64>() < rate {
                *value += normal.sample(&mut rng);
                self.apply_constraints(key, value);
                self.mutation_stats.total_mutations += 1;
            }
        }
    }

    pub fn mutate_directed(&mut self, genome: &mut StrategyGenome, rate: f64, direction: f64) {
        let mut rng = rand::thread_rng();
        for (key, value) in genome.genes.iter_mut() {
            if rng.gen::<f64>() < rate {
                let delta = direction * rng.gen_range(0.01..0.1) * value.abs();
                *value += delta;
                self.apply_constraints(key, value);
                self.mutation_stats.total_mutations += 1;
            }
        }
    }

    fn apply_constraints(&self, key: &str, value: &mut f64) {
        match key {
            "fast_period" | "slow_period" | "period" => *value = value.clamp(2.0, 200.0).round(),
            "overbought" => *value = value.clamp(60.0, 90.0),
            "oversold"   => *value = value.clamp(10.0, 40.0),
            "stop_loss_pct" | "take_profit_pct" => *value = value.clamp(0.1, 20.0),
            "signal_threshold" => *value = value.clamp(0.0001, 0.1),
            _ => *value = value.max(0.0),
        }
    }

    pub fn evaluate_impact(&mut self, old_f: f64, new_f: f64) {
        match new_f - old_f {
            d if d > 1.0  => self.mutation_stats.beneficial_mutations += 1,
            d if d < -1.0 => self.mutation_stats.harmful_mutations += 1,
            _             => self.mutation_stats.neutral_mutations += 1,
        }
    }

    pub fn get_summary(&self) -> String {
        let total = self.mutation_stats.total_mutations as f64;
        if total == 0.0 { return "Mutasyon verisi yok".to_owned(); }
        format!(
            "Mutasyon Özeti | Toplam: {} | Faydalı: {:.1}% | Zararlı: {:.1}%",
            total, (self.mutation_stats.beneficial_mutations as f64 / total) * 100.0, (self.mutation_stats.harmful_mutations as f64 / total) * 100.0
        )
    }
}

impl Default for MutationEngine { fn default() -> Self { Self::new(MutationType::Adaptive) } }
