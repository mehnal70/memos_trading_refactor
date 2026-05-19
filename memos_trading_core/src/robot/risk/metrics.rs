// advanced_risk_metrics.rs - Risk-Adjusted Performans Analiz Motoru
use crate::prelude::*;
use serde::{Serialize, Deserialize};

/// Sharpe Rasyosu - Risk-adjusted getiri (O(n) performans)
pub struct SharpeCalculator;

impl SharpeCalculator {
    /// Sharpe — sample (n-1) varyansı (Bessel düzeltmesi). Trading standardı.
    pub fn calculate(returns: &[f64], rf_rate: f64) -> f64 {
        let n = returns.len();
        if n < 2 { return 0.0; }

        let avg_return = returns.iter().sum::<f64>() / n as f64;
        let variance_sum: f64 = returns.iter()
            .map(|&r| (r - avg_return).powi(2))
            .sum();
        let std_dev = (variance_sum / (n - 1) as f64).sqrt();

        if std_dev < f64::EPSILON { 0.0 } else { (avg_return - rf_rate) / std_dev }
    }
}

/// Sortino Rasyosu - Sadece negatif volatilite odaklı risk ölçümü
pub struct SortinoCalculator;

impl SortinoCalculator {
    /// Sortino — sample (n-1) downside deviation (sadece hedef altı sapma).
    pub fn calculate(returns: &[f64], rf_rate: f64, target_return: f64) -> f64 {
        let n = returns.len();
        if n < 2 { return 0.0; }

        let avg_return = returns.iter().sum::<f64>() / n as f64;
        let downside_var_sum: f64 = returns.iter()
            .map(|&r| {
                let diff = target_return - r;
                if diff > 0.0 { diff.powi(2) } else { 0.0 }
            })
            .sum();
        let downside_std = (downside_var_sum / (n - 1) as f64).sqrt();

        if downside_std < f64::EPSILON { 0.0 } else { (avg_return - rf_rate) / downside_std }
    }
}

/// Calmar Rasyosu - Getiri / Maksimum Çekilme (Kriz dayanıklılığı)
pub struct CalmarCalculator;

impl CalmarCalculator {
    #[inline]
    pub fn calculate(annual_return: f64, max_drawdown: f64) -> f64 {
        let dd_abs = max_drawdown.abs();
        if dd_abs < f64::EPSILON {
            if annual_return > 0.0 { f64::INFINITY } else { 0.0 }
        } else {
            annual_return / dd_abs
        }
    }
}

/// Information Ratio - Benchmark'a göre göreceli başarı
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct InformationRatio {
    pub excess_return: f64,
    pub tracking_error: f64,
    pub ratio: f64,
}

impl InformationRatio {
    pub fn calculate(strategy_returns: &[f64], benchmark_returns: &[f64]) -> Option<Self> {
        let n = strategy_returns.len();
        if n != benchmark_returns.len() || n < 2 { return None; }
        
        // Zero-allocation excess return iterasyonu
        let avg_excess: f64 = strategy_returns.iter()
            .zip(benchmark_returns)
            .map(|(s, b)| s - b)
            .sum::<f64>() / n as f64;
            
        let te_sum: f64 = strategy_returns.iter()
            .zip(benchmark_returns)
            .map(|(s, b)| ((s - b) - avg_excess).powi(2))
            .sum();

        // Sample (Bessel) düzeltmesi — tracking error standardı.
        let tracking_error = (te_sum / (n - 1) as f64).sqrt();
        let ratio = if tracking_error < f64::EPSILON { 0.0 } else { avg_excess / tracking_error };
        
        Some(InformationRatio { excess_return: avg_excess, tracking_error, ratio })
    }
}

/// Omega Rasyosu - Kazanma/Kaybetme olasılık dengesi
pub struct OmegaCalculator;

impl OmegaCalculator {
    pub fn calculate(returns: &[f64], threshold: f64) -> f64 {
        // Tek bir döngüde kazanımları ve kayıpları topla (O(n))
        let (gains, losses) = returns.iter().fold((0.0, 0.0), |(g, l), &r| {
            if r > threshold { (g + (r - threshold), l) }
            else if r < threshold { (g, l + (threshold - r)) }
            else { (g, l) }
        });
        
        if losses < f64::EPSILON {
            if gains > 0.0 { f64::INFINITY } else { 0.0 }
        } else {
            gains / losses
        }
    }
}
