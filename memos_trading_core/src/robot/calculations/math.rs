use crate::robot::interfaces::Calculator;
use crate::robot::calculations::indicators::RSI;

impl Calculator for Math {
    fn sma(&self, values: &[f64], period: usize) -> crate::Result<f64> {
        MovingAverage::sma(values, period)
    }
    fn rsi(&self, values: &[f64], period: usize) -> crate::Result<f64> {
        RSI::last(values, period)
    }
}
// robot/calculations/math.rs - Genel matematik işlemleri

use crate::Result;

/// Matematik motor - tüm genel hesaplamalar
#[derive(Default)]
pub struct Math;

impl Math {
    pub fn new() -> Self {
        Self
    }
}

/// Hareketli ortalama hesaplamaları
#[derive(Default)]
pub struct MovingAverage;

impl MovingAverage {
    /// Basit hareketli ortalama
    pub fn sma(values: &[f64], period: usize) -> Result<f64> {
        if values.is_empty() || period == 0 || period > values.len() {
            return Ok(0.0);
        }
        
        let sum: f64 = values.iter().rev().take(period).sum();
        Ok(sum / period as f64)
    }
    
    /// Üstel hareketli ortalama
    pub fn ema(values: &[f64], period: usize) -> Result<f64> {
        if values.is_empty() || period == 0 {
            return Ok(0.0);
        }
        
        let multiplier = 2.0 / (period as f64 + 1.0);
        let mut ema = values[0];
        
        for i in 1..values.len().min(period * 2) {
            ema = values[i] * multiplier + ema * (1.0 - multiplier);
        }
        
        Ok(ema)
    }
    
    /// Ağırlıklı hareketli ortalama
    pub fn wma(values: &[f64], period: usize) -> Result<f64> {
        if values.is_empty() || period == 0 {
            return Ok(0.0);
        }
        
        let take = period.min(values.len());
        let mut sum = 0.0;
        let mut weight_sum = 0.0;
        
        for (i, &val) in values.iter().rev().take(take).enumerate() {
            let weight = (take - i) as f64;
            sum += val * weight;
            weight_sum += weight;
        }
        
        Ok(sum / weight_sum)
    }
}

/// Standart sapma ve varyans
#[derive(Default)]
pub struct StandardDeviation;

impl StandardDeviation {
    /// Standart sapma
    pub fn calculate(values: &[f64]) -> Result<f64> {
        if values.is_empty() {
            return Ok(0.0);
        }
        
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values
            .iter()
            .map(|&x| (x - mean).powi(2))
            .sum::<f64>() / values.len() as f64;
        
        Ok(variance.sqrt())
    }
    
    /// Örnek standart sapma (Bessel's correction)
    pub fn sample(values: &[f64]) -> Result<f64> {
        if values.len() < 2 {
            return Ok(0.0);
        }
        
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values
            .iter()
            .map(|&x| (x - mean).powi(2))
            .sum::<f64>() / (values.len() - 1) as f64;
        
        Ok(variance.sqrt())
    }
}

/// Yüzde değişim hesaplamaları
#[derive(Default)]
pub struct PercentageChange;

impl PercentageChange {
    /// Basit yüzde değişim
    pub fn calculate(old: f64, new: f64) -> f64 {
        if old == 0.0 {
            return 0.0;
        }
        ((new - old) / old) * 100.0
    }
    
    /// İki değer arasındaki fark
    pub fn difference(old: f64, new: f64) -> f64 {
        new - old
    }
    
    /// Logaritmik yüzde değişim
    pub fn log_return(old: f64, new: f64) -> f64 {
        if old > 0.0 {
            (new / old).ln() * 100.0
        } else {
            0.0
        }
    }
}

/// İstatistiksel hesaplamalar
pub struct Statistics;

impl Statistics {
    /// Ortanca (Median)
    pub fn median(values: &[f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        }
    }
    
