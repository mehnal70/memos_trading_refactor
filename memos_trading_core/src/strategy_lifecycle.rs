// strategy_lifecycle.rs
// Otonom Strateji Yönetimi Modülü
// Strateji portföyü, başarı izleme ve otomatik ekleme/çıkarma

use crate::types::{StrategyParams};

/// Strateji performans kaydı
#[derive(Debug, Clone)]
pub struct StrategyPerformance {
    pub name: String,
    pub trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

/// Strateji yaşam döngüsü yöneticisi trait'i
pub trait StrategyLifecycleManager {
    fn register_strategy(&mut self, name: String, params: StrategyParams);
    fn remove_strategy(&mut self, name: &str);
    fn update_performance(&mut self, name: &str, perf: StrategyPerformance);
    fn select_active_strategies(&self) -> Vec<String>;
}

/// Basit örnek: Başarı oranı düşük olanı çıkar, yeni geleni ekle
pub struct SimpleStrategyLifecycleManager {
    pub strategies: Vec<(String, StrategyParams)>,
    pub performances: Vec<StrategyPerformance>,
    pub min_win_rate: f64,
}

impl StrategyLifecycleManager for SimpleStrategyLifecycleManager {
    fn register_strategy(&mut self, name: String, params: StrategyParams) {
        self.strategies.push((name, params));
    }
    fn remove_strategy(&mut self, name: &str) {
        self.strategies.retain(|(n, _)| n != name);
        self.performances.retain(|p| p.name != name);
    }
    fn update_performance(&mut self, name: &str, perf: StrategyPerformance) {
        if let Some(existing) = self.performances.iter_mut().find(|p| p.name == name) {
            *existing = perf;
        } else {
            self.performances.push(perf);
        }
    }
    fn select_active_strategies(&self) -> Vec<String> {
        self.performances
            .iter()
            .filter(|p| p.win_rate >= self.min_win_rate)
            .map(|p| p.name.clone())
            .collect()
    }
}
