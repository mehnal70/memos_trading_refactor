// robot/execution/mod.rs — Faz 4 c3: Yürütme katmanı policy plug-in'leri.
//
// `TradeExecutor` trait'i (infra/interfaces.rs) borsa adapter sözleşmesidir.
// `RoboticTradeExecutor` (engines/executor.rs) bu adapter'ın etrafında basket +
// market saatleri + politika kontrolleri yapan wrap'leyicidir. Bu modül o wrap
// davranışlarını sert-kodlu olmaktan çıkartıp `ExecutionPolicy` zincirine
// taşır → RiskFilter chain (Faz 4 c1) ve StrategyRegistry (Faz 4 c2) ile
// aynı yapı.

pub mod policy;

pub use policy::{
    default_chain, BasketEmptyPolicy, ExecutionContext, ExecutionDecision,
    ExecutionPolicy, IdleStrategyPolicy, MarketHoursPolicy,
};
