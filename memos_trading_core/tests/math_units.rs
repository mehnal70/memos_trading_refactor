// Math ve risk modüllerinin saf birim testleri (D9 audit kapsamı).
//
// Kapsam:
//   core::math: calculate_pnl, calculate_roe, safe_profit_factor (Option),
//               Statistics::median, RiskMetrics::sharpe_ratio (sample),
//               RiskMetrics::max_drawdown, Correlation::pearson (Option)
//   robot::risk::kelly: KellyCriterion::calculate
//   robot::risk::var: ValueAtRisk::historical, ValueAtRisk::parametric (Acklam),
//                     inverse_normal_cdf
//   robot::risk::metrics: SharpeCalculator, SortinoCalculator, CalmarCalculator,
//                          OmegaCalculator, InformationRatio
//   robot::strategies::ensemble: weighted_tally

use memos_trading_core::core::math::{
    calculate_pnl, calculate_roe, safe_profit_factor,
    Correlation, RiskMetrics, Statistics,
};
use memos_trading_core::robot::risk::kelly::KellyCriterion;
use memos_trading_core::robot::risk::var::{inverse_normal_cdf, ValueAtRisk};
use memos_trading_core::robot::risk::metrics::{
    SharpeCalculator, SortinoCalculator, CalmarCalculator, OmegaCalculator, InformationRatio,
};

// ─────────────────────────────────────────────────────────────────────────────
// core::math
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pnl_long_and_short_directions() {
    assert!((calculate_pnl(100.0, 110.0, 2.0, true)  - 20.0).abs() < 1e-12);
    assert!((calculate_pnl(100.0, 110.0, 2.0, false) + 20.0).abs() < 1e-12);
    assert!((calculate_pnl(100.0,  90.0, 2.0, true)  + 20.0).abs() < 1e-12);
    assert!((calculate_pnl(100.0,  90.0, 2.0, false) - 20.0).abs() < 1e-12);
}

#[test]
fn pnl_no_rounding_artifact() {
    // 4-hane yuvarlama kaldı — küçük pnl olduğu gibi kalır (ham f64).
    let p = calculate_pnl(0.00001234, 0.00001500, 100_000.0, true);
    // (1.5e-5 - 1.234e-5) * 1e5 = 0.266
    assert!((p - 0.266).abs() < 1e-9, "Ham PnL precision: {}", p);
}

#[test]
fn roe_levered_long() {
    // Entry 100 → 110, 5x kaldıraç, LONG → ROE = +10% * 5 = +50%
    let r = calculate_roe(100.0, 110.0, 5.0, true);
    assert!((r - 50.0).abs() < 1e-12, "ROE: {}", r);
}

#[test]
fn roe_levered_short() {
    // Entry 100 → 110, 5x kaldıraç, SHORT → ROE = -10% * 5 = -50%
    let r = calculate_roe(100.0, 110.0, 5.0, false);
    assert!((r + 50.0).abs() < 1e-12);
}

#[test]
fn safe_pf_option_semantics() {
    // Win + Loss yoksa Some(0.0) — "henüz veri yok" anlamı
    assert_eq!(safe_profit_factor(0.0, 0.0), Some(0.0));
    // Sadece win, loss = 0 → None (tanımsız, sınırsız PF)
    assert_eq!(safe_profit_factor(10.0, 0.0), None);
    // Sadece loss, win = 0 → Some(0.0)
    assert_eq!(safe_profit_factor(0.0, -5.0), Some(0.0));
    // Normal — gross_win=30, gross_loss=-15 → PF=2.0
    assert_eq!(safe_profit_factor(30.0, -15.0), Some(2.0));
    // Loss pozitif girilirse de absolute alır
    assert_eq!(safe_profit_factor(30.0, 15.0), Some(2.0));
}

