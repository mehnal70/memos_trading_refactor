// profitability_manager.rs
// Kârlılık ve Maliyet Analizi Modülü

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
    fn net_profit(&self) -> f64;
    fn all_records(&self) -> &[ProfitabilityRecord]; // &Vec yerine Slice (&[T])
}

pub struct SimpleProfitabilityManager {
    pub records: Vec<ProfitabilityRecord>,
    // Performans: O(n) taramadan kaçınmak için kümülatif değerleri cache'liyoruz
    cached_pnl: f64,
    cached_cost: f64,
}

impl SimpleProfitabilityManager {
    pub fn new() -> Self {
        Self {
            records: Vec::with_capacity(500),
            cached_pnl: 0.0,
            cached_cost: 0.0,
        }
    }
}

impl ProfitabilityManager for SimpleProfitabilityManager {
    fn record_profitability(&mut self, record: ProfitabilityRecord) {
        // Değerleri anında güncelle (O(1))
        self.cached_pnl += record.pnl;
        self.cached_cost += record.cost;
        self.records.push(record);
    }

    #[inline]
    fn total_pnl(&self) -> f64 {
        self.cached_pnl
    }

    #[inline]
    fn total_cost(&self) -> f64 {
        self.cached_cost
    }

    /// Net kâr: PnL - Maliyet
    #[inline]
    fn net_profit(&self) -> f64 {
        self.cached_pnl - self.cached_cost
    }

    #[inline]
    fn all_records(&self) -> &[ProfitabilityRecord] {
        &self.records
    }
}
