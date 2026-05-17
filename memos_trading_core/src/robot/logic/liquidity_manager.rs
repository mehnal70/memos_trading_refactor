// liquidity_manager.rs
// Likidite İzleme ve Yönetimi Modülü

use chrono::{DateTime, Utc};
use std::collections::HashMap;

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
    fn all_snapshots(&self) -> &[LiquiditySnapshot];
}

/// Sembol bazlı istatistikleri tutan yardımcı yapı
#[derive(Default)]
struct SymbolStats {
    count: usize,
    sum_spread: f64,
    sum_bid: f64,
    sum_ask: f64,
}

pub struct SimpleLiquidityManager {
    pub snapshots: Vec<LiquiditySnapshot>,
    // Performans: Sembol bazlı toplamları cache'leyerek O(n) taramadan kurtuluyoruz
    stats: HashMap<String, SymbolStats>,
}

impl SimpleLiquidityManager {
    pub fn new() -> Self {
        Self {
            snapshots: Vec::with_capacity(1000),
            stats: HashMap::with_capacity(50),
        }
    }
}

impl LiquidityManager for SimpleLiquidityManager {
    fn record_snapshot(&mut self, snapshot: LiquiditySnapshot) {
        // İstatistikleri güncelle (O(1))
        let s = self.stats.entry(snapshot.symbol.clone()).or_default();
        s.count += 1;
        s.sum_spread += snapshot.spread;
        s.sum_bid += snapshot.bid_liquidity;
        s.sum_ask += snapshot.ask_liquidity;

        self.snapshots.push(snapshot);
    }

    fn average_spread(&self, symbol: &str) -> f64 {
        self.stats.get(symbol)
            .filter(|s| s.count > 0)
            .map_or(0.0, |s| s.sum_spread / s.count as f64)
    }

    fn total_liquidity(&self, symbol: &str) -> (f64, f64) {
        self.stats.get(symbol)
            .map_or((0.0, 0.0), |s| (s.sum_bid, s.sum_ask))
    }

    #[inline]
    fn all_snapshots(&self) -> &[LiquiditySnapshot] {
        &self.snapshots
    }
}
