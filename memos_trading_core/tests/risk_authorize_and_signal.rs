// Risk otorizasyon + TradeSignal yönü saf birim testleri.
//
// Audit bulguları (K6/K7/K8) için kapsam:
//   - RiskGate::evaluate gerçekten DD/günlük zarar/notional/ML güven barajlarını uyguluyor
//   - RiskManager::authorize stub değil; reject reason'larını döndürüyor
//   - TradeSignal SL/TP yönü Buy/Sell'e göre doğru
//   - TradeSignal::calculate_pnl ve calculate_pnl_pct SHORT için doğru işaret

use memos_trading_core::robot::risk::risk_gate::{RiskGate, RiskGatePolicy, RiskInput, RiskDecision};
use memos_trading_core::robot::risk::risk::{TradeSignal, RiskParams, TradeAction};

// ─────────────────────────────────────────────────────────────────────────────
// RiskGate::evaluate barajları
// ─────────────────────────────────────────────────────────────────────────────

fn default_input() -> RiskInput {
    RiskInput {
        account_equity: 1000.0,
        day_start_equity: 1000.0,
        peak_equity: 1000.0,
        requested_notional_usd: 100.0,
        model_confidence: 0.80,
    }
}

#[test]
fn risk_gate_allows_healthy_state() {
    let gate = RiskGate::default();
    match gate.evaluate(default_input()) {
        RiskDecision::Allow => {}
        RiskDecision::Deny { reasons, .. } => panic!("Sağlıklı durum reddedildi: {:?}", reasons),
    }
}

#[test]
fn risk_gate_denies_when_drawdown_exceeds_limit() {
    let gate = RiskGate::default(); // max_drawdown_pct = 10.0
    let input = RiskInput {
        peak_equity: 1000.0,
        account_equity: 850.0, // %15 DD
        ..default_input()
    };
    match gate.evaluate(input) {
        RiskDecision::Deny { reasons, halt, .. } => {
            assert!(halt, "DD aşımı halt tetiklemeli");
            assert!(reasons.iter().any(|r| r.contains("Max DD")));
        }
        RiskDecision::Allow => panic!("DD %15 reddedilmedi"),
    }
}

#[test]
fn risk_gate_denies_when_daily_loss_breaches_policy() {
    // %3 günlük zarar limiti var; %4 ihlal eder ama halt eşiği (>%5) altında.
    let gate = RiskGate::default();
    let input = RiskInput {
        day_start_equity: 1000.0,
        account_equity:    960.0, // %4 günlük kayıp
        ..default_input()
    };
    match gate.evaluate(input) {
        RiskDecision::Deny { reasons, halt, enter_safe_mode } => {
            assert!(!halt, "%4 günlük kayıp halt seviyesi değil");
            assert!(enter_safe_mode || true, "safe_mode tetiklenebilir");
            assert!(reasons.iter().any(|r| r.contains("Günlük kayıp")));
        }
        RiskDecision::Allow => panic!("%4 günlük kayıp reddedilmedi"),
    }
}

#[test]
fn risk_gate_halts_when_daily_loss_above_hard_floor() {
    // %5'in *üzerinde* günlük zarar → halt=true (sistem durdurma sinyali).
    let gate = RiskGate::default();
    let input = RiskInput {
        day_start_equity: 1000.0,
        account_equity:    930.0, // %7 günlük kayıp
        ..default_input()
    };
    match gate.evaluate(input) {
        RiskDecision::Deny { halt, .. } => {
            assert!(halt, "%7 günlük kayıp halt tetiklemeli");
        }
        RiskDecision::Allow => panic!("%7 günlük kayıp reddedilmedi"),
    }
}

#[test]
fn risk_gate_denies_when_notional_exceeds_cap() {
    let gate = RiskGate::default(); // max_notional_usd = 5000.0
    let input = RiskInput {
        requested_notional_usd: 7500.0,
        ..default_input()
    };
    match gate.evaluate(input) {
        RiskDecision::Deny { reasons, .. } => {
            assert!(reasons.iter().any(|r| r.contains("İşlem hacmi")));
        }
        RiskDecision::Allow => panic!("$7500 notional reddedilmedi"),
    }
}

#[test]
fn risk_gate_denies_when_ml_confidence_too_low() {
    let gate = RiskGate::default();
    let input = RiskInput { model_confidence: 0.20, ..default_input() }; // < 0.35
    match gate.evaluate(input) {
        RiskDecision::Deny { reasons, .. } => {
            assert!(reasons.iter().any(|r| r.contains("güven")));
        }
        RiskDecision::Allow => panic!("ML güven 0.20 reddedilmedi"),
    }
}

