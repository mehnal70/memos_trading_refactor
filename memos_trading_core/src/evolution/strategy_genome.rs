// Strategy Genome - Strateji DNA'sı
// Her strateji bir organizma, parametreler onun genetik kodu

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Bir stratejinin genetik kodunu temsil eder
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyGenome {
    /// Benzersiz ID (nesil.birey formatında, örn: "G5-I42")
    pub id: String,
    
    /// Hangi nesilden geldiği
    pub generation: u32,
    
    /// Strateji türü (MA, RSI, MACD, vb.)
    pub strategy_type: String,
    
    /// Genetik parametreler (örn: {"fast_period": 10, "slow_period": 30})
    pub genes: HashMap<String, f64>,
    
    /// Fitness skoru (ne kadar başarılı)
    pub fitness: f64,
    
    /// Kaç trade yaptı
    pub trade_count: usize,
    
    /// Toplam kar/zarar (%)
    pub total_pnl_pct: f64,
    
    /// Win rate (%)
    pub win_rate: f64,
    
    /// Sharpe ratio (risk-adjusted return)
    pub sharpe_ratio: f64,
    
    /// Max drawdown (%)
    pub max_drawdown_pct: f64,
    
    /// Yaşayan trade sayısı (kaç döngü hayatta kaldı)
    pub survival_cycles: u32,
    
    /// Anne ve baba ID'leri (crossover sonucu oluşmuşsa)
    pub parents: Option<(String, String)>,
    
    /// Mutasyon geçmişi
    pub mutation_history: Vec<String>,
}

impl StrategyGenome {
    /// Yeni bir strateji genomu oluştur (ilk nesil için random)
    pub fn new_random(generation: u32, individual_id: u32, strategy_type: String) -> Self {
        let id = format!("G{}-I{}", generation, individual_id);
        
        // Strateji tipine göre varsayılan genler
        let genes = match strategy_type.as_str() {
            "MA" => {
                let mut g = HashMap::new();
                g.insert("fast_period".to_string(), rand_range(5.0, 20.0));
                g.insert("slow_period".to_string(), rand_range(20.0, 50.0));
                g.insert("signal_threshold".to_string(), rand_range(0.001, 0.01));
                g
            }
            "RSI" => {
                let mut g = HashMap::new();
                g.insert("period".to_string(), rand_range(7.0, 21.0));
                g.insert("overbought".to_string(), rand_range(65.0, 80.0));
                g.insert("oversold".to_string(), rand_range(20.0, 35.0));
                g
            }
            _ => HashMap::new(),
        };
        
        Self {
            id,
            generation,
            strategy_type,
            genes,
            fitness: 0.0,
            trade_count: 0,
            total_pnl_pct: 0.0,
            win_rate: 0.0,
            sharpe_ratio: 0.0,
            max_drawdown_pct: 0.0,
            survival_cycles: 0,
            parents: None,
            mutation_history: Vec::new(),
        }
    }
    
    /// İki genomu çaprazlayarak yeni genom oluştur (crossover)
    pub fn crossover(parent1: &Self, parent2: &Self, generation: u32, individual_id: u32) -> Self {
        let id = format!("G{}-I{}", generation, individual_id);
        
        // Anne ve babadan rastgele gen al
        let mut genes = HashMap::new();
        for (key, _) in &parent1.genes {
            if rand_bool() {
                genes.insert(key.clone(), *parent1.genes.get(key).unwrap());
            } else if let Some(val) = parent2.genes.get(key) {
                genes.insert(key.clone(), *val);
            }
        }
        
        Self {
            id,
            generation,
            strategy_type: parent1.strategy_type.clone(),
            genes,
            fitness: 0.0,
            trade_count: 0,
            total_pnl_pct: 0.0,
            win_rate: 0.0,
            sharpe_ratio: 0.0,
            max_drawdown_pct: 0.0,
            survival_cycles: 0,
            parents: Some((parent1.id.clone(), parent2.id.clone())),
            mutation_history: Vec::new(),
        }
    }
    