#[test]
fn median_odd_and_even_length() {
    assert_eq!(Statistics::median(&[5.0, 1.0, 3.0]), 3.0);
    assert_eq!(Statistics::median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
    assert_eq!(Statistics::median(&[]), 0.0);
}

#[test]
fn sharpe_uses_sample_variance() {
    // Returns: [1, 2, 3, 4, 5], mean=3.
    // Sample var = (4+1+0+1+4)/(5-1) = 2.5; std = 1.5811.
    // Sharpe(rf=0) = mean / std = 3 / 1.5811 ≈ 1.8974
    let r = RiskMetrics::sharpe_ratio(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.0);
    assert!((r - 1.897366596).abs() < 1e-6, "Sample-variance Sharpe: {}", r);
}

#[test]
fn max_drawdown_basic() {
    // 100 → 120 (peak) → 90 → 95. DD = (90-120)/120 = -25%.
    let dd = RiskMetrics::max_drawdown(&[100.0, 120.0, 90.0, 95.0]);
    assert!((dd - (-25.0)).abs() < 1e-9);
}

#[test]
fn max_drawdown_no_peak_returns_zero() {
    assert_eq!(RiskMetrics::max_drawdown(&[0.0, 0.0, 0.0]), 0.0);
    assert_eq!(RiskMetrics::max_drawdown(&[]), 0.0);
}

#[test]
fn pearson_option_semantics() {
    // Mükemmel pozitif korelasyon
    let p = Correlation::pearson(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]);
    assert!((p.unwrap() - 1.0).abs() < 1e-12);
    // Mükemmel negatif korelasyon
    let p = Correlation::pearson(&[1.0, 2.0, 3.0], &[6.0, 4.0, 2.0]);
    assert!((p.unwrap() + 1.0).abs() < 1e-12);
    // Boyut uyuşmazlığı → None
    assert!(Correlation::pearson(&[1.0, 2.0], &[1.0]).is_none());
    // Boş → None
    assert!(Correlation::pearson(&[], &[]).is_none());
    // İki seri de sabit (varyans 0) → None (tanımsız)
    assert!(Correlation::pearson(&[5.0, 5.0, 5.0], &[3.0, 3.0, 3.0]).is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// Kelly
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn kelly_classic_formula() {
    // WR=0.60, avg_win=200, avg_loss=100 → b=2; f* = (0.6*2 - 0.4)/2 = 0.40
    let k = KellyCriterion::calculate(0.60, 200.0, 100.0);
    assert!((k.kelly_fraction - 0.40).abs() < 1e-9, "Kelly: {}", k.kelly_fraction);
}

#[test]
fn kelly_negative_edge_clamps_to_zero() {
    // WR=0.30, b=1 → f* = -0.4, kelly_fraction max(0.0) = 0
    let k = KellyCriterion::calculate(0.30, 100.0, 100.0);
    assert_eq!(k.kelly_fraction, 0.0);
}

#[test]
fn kelly_zero_inputs_safe() {
    let k = KellyCriterion::calculate(0.6, 0.0, 100.0);
    assert_eq!(k.kelly_fraction, 0.0);
    let k = KellyCriterion::calculate(0.6, 100.0, 0.0);
    assert_eq!(k.kelly_fraction, 0.0);
}

// ─────────────────────────────────────────────────────────────────────────────
// VaR
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn historical_var_picks_left_tail() {
    // Getiri serisi: [-0.10, -0.05, -0.02, 0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07]
    // 0.95 confidence: en kötü %5 → index ceil(0.05 * 10) = 1 → sorted[1] = -0.05
    let returns = vec![-0.10, -0.05, -0.02, 0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07];
    let v = ValueAtRisk::historical(&returns, 0.95, 1000.0);
    // VaR = position * worst.abs() = 1000 * 0.05 = 50
    assert!((v.var_amount - 50.0).abs() < 1e-9, "VaR amount: {}", v.var_amount);
    // CVaR = position * |mean of tail| ; tail = [-0.10], mean = -0.10
    assert!(v.cvar.is_some());
    assert!((v.cvar.unwrap() - 100.0).abs() < 1e-9);
}

#[test]
fn inverse_normal_cdf_classic_values() {
    // Klasik VaR Z değerleri — Acklam 4 ondalık hassasiyet
    assert!((inverse_normal_cdf(0.95) - 1.6448536).abs() < 1e-4);
    assert!((inverse_normal_cdf(0.99) - 2.3263479).abs() < 1e-4);
    assert!((inverse_normal_cdf(0.975) - 1.9599640).abs() < 1e-4);
    // Simetri: Φ⁻¹(0.5) = 0
    assert!(inverse_normal_cdf(0.5).abs() < 1e-9);
    // p=0.05 → -1.6449
    assert!((inverse_normal_cdf(0.05) + 1.6448536).abs() < 1e-4);
}

#[test]
fn parametric_var_uses_acklam_z() {
    // returns: mean=0, std≈... 11 simetrik değer
    let returns: Vec<f64> = (-5..=5).map(|i| i as f64 * 0.01).collect();
    let v95 = ValueAtRisk::parametric(&returns, 0.95, 1000.0);
    let v99 = ValueAtRisk::parametric(&returns, 0.99, 1000.0);
    // 99 confidence VaR > 95 confidence VaR
    assert!(v99.var_amount > v95.var_amount, "v99={} v95={}", v99.var_amount, v95.var_amount);
}

// ─────────────────────────────────────────────────────────────────────────────
// metrics: Sharpe / Sortino / Calmar / Omega / IR
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn sharpe_calculator_matches_math_riskmetrics() {
    let returns = [1.0, 2.0, 3.0, 4.0, 5.0];
    let a = SharpeCalculator::calculate(&returns, 0.0);
    let b = RiskMetrics::sharpe_ratio(&returns, 0.0);
    assert!((a - b).abs() < 1e-12, "Sharpe iki kaynak: {} vs {}", a, b);
}

#[test]
fn sortino_only_downside_volatility() {
    // [0.05, -0.10, 0.05, -0.05] target=0
    // mean = -0.0125
    // downside (target-r > 0): for -0.10 → 0.10; for -0.05 → 0.05. Squared sum = 0.0125
    // sample (n-1=3) → variance = 0.0125/3 ≈ 0.004166...; std ≈ 0.0645
    let s = SortinoCalculator::calculate(&[0.05, -0.10, 0.05, -0.05], 0.0, 0.0);
    // mean / std (negatif beklenir)
    assert!(s < 0.0, "Sortino negatif olmalı: {}", s);
}

#[test]
fn calmar_infinity_when_no_drawdown() {
    let v = CalmarCalculator::calculate(10.0, 0.0);
    assert!(v.is_infinite() && v > 0.0);
    let v = CalmarCalculator::calculate(0.0, 0.0);
    assert_eq!(v, 0.0);
}

#[test]
fn calmar_basic_ratio() {
    // annual_return=20, max_dd=-10 → calmar = 20/10 = 2.0
    let v = CalmarCalculator::calculate(20.0, -10.0);
    assert!((v - 2.0).abs() < 1e-12);
}

#[test]
fn omega_threshold_separation() {
    // returns: [+2, -1, +3, -2] threshold=0
    // gains: 2 + 3 = 5; losses: 1 + 2 = 3 → omega = 5/3 ≈ 1.6667
    let o = OmegaCalculator::calculate(&[2.0, -1.0, 3.0, -2.0], 0.0);
    assert!((o - 5.0/3.0).abs() < 1e-9);
}

#[test]
fn information_ratio_some_when_valid() {
    let strat = [0.02, 0.03, 0.01, 0.04];
    let bench = [0.01, 0.02, 0.01, 0.02];
    let ir = InformationRatio::calculate(&strat, &bench).expect("IR Some");
    assert!(ir.tracking_error > 0.0);
    assert!(ir.excess_return > 0.0);
}

#[test]
fn information_ratio_none_on_dim_mismatch() {
    assert!(InformationRatio::calculate(&[0.01, 0.02], &[0.01]).is_none());
}
