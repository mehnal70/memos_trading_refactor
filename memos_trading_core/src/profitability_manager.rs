// profitability_manager.rs
// Kârlılık ve Maliyet Analizi Modülü
// Kârlılık hesaplama, maliyet izleme, performans raporlama

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct ProfitabilityRecord {
    pub record_id: String,
    pub timestamp: DateTime<Utc>,
    pub pnl: f64,
    pub cost: f64,
    pub description: String,
}

pub trait ProfitabilityManager {
    fn record_profitability(&mut self, record: ProfitabilityRecord);
    fn total_pnl(&self) -> f64;
    fn total_cost(&self) -> f64;
    fn all_records(&self) -> &Vec<ProfitabilityRecord>;
}

pub struct SimpleProfitabilityManager {
    pub records: Vec<ProfitabilityRecord>,
}

impl ProfitabilityManager for SimpleProfitabilityManager {
    fn record_profitability(&mut self, record: ProfitabilityRecord) {
        self.records.push(record);
    }
    fn total_pnl(&self) -> f64 {
        self.records.iter().map(|r| r.pnl).sum()
    }
    fn total_cost(&self) -> f64 {
        self.records.iter().map(|r| r.cost).sum()
    }
    fn all_records(&self) -> &Vec<ProfitabilityRecord> {
        &self.records
    }
}
