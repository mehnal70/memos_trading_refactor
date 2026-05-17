use crate::robot::infra::interfaces::TradeExecutor;
use crate::robot::state::state_manager::StateManager;
use crate::core::types::{Signal, Trade};
use crate::Result;
use chrono::{Utc, Timelike};

pub struct RoboticTradeExecutor<'a> {
    pub executor: &'a dyn TradeExecutor,
    pub market_hours: Option<(u32, u32)>, // ör: (9, 18)
    pub basket: Vec<String>, // Semboller (opsiyonel, StateManager ile override edilebilir)
    pub state: Option<&'a dyn StateManager>,
}

impl<'a> RoboticTradeExecutor<'a> {
    pub fn new(executor: &'a dyn TradeExecutor, basket: Vec<String>, market_hours: Option<(u32, u32)>) -> Self {
        Self { executor, basket, market_hours, state: None }
    }

    /// StateManager ile otomatik basket ve balance yönetimi
    pub fn with_state(executor: &'a dyn TradeExecutor, state: &'a dyn StateManager, market_hours: Option<(u32, u32)>) -> Self {
        let basket = state.get_symbols().unwrap_or_default();
        Self { executor, basket, market_hours, state: Some(state) }
    }

    /// Market saatleri kontrolü
    pub fn is_market_open(&self) -> bool {
        if let Some((start, end)) = self.market_hours {
            let hour = Utc::now().hour();
            hour >= start && hour < end
        } else {
            true
        }
    }

    /// Basket içindeki tüm semboller için trade uygula
    pub fn execute_basket(&self, signal: Signal, amount: f64) -> Vec<Result<Trade>> {
        if !self.is_market_open() {
            println!("[SCHEDULER] Market kapalı, işlem yapılmadı.");
            return vec![];
        }
        let symbols = if let Some(state) = self.state {
            state.get_symbols().unwrap_or_else(|_| self.basket.clone())
        } else {
            self.basket.clone()
        };
        symbols.iter().map(|sym| self.executor.execute(signal.clone(), sym, amount)).collect()
    }

    /// Giriş emirleri için POST_ONLY Limit Maker basket.
    /// `price`: VWAP giriş fiyatı. `timeout_ms`: fill bekleme süresi.
    /// Çıkış emirlerinde (SL/TP) execute_basket kullanılmaya devam eder.
    pub fn execute_basket_limit(&self, signal: Signal, amount: f64, price: f64, timeout_ms: u64) -> Vec<Result<Trade>> {
        if !self.is_market_open() {
            return vec![];
        }
        let symbols = if let Some(state) = self.state {
            state.get_symbols().unwrap_or_else(|_| self.basket.clone())
        } else {
            self.basket.clone()
        };
        symbols.iter().map(|sym| {
            self.executor.execute_limit(signal.clone(), sym, amount, price, timeout_ms)
        }).collect()
    }

    /// Tüm işlemleri iptal et
    pub fn cancel_all(&self) {
        let symbols = if let Some(state) = self.state {
            state.get_symbols().unwrap_or_else(|_| self.basket.clone())
        } else {
            self.basket.clone()
        };
        for sym in &symbols {
            let _ = self.executor.cancel_all(sym);
        }
    }
    /// StateManager'dan balance çek
    pub fn get_balance(&self) -> Option<f64> {
        self.state.and_then(|s| s.get_balance().ok())
    }
}
