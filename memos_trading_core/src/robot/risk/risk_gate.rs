// src/robot/risk/risk_gate.rs - Srivastava ATP Merkezi Risk Denetim Kapısı
// Bu modül, operasyonel infaz öncesi son 'Ateş Serbest' onayını verir.
use crate::prelude::*;
use serde::{Serialize, Deserialize};

#[derive(Default)] 
pub struct RiskGate {
    pub policy: RiskGatePolicy
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
        } else { 0.0 };
        
        if dd > self.policy.max_drawdown_pct {
            reasons.push(format!("Kritik Max DD aşıldı: {:.2}%", dd));
            halt = true;
        }

        // 2. Günlük Kayıp Denetimi (Gün başı sermaye üzerinden)
        let daily_loss = (input.day_start_equity - input.account_equity) / input.day_start_equity * 100.0;
        if daily_loss > self.policy.max_daily_loss_pct {
            reasons.push(format!("Günlük kayıp sınırı ihlal edildi: {:.2}%", daily_loss));
            // Günlük kayıp çok sertse (%5 üstü) sistemi tamamen durdur
            if daily_loss > 5.0 { halt = true; }
        }

        // 3. Pozisyon Büyüklüğü Denetimi
        if input.requested_notional_usd > self.policy.max_notional_usd {
            reasons.push(format!("İşlem hacmi limit dışı: ${:.2}", input.requested_notional_usd));
        }

        // 4. ML Güven Barajı (Yeni Derinlik)
        if input.model_confidence < 0.35 {
            reasons.push("ML Model güven seviyesi yetersiz".to_string());
        }

        if reasons.is_empty() {
            RiskDecision::Allow
        } else {
            // Karar Verici: Safe mode mu yoksa tam durdurma mı?
            RiskDecision::Deny {
                reasons,
                enter_safe_mode: dd > (self.policy.max_drawdown_pct * self.policy.safe_mode_threshold) || daily_loss > 2.0,
                halt,
            }
        }
    }
    pub fn is_approved(&self, _sig: &Signal) -> bool { true }

}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskGatePolicy {
    pub max_notional_usd: f64,
    pub max_drawdown_pct: f64,
    pub max_daily_loss_pct: f64,
    pub safe_mode_threshold: f64, // DD'nin yüzde kaçında güvenli moda geçilsin?
}

impl Default for RiskGatePolicy {
    fn default() -> Self {
        Self {
            max_notional_usd: 5000.0,
            max_drawdown_pct: 10.0,
            max_daily_loss_pct: 3.0,
            safe_mode_threshold: 0.8, // %80 dolulukta vites düşür
        }
    }
}

#[derive(Debug, Clone)]
pub enum RiskDecision {
    Allow,
    Deny { 
        reasons: Vec<String>, 
        enter_safe_mode: bool, 
        halt: bool 
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