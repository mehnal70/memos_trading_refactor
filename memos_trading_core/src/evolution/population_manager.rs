// Population Manager - Popülasyon Yöneticisi
// "Doğal seleksiyon" - En iyiler hayatta kalır, zayıflar elenir

use crate::evolution::{StrategyGenome, GeneticParams};
use serde::{Deserialize, Serialize};

/// Popülasyon yöneticisi - strateji nesillerin evrimleşmesini kontrol eder
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PopulationManager {
    /// Mevcut popülasyon (yaşayan stratejiler)
    pub current_population: Vec<StrategyGenome>,
    
    /// Şu anki nesil numarası
    pub current_generation: u32,
    
    /// Şimdiye kadar oluşturulan toplam birey sayısı
    pub total_individuals_created: u32,
    
    /// Genetik parametreler
    pub params: GeneticParams,
    
    /// Hall of Fame: Tüm zamanların en iyi stratejileri
    pub hall_of_fame: Vec<StrategyGenome>,
    
    /// Nesil geçmişi (her neslin ortalama fitness'ı)
    pub generation_history: Vec<GenerationStats>,
}

/// Bir neslin istatistikleri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationStats {
    pub generation: u32,
    pub avg_fitness: f64,
    pub max_fitness: f64,
    pub min_fitness: f64,
    pub best_genome_id: String,
    pub diversity_score: f64, // Popülasyondaki çeşitlilik
}

/// Seçim stratejisi
#[derive(Debug, Clone)]
pub enum SelectionStrategy {
    /// Turnuva seçimi: Rastgele grup al, en iyisini seç
    Tournament { size: usize },
    
    /// Rulet seçimi: Fitness'a göre olasılıklı seçim
    Roulette,
    
    /// Elit seçim: Sadece en iyiler
    Elite { top_n: usize },
    
    /// Rank-based: Sıralamaya göre seçim
    RankBased,
}

impl PopulationManager {
    /// Yeni bir popülasyon yöneticisi oluştur (ilk nesil random)
    pub fn new(strategy_type: String, params: GeneticParams) -> Self {
        let mut population = Vec::new();
        
        // İlk nesli random oluştur
        for i in 0..params.population_size {
            let genome = StrategyGenome::new_random(0, i as u32, strategy_type.clone());
            population.push(genome);
        }
        
        Self {
            current_population: population,
            current_generation: 0,
            total_individuals_created: params.population_size as u32,
            params,
            hall_of_fame: Vec::new(),
            generation_history: Vec::new(),
        }
    }
    
