// Kelly Criterion - Optimal Position Size Calculator
//
// Srivastava mimarisi: Riski minimize ederken return'ü maximize etmek için
// Optimal bahis miktarı = (Win% - Loss%*(1-Win%)/Win%) / (Oran - 1)

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KellyCriterion {
    /// Kazanma olasılığı (0-1)
    pub win_probability: f64,
    
    /// Ortalama kazanç
    pub avg_win: f64,
    
    /// Ortalama kayıp
    pub avg_loss: f64,
    
    /// Kelly fraction (pozisyonun yüzdesi)
    pub kelly_fraction: f64,
}

impl KellyCriterion {
    /// Kelly Criterion hesapla
    pub fn calculate(win_probability: f64, avg_win: f64, avg_loss: f64) -> Self {
        // Validation
        let win_prob = win_probability.max(0.0).min(1.0);
        
        if avg_win <= 0.0 || avg_loss <= 0.0 {
            return Self {
                win_probability: win_prob,
                avg_win,
                avg_loss,
                kelly_fraction: 0.0,
            };
        }
        
        let loss_prob = 1.0 - win_prob;
        let win_ratio = avg_win / avg_loss;
        
        // Kelly formula: f* = (p*b - q) / b
        // p = win probability
        // q = loss probability (1-p)
        // b = win/loss ratio
        let kelly = if win_ratio > 0.0 {
            (win_prob * win_ratio - loss_prob) / win_ratio
        } else {
            0.0
        };
        
        // Negative kelly'yi 0'a sıfırla
        let kelly_fraction = kelly.max(0.0);
        
        Self {
            win_probability: win_prob,
            avg_win,
            avg_loss,
            kelly_fraction,
        }
    }
    
    /// Fractional Kelly (risk azaltmak için)
    /// Örnek: 0.25 = 1/4 Kelly
    pub fn fractional(&self, fraction: f64) -> f64 {
        self.kelly_fraction * fraction.max(0.0).min(1.0)
    }
    
    /// Position size hesapla
    pub fn calculate_position_size(&self, account_size: f64) -> f64 {
        account_size * self.kelly_fraction
    }
    
    /// Fractional position size
    pub fn calculate_fractional_position_size(&self, account_size: f64, fraction: f64) -> f64 {
        account_size * self.fractional(fraction)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KellyRecommendation {
    /// Kelly fraction %
    pub kelly_pct: f64,
    
    /// 1/2 Kelly tavsiyesi %
    pub half_kelly_pct: f64,
    
    /// 1/4 Kelly tavsiyesi %
    pub quarter_kelly_pct: f64,
    
    /// Durum tavsiyesi
    pub recommendation: String,
}

impl KellyRecommendation {
    pub fn from_kelly(kelly: &KellyCriterion) -> Self {
        let kelly_pct = kelly.kelly_fraction * 100.0;
        let half = kelly.fractional(0.5) * 100.0;
        let quarter = kelly.fractional(0.25) * 100.0;
        
        let recommendation = if kelly_pct < 0.05 {
            "Too risky - use 1/4 Kelly or lower".to_string()
        } else if kelly_pct < 0.10 {
            "Risky - recommend 1/2 Kelly".to_string()
        } else if kelly_pct < 0.25 {
            "Moderate - Full Kelly or 3/4".to_string()
        } else {
            "Very profitable - Full Kelly recommended".to_string()
        };
        
        Self {
            kelly_pct,
            half_kelly_pct: half,
            quarter_kelly_pct: quarter,
            recommendation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_kelly_criterion() {
        let kelly = KellyCriterion::calculate(
            0.6,    // 60% win rate
            100.0,  // avg win
            80.0,   // avg loss
        );
        
        assert!(kelly.kelly_fraction > 0.0);
        assert!(kelly.kelly_fraction < 1.0);
    }
    
    #[test]
    fn test_kelly_position_sizing() {
        let kelly = KellyCriterion::calculate(0.55, 100.0, 100.0);
        
        let full_size = kelly.calculate_position_size(10000.0);
        let half_size = kelly.calculate_fractional_position_size(10000.0, 0.5);
        
        assert!(half_size == full_size / 2.0);
    }
    
    #[test]
    fn test_kelly_recommendation() {
        let kelly = KellyCriterion::calculate(0.65, 150.0, 100.0);
        let rec = KellyRecommendation::from_kelly(&kelly);
        
        assert!(rec.kelly_pct > 0.0);
        assert!(rec.half_kelly_pct == rec.kelly_pct / 2.0);
    }
}
