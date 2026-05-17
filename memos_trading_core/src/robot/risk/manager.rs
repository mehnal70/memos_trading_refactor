// src/robot/risk/manager.rs - Merkezi Risk İnfaz ve Otorizasyon Motoru
// Srivastava ATP - İşlevsel Çarklar Odası

use crate::prelude::*;
use super::risk_gate::RiskGate;
use super::guardrails::Guardrails;
use super::kelly::KellyCalculator;
use super::var::VarEngine;

/// Srivastava ATP - Entegre Risk Yönetim Merkezi
pub struct RiskManager {
    pub gate: RiskGate,
    pub limits: Guardrails,
    pub kelly: KellyCalculator,
    pub var_engine: VarEngine,
}

impl Default for RiskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskManager {
    pub fn new() -> Self {
        Self {
            gate: RiskGate::default(),
            limits: Guardrails::default(),
            kelly: KellyCalculator::default(),
            var_engine: VarEngine::default(),
        }
    }

    /// ⚔️ NİHAİ OTORİZASYON: Bir işlemin borsaya gidip gitmeyeceğine karar verir.
    /// Tüm alt süzgeçler (Bakanlık onayları) 'true' dönmek zorundadır.
    pub fn authorize(&self, signal: &Signal, snap: &MissionControl) -> bool {
        // 1. Statik Limitler (Bakiye, Kaldıraç Sınırı vb. - Guardrails)
        if !self.limits.check_safety(signal) { 
            crate::robot::infra::reporting::reporting::ErrorLogger::log_error("RISK_GATE", "İşlem statik limit engeline (Guardrails) takıldı.");
            return false; 
        }

        // 2. Kelly Sermaye Kontrolü (Otonom kasa yönetimi uygun mu?)
        if !self.kelly.validate_allocation(signal, snap.finance.total_equity) { 
            crate::robot::infra::reporting::reporting::ErrorLogger::log_error("RISK_KELLY", "Kelly sermaye tahsis kontrolü işlemi veto etti.");
            return false; 
        }

        // 3. Stratejik Onay (RiskGate - GBT ve Trend rejim uyumu)
        if !self.gate.is_approved(signal) { 
            crate::robot::infra::reporting::reporting::ErrorLogger::log_error("RISK_STRATEGY", "RiskGate stratejik rejim/GBT uyumsuzluğu saptadı.");
            return false; 
        }

        // 4. VaR (Value at Risk - Portföy kümülatif batma riski analizi)
        if !self.var_engine.check_exposure(snap) { 
            crate::robot::infra::reporting::reporting::ErrorLogger::log_error("RISK_VaR", "Kümülatif portföy maruziyeti (Value at Risk) sınır dışı.");
            return false; 
        }

        true // TÜM BARİKATLAR GEÇİLDİ: ATEŞ SERBEST! 🚀
    }
}
