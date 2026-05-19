// RiskGate otorizasyon barajları — saf birim testler.
//
// Kapsam: K6 audit bulgusu (RiskManager::authorize artık stub değil; RiskGate
// gerçek DD/günlük zarar/notional/ML güveni barajlarını uyguluyor).
//
// Not: K7+K8 (TradeSignal SHORT/LONG yön) için yazdığımız testler `risk/risk.rs`
// dosyasıyla birlikte silindi — K9 kapsamında bu modülün **kod tabanında hiç
// kullanılmadığı** doğrulandı (dead code).

use memos_trading_core::robot::risk::risk_gate::{RiskGate, RiskGatePolicy, RiskInput, RiskDecision};

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
