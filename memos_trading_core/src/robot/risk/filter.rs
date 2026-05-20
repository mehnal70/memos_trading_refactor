// src/robot/risk/filter.rs - Risk plug-in zinciri sözleşmesi (Faz 4 c1)
//
// `RiskManager.authorize` artık sert-kodlu sıra değil, `Vec<Box<dyn RiskFilter>>`
// üzerinde çalışan plug-in chain'idir. Her filter tek bir karar verir; ilk Deny
// döndüğünde chain kısa-devre olur. Yeni filtreler (örn. CorrelationFilter,
// VolatilityRegimeFilter) `RiskManager::push_filter` ile eklenebilir.
//
// Tasarım notu — saf çekirdek ayrımı:
//   Her filter iki yüzlüdür:
//     (a) `evaluate(&RiskContext)`  → trait imzası; chain'in çağırdığı genel uç.
//     (b) `evaluate_<inputs>(...)`  → saf yardımcı; bağımlılığı yok, doğrudan
//         test edilebilir. Trait yöntemi yalnız bağlamı söker ve (b)'yi çağırır.
//   Bu sayede filtre matematiği MissionControl literal'i kurmadan birim test
//   edilebilir; chain entegrasyonu integration test'lerde doğrulanır.

use super::risk_gate::{RiskDecision, RiskGate, RiskGatePolicy, RiskInput};
use super::kelly::KellyCriterion;
use super::var::ValueAtRisk;
use crate::prelude::*;

/// Plug-in chain'in paylaştığı salt-okunur değerlendirme bağlamı.
pub struct RiskContext<'a> {
    pub signal: &'a Signal,
    pub snap: &'a MissionControl,
    pub edge_score: f64,
    pub requested_notional_usd: f64,
}

/// Tek bir risk plug-in'inin sözleşmesi.
///
/// Implementorlar yalnızca bağlamı okur ve `RiskDecision::Allow` veya
/// `RiskDecision::Deny { .. }` döndürür. Birden çok filtre veto edebilir
/// ama chain ilk Deny'da durur.
pub trait RiskFilter: Send + Sync {
    /// Loglara/raporlara yansıyacak insan-okunabilir filtre adı.
    fn name(&self) -> &str;

    /// Salt-okunur değerlendirme. Yan etki yasak.
    fn evaluate(&self, ctx: &RiskContext<'_>) -> RiskDecision;
}

// ─────────────────────────────────────────────────────────────────────────────
// 1) RiskGateFilter — DD / günlük zarar / notional / ML güveni baraj kontrolleri
// ─────────────────────────────────────────────────────────────────────────────

pub struct RiskGateFilter {
    pub gate: RiskGate,
}

impl Default for RiskGateFilter {
    fn default() -> Self { Self { gate: RiskGate::default() } }
}

impl RiskGateFilter {
    pub fn new(policy: RiskGatePolicy) -> Self { Self { gate: RiskGate::new(policy) } }

    /// Saf çekirdek: doğrudan hazırlanmış `RiskInput` üzerinde çalışır.
    pub fn evaluate_input(&self, input: RiskInput) -> RiskDecision {
        self.gate.evaluate(input)
    }
}

impl RiskFilter for RiskGateFilter {
    fn name(&self) -> &str { "risk_gate" }

