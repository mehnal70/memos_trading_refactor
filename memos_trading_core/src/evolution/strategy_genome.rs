// evolution/strategy_genome.rs - Strateji DNA'sı ve Evrimsel Gelişim Modülü

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use rand::Rng;

// --- 1. GENETİK ALGORİTMA PARAMETRELERİ ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SelectionMethod {
    Tournament,
    Roulette,
    Elite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneticParams {
    pub population_size: usize,
    pub mutation_rate: f64,
    pub mutation_strength: f64,
    pub max_generations: u32,
    pub elitism_count: usize,
    pub crossover_rate: f64,
    pub selection_method: SelectionMethod,
}

impl Default for GeneticParams {
    fn default() -> Self {
        Self {
            population_size: 25,
            mutation_rate: 0.10,
            mutation_strength: 0.15,
            max_generations: 100,
            elitism_count: 3,
            crossover_rate: 0.75,
            selection_method: SelectionMethod::Tournament,
        }
    }
}

// --- 2. ANA STRATEJİ GENOMU ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyGenome {
    pub id: String,
    pub generation: u32,
    pub strategy_type: String,
    pub genes: HashMap<String, f64>,
    
    // Performans Metrikleri
    pub fitness: f64,
    pub trade_count: usize,
    pub total_pnl_pct: f64,
    pub win_rate: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown_pct: f64,
    pub survival_cycles: u32,
    
    pub parents: Option<(String, String)>,
    pub mutation_history: Vec<String>,
}

impl StrategyGenome {
    /// Otonom Başlangıç: Rastgele ama finansal sınırlara uyumlu ilk nesil üretimi.
    pub fn new_random(generation: u32, individual_id: u32, strategy_type: String) -> Self {
        let mut rng = rand::thread_rng();
        let mut genes = HashMap::with_capacity(4);

        match strategy_type.as_str() {
            "MA" => {
                genes.insert("fast_period".to_owned(), rng.gen_range(5.0..20.0));
                genes.insert("slow_period".to_owned(), rng.gen_range(20.0..50.0));
                genes.insert("signal_threshold".to_owned(), rng.gen_range(0.001..0.01));
            }
            "RSI" => {
                genes.insert("period".to_owned(), rng.gen_range(7.0..21.0));
                genes.insert("overbought".to_owned(), rng.gen_range(65.0..80.0));
                genes.insert("oversold".to_owned(), rng.gen_range(20.0..35.0));
            }
            _ => {}
        }

        Self {
            id: format!("G{}-I{}", generation, individual_id),
            generation, strategy_type, genes, fitness: 0.0, trade_count: 0,
            total_pnl_pct: 0.0, win_rate: 0.0, sharpe_ratio: 0.0,
            max_drawdown_pct: 0.0, survival_cycles: 0, parents: None,
            mutation_history: Vec::with_capacity(5),
        }
    }

    /// Uniform Crossover: İki organizmadan hibrit bir gen dizilimi oluşturur.
    pub fn crossover(p1: &Self, p2: &Self, gen: u32, ind_id: u32) -> Self {
        let mut rng = rand::thread_rng();
        let mut child_genes = HashMap::with_capacity(p1.genes.len());

        for key in p1.genes.keys() {
            let gene_source = if rng.gen_bool(0.5) { p1 } else { p2 };
            if let Some(&val) = gene_source.genes.get(key) {
                child_genes.insert(key.clone(), val);
            }
        }

        Self {
            id: format!("G{}-I{}", gen, ind_id),
            generation: gen, strategy_type: p1.strategy_type.clone(),
            genes: child_genes, fitness: 0.0, trade_count: 0, total_pnl_pct: 0.0,
            win_rate: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0, survival_cycles: 0,
            parents: Some((p1.id.clone(), p2.id.clone())), mutation_history: Vec::new(),
        }
    }

    /// Mutasyon: Stratejiyi yerel minimumdan otonom olarak kurtarır.
    pub fn mutate(&mut self, rate: f64, strength: f64) {
        let mut rng = rand::thread_rng();
        for (key, value) in self.genes.iter_mut() {
            if rng.gen::<f64>() < rate {
                let delta = rng.gen_range(-strength..strength);
                let old_val = *value;
                *value += delta;

                match key.as_str() {
                    "fast_period" | "slow_period" | "period" => { *value = value.clamp(2.0, 200.0).round(); }
                    "overbought" => *value = value.clamp(60.0, 90.0),
                    "oversold" => *value = value.clamp(10.0, 40.0),
                    "signal_threshold" => *value = value.clamp(0.0001, 0.1),
                    _ => {}
                }
                self.mutation_history.push(format!("{}:{:.2}->{:.2}", key, old_val, *value));
            }
        }
    }

    /// Multi-Objective Fitness: Kar, Risk ve Kıdemi tek bir puan haline getirir.
    pub fn calculate_fitness(&mut self) {
        if self.trade_count == 0 { self.fitness = 0.0; return; }

        let profit_comp = (self.total_pnl_pct * 10.0).clamp(-100.0, 100.0);
        let risk_comp = (20.0 - self.max_drawdown_pct).max(0.0);
        let consistency_comp = self.win_rate * 0.5;
        let sharpe_comp = (self.sharpe_ratio * 20.0).clamp(-20.0, 50.0);
        let survival_bonus = (self.survival_cycles as f64 * 0.2).min(15.0);

        let score = (profit_comp * 0.45) + (risk_comp * 0.20) + (consistency_comp * 0.15) 
                  + (sharpe_comp * 0.10) + (survival_bonus * 0.10);

        self.fitness = score.clamp(0.0, 150.0);
    }
}


