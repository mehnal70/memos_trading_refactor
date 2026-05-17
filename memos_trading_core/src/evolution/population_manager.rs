// population_manager.rs - Otonom Popülasyon ve Doğal Seleksiyon Motoru
// evolution/population_manager.rs - Otonom Strateji Nesilleri ve Evrim Motoru

use crate::evolution::{GeneticParams, StrategyGenome, strategy_genome::SelectionMethod};
use serde::{Deserialize, Serialize};
use rand::Rng;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationStats {
    pub generation: u32,
    pub avg_fitness: f64,
    pub max_fitness: f64,
    pub min_fitness: f64,
    pub best_genome_id: String,
    pub diversity_score: f64,
}

#[derive(Debug, Clone, Copy)]
pub enum SelectionStrategy {
    Tournament { size: usize },
    Roulette,
    Elite { top_n: usize },
    RankBased,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopulationManager {
    pub current_population: Vec<StrategyGenome>,
    pub current_generation: u32,
    pub total_individuals_created: u32,
    pub params: GeneticParams,
    pub hall_of_fame: Vec<StrategyGenome>,
    pub generation_history: Vec<GenerationStats>,
}

impl PopulationManager {
    pub fn new(strategy_type: String, params: GeneticParams) -> Self {
        let population = (0..params.population_size)
            .map(|i| StrategyGenome::new_random(0, i as u32, strategy_type.clone()))
            .collect();
        
        Self {
            current_population: population,
            current_generation: 0,
            total_individuals_created: params.population_size as u32,
            params,
            hall_of_fame: Vec::with_capacity(10),
            generation_history: Vec::with_capacity(100),
        }
    }

    // --- CONTROLLER KÖPRÜLERİ (Hata Giderici Metodlar) ---

    /// AutonomousController'ın beklediği ana evrim metodu.
    pub fn evolve(&mut self) {
        self.update_population_fitness();
        self.evolve_next_generation();
    }

    /// AutonomousController'ın beklediği en iyi genoma erişim metodu.
    pub fn get_best_strategy(&self) -> Option<&StrategyGenome> {
        self.current_population.first()
    }

    /// Otonom kontrol panelleri için kısa özet (gen no, popülasyon, en iyi fitness).
    pub fn get_summary(&self) -> String {
        let best_fit = self.current_population.first().map(|g| g.fitness).unwrap_or(0.0);
        format!("gen={} pop={} best_fit={:.3}",
                self.current_generation, self.current_population.len(), best_fit)
    }

    // --- EVRİMSEL ÇEKİRDEK ---

