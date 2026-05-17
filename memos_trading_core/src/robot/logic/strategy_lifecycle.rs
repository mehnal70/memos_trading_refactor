// strategy_lifecycle.rs
// Otonom Strateji Yönetimi Modülü

use crate::core::types::StrategyParams;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Strateji performans kaydı
#[derive(Debug, Clone)]
pub struct StrategyPerformance {
    pub name: String,
    pub trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub last_updated: DateTime<Utc>,
}

/// Strateji yaşam döngüsü yöneticisi trait'i - Thread-safety desteği eklendi
pub trait StrategyLifecycleManager: Send + Sync {
    fn register_strategy(&mut self, name: String, params: StrategyParams);
    fn remove_strategy(&mut self, name: &str);
    fn update_performance(&mut self, name: &str, perf: StrategyPerformance);
    fn select_active_strategies(&self) -> Vec<String>;
}

pub struct SimpleStrategyLifecycleManager {
    // Performans: İsim bazlı anında erişim için HashMap
    pub strategies: HashMap<String, StrategyParams>,
    pub performances: HashMap<String, StrategyPerformance>,
    pub min_win_rate: f64,
}

impl SimpleStrategyLifecycleManager {
    pub fn new(min_win_rate: f64) -> Self {
        Self {
            strategies: HashMap::with_capacity(20),
            performances: HashMap::with_capacity(20),
            min_win_rate,
        }
    }
}

impl StrategyLifecycleManager for SimpleStrategyLifecycleManager {
    fn register_strategy(&mut self, name: String, params: StrategyParams) {
        // Entry API kullanarak veya doğrudan insert ile sahipliği alıyoruz
        self.strategies.insert(name, params);
    }

    fn remove_strategy(&mut self, name: &str) {
        // O(1) maliyetle silme işlemi
        self.strategies.remove(name);
        self.performances.remove(name);
    }

    fn update_performance(&mut self, name: &str, perf: StrategyPerformance) {
        // linear search yerine direkt anahtar güncelleme
        self.performances.insert(name.to_owned(), perf);
    }

    fn select_active_strategies(&self) -> Vec<String> {
        // İteratörlerle hızlı filtreleme ve isimleri toplama
        self.performances
            .values()
            .filter(|p| p.win_rate >= self.min_win_rate)
            .map(|p| p.name.clone())
            .collect()
    }
}
