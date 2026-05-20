// Faz 4 c3 — ExecutionPolicy zinciri ve RoboticTradeExecutor entegrasyonu.
//
// Policy modülünün kendi unit testleri `src/robot/execution/policy.rs` içinde;
// buradakiler kütüphane sınırı dışından bakar ve `RoboticTradeExecutor`'ün
// gerçekten zinciri uyguladığını doğrular.

use memos_trading_core::core::types::{Signal, Trade};
use memos_trading_core::robot::engines::executor::RoboticTradeExecutor;
use memos_trading_core::robot::execution::{
    default_chain, BasketEmptyPolicy, ExecutionContext, ExecutionDecision,
    ExecutionPolicy, IdleStrategyPolicy, MarketHoursPolicy,
};
use memos_trading_core::robot::infra::interfaces::TradeExecutor;
use memos_trading_core::Result;
use std::sync::atomic::{AtomicUsize, Ordering};

// ─────────────────────────────────────────────────────────────────────────────
// Test çiftleri: hesaplanan emir sayısını sayan dummy executor
// ─────────────────────────────────────────────────────────────────────────────

struct CountingExecutor {
    calls: AtomicUsize,
}

impl CountingExecutor {
    fn new() -> Self { Self { calls: AtomicUsize::new(0) } }
    fn calls(&self) -> usize { self.calls.load(Ordering::SeqCst) }
}

impl TradeExecutor for CountingExecutor {
    fn execute(&self, signal: Signal, symbol: &str, amount: f64) -> Result<Trade> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Trade {
            id: Some(1),
            symbol: symbol.to_string(),
            entry_price: 100.0,
            exit_price: None,
            amount,
            entry_time: chrono::Utc::now(),
            exit_time: None,
            pnl: None,
            pnl_pct: None,
            strategy: format!("counting-{:?}", signal),
        })
    }

    fn cancel_all(&self, _symbol: &str) -> Result<()> { Ok(()) }
}

// ─────────────────────────────────────────────────────────────────────────────
// Custom policy: belirli sembolü her zaman veto eder
// ─────────────────────────────────────────────────────────────────────────────

struct SymbolBlocklistPolicy {
    blocked: &'static str,
}

impl ExecutionPolicy for SymbolBlocklistPolicy {
    fn name(&self) -> &str { "symbol_blocklist" }
    fn evaluate(&self, ctx: &ExecutionContext<'_>) -> ExecutionDecision {
        if ctx.symbol == self.blocked {
            ExecutionDecision::Skip { reason: format!("{} bloklu", self.blocked) }
        } else {
            ExecutionDecision::Allow
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// default_chain davranışı — RoboticTradeExecutor üzerinden uçtan uca
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn default_chain_allows_when_market_open_and_basket_nonempty() {
    let exec = CountingExecutor::new();
    // market_hours = None → 7/24 açık, IDLE strateji yok, basket 2 sembol.
    let rte = RoboticTradeExecutor::new(&exec, vec!["BTCUSDT".into(), "ETHUSDT".into()], None);
    let results = rte.execute_basket(Signal::Buy, 1.0);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.is_ok()));
    assert_eq!(exec.calls(), 2);
}

#[test]
fn basket_empty_policy_short_circuits_default_chain() {
    let exec = CountingExecutor::new();
    let rte = RoboticTradeExecutor::new(&exec, vec![], None);
    let results = rte.execute_basket(Signal::Buy, 1.0);
    // Boş basket → execute hiç çağrılmaz, results da boş döner.
    assert!(results.is_empty());
    assert_eq!(exec.calls(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Custom policy enjeksiyonu — sembol bloklama
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn custom_policy_can_block_specific_symbol() {
    let exec = CountingExecutor::new();
    let mut policies: Vec<Box<dyn ExecutionPolicy>> = default_chain();
    policies.push(Box::new(SymbolBlocklistPolicy { blocked: "ETHUSDT" }));

    let rte = RoboticTradeExecutor::new(
        &exec,
        vec!["BTCUSDT".into(), "ETHUSDT".into(), "BNBUSDT".into()],
        None,
    ).with_policies(policies);

    let results = rte.execute_basket(Signal::Buy, 1.0);
    assert_eq!(results.len(), 3);
    assert!(results[0].is_ok(), "BTCUSDT geçmeli");
    assert!(results[1].is_err(), "ETHUSDT bloklu olmalı");
    assert!(results[2].is_ok(), "BNBUSDT geçmeli");
    let err = results[1].as_ref().err().unwrap().to_string();
    assert!(err.contains("symbol_blocklist") || err.contains("ETHUSDT bloklu"),
        "skip reason loga akmalı: {err}");
    // Bloklu sembol için execute çağrılmamış olmalı → 3 sembolden 2 çağrı.
    assert_eq!(exec.calls(), 2);
}

#[test]
fn push_policy_appends_to_existing_chain() {
    let exec = CountingExecutor::new();
    let mut rte = RoboticTradeExecutor::new(&exec, vec!["BTCUSDT".into()], None);
    let baseline_policies = rte.policies.len();
    rte.push_policy(Box::new(SymbolBlocklistPolicy { blocked: "BTCUSDT" }));
    assert_eq!(rte.policies.len(), baseline_policies + 1);

    let results = rte.execute_basket(Signal::Buy, 1.0);
    assert_eq!(results.len(), 1);
    assert!(results[0].is_err(), "tek sembol bloklu olmalı");
    assert_eq!(exec.calls(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// is_market_open geriye dönük uyumlu
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn is_market_open_matches_market_hours_policy() {
    let exec = CountingExecutor::new();
    // None → 7/24 açık
    let rte = RoboticTradeExecutor::new(&exec, vec!["BTCUSDT".into()], None);
    assert!(rte.is_market_open());

    let mhp = MarketHoursPolicy;
    // Aynı saat değerinde MarketHoursPolicy de Allow demeli.
    assert!(mhp.evaluate_hours(None, chrono::Timelike::hour(&chrono::Utc::now())).is_allow());
}

// ─────────────────────────────────────────────────────────────────────────────
// Boş zincir → her sembol Allow
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn empty_policy_chain_allows_every_symbol() {
    let exec = CountingExecutor::new();
    let rte = RoboticTradeExecutor::new(&exec, vec!["BTCUSDT".into(), "ETHUSDT".into()], None)
        .with_policies(vec![]);
    let results = rte.execute_basket(Signal::Buy, 1.0);
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2);
    assert_eq!(exec.calls(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// Policy isimleri public API olarak görünür
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn default_policy_names_match_documented_order() {
    let c = default_chain();
    let names: Vec<&str> = c.iter().map(|p| p.name()).collect();
    assert_eq!(names, vec!["market_hours", "idle_strategy", "basket_empty"]);
    // Spot check: re-export'lar erişilebilir.
    let _ = MarketHoursPolicy;
    let _ = IdleStrategyPolicy;
    let _ = BasketEmptyPolicy;
}