    /// Mevcut popülasyonun fitness'larını güncelle
    pub fn update_population_fitness(&mut self) {
        for genome in &mut self.current_population {
            genome.calculate_fitness();
        }
        
        // Fitness'a göre sırala (azalan)
        self.current_population.sort_by(|a, b| {
            b.fitness.partial_cmp(&a.fitness).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    
    /// Yeni nesil oluştur (evrim adımı)
    pub fn evolve_next_generation(&mut self) {
        // Mevcut neslin istatistiklerini kaydet
        self.record_generation_stats();
        
        // En iyileri Hall of Fame'e ekle
        self.update_hall_of_fame();
        
        // Yeni nesil oluştur
        let mut next_generation = Vec::new();
        
        // 1. Elitizm: En iyi N stratejiyi direkt geçir
        for i in 0..self.params.elitism_count.min(self.current_population.len()) {
            let mut elite = self.current_population[i].clone();
            elite.generation = self.current_generation + 1;
            elite.survival_cycles += 1;
            next_generation.push(elite);
        }
        
        // 2. Kalan yerleri crossover + mutasyon ile doldur
        let selection_strategy = match self.params.selection_method.as_str() {
            "tournament" => SelectionStrategy::Tournament { size: 3 },
            "roulette" => SelectionStrategy::Roulette,
            "elite" => SelectionStrategy::Elite { top_n: 5 },
            _ => SelectionStrategy::Tournament { size: 3 },
        };
        
        while next_generation.len() < self.params.population_size {
            // Crossover yapılacak mı?
            if rand_range(0.0, 1.0) < self.params.crossover_rate {
                // İki ebeveyn seç
                let parent1 = self.select_parent(&selection_strategy);
                let parent2 = self.select_parent(&selection_strategy);
                
                // Crossover yap
                let mut child = StrategyGenome::crossover(
                    parent1,
                    parent2,
                    self.current_generation + 1,
                    self.total_individuals_created,
                );
                
                // Mutasyon uygula
                child.mutate(self.params.mutation_rate, self.params.mutation_strength);
                
                next_generation.push(child);
                self.total_individuals_created += 1;
            } else {
                // Sadece mutasyon (klonlama + mutasyon)
                let parent = self.select_parent(&selection_strategy);
                let mut child = parent.clone();
                child.id = format!("G{}-I{}", self.current_generation + 1, self.total_individuals_created);
                child.generation = self.current_generation + 1;
                child.mutate(self.params.mutation_rate, self.params.mutation_strength);
                
                next_generation.push(child);
                self.total_individuals_created += 1;
            }
        }
        
        // Yeni nesli mevcut popülasyon yap
        self.current_population = next_generation;
        self.current_generation += 1;
        
    }
    
    /// Ebeveyn seç (selection strategy'ye göre)
    fn select_parent(&self, strategy: &SelectionStrategy) -> &StrategyGenome {
        match strategy {
            SelectionStrategy::Tournament { size } => {
                // Rastgele 'size' kadar birey al, en iyisini seç
                let mut best: Option<&StrategyGenome> = None;
                for _ in 0..*size {
                    let idx = rand_range(0.0, self.current_population.len() as f64) as usize;
                    let candidate = &self.current_population[idx];
                    if best.is_none() || candidate.fitness > best.unwrap().fitness {
                        best = Some(candidate);
                    }
                }
                best.unwrap()
            }
            SelectionStrategy::Roulette => {
                // Fitness'a göre olasılıklı seçim
                let total_fitness: f64 = self.current_population.iter().map(|g| g.fitness.max(0.0)).sum();
                if total_fitness == 0.0 {
                    return &self.current_population[0];
                }
                
                let mut rand_val = rand_range(0.0, total_fitness);
                for genome in &self.current_population {
                    rand_val -= genome.fitness.max(0.0);
                    if rand_val <= 0.0 {
                        return genome;
                    }
                }
                &self.current_population[0]
            }
            SelectionStrategy::Elite { top_n } => {
                // En iyi N'den birini rastgele seç
                let idx = rand_range(0.0, (*top_n).min(self.current_population.len()) as f64) as usize;
                &self.current_population[idx]
            }
            SelectionStrategy::RankBased => {
                // Rank-based selection (düşük rank = yüksek seçilme olasılığı)
                let total_ranks: usize = (self.current_population.len() * (self.current_population.len() + 1)) / 2;
                let mut rand_val = rand_range(0.0, total_ranks as f64) as usize;
                
                for (rank, genome) in self.current_population.iter().enumerate() {
                    let rank_value = self.current_population.len() - rank;
                    if rand_val <= rank_value {
                        return genome;
                    }
                    rand_val -= rank_value;
                }
                &self.current_population[0]
            }
        }
    }
    
    /// Nesil istatistiklerini kaydet
    fn record_generation_stats(&mut self) {
        if self.current_population.is_empty() {
            return;
        }
        
        let fitnesses: Vec<f64> = self.current_population.iter().map(|g| g.fitness).collect();
        let avg_fitness = fitnesses.iter().sum::<f64>() / fitnesses.len() as f64;
        let max_fitness = fitnesses.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min_fitness = fitnesses.iter().cloned().fold(f64::INFINITY, f64::min);
        
        let best_genome_id = self.current_population[0].id.clone();
        
        // Çeşitlilik skoru hesapla (gen varyanslarının ortalaması)
        let diversity_score = self.calculate_diversity();
        
        let stats = GenerationStats {
            generation: self.current_generation,
            avg_fitness,
            max_fitness,
            min_fitness,
            best_genome_id,
            diversity_score,
        };
        
        self.generation_history.push(stats);
    }
    
    /// Popülasyondaki gen çeşitliliğini hesapla
    fn calculate_diversity(&self) -> f64 {
        if self.current_population.is_empty() {
            return 0.0;
        }
        
        // Her gen için varyans hesapla, ortalamasını al
        let first_genome = &self.current_population[0];
        let mut total_variance = 0.0;
        let mut gene_count = 0;
        
        for (gene_name, _) in &first_genome.genes {
            let values: Vec<f64> = self.current_population
                .iter()
                .filter_map(|g| g.genes.get(gene_name).copied())
                .collect();
            
            if values.len() < 2 {
                continue;
            }
            
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
            
            total_variance += variance;
            gene_count += 1;
        }
        
        if gene_count > 0 {
            total_variance / gene_count as f64
        } else {
            0.0
        }
    }
    
    /// Hall of Fame'i güncelle (tüm zamanların en iyileri)
    fn update_hall_of_fame(&mut self) {
        // En iyi 3'ü al
        for i in 0..3.min(self.current_population.len()) {
            let genome = self.current_population[i].clone();
            
            // Zaten Hall of Fame'de mi?
            if !self.hall_of_fame.iter().any(|g| g.id == genome.id) {
                self.hall_of_fame.push(genome);
            }
        }
        
        // Hall of Fame'i fitness'a göre sırala
        self.hall_of_fame.sort_by(|a, b| {
            b.fitness.partial_cmp(&a.fitness).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        // En iyi 10'u tut
        self.hall_of_fame.truncate(10);
    }
    
    /// En iyi stratejiyi al
    pub fn get_best_strategy(&self) -> Option<&StrategyGenome> {
        self.current_population.first()
    }
    
    /// Hall of Fame'deki en iyiyi al
    pub fn get_hall_of_fame_best(&self) -> Option<&StrategyGenome> {
        self.hall_of_fame.first()
    }
    
    /// Popülasyon özeti
    pub fn get_summary(&self) -> String {
        format!(
            "Nesil: {}, Popülasyon: {}, En İyi Fitness: {:.2}, Ortalama: {:.2}, Hall of Fame: {}",
            self.current_generation,
            self.current_population.len(),
            self.current_population.first().map(|g| g.fitness).unwrap_or(0.0),
            self.current_population.iter().map(|g| g.fitness).sum::<f64>() / self.current_population.len() as f64,
            self.hall_of_fame.len()
        )
    }
}

// Yardımcı fonksiyon
fn rand_range(min: f64, max: f64) -> f64 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    rng.gen_range(min..=max)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_population_creation() {
        let params = GeneticParams::default();
        let manager = PopulationManager::new("MA".to_string(), params);
        
        assert_eq!(manager.current_population.len(), 20);
        assert_eq!(manager.current_generation, 0);
    }
    
    #[test]
    fn test_evolution() {
        let params = GeneticParams::default();
        let mut manager = PopulationManager::new("MA".to_string(), params);
        
        // İlk nesle fitness ver
        for genome in &mut manager.current_population {
            genome.trade_count = 50;
            genome.total_pnl_pct = rand_range(-10.0, 20.0);
            genome.win_rate = rand_range(40.0, 70.0);
        }
        
        manager.update_population_fitness();
        let _initial_best = manager.get_best_strategy().unwrap().fitness;
        
        // 5 nesil evrimleştir
        for _ in 0..5 {
            manager.evolve_next_generation();
        }
        
        assert_eq!(manager.current_generation, 5);
        assert!(manager.hall_of_fame.len() > 0);
    }
}