    fn evaluate(&self, ctx: &RiskContext<'_>) -> RiskDecision {
        let starting = ctx.snap.finance.starting_capital.max(1e-9);
        let peak     = ctx.snap.charts.peak_equity.max(starting);
        let input = RiskInput {
            account_equity: ctx.snap.finance.total_equity,
            day_start_equity: starting,
            peak_equity: peak,
            requested_notional_usd: ctx.requested_notional_usd,
            model_confidence: ctx.edge_score.clamp(0.0, 1.0),
        };
        self.evaluate_input(input)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2) KellyEdgeFilter — yeterli geçmişte negatif edge tespit edilirse veto eder
// ─────────────────────────────────────────────────────────────────────────────

pub struct KellyEdgeFilter {
    /// Veto için trade-history eşiği (kapanmış trade adedi). Default 10.
    pub min_trades: usize,
}

impl Default for KellyEdgeFilter {
    fn default() -> Self { Self { min_trades: 10 } }
}

impl KellyEdgeFilter {
    /// Saf çekirdek: ham trade PnL'leri üzerinde Kelly ham f* hesaplayıp
    /// negatifse `Deny` döndürür. Trait evaluate() bu fonksiyonu çağırır.
    pub fn evaluate_pnls(&self, pnls: &[f64]) -> RiskDecision {
        if pnls.len() < self.min_trades {
            return RiskDecision::Allow;
        }
        let mut wins: Vec<f64>   = Vec::new();
        let mut losses: Vec<f64> = Vec::new();
        for p in pnls {
            if *p > 0.0      { wins.push(*p); }
            else if *p < 0.0 { losses.push(-*p); }
        }
        let total = wins.len() + losses.len();
        if total < self.min_trades || wins.is_empty() || losses.is_empty() {
            return RiskDecision::Allow;
        }

        let win_prob = wins.len() as f64 / total as f64;
        let avg_win  = wins.iter().sum::<f64>() / wins.len() as f64;
        let avg_loss = losses.iter().sum::<f64>() / losses.len() as f64;

        // KellyCriterion.kelly_fraction max(0.0) ile clamp'li olduğundan,
        // ayırt edici negatif edge sinyalini almak için ham f*'yi de hesaplıyoruz.
        let _kelly = KellyCriterion::calculate(win_prob, avg_win, avg_loss);
        let b = avg_win / avg_loss;
        let raw = if b > f64::EPSILON {
            (win_prob * b - (1.0 - win_prob)) / b
        } else { 0.0 };

        if raw < 0.0 {
            return RiskDecision::Deny {
                reasons: vec![format!(
                    "Kelly edge negatif: f*={:.3} (WR={:.1}% R/R={:.2})",
                    raw, win_prob * 100.0, b
                )],
                enter_safe_mode: true,
                halt: false,
            };
        }
        RiskDecision::Allow
    }
}

impl RiskFilter for KellyEdgeFilter {
    fn name(&self) -> &str { "kelly_edge" }

    fn evaluate(&self, ctx: &RiskContext<'_>) -> RiskDecision {
        let pnls: Vec<f64> = ctx.snap.trade_history.iter().map(|t| t.pnl).collect();
        self.evaluate_pnls(&pnls)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3) VarFilter — Historical VaR portföy maruziyet limiti (%)
// ─────────────────────────────────────────────────────────────────────────────

pub struct VarFilter {
    /// VaR hesabı için minimum kapanış adedi. Default 20.
    pub min_trades: usize,
    /// Tetikleme eşiği: günlük historical VaR (%) bunu aşarsa veto. Default 5.0.
    pub max_daily_var_pct: f64,
    /// Historical VaR güven seviyesi. Default 0.95.
    pub confidence: f64,
}

impl Default for VarFilter {
    fn default() -> Self {
        Self { min_trades: 20, max_daily_var_pct: 5.0, confidence: 0.95 }
    }
}

impl VarFilter {
    /// Saf çekirdek: trade return serisini (pnl_pct/100) ve toplam equity'yi alır.
    pub fn evaluate_returns(&self, returns: &[f64], equity: f64) -> RiskDecision {
        if returns.len() < self.min_trades {
            return RiskDecision::Allow;
        }
        let var = ValueAtRisk::historical(returns, self.confidence, equity);
        if var.var_pct > self.max_daily_var_pct {
            return RiskDecision::Deny {
                reasons: vec![format!(
                    "Günlük VaR limiti: {:.2}% > {:.2}%",
                    var.var_pct, self.max_daily_var_pct
                )],
                enter_safe_mode: true,
                halt: false,
            };
        }
        RiskDecision::Allow
    }
}

impl RiskFilter for VarFilter {
    fn name(&self) -> &str { "value_at_risk" }

    fn evaluate(&self, ctx: &RiskContext<'_>) -> RiskDecision {
        let returns: Vec<f64> = ctx.snap.trade_history.iter()
            .map(|t| t.pnl_pct / 100.0)
            .collect();
        self.evaluate_returns(&returns, ctx.snap.finance.total_equity)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Yardımcı: default chain — RiskManager::new() bunu kullanır.
// ─────────────────────────────────────────────────────────────────────────────

pub fn default_chain() -> Vec<Box<dyn RiskFilter>> {
    vec![
        Box::new(RiskGateFilter::default()),
        Box::new(KellyEdgeFilter::default()),
        Box::new(VarFilter::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_healthy() -> RiskInput {
        RiskInput {
            account_equity: 1000.0,
            day_start_equity: 1000.0,
            peak_equity: 1000.0,
            requested_notional_usd: 100.0,
            model_confidence: 0.80,
        }
    }

    // ── RiskGateFilter ──────────────────────────────────────────────────

    #[test]
    fn risk_gate_filter_allows_healthy_input() {
        let f = RiskGateFilter::default();
        assert!(matches!(f.evaluate_input(input_healthy()), RiskDecision::Allow));
    }

    #[test]
    fn risk_gate_filter_denies_low_confidence() {
        let f = RiskGateFilter::default();
        let inp = RiskInput { model_confidence: 0.10, ..input_healthy() };
        match f.evaluate_input(inp) {
            RiskDecision::Deny { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("güven")), "{:?}", reasons);
            }
            other => panic!("Deny bekleniyordu: {:?}", other),
        }
    }

    #[test]
    fn risk_gate_filter_propagates_custom_policy() {
        let strict = RiskGatePolicy {
            max_notional_usd: 50.0,
            max_drawdown_pct: 1.0,
            max_daily_loss_pct: 1.0,
            safe_mode_threshold: 0.5,
        };
        let f = RiskGateFilter::new(strict);
        let inp = RiskInput { requested_notional_usd: 200.0, ..input_healthy() };
        match f.evaluate_input(inp) {
            RiskDecision::Deny { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("İşlem hacmi")), "{:?}", reasons);
            }
            other => panic!("Deny bekleniyordu: {:?}", other),
        }
    }

