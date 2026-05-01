// liquidity_manager.rs
// Likidite İzleme ve Yönetimi Modülü
// Piyasa derinliği, anlık likidite, likidite riski analizi

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct LiquiditySnapshot {
    pub snapshot_id: String,
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub bid_liquidity: f64,
    pub ask_liquidity: f64,
    pub spread: f64,
}

pub trait LiquidityManager {
    fn record_snapshot(&mut self, snapshot: LiquiditySnapshot);
    fn average_spread(&self, symbol: &str) -> f64;
    fn total_liquidity(&self, symbol: &str) -> (f64, f64);
    fn all_snapshots(&self) -> &Vec<LiquiditySnapshot>;
}

pub struct SimpleLiquidityManager {
    pub snapshots: Vec<LiquiditySnapshot>,
}

impl LiquidityManager for SimpleLiquidityManager {
    fn record_snapshot(&mut self, snapshot: LiquiditySnapshot) {
        self.snapshots.push(snapshot);
    }
    fn average_spread(&self, symbol: &str) -> f64 {
        let filtered: Vec<&LiquiditySnapshot> = self.snapshots.iter().filter(|s| s.symbol == symbol).collect();
        let n = filtered.len();
        if n == 0 { 0.0 } else { filtered.iter().map(|s| s.spread).sum::<f64>() / n as f64 }
    }
    fn total_liquidity(&self, symbol: &str) -> (f64, f64) {
        let filtered: Vec<&LiquiditySnapshot> = self.snapshots.iter().filter(|s| s.symbol == symbol).collect();
        let bid: f64 = filtered.iter().map(|s| s.bid_liquidity).sum();
        let ask: f64 = filtered.iter().map(|s| s.ask_liquidity).sum();
        (bid, ask)
    }
    fn all_snapshots(&self) -> &Vec<LiquiditySnapshot> {
        &self.snapshots
    }
}
