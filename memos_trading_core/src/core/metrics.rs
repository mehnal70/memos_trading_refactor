// src/core/metrics.rs - Srivastava ATP Merkezi Finansal Analiz Birimi
use serde::{Serialize, Deserialize};

// --- CALCULATORS (Matematiksel Motorlar) ---

/// Sharpe Rasyosu - Risk-adjusted getiri analizi
pub struct SharpeCalculator;
impl SharpeCalculator {
    pub fn calculate(returns: &[f64], rf_rate: f64) -> f64 {
        let n = returns.len();
        if n < 2 { return 0.0; }
        
        let avg = returns.iter().sum::<f64>() / n as f64;
        let variance = returns.iter()
            .map(|&r| (r - avg).powi(2))
            .sum::<f64>() / n as f64;
        
        let std_dev = variance.sqrt();
        
        if std_dev < f64::EPSILON { 0.0 } else { (avg - rf_rate) / std_dev }
    }
}

/// Sortino Rasyosu - Sadece negatif volatilite odaklı risk ölçümü
pub struct SortinoCalculator;
impl SortinoCalculator {
    pub fn calculate(returns: &[f64], rf_rate: f64, target: f64) -> f64 {
        let n = returns.len();
        if n < 2 { return 0.0; }
        
        let avg = returns.iter().sum::<f64>() / n as f64;
        let downside_var = returns.iter()
            .map(|&r| if r < target { (target - r).powi(2) } else { 0.0 })
            .sum::<f64>() / n as f64;
        
        let downside_std = downside_var.sqrt();
        
        if downside_std < f64::EPSILON { 0.0 } else { (avg - rf_rate) / downside_std }
    }
}

/// Calmar Rasyosu - Getiri / Maksimum Çekilme oranı
pub struct CalmarCalculator;
impl CalmarCalculator {
    pub fn calculate(annual_return: f64, max_drawdown: f64) -> f64 {
        let dd_abs = max_drawdown.abs();
        if dd_abs < f64::EPSILON {
            if annual_return > 0.0 { 999.99 } else { 0.0 }
        } else {
            annual_return / dd_abs
        }
    }
}

// --- SCORECARD (Performans Karnesi) ---

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PerformanceScorecard {
    pub sharpe: f64,
    pub sortino: f64,
    pub calmar: f64,
    pub omega: f64,
    pub win_rate: f64,
    pub total_trades: usize,
}

impl PerformanceScorecard {
    /// Ham verilerden komple performans karnesini üretir
    pub fn generate(returns: &[f64], rf_rate: f64, annual_return: f64, max_dd: f64) -> Self {
        Self {
            sharpe: SharpeCalculator::calculate(returns, rf_rate),
            sortino: SortinoCalculator::calculate(returns, rf_rate, 0.0),
            calmar: CalmarCalculator::calculate(annual_return, max_dd),
            win_rate: 0.0, // Gerektiğinde eklenecek
            total_trades: returns.len(),
            ..Default::default()
        }
    }
}
