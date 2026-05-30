// robot/order_management/validator.rs - Otonom Emir Validasyonu ve Risk Bariyeri

use crate::core::types::Trade;
use crate::Result;

/// Validasyon Kuralları: Otonom sistemin kırmızı çizgileri.
#[derive(Debug, Clone)]
pub struct ValidationRules {
    pub max_position_size_pct: f64,
    pub max_daily_loss_pct: f64,
    pub min_risk_reward_ratio: f64,
    pub max_consecutive_losses: usize,
    pub require_stop_loss: bool,
    pub require_take_profit: bool,
}

impl Default for ValidationRules {
    fn default() -> Self {
        Self {
            max_position_size_pct: 5.0,  // Tek işlemde maks %5 bakiye riski
            max_daily_loss_pct: 2.0,     // Günlük maks %2 özkaynak kaybı
            min_risk_reward_ratio: 1.5,  // Minimum 1:1.5 R/R oranı
            max_consecutive_losses: 3,   // 3 ardışık zararda dur
            require_stop_loss: true,
            require_take_profit: true,
        }
    }
}

/// OrderValidator: İşlem öncesi tüm matematiksel ve finansal engelleri denetler.
pub struct OrderValidator {
    pub rules: ValidationRules,
}

impl OrderValidator {
    pub fn new(rules: ValidationRules) -> Self {
        Self { rules }
    }

    pub fn with_defaults() -> Self {
        Self { rules: ValidationRules::default() }
    }

    /// Pozisyon boyutunun hesap büyüklüğüne oranını otonom denetler.
    pub fn validate_position_size(&self, account_balance: f64, position_size: f64) -> Result<()> {
        if account_balance <= 0.0 { return Err("Hesap bakiyesi yetersiz veya sıfır".into()); }
        let position_pct = (position_size / account_balance) * 100.0;
        
        if position_pct > self.rules.max_position_size_pct {
            return Err(format!("Pozisyon boyutu sınırı aşıldı: %{:.2} > %{:.2}", 
                position_pct, self.rules.max_position_size_pct).into());
        }
        Ok(())
    }

    /// Risk/Reward (R/R) oranını otonom matematiksel olarak doğrular.
    pub fn validate_risk_reward(
        &self,
        entry_price: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    ) -> Result<()> {
        if self.rules.require_stop_loss && stop_loss.is_none() {
            return Err("Stop-Loss (SL) zorunludur".into());
        }
        if self.rules.require_take_profit && take_profit.is_none() {
            return Err("Take-Profit (TP) zorunludur".into());
        }

        if let (Some(sl), Some(tp)) = (stop_loss, take_profit) {
            let risk = (entry_price - sl).abs();
            let reward = (tp - entry_price).abs();
            
            if risk > 0.0 {
                let ratio = reward / risk;
                if ratio < self.rules.min_risk_reward_ratio {
                    return Err(format!("Düşük R/R Oranı: {:.2} < {:.2}", 
                        ratio, self.rules.min_risk_reward_ratio).into());
                }
            } else {
                return Err("Geçersiz SL seviyesi: Risk sıfır olamaz".into());
            }
        }
        Ok(())
    }

    /// Günlük kümülatif kaybı otonom denetler.
    pub fn validate_daily_loss(&self, account_balance: f64, daily_loss: f64) -> Result<()> {
        if account_balance <= 0.0 { return Ok(()); }
        let loss_pct = (daily_loss.abs() / account_balance) * 100.0;
        
        if loss_pct > self.rules.max_daily_loss_pct {
            return Err(format!("Günlük kayıp sınırı aşıldı: %{:.2}", loss_pct).into());
        }
        Ok(())
    }

    /// Ardışık zarar (Loss Streak) durumunda otonom blokaj uygular.
    pub fn validate_consecutive_losses(&self, recent_trades: &[Trade]) -> Result<()> {
        let consecutive_losses = recent_trades
            .iter()
            .rev()
            // &&Trade karmaşasını çözmek için dereference yapıyoruz veya doğrudan erişiyoruz
            // 'realized_pnl' yerine 'pnl' alanını kullanıyoruz (types.rs ile uyumlu)
            .take_while(|t| t.pnl.is_some_and(|p| p < 0.0))
            .count();
        
        if consecutive_losses >= self.rules.max_consecutive_losses {
            return Err(format!(
                "{} ardışık zarar sonrası otonom blokaj aktif (Limit: {})", 
                consecutive_losses, self.rules.max_consecutive_losses
            ).into());
        }
        
        Ok(())
    }

    /// Merkezi Validasyon: Tüm kuralları tek seferde otonom koordine eder.
    /// robotic_loop içindeki 'can_open_position' kontrolünün yerini alır.
    pub fn validate(
        &self,
        account_balance: f64,
        position_size: f64,
        entry_price: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
        daily_loss: f64,
        recent_trades: &[Trade],
    ) -> Result<()> {
        self.validate_position_size(account_balance, position_size)?;
        self.validate_risk_reward(entry_price, stop_loss, take_profit)?;
        self.validate_daily_loss(account_balance, daily_loss)?;
        self.validate_consecutive_losses(recent_trades)?;
        Ok(())
    }
}
