// robot/order_management/validator.rs - Trade order validation

use crate::types::Trade;
use crate::Result;

/// Pre-trade validation rules
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
            max_position_size_pct: 5.0,
            max_daily_loss_pct: 2.0,
            min_risk_reward_ratio: 1.5,
            max_consecutive_losses: 3,
            require_stop_loss: true,
            require_take_profit: true,
        }
    }
}

/// Order validator
pub struct OrderValidator {
    rules: ValidationRules,
}

impl OrderValidator {
    pub fn new(rules: ValidationRules) -> Self {
        Self { rules }
    }

    pub fn with_defaults() -> Self {
        Self {
            rules: ValidationRules::default(),
        }
    }

    pub fn validate_position_size(
        &self,
        account_balance: f64,
        position_size: f64,
    ) -> Result<()> {
        let position_pct = (position_size / account_balance) * 100.0;
        
        if position_pct > self.rules.max_position_size_pct {
            return Err("Position size exceeds maximum".into());
        }
        
        Ok(())
    }

    pub fn validate_risk_reward(
        &self,
        _entry_price: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    ) -> Result<()> {
        if self.rules.require_stop_loss && stop_loss.is_none() {
            return Err("Stop loss required".into());
        }
        
        if self.rules.require_take_profit && take_profit.is_none() {
            return Err("Take profit required".into());
        }

        if let (Some(sl), Some(tp)) = (stop_loss, take_profit) {
            let risk = (_entry_price - sl).abs();
            let reward = (tp - _entry_price).abs();
            
            if risk > 0.0 {
                let ratio = reward / risk;
                if ratio < self.rules.min_risk_reward_ratio {
                    return Err("Risk/Reward ratio too low".into());
                }
            }
        }

        Ok(())
    }

    pub fn validate_daily_loss(
        &self,
        account_balance: f64,
        daily_loss: f64,
    ) -> Result<()> {
        let loss_pct = (daily_loss / account_balance) * 100.0;
        
        if loss_pct > self.rules.max_daily_loss_pct {
            return Err("Daily loss exceeds maximum".into());
        }
        
        Ok(())
    }

    pub fn validate_consecutive_losses(
        &self,
        recent_trades: &[Trade],
    ) -> Result<()> {
        let consecutive_losses = recent_trades
            .iter()
            .rev()
            .take_while(|t| t.pnl.map_or(false, |p| p < 0.0))
            .count();
        
        if consecutive_losses > self.rules.max_consecutive_losses {
            return Err("Too many consecutive losses".into());
        }
        
        Ok(())
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_size() {
        let v = OrderValidator::with_defaults();
        assert!(v.validate_position_size(10000.0, 300.0).is_ok());
        assert!(v.validate_position_size(10000.0, 1000.0).is_err());
    }

    #[test]
    fn test_stop_loss_required() {
        let v = OrderValidator::with_defaults();
        assert!(v.validate_risk_reward(100.0, None, Some(115.0)).is_err());
    }

    #[test]
    fn test_daily_loss() {
        let v = OrderValidator::with_defaults();
        assert!(v.validate_daily_loss(10000.0, 100.0).is_ok());
        assert!(v.validate_daily_loss(10000.0, 300.0).is_err());
    }
}
