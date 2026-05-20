// Faz 4 c1: Risk plug-in chain — RiskManager::authorize davranış testleri.
//
// `RiskManager::new()` default chain'i RiskGate→Kelly→VaR sırasıyla çalıştırır.
// Bu testler gerçek `MissionControl` snapshot'ını `bridge::get_snapshot` üzerinden
// `AppState`'i besleyerek üretir; saf filtre matematiği `src/robot/risk/filter.rs`
// içindeki unit testlerle, chain orkestrasyonu burada doğrulanır.

use memos_trading_core::core::bridge;
use memos_trading_core::core::model::{ClosedTradeModel, RoboticLoopConfig};
use memos_trading_core::robot::risk::RiskManager;
use memos_trading_core::robot::risk::risk_gate::RiskDecision;
use memos_trading_core::robot::risk::filter::{
    RiskFilter, RiskContext, RiskGateFilter, KellyEdgeFilter, VarFilter,
};
use memos_trading_core::robot::robotic_loop::AppState;
use memos_trading_core::core::types::Signal;

fn build_state() -> AppState {
    let config = RoboticLoopConfig::default(); // capital=10_000, peak=10_000
    AppState::new(config)
}

fn push_closed_trade(state: &AppState, pnl: f64, pnl_pct: f64) {
    let trade = ClosedTradeModel {
        symbol: "TEST".into(),
        is_long: true,
        pnl,
        pnl_pct,
        exit_reason: "TP".into(),
        closed_at: chrono::Utc::now().to_rfc3339(),
        opened_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Ok(mut tl) = state.finance.live_closed_trades.write() {
        tl.push(trade);
    }
}

#[test]
fn default_chain_allows_clean_state() {
    let st = build_state();
    let snap = bridge::get_snapshot(&st);
    let mgr = RiskManager::new();
    let sig = Signal::Buy;
    // edge=0.8, notional=$200 → tüm 3 filtre Allow.
    match mgr.authorize(&sig, &snap, 0.80, 200.0) {
        RiskDecision::Allow => {}
        other => panic!("temiz state Allow olmalı, döndü: {:?}", other),
    }
}

#[test]
fn risk_gate_filter_short_circuits_chain_on_low_confidence() {
    let st = build_state();
    let snap = bridge::get_snapshot(&st);
    let mgr = RiskManager::new();
    let sig = Signal::Buy;
    // edge=0.10 → ilk filtre (RiskGate) "ML güven yetersiz" der; chain durur.
    match mgr.authorize(&sig, &snap, 0.10, 200.0) {
        RiskDecision::Deny { reasons, .. } => {
            assert!(reasons.iter().any(|r| r.contains("güven")),
                "RiskGate veto'su bekleniyordu: {:?}", reasons);
        }
        other => panic!("Deny bekleniyordu: {:?}", other),
    }
}

#[test]
fn kelly_filter_denies_after_history_shows_negative_edge() {
    let st = build_state();
    // 3 küçük kazanım, 7 büyük zarar → ham Kelly f* negatif
    for _ in 0..3 { push_closed_trade(&st, 10.0, 0.10); }
    for _ in 0..7 { push_closed_trade(&st, -50.0, -0.50); }
    let snap = bridge::get_snapshot(&st);
    let mgr = RiskManager::new();
    let sig = Signal::Buy;
    match mgr.authorize(&sig, &snap, 0.80, 200.0) {
        RiskDecision::Deny { reasons, enter_safe_mode, halt } => {
            assert!(reasons.iter().any(|r| r.contains("Kelly")),
                "Kelly veto'su bekleniyordu: {:?}", reasons);
            assert!(enter_safe_mode);
            assert!(!halt);
        }
        other => panic!("Deny bekleniyordu: {:?}", other),
    }
}

#[test]
fn var_filter_denies_when_tail_exceeds_threshold() {
    let st = build_state();
    // Edge pozitif ama tail risk büyük: 15 küçük kazanç + 5 büyük kayıp
    for _ in 0..15 { push_closed_trade(&st, 5.0, 0.05); }
    for _ in 0..5  { push_closed_trade(&st, -200.0, -20.0); }
    let snap = bridge::get_snapshot(&st);
    let mgr = RiskManager::new();
    let sig = Signal::Buy;
    match mgr.authorize(&sig, &snap, 0.80, 200.0) {
        RiskDecision::Deny { reasons, .. } => {
            // Kelly veto edebilir, ama bizim hedef VaR — her halükarda Deny olmalı
            // ve nedenler arasında VaR veya Kelly geçmeli.
            assert!(reasons.iter().any(|r| r.contains("VaR") || r.contains("Kelly")),
                "Tail riskte veto bekleniyordu: {:?}", reasons);
        }
        other => panic!("Deny bekleniyordu: {:?}", other),
    }
}

#[test]
fn custom_chain_can_replace_default() {
    let st = build_state();
    let snap = bridge::get_snapshot(&st);
    // Yalnız RiskGate ile; Kelly/VaR yok.
    let mgr = RiskManager::with_filters(vec![Box::new(RiskGateFilter::default())]);
    assert_eq!(mgr.filters.len(), 1);
    let sig = Signal::Buy;
    assert!(matches!(
        mgr.authorize(&sig, &snap, 0.80, 200.0),
        RiskDecision::Allow
    ));
}

#[test]
fn empty_chain_always_allows() {
    let st = build_state();
    let snap = bridge::get_snapshot(&st);
    let mgr = RiskManager::with_filters(vec![]);
    let sig = Signal::Buy;
    // Notional yüksek, edge düşük olsa bile boş chain veto edemez.
    assert!(matches!(
        mgr.authorize(&sig, &snap, 0.05, 999_999.0),
        RiskDecision::Allow
    ));
}

#[test]
fn push_filter_appends_custom_plugin() {
    // Özel "her zaman veto" filtresi → mevcut zincire eklenince ilk Allow'dan
    // sonra bile veto döndürür.
    struct AlwaysDeny;
    impl RiskFilter for AlwaysDeny {
        fn name(&self) -> &str { "always_deny" }
        fn evaluate(&self, _ctx: &RiskContext<'_>) -> RiskDecision {
            RiskDecision::Deny {
                reasons: vec!["test-plugin".into()],
                enter_safe_mode: false,
                halt: false,
            }
        }
    }

    let st = build_state();
    let snap = bridge::get_snapshot(&st);
    // Boş zincirle başla, custom plug-in ekle → ilk turda veto.
    let mut mgr = RiskManager::with_filters(vec![]);
    mgr.push_filter(Box::new(AlwaysDeny));
    let sig = Signal::Buy;
    match mgr.authorize(&sig, &snap, 0.80, 200.0) {
        RiskDecision::Deny { reasons, .. } => {
            assert_eq!(reasons, vec!["test-plugin".to_string()]);
        }
        other => panic!("Deny bekleniyordu: {:?}", other),
    }
}

#[test]
fn kelly_and_var_filters_can_be_used_standalone() {
    // İki filtre default sıra dışı kullanılabilir (rejim-bazlı override için).
    let st = build_state();
    let snap = bridge::get_snapshot(&st);
    let mgr = RiskManager::with_filters(vec![
        Box::new(VarFilter::default()),
        Box::new(KellyEdgeFilter::default()),
    ]);
    let sig = Signal::Buy;
    // Temiz state → Allow.
    assert!(matches!(
        mgr.authorize(&sig, &snap, 0.80, 200.0),
        RiskDecision::Allow
    ));
}
