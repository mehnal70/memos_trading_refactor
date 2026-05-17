// robot/order_management/orderbook_sim.rs - Otonom L2 Derinlik ve Slippage Simülatörü

use serde::{Deserialize, Serialize};

// --- 1. TEMEL VERI YAPILARI ---

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct BookLevel {
    pub price: f64,
    pub qty: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillResult {
    pub requested_qty: f64,
    pub filled_qty: f64,
    pub avg_fill_price: f64,
    pub slippage_pct: f64,
    pub unfilled_qty: f64,
    pub fills: Vec<PartialFill>,
    pub is_full_fill: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialFill {
    pub price: f64,
    pub qty:   f64,
    pub notional: f64,
}

// --- 2. L2 EMİR DEFTERİ MOTORU ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub mid_price: f64,
}

impl OrderBook {
    pub fn new(bids: Vec<BookLevel>, asks: Vec<BookLevel>) -> Self {
        let mid = if let (Some(b), Some(a)) = (bids.first(), asks.first()) {
            (b.price + a.price) / 2.0
        } else { 0.0 };
        Self { bids, asks, mid_price: mid }
    }

    pub fn fill_market_buy(&self, qty: f64) -> FillResult { self.fill_order(qty, &self.asks, true) }
    pub fn fill_market_sell(&self, qty: f64) -> FillResult { self.fill_order(qty, &self.bids, false) }

    fn fill_order(&self, qty: f64, levels: &[BookLevel], is_buy: bool) -> FillResult {
        let mut remaining = qty;
        let mut total_cost = 0.0;
        let mut total_filled = 0.0;
        let mut fills = Vec::new();

        for level in levels {
            if remaining <= 1e-10 { break; }
            let take = remaining.min(level.qty);
            let notional = take * level.price;
            fills.push(PartialFill { price: level.price, qty: take, notional });
            total_cost += notional;
            total_filled += take;
            remaining -= take;
        }

        let avg_price = if total_filled > 0.0 { total_cost / total_filled } else { 0.0 };
        let slippage = if self.mid_price > 0.0 && avg_price > 0.0 {
            let diff = if is_buy { avg_price - self.mid_price } else { self.mid_price - avg_price };
            (diff / self.mid_price) * 100.0
        } else { 0.0 };

        FillResult {
            requested_qty: qty, filled_qty: total_filled, avg_fill_price: avg_price,
            slippage_pct: slippage.max(0.0), unfilled_qty: remaining,
            is_full_fill: remaining <= 1e-10, fills,
        }
    }
}

// --- 3. SENTETİK DEFTER ÜRETİMİ ---

#[derive(Debug, Clone)]
pub struct SyntheticBookConfig {
    pub mid_price: f64,
    pub depth_levels: usize,
    pub spread_pct: f64,
    pub tick_pct: f64,
    pub top_qty: f64,
    pub qty_multiplier: f64,
}

impl SyntheticBookConfig {
    pub fn liquid(mid_price: f64) -> Self {
        Self { mid_price, depth_levels: 10, spread_pct: 0.02, tick_pct: 0.01, top_qty: 1.0, qty_multiplier: 1.5 }
    }
    pub fn illiquid(mid_price: f64) -> Self {
        Self { mid_price, depth_levels: 5, spread_pct: 0.15, tick_pct: 0.05, top_qty: 0.1, qty_multiplier: 1.3 }
    }
}

pub fn build_synthetic_book(cfg: &SyntheticBookConfig) -> OrderBook {
    let half_spread = cfg.mid_price * cfg.spread_pct / 200.0;
    let (best_ask, best_bid) = (cfg.mid_price + half_spread, cfg.mid_price - half_spread);
    let tick = cfg.mid_price * cfg.tick_pct / 100.0;

    let mut asks = Vec::with_capacity(cfg.depth_levels);
    let mut bids = Vec::with_capacity(cfg.depth_levels);
    let mut qty = cfg.top_qty;

    for i in 0..cfg.depth_levels {
        asks.push(BookLevel { price: best_ask + i as f64 * tick, qty });
        bids.push(BookLevel { price: best_bid - i as f64 * tick, qty });
        qty *= cfg.qty_multiplier;
    }
    OrderBook::new(bids, asks)
}

// --- 4. SİMÜLATÖR ORKESTRATÖRÜ ---

pub struct OrderBookSimulator {
    pub config: SyntheticBookConfig,
}

impl OrderBookSimulator {
    pub fn new(config: SyntheticBookConfig) -> Self { Self { config } }
    pub fn update_price(&mut self, new_mid: f64) { self.config.mid_price = new_mid; }
    pub fn simulate_buy(&self, qty: f64) -> FillResult { build_synthetic_book(&self.config).fill_market_buy(qty) }
    pub fn simulate_sell(&self, qty: f64) -> FillResult { build_synthetic_book(&self.config).fill_market_sell(qty) }
    pub fn simulate_buy_notional(&self, notional_usd: f64) -> FillResult {
        let qty = if self.config.mid_price > 0.0 { notional_usd / self.config.mid_price } else { 0.0 };
        self.simulate_buy(qty)
    }
}
