// cost_manager.rs
// Maliyet ve Likidite Yönetimi Modülü
// İşlem maliyeti, likidite izleme, maliyet analizi

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct CostRecord {
    pub record_id: String,
    pub timestamp: DateTime<Utc>,
    pub cost: f64,
    pub liquidity: f64,
    pub description: String,
}

pub trait CostManager {
    fn record_cost(&mut self, record: CostRecord);
    fn total_cost(&self) -> f64;
    fn average_liquidity(&self) -> f64;
    fn all_records(&self) -> &Vec<CostRecord>;
}

pub struct SimpleCostManager {
    pub records: Vec<CostRecord>,
}

impl CostManager for SimpleCostManager {
    fn record_cost(&mut self, record: CostRecord) {
        self.records.push(record);
    }
    fn total_cost(&self) -> f64 {
        self.records.iter().map(|r| r.cost).sum()
    }
    fn average_liquidity(&self) -> f64 {
        let n = self.records.len();
        if n == 0 { 0.0 } else { self.records.iter().map(|r| r.liquidity).sum::<f64>() / n as f64 }
    }
    fn all_records(&self) -> &Vec<CostRecord> {
        &self.records
    }
}
