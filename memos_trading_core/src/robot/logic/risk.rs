// risk_manager.rs - Gelişmiş Risk ve Sermaye Yönetimi Modülü

use crate::core::types::RiskParams;
use crate::Result;
use crate::robot::infra::monitoring::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

/// Risk yönetimi ve sermaye koruma merkezi
pub struct RiskManager {
    params: RiskParams,
}

impl RiskManager {
    pub fn new(params: RiskParams) -> Self {
        Self { params }
    }

    /// Stop loss seviyesini hesapla (Zero-copy)
    #[inline]
    pub fn calculate_stop_loss(&self, entry_price: f64) -> f64 {
        entry_price * (1.0 - self.params.stop_loss_pct / 100.0)
    }

    /// Take profit seviyesini hesapla
    #[inline]
    pub fn calculate_take_profit(&self, entry_price: f64) -> f64 {
        entry_price * (1.0 + self.params.take_profit_pct / 100.0)
    }

    /// Pozisyon boyutunu hesapla (Kelly, max risk, klasik)
    /// Modernize: Fonksiyonel zincirleme ve güvenli matematik.
    pub fn calculate_position_size(
        &self, 
        capital: f64, 
        entry_price: f64, 
        win_rate: Option<f64>, 
        win_loss_ratio: Option<f64>
    ) -> Result<f64> {
        if entry_price <= 0.0 { return Err("Geçersiz giriş fiyatı".into()); }

        // 1. Temel Risk Havuzu (Max Portfolio Risk)
        let max_risk_pool = self.params.max_portfolio_risk_pct
            .map_or(capital, |pct| capital * (pct / 100.0));

        // 2. Kelly Kriteri Uygulaması (Eğer aktifse)
        let kelly_factor = if self.params.use_kelly_criterion {
            match (win_rate, win_loss_ratio) {
                (Some(wr), Some(wlr)) if wlr > 0.0 => {
                    let q = 1.0 - wr;
                    let f_star = ((wlr * wr) - q) / wlr;
                    // Half-Kelly: Agresifliği azaltmak için 0.5 ile çarpılır ve [0, 1] arası clamp edilir.
                    f_star.clamp(0.0, 1.0) * 0.5
                },
                _ => {
                    log::warn!("Kelly verisi eksik, sabit risk kullanılacak.");
                    1.0 // Kelly devre dışı kalırsa havuzu tam kullan
                }
            }
        } else {
            1.0
        };

        // 3. Pozisyon Sınırı (Max Position Size)
        let mut final_amount = max_risk_pool * kelly_factor;
        
        if let Some(max_pos_pct) = self.params.max_position_size_pct {
            let max_allowed = capital * (max_pos_pct / 100.0);
            final_amount = final_amount.min(max_allowed);
        }

        Ok(final_amount / entry_price)
    }

    /// Trade başına risk edilen net nakit miktarı (R-multiple temeli)
    pub fn calculate_trade_risk(&self, capital: f64, entry_price: f64, stop_loss: f64) -> f64 {
        if entry_price <= 0.0 { return 0.0; }
        let risk_per_unit = (entry_price - stop_loss).abs();
        let total_units = capital / entry_price;
        risk_per_unit * total_units
    }
}

// --- SAĞLIK VE ANOMALİ İMPLEMENTASYONLARI ---

impl HealthCheck for RiskManager {
    fn check_health(&self) -> HealthStatus {
        match (self.params.stop_loss_pct, self.params.take_profit_pct) {
            (sl, _) if sl <= 0.0 || sl > 50.0 => {
                HealthStatus::Warning(format!("Stop loss parametresi riskli: %{}", sl))
            },
            (_, tp) if tp <= 0.0 || tp > 100.0 => {
                HealthStatus::Warning(format!("Take profit parametresi riskli: %{}", tp))
            },
            _ => HealthStatus::Healthy,
        }
    }
}

impl AnomalyDetector for RiskManager {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        // Match guards ile kritik limit kontrolü
        match self.params.max_position_size_pct {
            Some(pct) if pct > 90.0 => Some(AnomalyType::Custom(
                format!("Aşırı Pozisyon Riski: %{}", pct)
            )),
            _ => None,
        }
    }
}