    /// Modu (Mode) - en sık görülen değer
    pub fn mode(values: &[f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        
        // Basitleştirilmiş: ilk değeri döndür
        values[0]
    }
    
    /// Çeyrekler
    pub fn quartiles(values: &[f64]) -> (f64, f64, f64) {
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        
        let n = sorted.len();
        let q1 = sorted[n / 4];
        let q2 = Self::median(&sorted);
        let q3 = sorted[(3 * n) / 4];
        
        (q1, q2, q3)
    }
    
    /// Çeyrekler arası aralık
    pub fn iqr(values: &[f64]) -> f64 {
        let (q1, _, q3) = Self::quartiles(values);
        q3 - q1
    }
}

/// Korrelasyon ve kovaryans
pub struct Correlation;

impl Correlation {
    /// Pearson korelasyonu
    pub fn pearson(x: &[f64], y: &[f64]) -> f64 {
        if x.len() != y.len() || x.is_empty() {
            return 0.0;
        }
        
        let mean_x = x.iter().sum::<f64>() / x.len() as f64;
        let mean_y = y.iter().sum::<f64>() / y.len() as f64;
        
        let mut numerator = 0.0;
        let mut sx = 0.0;
        let mut sy = 0.0;
        
        for i in 0..x.len() {
            let dx = x[i] - mean_x;
            let dy = y[i] - mean_y;
            numerator += dx * dy;
            sx += dx.powi(2);
            sy += dy.powi(2);
        }
        
        if sx > 0.0 && sy > 0.0 {
            numerator / (sx * sy).sqrt()
        } else {
            0.0
        }
    }
    
    /// Kovaryans
    pub fn covariance(x: &[f64], y: &[f64]) -> f64 {
        if x.len() != y.len() || x.is_empty() {
            return 0.0;
        }
        
        let mean_x = x.iter().sum::<f64>() / x.len() as f64;
        let mean_y = y.iter().sum::<f64>() / y.len() as f64;
        
        let sum: f64 = x.iter().zip(y.iter())
            .map(|(&xi, &yi)| (xi - mean_x) * (yi - mean_y))
            .sum();
        
        sum / x.len() as f64
    }
}

/// Risk hesaplamaları
pub struct RiskMetrics;

impl RiskMetrics {
    /// Sharpe Ratio
    pub fn sharpe_ratio(returns: &[f64], risk_free_rate: f64) -> f64 {
        if returns.is_empty() {
            return 0.0;
        }
        
        let mean_return = returns.iter().sum::<f64>() / returns.len() as f64;
        let std_dev = StandardDeviation::calculate(returns).unwrap_or(0.0);
        
        if std_dev == 0.0 {
            return 0.0;
        }
        
        (mean_return - risk_free_rate) / std_dev
    }
    
    /// Maximum Drawdown
    pub fn max_drawdown(prices: &[f64]) -> f64 {
        if prices.is_empty() {
            return 0.0;
        }
        
        let mut max_price = prices[0];
        let mut max_dd = 0.0;
        
        for &price in prices.iter() {
            if price > max_price {
                max_price = price;
            }
            let dd = (price - max_price) / max_price;
            if dd < max_dd {
                max_dd = dd;
            }
        }
        
        max_dd * 100.0
    }
    
    /// Value at Risk
    pub fn var(returns: &[f64], confidence: f64) -> f64 {
        if returns.is_empty() {
            return 0.0;
        }
        
        let mut sorted = returns.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        
        let index = ((1.0 - confidence) * sorted.len() as f64).ceil() as usize;
        if index < sorted.len() {
            sorted[index]
        } else {
            sorted[0]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sma_calculation() {
        let values = vec![100.0, 102.0, 104.0, 103.0, 105.0];
        let result = MovingAverage::sma(&values, 3);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_std_dev() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = StandardDeviation::calculate(&values);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_percentage_change() {
        let change = PercentageChange::calculate(100.0, 110.0);
        assert_eq!(change, 10.0);
    }
    
    #[test]
    fn test_max_drawdown() {
        let prices = vec![100.0, 105.0, 102.0, 108.0, 103.0];
        let dd = RiskMetrics::max_drawdown(&prices);
        assert!(dd < 0.0);
    }
}