    /// Fitness hesapla (multi-objective: kar + risk + tutarlılık)
    pub fn calculate_fitness(&mut self) {
        // Eğer hiç trade yoksa fitness = 0
        if self.trade_count == 0 {
            self.fitness = 0.0;
            return;
        }
        
        // Kar bileşeni (normalize edilmiş, max +100)
        let profit_component = (self.total_pnl_pct * 10.0).min(100.0).max(-100.0);
        
        // Risk bileşeni (düşük drawdown = iyi)
        let risk_component = (20.0 - self.max_drawdown_pct).max(0.0);
        
        // Tutarlılık bileşeni (win rate)
        let consistency_component = self.win_rate * 0.5;
        
        // Sharpe ratio bileşeni
        let sharpe_component = self.sharpe_ratio * 20.0;
        
        // Hayatta kalma bonusu (uzun süre hayatta kalan stratejiler ödüllendirilir)
        let survival_bonus = (self.survival_cycles as f64 * 0.1).min(10.0);
        
        // Toplam fitness (ağırlıklı ortalama)
        self.fitness = (profit_component * 0.4)
            + (risk_component * 0.2)
            + (consistency_component * 0.2)
            + (sharpe_component * 0.1)
            + (survival_bonus * 0.1);
        
        // Minimum 0, maksimum 150
        self.fitness = self.fitness.max(0.0).min(150.0);
    }
    
    /// Genomu mutasyona uğrat
    pub fn mutate(&mut self, mutation_rate: f64, mutation_strength: f64) {
        for (key, value) in self.genes.iter_mut() {
            if rand_range(0.0, 1.0) < mutation_rate {
                // Mutasyon uygula: mevcut değeri +/- mutation_strength kadar değiştir
                let delta = rand_range(-mutation_strength, mutation_strength);
                *value += delta;
                
                // Parametreye özel sınırlar uygula
                match key.as_str() {
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
                    _ => {}
                }
                
                self.mutation_history.push(format!("{}:{:.4}", key, delta));
            }
        }
    }
    
    /// Genomu JSON string olarak export et
    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

// Yardımcı fonksiyonlar
fn rand_range(min: f64, max: f64) -> f64 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    rng.gen_range(min..=max)
}

fn rand_bool() -> bool {
    use rand::Rng;
    rand::thread_rng().gen_bool(0.5)
}

/// Genetik parametreler konfigürasyonu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneticParams {
    /// Popülasyon büyüklüğü (kaç strateji aynı anda yaşar)
    pub population_size: usize,
    
    /// Mutasyon oranı (0.0 - 1.0, örn: 0.1 = %10 genlerde mutasyon olur)
    pub mutation_rate: f64,
    
    /// Mutasyon gücü (parametrelerin ne kadar değişeceği)
    pub mutation_strength: f64,
    
    /// Seçim yöntemi: "tournament" | "roulette" | "elite"
    pub selection_method: String,
    
    /// Kaç nesil boyunca evrimleşir
    pub max_generations: u32,
    
    /// Elitizm: En iyi N strateji her nesilde direkt geçer
    pub elitism_count: usize,
    
    /// Crossover oranı (0.0 - 1.0, yeni neslin kaçı çaprazlamadan gelir)
    pub crossover_rate: f64,
}

impl Default for GeneticParams {
    fn default() -> Self {
        Self {
            population_size: 20,
            mutation_rate: 0.15,
            mutation_strength: 0.2,
            selection_method: "tournament".to_string(),
            max_generations: 50,
            elitism_count: 2,
            crossover_rate: 0.7,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_genome_creation() {
        let genome = StrategyGenome::new_random(1, 1, "MA".to_string());
        assert_eq!(genome.id, "G1-I1");
        assert_eq!(genome.generation, 1);
        assert!(genome.genes.contains_key("fast_period"));
    }
    
    #[test]
    fn test_fitness_calculation() {
        let mut genome = StrategyGenome::new_random(1, 1, "MA".to_string());
        genome.trade_count = 100;
        genome.total_pnl_pct = 15.0;
        genome.win_rate = 60.0;
        genome.max_drawdown_pct = 8.0;
        genome.sharpe_ratio = 1.5;
        genome.survival_cycles = 50;
        
        genome.calculate_fitness();
        
        assert!(genome.fitness > 0.0);
        assert!(genome.fitness <= 150.0);
    }
}
