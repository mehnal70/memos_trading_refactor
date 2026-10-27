// src/robot/risk/risk_gate.rs - Srivastava ATP Merkezi Risk Denetim Kapısı
// Bu modül, operasyonel infaz öncesi son 'Ateş Serbest' onayını verir.
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct RiskGate {
    pub policy: RiskGatePolicy,
}

impl RiskGate {
    pub fn new(policy: RiskGatePolicy) -> Self {
        Self { policy }
    }

    /// Srivastava ATP - Otonom Değerlendirme Lojiği
    pub fn evaluate(&self, input: RiskInput) -> RiskDecision {
        let mut reasons = Vec::new();
        let mut halt = false;

        // 1. Drawdown (DD) Denetimi (Zirve Sermaye üzerinden)
        let dd = if input.peak_equity > 0.0 {
            (input.peak_equity - input.account_equity) / input.peak_equity * 100.0
        } else {
            0.0
        };

        if dd > self.policy.max_drawdown_pct {
            reasons.push(format!("Kritik Max DD aşıldı: {:.2}%", dd));
            halt = true;
        }

        // 2. Günlük Kayıp Denetimi (Gün başı sermaye üzerinden).
        // day_start_equity ≤ 0 ise oran tanımsız → 0% varsay (NaN üretme).
        let daily_loss = if input.day_start_equity > 0.0 {
            (input.day_start_equity - input.account_equity) / input.day_start_equity * 100.0
        } else {
            0.0
        };
        if daily_loss > self.policy.max_daily_loss_pct {
            reasons.push(format!(
                "Günlük kayıp sınırı ihlal edildi: {:.2}%",
                daily_loss
            ));
            // Günlük kayıp policy.halt_daily_loss_pct'i aşarsa HALT (default 5.0 —
            // eskiden sabit-kodluydu; davranış default ile birebir aynı).
            if daily_loss > self.policy.halt_daily_loss_pct {
                halt = true;
            }
        }

        // 3. Pozisyon Büyüklüğü Denetimi
        if input.requested_notional_usd > self.policy.max_notional_usd {
            reasons.push(format!(
                "İşlem hacmi limit dışı: ${:.2}",
                input.requested_notional_usd
            ));
        }

        // 4. ML Güven Barajı
        if input.model_confidence < self.policy.min_model_confidence {
            reasons.push("ML Model güven seviyesi yetersiz".to_string());
        }

        if reasons.is_empty() {
            RiskDecision::Allow
        } else {
            RiskDecision::Deny {
                reasons,
                enter_safe_mode: dd
                    > (self.policy.max_drawdown_pct * self.policy.safe_mode_threshold)
                    || daily_loss > self.policy.safe_mode_daily_loss_pct,
                halt,
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskGatePolicy {
    pub max_notional_usd: f64,
    pub max_drawdown_pct: f64,
    pub max_daily_loss_pct: f64,
    pub safe_mode_threshold: f64, // DD'nin yüzde kaçında güvenli moda geçilsin?

    /// Günlük kayıp bu yüzdeyi aşarsa HALT (yeni giriş durur). Eskiden sabit 5.0.
    #[serde(default = "default_halt_daily_loss_pct")]
    pub halt_daily_loss_pct: f64,
    /// Günlük kayıp bu yüzdeyi aşarsa safe-mode (boyut kısılır). Eskiden sabit 2.0.
    #[serde(default = "default_safe_mode_daily_loss_pct")]
    pub safe_mode_daily_loss_pct: f64,
    /// ML güven barajı — altında sinyal reddedilir. Eskiden sabit 0.35 (policy'de
    /// alanı YOKTU → yapılandırılamıyordu; ölü kopya autonomous_control.rs 0.55 tutuyordu).
    #[serde(default = "default_min_model_confidence")]
    pub min_model_confidence: f64,
}

fn default_halt_daily_loss_pct() -> f64 {
    5.0
}
fn default_safe_mode_daily_loss_pct() -> f64 {
    2.0
}
fn default_min_model_confidence() -> f64 {
    0.35
}

impl Default for RiskGatePolicy {
    fn default() -> Self {
        Self {
            max_notional_usd: 5000.0,
            max_drawdown_pct: 10.0,
            max_daily_loss_pct: 3.0,
            safe_mode_threshold: 0.8, // %80 dolulukta vites düşür
            halt_daily_loss_pct: default_halt_daily_loss_pct(),
            safe_mode_daily_loss_pct: default_safe_mode_daily_loss_pct(),
            min_model_confidence: default_min_model_confidence(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum RiskDecision {
    Allow,
    Deny {
        reasons: Vec<String>,
        enter_safe_mode: bool,
        halt: bool,
    },
}

/// İnfaz birimi tarafından hazırlanan giriş verileri
pub struct RiskInput {
    pub account_equity: f64,
    pub day_start_equity: f64,
    pub peak_equity: f64,
    pub requested_notional_usd: f64,
    pub model_confidence: f64, // ML modellerinden gelen güven skoru
}
