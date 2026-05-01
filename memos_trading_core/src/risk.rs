use crate::types::RiskParams;
use crate::Result;
use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

/// Risk yönetimi
pub struct RiskManager {
    params: RiskParams,
}

impl RiskManager {
    pub fn new(params: RiskParams) -> Self {
        Self { params }
    }

    /// Stop loss seviyesini hesapla
    pub fn calculate_stop_loss(&self, entry_price: f64) -> f64 {
        entry_price * (1.0 - self.params.stop_loss_pct / 100.0)
    }

    /// Take profit seviyesini hesapla
    pub fn calculate_take_profit(&self, entry_price: f64) -> f64 {
        entry_price * (1.0 + self.params.take_profit_pct / 100.0)
    }

    /// Pozisyon boyutunu hesapla (Kelly, max risk, klasik)
    pub fn calculate_position_size(&self, capital: f64, entry_price: f64, win_rate: Option<f64>, win_loss_ratio: Option<f64>) -> Result<f64> {
        // Kelly kriteri kullanılacaksa
        if self.params.use_kelly_criterion {
            if let (Some(wr), Some(wlr)) = (win_rate, win_loss_ratio) {
                // Kelly formülü: f* = (bp - q)/b
                // b = win_loss_ratio, p = win_rate, q = 1-p
                let b = wlr;
                let p = wr;
                let q = 1.0 - p;
                let kelly_fraction = ((b * p) - q) / b;
                let kelly_fraction = kelly_fraction.max(0.0).min(1.0) * 0.5; // Half-Kelly: tam Kelly aşırı agresif olduğundan yarıya indirilir
                let max_risk = if let Some(max_portfolio_risk) = self.params.max_portfolio_risk_pct {
                    capital * (max_portfolio_risk / 100.0)
                } else {
                    capital
                };
                return Ok((max_risk * kelly_fraction) / entry_price);
            } else {
                log::warn!("Kelly kriteri için win_rate ve win_loss_ratio gereklidir, klasik hesaplama yapılacak.");
            }
        }
        // Maksimum portföy riski uygulanacaksa
        let max_risk = if let Some(max_portfolio_risk) = self.params.max_portfolio_risk_pct {
            capital * (max_portfolio_risk / 100.0)
        } else {
            capital
        };
        if let Some(max_pct) = self.params.max_position_size_pct {
            let max_amount = capital * (max_pct / 100.0);
            Ok(max_amount.min(max_risk) / entry_price)
        } else {
            Ok(max_risk / entry_price)
        }
    }

    /// Trade başına risk edilen miktarı hesapla (R-multiple)
    pub fn calculate_trade_risk(&self, capital: f64, entry_price: f64, stop_loss: f64) -> f64 {
        let risk_per_unit = (entry_price - stop_loss).abs();
        let position_size = capital / entry_price;
        risk_per_unit * position_size
    }
}

// RiskManager için HealthCheck ve AnomalyDetector trait implementasyonları
impl HealthCheck for RiskManager {
    fn check_health(&self) -> HealthStatus {
        // Basit örnek: stop_loss_pct ve take_profit_pct makul aralıkta mı?
        if self.params.stop_loss_pct <= 0.0 || self.params.stop_loss_pct > 50.0 {
            HealthStatus::Warning(format!("Stop loss yüzdesi anormal: {}", self.params.stop_loss_pct))
        } else if self.params.take_profit_pct <= 0.0 || self.params.take_profit_pct > 100.0 {
            HealthStatus::Warning(format!("Take profit yüzdesi anormal: {}", self.params.take_profit_pct))
        } else {
            HealthStatus::Healthy
        }
    }
}

impl AnomalyDetector for RiskManager {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        // Basit örnek: max_position_size_pct çok yüksekse anomali
        if let Some(max_pct) = self.params.max_position_size_pct {
            if max_pct > 90.0 {
                return Some(AnomalyType::Custom(format!("Pozisyon boyutu yüzdesi çok yüksek: {}", max_pct)));
            }
        }
        None
    }
}
