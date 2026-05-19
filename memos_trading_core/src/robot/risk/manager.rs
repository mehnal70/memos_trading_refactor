// src/robot/risk/manager.rs - Merkezi Risk İnfaz ve Otorizasyon Motoru
//
// authorize() artık 4 stub'ı (her zaman true dönen) değil, gerçek kontrolleri çağırır:
//   1. RiskGate::evaluate — DD limiti, günlük zarar, notional tavanı, ML güveni
//   2. KellyCriterion   — yeterli trade geçmişinde edge negatifse reddet
//   3. ValueAtRisk      — 20+ kapanışta günlük historical VaR equity'nin %5'ini aşarsa reddet
//
// Dönüş tipi RiskDecision (Allow / Deny{reasons, safe_mode, halt}) — caller
// reddedildiyse `reasons`'ı log'a basabilir, halt=true ise sistemi durdurabilir.

use crate::prelude::*;
use super::risk_gate::{RiskGate, RiskInput, RiskDecision};
use super::kelly::KellyCriterion;
use super::var::ValueAtRisk;

/// Srivastava ATP - Entegre Risk Yönetim Merkezi
pub struct RiskManager {
    pub gate: RiskGate,
    /// Trade-history yeterli olduğunda Kelly edge kontrolüne dahil olur (default 10).
    pub min_trades_for_kelly: usize,
    /// VaR kontrolü için minimum kapanış sayısı (default 20).
    pub min_trades_for_var: usize,
    /// Günlük historical VaR limiti (%); aşılırsa veto. Default 5.0.
    pub max_daily_var_pct: f64,
}

impl Default for RiskManager {
    fn default() -> Self { Self::new() }
}

impl RiskManager {
    pub fn new() -> Self {
        Self {
            gate: RiskGate::default(),
            min_trades_for_kelly: 10,
            min_trades_for_var: 20,
            max_daily_var_pct: 5.0,
        }
    }

    /// ⚔️ NİHAİ OTORİZASYON: Bir işlemin borsaya gidip gitmeyeceğine karar verir.
    ///
    /// `signal`: yön (Buy/Sell). Hold için çağırma — burada Buy/Sell beklenir.
    /// `snap`: MissionControl anlık görüntüsü (equity, peak, trade_history vb.).
    /// `edge_score`: 0..1 ML/strateji güven skoru (master.rs::compute_edge_score üretir).
    /// `requested_notional_usd`: açılması düşünülen pozisyonun USD büyüklüğü.
    pub fn authorize(
        &self,
        _signal: &Signal,
        snap: &MissionControl,
        edge_score: f64,
        requested_notional_usd: f64,
    ) -> RiskDecision {
        // --- 1. RiskGate: DD + günlük zarar + notional + ML güven barajları ---
        let starting = snap.finance.starting_capital.max(1e-9);
        let peak = snap.charts.peak_equity.max(starting);
        let input = RiskInput {
            account_equity: snap.finance.total_equity,
            day_start_equity: starting,
            peak_equity: peak,
            requested_notional_usd,
            model_confidence: edge_score.clamp(0.0, 1.0),
        };
        match self.gate.evaluate(input) {
            RiskDecision::Allow => {}
            decision @ RiskDecision::Deny { .. } => return decision,
        }

        // --- 2. Kelly Edge: yeterli geçmişte negatif edge → veto ---
        if snap.trade_history.len() >= self.min_trades_for_kelly {
            let (mut wins, mut losses) = (Vec::new(), Vec::new());
            for t in &snap.trade_history {
                if t.pnl > 0.0 { wins.push(t.pnl); }
                else if t.pnl < 0.0 { losses.push(-t.pnl); }
            }
            let total = wins.len() + losses.len();
            if total >= self.min_trades_for_kelly && !wins.is_empty() && !losses.is_empty() {
                let win_prob = wins.len() as f64 / total as f64;
                let avg_win  = wins.iter().sum::<f64>() / wins.len() as f64;
                let avg_loss = losses.iter().sum::<f64>() / losses.len() as f64;
                let kelly = KellyCriterion::calculate(win_prob, avg_win, avg_loss);

                // Saf Kelly raw formülü: f* = (p*b - q) / b; b = avg_win / avg_loss.
                // KellyCriterion .kelly_fraction zaten max(0.0) ile clamp'lenmiş, yani
                // negatif edge'i ayırt etmek için b ve raw_f'i ayrıca hesaplıyoruz.
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
                // kelly_fraction=0 ise edge yok ama saf zarar da yok; cooldown yerine geç.
                let _ = kelly;
            }
        }

        // --- 3. VaR: 20+ kapanışta günlük historical VaR equity'nin %5'ini aşarsa veto ---
        if snap.trade_history.len() >= self.min_trades_for_var {
            // PnL/equity getiri yaklaşımı: pnl_pct/100 = single-trade return.
            let returns: Vec<f64> = snap.trade_history.iter()
                .map(|t| t.pnl_pct / 100.0)
                .collect();
            let var = ValueAtRisk::historical(&returns, 0.95, snap.finance.total_equity);
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
        }

        RiskDecision::Allow
    }
}
