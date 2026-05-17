// robot/order_management/paper_executor.rs - Otonom Sanal İnfaz ve Maliyet Simülatörü

use crate::core::types::Trade;
use crate::Result;
use chrono::Utc;
use super::orderbook_sim::{OrderBookSimulator, SyntheticBookConfig};

// --- 1. MALİYET YAPILANDIRMASI ---

#[derive(Debug, Clone)]
pub struct ExecutionCostConfig {
    pub spread_pct:           f64,
    pub slippage_pct:         f64,
    pub commission_pct:       f64,
    pub market_impact_factor: f64,
    pub ref_notional:         f64,
}

impl ExecutionCostConfig {
    pub fn binance_spot() -> Self {
        Self { spread_pct: 0.02, slippage_pct: 0.03, commission_pct: 0.10, market_impact_factor: 0.01, ref_notional: 10_000.0 }
    }

    pub fn binance_futures() -> Self {
        Self { spread_pct: 0.01, slippage_pct: 0.02, commission_pct: 0.04, market_impact_factor: 0.008, ref_notional: 10_000.0 }
    }

    pub fn market_impact_pct(&self, notional: f64) -> f64 {
        if self.market_impact_factor == 0.0 || self.ref_notional <= 0.0 { return 0.0; }
        self.market_impact_factor * (notional / self.ref_notional).sqrt()
    }

    pub fn round_trip_with_impact(&self, notional: f64) -> f64 {
        let impact = self.market_impact_pct(notional);
        self.spread_pct + self.slippage_pct * 2.0 + self.commission_pct * 2.0 + impact * 2.0
    }
}

impl Default for ExecutionCostConfig { fn default() -> Self { Self::binance_spot() } }

// --- 2. RAPORLAMA VE POZİSYON YAPILARI ---

#[derive(Debug, Clone)]
pub struct ExecutionCostBreakdown {
    pub expected_price: f64,
    pub executed_price: f64,
    pub market_impact_cost_usd: f64,
    pub spread_cost_usd: f64,
    pub slippage_cost_usd: f64,
    pub commission_usd: f64,
    pub total_cost_usd: f64,
    pub total_cost_pct: f64,
}

#[derive(Debug, Clone)]
struct OpenPosition {
    symbol: String,
    entry_price: f64,
    ideal_price: f64,
    amount: f64,
    entry_time: chrono::DateTime<Utc>,
    entry_cost: ExecutionCostBreakdown,
}

// --- 3. ANA SİMÜLATÖR MOTORU ---

pub struct PaperTradingExecutor {
    balance: f64,
    initial_balance: f64,
    trades: Vec<Trade>,
    open_position: Option<OpenPosition>,
    cost_config: ExecutionCostConfig,
    total_commission_paid: f64,
    total_spread_cost: f64,
    total_slippage_cost: f64,
    total_market_impact_cost: f64,
    trade_count: usize,
    orderbook_sim: Option<OrderBookSimulator>,
}

impl PaperTradingExecutor {
    pub fn new(initial_balance: f64) -> Self {
        Self {
            balance: initial_balance, initial_balance, trades: vec![], open_position: None,
            cost_config: ExecutionCostConfig::default(), total_commission_paid: 0.0,
            total_spread_cost: 0.0, total_slippage_cost: 0.0, total_market_impact_cost: 0.0,
            trade_count: 0, orderbook_sim: None,
        }
    }

    pub fn update_price(&mut self, new_price: f64) {
        if let Some(sim) = &mut self.orderbook_sim { sim.update_price(new_price); }
    }

    fn apply_buy_leakage(&self, ideal_price: f64, notional: f64) -> f64 {
        let impact = ideal_price * self.cost_config.market_impact_pct(notional) / 100.0;
        let spread = ideal_price * self.cost_config.spread_pct / 200.0;
        let slip = ideal_price * self.cost_config.slippage_pct / 100.0;
        ideal_price + impact + spread + slip
    }

    fn apply_sell_leakage(&self, ideal_price: f64, notional: f64) -> f64 {
        let impact = ideal_price * self.cost_config.market_impact_pct(notional) / 100.0;
        let spread = ideal_price * self.cost_config.spread_pct / 200.0;
        let slip = ideal_price * self.cost_config.slippage_pct / 100.0;
        ideal_price - impact - spread - slip
    }

