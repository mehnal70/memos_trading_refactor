// backtest_manager.rs
// Geriye Dönük Test ve Simülasyon Modülü - Optimize Edilmiş Versiyon

use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub test_id: String,
    pub strategy: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub pnl: f64,
    pub trades: usize,
    pub description: String,
}

pub trait BacktestManager {
    fn run_backtest(&mut self, strategy: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> BacktestResult;
    fn record_result(&mut self, result: BacktestResult);
    // Pipeline uyumu için referans listesi dönen metod
    fn all_results(&self) -> Vec<&BacktestResult>;
    // ID bazlı hızlı erişim metodu
    fn get_result_by_id(&self, test_id: &str) -> Option<&BacktestResult>;
}

pub struct SimpleBacktestManager {
    // Arama performansı için Vec yerine HashMap (O(1) erişim)
    pub results: HashMap<String, BacktestResult>,
}

impl SimpleBacktestManager {
    pub fn new() -> Self {
        Self {
            results: HashMap::with_capacity(100), // Re-allocation maliyetini önlemek için
        }
    }
}

impl BacktestManager for SimpleBacktestManager {
    fn run_backtest(&mut self, strategy: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> BacktestResult {
        // Modern Rust: String formatlama optimizasyonu
        let test_id = format!("{}-{}", strategy, Utc::now().timestamp());
        
        let result = BacktestResult {
            test_id: test_id.clone(),
            strategy: strategy.to_owned(),
            start_time: start,
            end_time: end,
            pnl: 0.0,
            trades: 0,
            description: "Simülasyon sonucu".to_owned(),
        };

        // Kendi metodumuz üzerinden kaydet (kod tekrarı önlendi)
        self.record_result(result.clone());
        result
    }

    fn record_result(&mut self, result: BacktestResult) {
        // ID çakışması durumunda üzerine yazar (veya log atılabilir)
        self.results.insert(result.test_id.clone(), result);
    }

    /// Pipeline uyumu: HashMap içindeki değerleri referans olarak Vec içinde toplar.
    /// Bu yöntem, veriyi kopyalamadan (cloning) listelemeyi sağlar.
    fn all_results(&self) -> Vec<&BacktestResult> {
        self.results.values().collect()
    }

    /// ID ile sonuca anında erişim (HashMap avantajı)
    fn get_result_by_id(&self, test_id: &str) -> Option<&BacktestResult> {
        self.results.get(test_id)
    }
}
