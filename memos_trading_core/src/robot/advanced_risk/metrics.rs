// Advanced Risk Metrics - Sharpe, Sortino, Calmar Ratios
//
// Srivastava mimarisi: Risk-adjusted return metrikleri

use serde::{Serialize, Deserialize};

/// Sharpe Ratio Calculator
/// Risk-adjusted return = (Ortalama return - Risk-free rate) / Standart sapma
pub struct SharpeCalculator;

impl SharpeCalculator {
    /// Sharpe Ratio hesapla
    /// returns: Dönem dönem returns (0.01 = %1)
    /// rf_rate: Risk-free rate (örnek: 0.001 = %0.1)
    pub fn calculate(returns: &[f64], rf_rate: f64) -> f64 {
        if returns.len() < 2 {
            return 0.0;
        }
        
        // Ortalama return
        let avg_return = returns.iter().sum::<f64>() / returns.len() as f64;
        
        // Standart sapma
        let variance = returns
            .iter()
            .map(|r| (r - avg_return).powi(2))
            .sum::<f64>() / returns.len() as f64;
        
        let std_dev = variance.sqrt();
        
        if std_dev == 0.0 {
            0.0
        } else {
            (avg_return - rf_rate) / std_dev
        }
    }
}

/// Sortino Ratio Calculator
/// Sharpe'ye benzer ama sadece downside volatility'yi sayar
pub struct SortinoCalculator;

impl SortinoCalculator {
    pub fn calculate(returns: &[f64], rf_rate: f64, target_return: f64) -> f64 {
        if returns.len() < 2 {
            return 0.0;
        }
        
        // Ortalama return
        let avg_return = returns.iter().sum::<f64>() / returns.len() as f64;
        
        // Downside deviation (target'ın altındaki variance)
        let downside_var = returns
            .iter()
            .map(|r| {
                let diff = target_return - r;
                if diff > 0.0 { diff.powi(2) } else { 0.0 }
            })
            .sum::<f64>() / returns.len() as f64;
        
        let downside_std = downside_var.sqrt();
        
        if downside_std == 0.0 {
            0.0
        } else {
            (avg_return - rf_rate) / downside_std
        }
    }
}

/// Calmar Ratio Calculator
/// (Ortalama yıllık return) / (Max drawdown)
pub struct CalmarCalculator;

impl CalmarCalculator {
    pub fn calculate(annual_return: f64, max_drawdown: f64) -> f64 {
        if max_drawdown == 0.0 {
            if annual_return > 0.0 { f64::INFINITY } else { 0.0 }
        } else {
            annual_return.abs() / max_drawdown.abs()
        }
    }
}

/// Information Ratio
/// (Strateji return - Benchmark return) / Tracking error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InformationRatio {
    pub excess_return: f64,
    pub tracking_error: f64,
    pub ratio: f64,
}

impl InformationRatio {
    pub fn calculate(strategy_returns: &[f64], benchmark_returns: &[f64]) -> Option<Self> {
        if strategy_returns.len() != benchmark_returns.len() || strategy_returns.len() < 2 {
            return None;
        }
        
        // Excess returns
        let excess: Vec<f64> = strategy_returns
            .iter()
            .zip(benchmark_returns.iter())
            .map(|(s, b)| s - b)
            .collect();
        
        let avg_excess = excess.iter().sum::<f64>() / excess.len() as f64;
        let te = (excess
            .iter()
            .map(|e| (e - avg_excess).powi(2))
            .sum::<f64>() / excess.len() as f64)
            .sqrt();
        
        let ratio = if te == 0.0 { 0.0 } else { avg_excess / te };
        
        Some(InformationRatio {
            excess_return: avg_excess,
            tracking_error: te,
            ratio,
        })
    }
}

/// Omega Ratio
/// Win/Loss oranını risk perspective'ten hesapla
pub struct OmegaCalculator;

impl OmegaCalculator {
    pub fn calculate(returns: &[f64], threshold: f64) -> f64 {
        let gains: f64 = returns
            .iter()
            .filter(|r| *r > &threshold)
            .map(|r| r - threshold)
            .sum();
        
        let losses: f64 = returns
            .iter()
            .filter(|r| *r < &threshold)
            .map(|r| threshold - r)
            .sum();
        
        if losses == 0.0 {
            if gains > 0.0 { f64::INFINITY } else { 0.0 }
        } else {
            gains / losses
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sharpe_ratio() {
        // Pozitif returns
        let returns = vec![0.01, 0.02, 0.015, 0.005, 0.03];
        let rf = 0.001;
        
        let sharpe = SharpeCalculator::calculate(&returns, rf);
        assert!(sharpe > 0.0);
    }
    
    #[test]
    fn test_sortino_ratio() {
        let returns = vec![0.01, 0.02, -0.005, 0.015, 0.03];
        let rf = 0.001;
        let target = 0.0;
        
        let sortino = SortinoCalculator::calculate(&returns, rf, target);
        // Sharpe'den yüksek olmalı (sadece downside sayıyor)
        let sharpe = SharpeCalculator::calculate(&returns, rf);
        assert!(sortino >= sharpe);
    }
    
    #[test]
    fn test_calmar_ratio() {
        let annual_return = 0.15; // 15%
        let max_dd = 0.05; // 5%
        
        let calmar = CalmarCalculator::calculate(annual_return, max_dd);
        assert!((calmar - 3.0).abs() < 1e-10); // Allow for floating point precision
    }
    
    #[test]
    fn test_omega_ratio() {
        let returns = vec![0.02, 0.03, 0.01, 0.05, -0.01];
        let omega = OmegaCalculator::calculate(&returns, 0.0);
        
        assert!(omega > 0.0);
    }
}