    pub fn update_population_fitness(&mut self) {
        for genome in &mut self.current_population {
            genome.calculate_fitness();
        }
        self.current_population.sort_by(|a, b| {
            b.fitness.partial_cmp(&a.fitness).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    pub fn evolve_next_generation(&mut self) {
        self.record_generation_stats();
        self.update_hall_of_fame();
        
        let mut next_gen = Vec::with_capacity(self.params.population_size);
        let next_gen_num = self.current_generation + 1;

        // 1. Elitizm
        let elite_count = self.params.elitism_count.min(self.current_population.len());
        for i in 0..elite_count {
            let mut elite = self.current_population[i].clone();
            elite.generation = next_gen_num;
            elite.survival_cycles += 1;
            next_gen.push(elite);
        }

        // 2. GENETİK OPERATÖRLER
        // Enum üzerinden doğrudan eşleştirme (Pattern Matching)
        let selection = match self.params.selection_method {
            SelectionMethod::Roulette  => SelectionStrategy::Roulette,
            SelectionMethod::Elite    => SelectionStrategy::Elite { top_n: 5 },
            SelectionMethod::Tournament => SelectionStrategy::Tournament { size: 3 },
        };

        // 2. Seçim ve Çaprazlama
/*         let selection = match self.params.selection_method.as_str() {
            "roulette" => SelectionStrategy::Roulette,
            "elite"    => SelectionStrategy::Elite { top_n: 5 },
            _          => SelectionStrategy::Tournament { size: 3 },
        };
 */
        let mut rng = rand::thread_rng();
        while next_gen.len() < self.params.population_size {
            if rng.gen::<f64>() < self.params.crossover_rate {
                let p1 = self.select_parent(&selection);
                let p2 = self.select_parent(&selection);
                let mut child = StrategyGenome::crossover(p1, p2, next_gen_num, self.total_individuals_created);
                child.mutate(self.params.mutation_rate, self.params.mutation_strength);
                next_gen.push(child);
            } else {
                let parent = self.select_parent(&selection);
                let mut child = parent.clone();
                child.id = format!("G{}-I{}", next_gen_num, self.total_individuals_created);
                child.generation = next_gen_num;
                child.mutate(self.params.mutation_rate, self.params.mutation_strength);
                next_gen.push(child);
            }
            self.total_individuals_created += 1;
        }

        self.current_population = next_gen;
        self.current_generation = next_gen_num;
        self.update_population_fitness(); // Yeni nesli sırala
    }

    fn select_parent(&self, strategy: &SelectionStrategy) -> &StrategyGenome {
        let mut rng = rand::thread_rng();
        let pop_len = self.current_population.len();
        if pop_len == 0 { panic!("Boş popülasyondan seçim yapılamaz!"); }

        match strategy {
            SelectionStrategy::Tournament { size } => {
                let mut best: Option<&StrategyGenome> = None;
                for _ in 0..*size {
                    let candidate = &self.current_population[rng.gen_range(0..pop_len)];
                    if best.map_or(true, |b| candidate.fitness > b.fitness) { best = Some(candidate); }
                }
                best.unwrap_or(&self.current_population[0])
            }
            SelectionStrategy::Roulette => {
                let total_f: f64 = self.current_population.iter().map(|g| g.fitness.max(0.0)).sum();
                if total_f <= 0.0 { return &self.current_population[rng.gen_range(0..pop_len)]; }
                let mut pick = rng.gen_range(0.0..total_f);
                for g in &self.current_population {
                    pick -= g.fitness.max(0.0);
                    if pick <= 0.0 { return g; }
                }
                &self.current_population[0]
            }
            SelectionStrategy::Elite { top_n } => &self.current_population[rng.gen_range(0..(*top_n).min(pop_len))],
            SelectionStrategy::RankBased => {
                let total_ranks = (pop_len * (pop_len + 1)) / 2;
                let mut pick = rng.gen_range(0..total_ranks);
                for (rank, g) in self.current_population.iter().enumerate() {
                    let weight = pop_len - rank;
                    if pick < weight { return g; }
                    pick -= weight;
                }
                &self.current_population[0]
            }
        }
    }

    fn update_hall_of_fame(&mut self) {
        let top_candidates: Vec<_> = self.current_population.iter().take(3).cloned().collect();
        for cand in top_candidates {
            if !self.hall_of_fame.iter().any(|h| h.id == cand.id) { self.hall_of_fame.push(cand); }
        }
        self.hall_of_fame.sort_by(|a, b| b.fitness.partial_cmp(&a.fitness).unwrap_or(std::cmp::Ordering::Equal));
        self.hall_of_fame.truncate(10);
    }

    fn record_generation_stats(&mut self) {
        if let Some(best) = self.current_population.first() {
            let n = self.current_population.len() as f64;
            let avg = self.current_population.iter().map(|g| g.fitness).sum::<f64>() / n;
            self.generation_history.push(GenerationStats {
                generation: self.current_generation, avg_fitness: avg,
                max_fitness: best.fitness, min_fitness: self.current_population.last().map_or(0.0, |g| g.fitness),
                best_genome_id: best.id.clone(), diversity_score: 0.0,
            });
        }
    }
}
