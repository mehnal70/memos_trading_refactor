// Fitness Evaluator - Performans Değerlendirici
// Multi-objective fitness: Kar + Risk + Tutarlılık

use serde::{Deserialize, Serialize};

/// Fitness skoru - strateji başarısını ölçer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitnessScore {
    /// Toplam fitness (0-150)
    pub total: f64,
    
    /// Bileşenler
    pub profit_component: f64,
    pub risk_component: f64,
    pub consistency_component: f64,
    pub sharpe_component: f64,
    pub survival_bonus: f64,
}

/// Performans metrikleri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Toplam trade sayısı
    pub trade_count: usize,
    
    /// Kazanan trade sayısı
    pub winning_trades: usize,
    
    /// Kaybeden trade sayısı
    pub losing_trades: usize,
    
    /// Win rate (%)
    pub win_rate: f64,
    
    /// Toplam kar/zarar (%)
    pub total_pnl_pct: f64,
    
    /// Ortalama kazanç (%)
    pub avg_win_pct: f64,
    
    /// Ortalama kayıp (%)
    pub avg_loss_pct: f64,
    
    /// Profit factor (toplam kazanç / toplam kayıp)
    pub profit_factor: f64,
    
    /// Maximum drawdown (%)
    pub max_drawdown_pct: f64,
    
    /// Sharpe ratio (risk-adjusted return)
    pub sharpe_ratio: f64,
    
    /// Sortino ratio (downside risk-adjusted return)
    pub sortino_ratio: f64,
    
    /// Calmar ratio (return / max drawdown)
    pub calmar_ratio: f64,
    
    /// Recovery factor (net profit / max drawdown)
    pub recovery_factor: f64,
    
    /// Maksimum ardışık kazanç
    pub max_consecutive_wins: usize,
    
    /// Maksimum ardışık kayıp
    pub max_consecutive_losses: usize,
    
    /// Ortalama trade süresi (bar cinsinden)
    pub avg_trade_duration: f64,
}

impl PerformanceMetrics {
    /// Trade sonuçlarından metrik hesapla
    pub fn from_trade_results(trade_pnl_pcts: &[f64]) -> Self {
        if trade_pnl_pcts.is_empty() {
            return Self::default();
        }
        
        let trade_count = trade_pnl_pcts.len();
        let winning_trades = trade_pnl_pcts.iter().filter(|&&pnl| pnl > 0.0).count();
        let losing_trades = trade_pnl_pcts.iter().filter(|&&pnl| pnl < 0.0).count();
        
        let win_rate = (winning_trades as f64 / trade_count as f64) * 100.0;
        let total_pnl_pct: f64 = trade_pnl_pcts.iter().sum();
        
        let wins: Vec<f64> = trade_pnl_pcts.iter().filter(|&&p| p > 0.0).copied().collect();
        let losses: Vec<f64> = trade_pnl_pcts.iter().filter(|&&p| p < 0.0).copied().collect();
        
        let avg_win_pct = if !wins.is_empty() {
            wins.iter().sum::<f64>() / wins.len() as f64
        } else {
            0.0
        };
        
        let avg_loss_pct = if !losses.is_empty() {
            losses.iter().sum::<f64>() / losses.len() as f64
        } else {
            0.0
        };
        
        let total_wins: f64 = wins.iter().sum();
        let total_losses: f64 = losses.iter().sum::<f64>().abs();
        let profit_factor = if total_losses > 0.0 {
            total_wins / total_losses
        } else {
            0.0
        };
        
        // Drawdown hesapla
        let mut cumulative_pnl = 0.0;
        let mut peak = 0.0;
        let mut max_drawdown = 0.0;
        
        for &pnl in trade_pnl_pcts {
            cumulative_pnl += pnl;
            if cumulative_pnl > peak {
                peak = cumulative_pnl;
            }
            let drawdown = peak - cumulative_pnl;
            if drawdown > max_drawdown {
                max_drawdown = drawdown;
            }
        }
        
        // Sharpe ratio hesapla
        let mean_return = total_pnl_pct / trade_count as f64;
        let variance = trade_pnl_pcts.iter()
            .map(|&p| (p - mean_return).powi(2))
            .sum::<f64>() / trade_count as f64;
        let std_dev = variance.sqrt();
        let sharpe_ratio = if std_dev > 0.0 {
            mean_return / std_dev
        } else {
            0.0
        };
        
        // Sortino ratio (sadece downside volatility)
        let downside_returns: Vec<f64> = trade_pnl_pcts.iter()
            .filter(|&&p| p < mean_return)
            .copied()
            .collect();
        
        let downside_variance = if !downside_returns.is_empty() {
            downside_returns.iter()
                .map(|&p| (p - mean_return).powi(2))
                .sum::<f64>() / downside_returns.len() as f64
        } else {
            1.0
        };
        
        let downside_std_dev = downside_variance.sqrt();
        let sortino_ratio = if downside_std_dev > 0.0 {
            mean_return / downside_std_dev
        } else {
            0.0
        };
        
        // Calmar ratio (annual return / max drawdown)
        let calmar_ratio = if max_drawdown > 0.0 {
            total_pnl_pct / max_drawdown
        } else {
            0.0
        };
        
        // Recovery factor
        let recovery_factor = if max_drawdown > 0.0 {
            total_pnl_pct / max_drawdown
        } else {
            0.0
        };
        
        // Ardışık kazanç/kayıp
        let (max_consecutive_wins, max_consecutive_losses) = 
            Self::calculate_consecutive_streaks(trade_pnl_pcts);
        
        Self {
            trade_count,
            winning_trades,
            losing_trades,
            win_rate,
            total_pnl_pct,
            avg_win_pct,
            avg_loss_pct,
            profit_factor,
            max_drawdown_pct: max_drawdown,
            sharpe_ratio,
            sortino_ratio,
            calmar_ratio,
            recovery_factor,
            max_consecutive_wins,
            max_consecutive_losses,
            avg_trade_duration: 0.0, // Bu başka bir yerden hesaplanacak
        }
    }
    
