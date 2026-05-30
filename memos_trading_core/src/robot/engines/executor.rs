use crate::robot::execution::policy::{
    default_chain, evaluate_chain, ExecutionContext, ExecutionDecision, ExecutionPolicy,
    MarketHoursPolicy,
};
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
    /// Faz 4 c3: yürütme öncesi policy zinciri. İlk Skip → sembol atlanır.
    /// Default chain market_hours + idle_strategy + basket_empty kontrol eder.
    pub policies: Vec<Box<dyn ExecutionPolicy>>,
}

impl<'a> RoboticTradeExecutor<'a> {
    pub fn new(executor: &'a dyn TradeExecutor, basket: Vec<String>, market_hours: Option<(u32, u32)>) -> Self {
        Self { executor, basket, market_hours, state: None, policies: default_chain() }
    }

    /// StateManager ile otomatik basket ve balance yönetimi
    pub fn with_state(executor: &'a dyn TradeExecutor, state: &'a dyn StateManager, market_hours: Option<(u32, u32)>) -> Self {
        let basket = state.get_symbols().unwrap_or_default();
        Self { executor, basket, market_hours, state: Some(state), policies: default_chain() }
    }

    /// Default zinciri yepyeni bir policy listesiyle değiştir (test/özel kullanım).
    pub fn with_policies(mut self, policies: Vec<Box<dyn ExecutionPolicy>>) -> Self {
        self.policies = policies;
        self
    }

    /// Mevcut zincire yeni policy ekle (RiskFilter::push_filter ile aynı çizgide).
    pub fn push_policy(&mut self, policy: Box<dyn ExecutionPolicy>) {
        self.policies.push(policy);
    }

    /// Market saatleri kontrolü — geriye dönük uyumlu; default MarketHoursPolicy
    /// ile aynı kararı verir.
    pub fn is_market_open(&self) -> bool {
        MarketHoursPolicy
            .evaluate_hours(self.market_hours, Utc::now().hour())
            .is_allow()
    }

    fn resolve_symbols(&self) -> Vec<String> {
        if let Some(state) = self.state {
            state.get_symbols().unwrap_or_else(|_| self.basket.clone())
        } else {
            self.basket.clone()
        }
    }

    fn policy_decision(&self, signal: &Signal, symbol: &str, amount: f64, basket_size: usize) -> ExecutionDecision {
        let ctx = ExecutionContext {
            signal,
            symbol,
            amount,
            strategy_name: None,
            market_hours: self.market_hours,
            current_hour: Utc::now().hour(),
            basket_size,
        };
        let (decision, _who) = evaluate_chain(&self.policies, &ctx);
        decision
    }

    /// Basket içindeki tüm semboller için trade uygula. Policy zinciri her
    /// sembol için bağımsız değerlendirilir: skip alan sembol için Err'lı
    /// sonuç döner, kalanlar trade'e devam eder. (Eski davranış market kapalı
    /// olduğunda boş vec dönüyordu — şimdi her sembol için yapılan kontrol
    /// "Skip" reason'ı ile loglanabilir.)
    pub fn execute_basket(&self, signal: Signal, amount: f64) -> Vec<Result<Trade>> {
        let symbols = self.resolve_symbols();
        let basket_size = symbols.len();
        symbols
            .iter()
            .map(|sym| match self.policy_decision(&signal, sym, amount, basket_size) {
                ExecutionDecision::Allow => self.executor.execute(signal, sym, amount),
                ExecutionDecision::Skip { reason } => {
                    Err(format!("ExecutionPolicy skip [{sym}]: {reason}").into())
                }
            })
            .collect()
    }

    /// Tüm işlemleri iptal et
    pub fn cancel_all(&self) {
        let symbols = self.resolve_symbols();
        for sym in &symbols {
            let _ = self.executor.cancel_all(sym);
        }
    }
    /// StateManager'dan balance çek
    pub fn get_balance(&self) -> Option<f64> {
        self.state.and_then(|s| s.get_balance().ok())
    }
}