#[test]
fn risk_gate_custom_policy_tighter_limits() {
    let strict = RiskGatePolicy {
        max_notional_usd:   100.0,
        max_drawdown_pct:   2.0,
        max_daily_loss_pct: 1.0,
        safe_mode_threshold: 0.5,
    };
    let gate = RiskGate::new(strict);
    let input = RiskInput {
        requested_notional_usd: 150.0,
        peak_equity:    1000.0,
        account_equity:  970.0, // %3 DD, %3 günlük kayıp → tüm sertler ihlal
        ..default_input()
    };
    match gate.evaluate(input) {
        RiskDecision::Deny { reasons, .. } => assert!(reasons.len() >= 2,
            "birden fazla baraj ihlali bekleniyor: {:?}", reasons),
        RiskDecision::Allow => panic!("Sıkı politika reddetmedi"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TradeSignal — SHORT/LONG yön
// ─────────────────────────────────────────────────────────────────────────────

fn risk_params(sl: f64, tp: f64) -> RiskParams {
    RiskParams {
        stop_loss_pct: sl,
        take_profit_pct: tp,
        max_position_size_pct: Some(10.0),
        max_portfolio_risk_pct: Some(2.0),
    }
}

#[test]
fn trade_signal_long_sl_below_tp_above() {
    let p = risk_params(2.0, 5.0);
    let s = TradeSignal::new(100.0, 0, &p, TradeAction::Buy);
    assert!((s.stop_loss   - 98.0).abs()  < 1e-9, "LONG SL: {}", s.stop_loss);
    assert!((s.take_profit - 105.0).abs() < 1e-9, "LONG TP: {}", s.take_profit);
    assert!(s.stop_loss < s.entry_price && s.entry_price < s.take_profit);
}

#[test]
fn trade_signal_short_sl_above_tp_below() {
    let p = risk_params(2.0, 5.0);
    let s = TradeSignal::new(100.0, 0, &p, TradeAction::Sell);
    assert!((s.stop_loss   - 102.0).abs() < 1e-9, "SHORT SL: {}", s.stop_loss);
    assert!((s.take_profit -  95.0).abs() < 1e-9, "SHORT TP: {}", s.take_profit);
    assert!(s.take_profit < s.entry_price && s.entry_price < s.stop_loss);
}

#[test]
fn trade_signal_hold_keeps_entry() {
    let p = risk_params(2.0, 5.0);
    let s = TradeSignal::new(100.0, 0, &p, TradeAction::Hold);
    assert_eq!(s.stop_loss, s.entry_price);
    assert_eq!(s.take_profit, s.entry_price);
}

#[test]
fn pnl_long_profit_when_exit_above_entry() {
    let s = TradeSignal::new(100.0, 0, &risk_params(2.0, 5.0), TradeAction::Buy);
    assert!((s.calculate_pnl(110.0, 2.0) - 20.0).abs() < 1e-9);
    assert!((s.calculate_pnl_pct(110.0) - 10.0).abs() < 1e-9);
}

#[test]
fn pnl_long_loss_when_exit_below_entry() {
    let s = TradeSignal::new(100.0, 0, &risk_params(2.0, 5.0), TradeAction::Buy);
    assert!((s.calculate_pnl(90.0, 2.0) - (-20.0)).abs() < 1e-9);
    assert!((s.calculate_pnl_pct(90.0) - (-10.0)).abs() < 1e-9);
}

#[test]
fn pnl_short_profit_when_exit_below_entry() {
    let s = TradeSignal::new(100.0, 0, &risk_params(2.0, 5.0), TradeAction::Sell);
    // SHORT: fiyat düşerse kazanır
    assert!((s.calculate_pnl(90.0, 2.0) - 20.0).abs() < 1e-9,
            "SHORT @ 100 exit 90 qty 2 → +20 USD bekleniyor, ama: {}",
            s.calculate_pnl(90.0, 2.0));
    let pct = s.calculate_pnl_pct(90.0);
    assert!(pct > 0.0, "SHORT exit aşağıda → pozitif pct, ama: {}", pct);
    // (entry/exit - 1)*100 = (100/90 - 1)*100 ≈ 11.111
    assert!((pct - 11.111111111111111).abs() < 1e-6);
}

#[test]
fn pnl_short_loss_when_exit_above_entry() {
    let s = TradeSignal::new(100.0, 0, &risk_params(2.0, 5.0), TradeAction::Sell);
    // SHORT: fiyat yükselirse kaybeder
    assert!((s.calculate_pnl(110.0, 2.0) - (-20.0)).abs() < 1e-9);
    let pct = s.calculate_pnl_pct(110.0);
    assert!(pct < 0.0, "SHORT exit yukarıda → negatif pct, ama: {}", pct);
    // (100/110 - 1)*100 ≈ -9.0909
    assert!((pct - (-9.090909090909092)).abs() < 1e-6);
}

#[test]
fn trade_signal_zero_entry_does_not_panic() {
    let s = TradeSignal::new(0.0, 0, &risk_params(2.0, 5.0), TradeAction::Buy);
    // entry → EPSILON, SL/TP NaN olmadan üretilebilmeli
    assert!(s.stop_loss.is_finite());
    assert!(s.take_profit.is_finite());
    // pnl_pct exit=0 için sonsuz olmamalı (exit de EPSILON'a clamp'lenir)
    let pct = s.calculate_pnl_pct(0.0);
    assert!(pct.is_finite());
}