    pub fn buy(&mut self, symbol: &str, ideal_price: f64, amount: f64) -> Result<ExecutionCostBreakdown> {
        if self.open_position.is_some() { return Err("Açık pozisyon mevcut".into()); }
        
        let notional = ideal_price * amount;
        let executed_price = self.apply_buy_leakage(ideal_price, notional);
        let commission = executed_price * amount * self.cost_config.commission_pct / 100.0;
        let total_cost = (executed_price * amount) + commission;

        if total_cost > self.balance { return Err("Yetersiz sanal bakiye".into()); }

        self.balance -= total_cost;
        let breakdown = ExecutionCostBreakdown {
            expected_price: ideal_price, executed_price, 
            market_impact_cost_usd: ideal_price * self.cost_config.market_impact_pct(notional) / 100.0 * amount,
            spread_cost_usd: ideal_price * self.cost_config.spread_pct / 200.0 * amount,
            slippage_cost_usd: ideal_price * self.cost_config.slippage_pct / 100.0 * amount,
            commission_usd: commission, total_cost_usd: total_cost - (ideal_price * amount),
            total_cost_pct: (total_cost - notional) / notional * 100.0,
        };

        self.total_commission_paid += breakdown.commission_usd;
        self.open_position = Some(OpenPosition {
            symbol: symbol.to_string(), entry_price: executed_price, ideal_price, amount,
            entry_time: Utc::now(), entry_cost: breakdown.clone(),
        });
        Ok(breakdown)
    }

    pub fn close_position(&mut self, ideal_exit_price: f64) -> Result<(Trade, ExecutionCostBreakdown)> {
        let pos = self.open_position.take().ok_or("Açık pozisyon yok")?;
        let notional = ideal_exit_price * pos.amount;
        let executed_price = self.apply_sell_leakage(ideal_exit_price, notional);
        let commission = executed_price * pos.amount * self.cost_config.commission_pct / 100.0;
        
        let proceeds = (executed_price * pos.amount) - commission;
        self.balance += proceeds;
        self.trade_count += 1;

        let pnl = (executed_price - pos.entry_price) * pos.amount - pos.entry_cost.commission_usd - commission;
        let trade = Trade {
            id: Some(self.trade_count as u64), symbol: pos.symbol, entry_price: pos.entry_price,
            exit_price: Some(executed_price), amount: pos.amount, entry_time: pos.entry_time,
            exit_time: Some(Utc::now()), pnl: Some(pnl), pnl_pct: Some(pnl / (pos.entry_price * pos.amount) * 100.0),
            strategy: "paper".to_string(),
        };

        self.trades.push(trade.clone());
        Ok((trade, ExecutionCostBreakdown { expected_price: ideal_exit_price, executed_price, ..breakdown_placeholder() }))
    }
}

// --- 4. RAPORLAMA YAPISI ---

#[derive(Debug, Clone)]
pub struct ExecutionCostReport {
    pub trade_count: usize,
    pub total_commission_paid: f64,
    pub total_spread_cost: f64,
    pub total_slippage_cost: f64,
    pub total_market_impact_cost: f64,
    pub total_cost_usd: f64,
    pub total_cost_pct: f64,
    pub paper_to_live_gap_usd: f64,
    pub avg_cost_per_trade: f64,
}

impl std::fmt::Display for ExecutionCostReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "İşlem={} | Komisyon=${:.2} | Toplam=${:.2} ({:.3}%) | Gap=${:.2}",
            self.trade_count, self.total_commission_paid, self.total_cost_usd, self.total_cost_pct, self.paper_to_live_gap_usd)
    }
}

fn breakdown_placeholder() -> ExecutionCostBreakdown { /* Dahili yardımcı */ 
    ExecutionCostBreakdown { expected_price: 0.0, executed_price: 0.0, market_impact_cost_usd: 0.0, spread_cost_usd: 0.0, slippage_cost_usd: 0.0, commission_usd: 0.0, total_cost_usd: 0.0, total_cost_pct: 0.0 }
}
