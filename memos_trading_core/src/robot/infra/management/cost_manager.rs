// cost_manager.rs
// Maliyet ve Likidite Yönetimi Modülü

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
    fn all_records(&self) -> &[CostRecord]; // &Vec yerine Slice (&[T])
}

pub struct SimpleCostManager {
    pub records: Vec<CostRecord>,
    // Performans: Sürekli toplama yapmamak için kümülatif değerleri tutuyoruz
    cached_total_cost: f64,
    cached_total_liquidity: f64,
}

impl Default for SimpleCostManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleCostManager {
    pub fn new() -> Self {
        Self {
            records: Vec::with_capacity(500),
            cached_total_cost: 0.0,
            cached_total_liquidity: 0.0,
        }
    }
}

impl CostManager for SimpleCostManager {
    fn record_cost(&mut self, record: CostRecord) {
        // Değerleri cache'e ekle (O(1) performans)
        self.cached_total_cost += record.cost;
        self.cached_total_liquidity += record.liquidity;
        self.records.push(record);
    }

    #[inline]
    fn total_cost(&self) -> f64 {
        self.cached_total_cost
    }

    fn average_liquidity(&self) -> f64 {
        let n = self.records.len();
        if n == 0 { 
            0.0 
        } else { 
            self.cached_total_liquidity / n as f64 
        }
    }

    #[inline]
    fn all_records(&self) -> &[CostRecord] {
        &self.records
    }
}
