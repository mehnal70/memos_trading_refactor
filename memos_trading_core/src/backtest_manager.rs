// backtest_manager.rs
// Geriye Dönük Test ve Simülasyon Modülü
// Strateji backtest, simülasyon, performans karşılaştırma

use chrono::{DateTime, Utc};

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
    fn all_results(&self) -> &Vec<BacktestResult>;
}

pub struct SimpleBacktestManager {
    pub results: Vec<BacktestResult>,
}

impl BacktestManager for SimpleBacktestManager {
    fn run_backtest(&mut self, strategy: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> BacktestResult {
        // Gerçek ortamda burada simülasyon ve strateji çağrısı yapılır
        let result = BacktestResult {
            test_id: format!("{}-{}", strategy, Utc::now().timestamp()),
            strategy: strategy.to_string(),
            start_time: start,
            end_time: end,
            pnl: 0.0,
            trades: 0,
            description: "Simülasyon sonucu (örnek)".to_string(),
        };
        self.results.push(result.clone());
        result
    }
    fn record_result(&mut self, result: BacktestResult) {
        self.results.push(result);
    }
    fn all_results(&self) -> &Vec<BacktestResult> {
        &self.results
    }
}