    /// Ardışık kazanç/kayıp serilerini hesapla
    fn calculate_consecutive_streaks(trade_pnl_pcts: &[f64]) -> (usize, usize) {
        let mut max_wins = 0;
        let mut max_losses = 0;
        let mut current_wins = 0;
        let mut current_losses = 0;
        
        for &pnl in trade_pnl_pcts {
            if pnl > 0.0 {
                current_wins += 1;
                current_losses = 0;
                max_wins = max_wins.max(current_wins);
            } else if pnl < 0.0 {
                current_losses += 1;
                current_wins = 0;
                max_losses = max_losses.max(current_losses);
            }
        }
        
        (max_wins, max_losses)
    }
    
    /// Fitness skoruna dönüştür
    pub fn to_fitness_score(&self, survival_cycles: u32) -> FitnessScore {
        // Kar bileşeni (normalize edilmiş, max +100)
        let profit_component = (self.total_pnl_pct * 10.0).min(100.0).max(-100.0);
        
        // Risk bileşeni (düşük drawdown = iyi)
        let risk_component = (20.0 - self.max_drawdown_pct).max(0.0);
        
        // Tutarlılık bileşeni (win rate)
        let consistency_component = self.win_rate * 0.5;
        
        // Sharpe ratio bileşeni
        let sharpe_component = self.sharpe_ratio * 20.0;
        
        // Hayatta kalma bonusu
        let survival_bonus = (survival_cycles as f64 * 0.1).min(10.0);
        
        // Toplam fitness
        let total = (profit_component * 0.4)
            + (risk_component * 0.2)
            + (consistency_component * 0.2)
            + (sharpe_component * 0.1)
            + (survival_bonus * 0.1);
        
        let total = total.max(0.0).min(150.0);
        
        FitnessScore {
            total,
            profit_component,
            risk_component,
            consistency_component,
            sharpe_component,
            survival_bonus,
        }
    }
    
    /// Özet rapor
    pub fn summary(&self) -> String {
        format!(
            "Trades: {}, Win Rate: {:.1}%, PnL: {:.2}%, Sharpe: {:.2}, Max DD: {:.2}%, PF: {:.2}",
            self.trade_count,
            self.win_rate,
            self.total_pnl_pct,
            self.sharpe_ratio,
            self.max_drawdown_pct,
            self.profit_factor
        )
    }
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            trade_count: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate: 0.0,
            total_pnl_pct: 0.0,
            avg_win_pct: 0.0,
            avg_loss_pct: 0.0,
            profit_factor: 0.0,
            max_drawdown_pct: 0.0,
            sharpe_ratio: 0.0,
            sortino_ratio: 0.0,
            calmar_ratio: 0.0,
            recovery_factor: 0.0,
            max_consecutive_wins: 0,
            max_consecutive_losses: 0,
            avg_trade_duration: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_performance_metrics() {
        // Simüle edilmiş trade sonuçları
        let trade_results = vec![
            5.0, -2.0, 3.0, 4.0, -1.5, 6.0, -3.0, 2.0, -1.0, 7.0
        ];
        
        let metrics = PerformanceMetrics::from_trade_results(&trade_results);
        
        assert_eq!(metrics.trade_count, 10);
        assert_eq!(metrics.winning_trades, 6);
        assert_eq!(metrics.losing_trades, 4);
        assert!(metrics.win_rate > 50.0);
        assert!(metrics.total_pnl_pct > 0.0);
        assert!(metrics.profit_factor > 1.0);
    }
    
    #[test]
    fn test_fitness_score() {
        let trade_results = vec![5.0, 3.0, 4.0, 6.0, 7.0]; // Tüm kazançlı
        let metrics = PerformanceMetrics::from_trade_results(&trade_results);
        let fitness = metrics.to_fitness_score(10);
        
        assert!(fitness.total > 50.0); // Pozitif fitness beklenir
        println!("Fitness: {:.2}", fitness.total);
    }
}
