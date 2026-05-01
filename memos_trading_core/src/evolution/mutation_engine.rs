// Mutation Engine - Akıllı Mutasyon Motoru
// "Adaptif mutasyon" - Başarısız olanlar daha çok mutasyona uğrar

use crate::evolution::StrategyGenome;
use serde::{Deserialize, Serialize};

/// Mutasyon türü
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MutationType {
    /// Rastgele mutasyon (klasik)
    Random,
    
    /// Adaptif mutasyon (fitness'a göre mutasyon gücü değişir)
    Adaptive,
    
    /// Gaussian mutasyon (normal dağılım)
    Gaussian { mean: f64, std_dev: f64 },
    
    /// Directed mutasyon (trend yönünde mutasyon)
    Directed { direction: f64 },
    
    /// Hybrid (birden fazla yöntemi karıştır)
    Hybrid,
}

/// Mutation Engine - Strateji parametrelerini akıllıca mutasyona uğratır
pub struct MutationEngine {
    /// Varsayılan mutasyon tipi
    pub default_mutation_type: MutationType,
    
    /// Mutasyon istatistikleri
    pub mutation_stats: MutationStats,
}

#[derive(Debug, Clone, Default)]
pub struct MutationStats {
    pub total_mutations: u64,
    pub beneficial_mutations: u64,
    pub neutral_mutations: u64,
    pub harmful_mutations: u64,
}

impl MutationEngine {
    pub fn new(mutation_type: MutationType) -> Self {
        Self {
            default_mutation_type: mutation_type,
            mutation_stats: MutationStats::default(),
        }
    }
    
    /// Stratejiyi mutasyona uğrat (adaptive, fitness'a göre mutasyon gücü ayarlanır)
    pub fn mutate_adaptive(
        &mut self,
        genome: &mut StrategyGenome,
        base_mutation_rate: f64,
        base_mutation_strength: f64,
    ) {
        // Düşük fitness = daha agresif mutasyon (daha hızlı değişim)
        // Yüksek fitness = daha az mutasyon (en iyi halleri koru)
        let fitness_factor = if genome.fitness < 50.0 {
            2.0 // Düşük fitness: 2x daha fazla mutasyon
        } else if genome.fitness < 100.0 {
            1.0 // Orta fitness: normal mutasyon
        } else {
            0.5 // Yüksek fitness: 0.5x daha az mutasyon
        };
        
        let adjusted_rate = base_mutation_rate * fitness_factor;
        let adjusted_strength = base_mutation_strength * fitness_factor;
        
        for (key, value) in genome.genes.iter_mut() {
            if rand_range(0.0, 1.0) < adjusted_rate {
                // Gaussian mutasyon uygula
                let delta = gaussian_random(0.0, adjusted_strength);
                let old_value = *value;
                *value += delta;
                
                // Parametreye özel sınırlar
                self.apply_parameter_constraints(key, value);
                
                // Mutasyon kaydı
                genome.mutation_history.push(format!(
                    "{}:{:.4}->{:.4}",
                    key, old_value, *value
                ));
                
                self.mutation_stats.total_mutations += 1;
            }
        }
    }
    
    /// Directed mutasyon (başarılı trende doğru mutasyon)
    pub fn mutate_directed(
        &mut self,
        genome: &mut StrategyGenome,
        mutation_rate: f64,
        direction: f64, // +1.0 = artır, -1.0 = azalt
    ) {
        for (key, value) in genome.genes.iter_mut() {
            if rand_range(0.0, 1.0) < mutation_rate {
                // Direction'a göre mutasyon
                let delta = direction * rand_range(0.01, 0.1) * (*value).abs();
                *value += delta;
                
                self.apply_parameter_constraints(key, value);
                
                genome.mutation_history.push(format!(
                    "directed_{}:{:+.4}",
                    key, delta
                ));
                
                self.mutation_stats.total_mutations += 1;
            }
        }
    }
    
    /// Parametre kısıtlarını uygula
    fn apply_parameter_constraints(&self, key: &str, value: &mut f64) {
        match key {
            "fast_period" | "slow_period" | "period" => {
                *value = value.max(2.0).min(200.0).round();
            }
            "overbought" => {
                *value = value.max(60.0).min(90.0);
            }
            "oversold" => {
                *value = value.max(10.0).min(40.0);
            }
            "signal_threshold" => {
                *value = value.max(0.0001).min(0.1);
            }
            "stop_loss_pct" | "take_profit_pct" => {
                *value = value.max(0.1).min(20.0);
            }
            "position_size_pct" => {
                *value = value.max(0.1).min(10.0);
            }
            _ => {}
        }
    }
    
    /// Mutasyon etkisini değerlendir (mutasyon sonrası fitness karşılaştırması)
    pub fn evaluate_mutation_impact(&mut self, old_fitness: f64, new_fitness: f64) {
        if new_fitness > old_fitness + 1.0 {
            self.mutation_stats.beneficial_mutations += 1;
        } else if new_fitness < old_fitness - 1.0 {
            self.mutation_stats.harmful_mutations += 1;
        } else {
            self.mutation_stats.neutral_mutations += 1;
        }
    }
    
    /// Mutasyon istatistiklerini göster
    pub fn get_stats_summary(&self) -> String {
        let total = self.mutation_stats.total_mutations as f64;
        if total == 0.0 {
            return "Henüz mutasyon yok".to_string();
        }
        
        let beneficial_pct = (self.mutation_stats.beneficial_mutations as f64 / total) * 100.0;
        let harmful_pct = (self.mutation_stats.harmful_mutations as f64 / total) * 100.0;
        
        format!(
            "Toplam: {}, Faydalı: {:.1}%, Zararlı: {:.1}%",
            self.mutation_stats.total_mutations,
            beneficial_pct,
            harmful_pct
        )
    }
}

impl Default for MutationEngine {
    fn default() -> Self {
        Self::new(MutationType::Adaptive)
    }
}

// Yardımcı fonksiyonlar
fn rand_range(min: f64, max: f64) -> f64 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    rng.gen_range(min..=max)
}

fn gaussian_random(mean: f64, std_dev: f64) -> f64 {
    use rand_distr::{Distribution, Normal};
    let normal = Normal::new(mean, std_dev).unwrap();
    let mut rng = rand::thread_rng();
    normal.sample(&mut rng)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_adaptive_mutation() {
        let mut engine = MutationEngine::default();
        let mut genome = StrategyGenome::new_random(1, 1, "MA".to_string());
        
        // Düşük fitness: Agresif mutasyon beklenir
        genome.fitness = 20.0;
        let initial_genes = genome.genes.clone();
        
        engine.mutate_adaptive(&mut genome, 0.5, 0.2);
        
        // Genler değişmiş olmalı
        assert!(genome.genes != initial_genes);
        assert!(engine.mutation_stats.total_mutations > 0);
    }
    
    #[test]
    fn test_parameter_constraints() {
        let engine = MutationEngine::default();
        let mut value = 300.0; // Sınırın üstünde
        
        engine.apply_parameter_constraints("fast_period", &mut value);
        
        // Sınırlanmış olmalı
        assert!(value >= 2.0 && value <= 200.0);
    }
}