    // ── KellyEdgeFilter ─────────────────────────────────────────────────

    #[test]
    fn kelly_filter_allows_when_history_too_short() {
        let f = KellyEdgeFilter::default(); // min_trades = 10
        let pnls = vec![-50.0, -50.0, -50.0, -50.0, -50.0]; // 5 < 10
        assert!(matches!(f.evaluate_pnls(&pnls), RiskDecision::Allow));
    }

    #[test]
    fn kelly_filter_allows_when_only_wins() {
        // Edge ham f* hesaplanabilmek için hem win hem loss gerek; hep kazanç → Allow.
        let f = KellyEdgeFilter::default();
        let pnls = vec![10.0; 12];
        assert!(matches!(f.evaluate_pnls(&pnls), RiskDecision::Allow));
    }

    #[test]
    fn kelly_filter_denies_on_negative_edge() {
        let f = KellyEdgeFilter::default();
        // 3 küçük kazanım vs 7 büyük zarar → ham f* negatif
        let mut pnls = vec![10.0, 10.0, 10.0];
        pnls.extend(std::iter::repeat(-50.0).take(7));
        match f.evaluate_pnls(&pnls) {
            RiskDecision::Deny { reasons, enter_safe_mode, halt } => {
                assert!(enter_safe_mode);
                assert!(!halt);
                assert!(reasons[0].contains("Kelly"), "{:?}", reasons);
            }
            other => panic!("Deny bekleniyordu: {:?}", other),
        }
    }

    #[test]
    fn kelly_filter_allows_on_positive_edge() {
        let f = KellyEdgeFilter::default();
        // 7 büyük kazanım vs 3 küçük zarar → ham f* pozitif
        let mut pnls = vec![50.0; 7];
        pnls.extend(std::iter::repeat(-10.0).take(3));
        assert!(matches!(f.evaluate_pnls(&pnls), RiskDecision::Allow));
    }

    // ── VarFilter ───────────────────────────────────────────────────────

    #[test]
    fn var_filter_allows_when_history_too_short() {
        let f = VarFilter::default(); // min_trades = 20
        let returns = vec![-0.10; 10];
        assert!(matches!(f.evaluate_returns(&returns, 10_000.0), RiskDecision::Allow));
    }

    #[test]
    fn var_filter_denies_when_tail_exceeds_threshold() {
        let f = VarFilter::default(); // max_daily_var_pct = 5.0
        // 15 küçük + 5 büyük negatif return; %95 güvende tail %20'lik return'a düşer.
        let mut returns = vec![0.005f64; 15];
        returns.extend(std::iter::repeat(-0.20).take(5));
        match f.evaluate_returns(&returns, 10_000.0) {
            RiskDecision::Deny { reasons, .. } => {
                assert!(reasons[0].contains("VaR"), "{:?}", reasons);
            }
            other => panic!("Deny bekleniyordu: {:?}", other),
        }
    }

    #[test]
    fn var_filter_allows_when_tail_within_threshold() {
        let f = VarFilter::default();
        // Hepsi küçük negatif → tail eşiği aşmaz.
        let returns = vec![-0.005f64; 25];
        assert!(matches!(f.evaluate_returns(&returns, 10_000.0), RiskDecision::Allow));
    }

    // ── Default chain mevcudiyeti ───────────────────────────────────────

    #[test]
    fn default_chain_has_three_filters() {
        let chain = default_chain();
        assert_eq!(chain.len(), 3);
        let names: Vec<&str> = chain.iter().map(|f| f.name()).collect();
        assert_eq!(names, vec!["risk_gate", "kelly_edge", "value_at_risk"]);
    }
}
