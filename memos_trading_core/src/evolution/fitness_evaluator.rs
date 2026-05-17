// evolution/fitness_evaluator.rs - Otonom Performans Analizi ve Fitness Puanlama

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitnessScore {
    pub total: f64,
    pub profit_component: f64,
    pub risk_component: f64,
    pub consistency_component: f64,
    pub sharpe_component: f64,
    pub survival_bonus: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub trade_count: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    pub total_pnl_pct: f64,
    pub avg_win_pct: f64,
    pub avg_loss_pct: f64,
    pub profit_factor: f64,
    pub max_drawdown_pct: f64,
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub calmar_ratio: f64,
    pub recovery_factor: f64,
    pub max_consecutive_wins: usize,
    pub max_consecutive_losses: usize,
    pub avg_trade_duration: f64,
}

impl PerformanceMetrics {
    pub fn from_trade_results(trade_pnl_pcts: &[f64]) -> Self {
        let n = trade_pnl_pcts.len();
        if n == 0 { return Self::default(); }

        let (mut winning_trades, mut losing_trades) = (0, 0);
        let (mut total_wins, mut total_losses, mut total_pnl_pct) = (0.0, 0.0, 0.0);
        let (mut cumulative_pnl, mut peak, mut max_drawdown) = (0.0_f64, 0.0_f64, 0.0_f64);

        for &pnl in trade_pnl_pcts {
            cumulative_pnl += pnl;
            peak = f64::max(peak, cumulative_pnl); // Metod yerine statik çağrı daha güvenlidir
            max_drawdown = f64::max(max_drawdown, peak - cumulative_pnl);
        }

        let mean_return = total_pnl_pct / n as f64;
        let (mut var_sum, mut down_var_sum, mut down_count) = (0.0, 0.0, 0);
        
        for &pnl in trade_pnl_pcts {
            let sq_diff = (pnl - mean_return).powi(2);
            var_sum += sq_diff;
            if pnl < mean_return { down_var_sum += sq_diff; down_count += 1; }
        }

        let std_dev = (var_sum / n as f64).sqrt();
        let downside_std = if down_count > 0 { (down_var_sum / down_count as f64).sqrt() } else { f64::EPSILON };

        let (max_con_wins, max_con_losses) = Self::calculate_consecutive_streaks(trade_pnl_pcts);

        Self {
            trade_count: n, winning_trades, losing_trades,
            win_rate: (winning_trades as f64 / n as f64) * 100.0,
            total_pnl_pct,
            avg_win_pct: if winning_trades > 0 { total_wins / winning_trades as f64 } else { 0.0 },
            avg_loss_pct: if losing_trades > 0 { total_losses / losing_trades as f64 } else { 0.0 },
            profit_factor: if total_losses > 0.0 { total_wins / total_losses } else { 0.0 },
            max_drawdown_pct: max_drawdown,
            sharpe_ratio: if std_dev > 0.0 { mean_return / std_dev } else { 0.0 },
            sortino_ratio: if downside_std > 0.0 { mean_return / downside_std } else { 0.0 },
            calmar_ratio: if max_drawdown > 0.0 { total_pnl_pct / max_drawdown } else { 0.0 },
            recovery_factor: if max_drawdown > 0.0 { total_pnl_pct / max_drawdown } else { 0.0 },
            max_consecutive_wins: max_con_wins, max_consecutive_losses: max_con_losses,
            avg_trade_duration: 0.0,
        }
    }

    fn calculate_consecutive_streaks(trade_pnl_pcts: &[f64]) -> (usize, usize) {
        let (mut max_w, mut max_l, mut cur_w, mut cur_l) = (0, 0, 0, 0);
        for &pnl in trade_pnl_pcts {
            if pnl > 0.0 { cur_w += 1; cur_l = 0; max_w = max_w.max(cur_w); }
            else if pnl < 0.0 { cur_l += 1; cur_w = 0; max_l = max_l.max(cur_l); }
        }
        (max_w, max_l)
    }

    pub fn to_fitness_score(&self, survival_cycles: u32) -> FitnessScore {
        let profit_comp = (self.total_pnl_pct * 10.0).clamp(-100.0, 100.0);
        let risk_comp = (20.0 - self.max_drawdown_pct).max(0.0);
        let consistency_comp = self.win_rate * 0.5;
        let sharpe_comp = self.sharpe_ratio * 20.0;
        let survival_bonus = (survival_cycles as f64 * 0.1).min(10.0);

        let total = (profit_comp * 0.4) + (risk_comp * 0.2) + (consistency_comp * 0.2) 
                  + (sharpe_comp * 0.1) + (survival_bonus * 0.1);

        FitnessScore {
            total: total.clamp(0.0, 150.0), profit_component: profit_comp,
            risk_component: risk_comp, consistency_component: consistency_comp,
            sharpe_component: sharpe_comp, survival_bonus,
        }
    }

    pub fn summary(&self) -> String {
        format!("TR: {} | WR: {:.1}% | PnL: {:.2}% | SR: {:.2} | DD: {:.2}% | PF: {:.2}",
            self.trade_count, self.win_rate, self.total_pnl_pct, self.sharpe_ratio, 
            self.max_drawdown_pct, self.profit_factor)
    }
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            trade_count: 0, winning_trades: 0, losing_trades: 0, win_rate: 0.0,
            total_pnl_pct: 0.0, avg_win_pct: 0.0, avg_loss_pct: 0.0, profit_factor: 0.0,
            max_drawdown_pct: 0.0, sharpe_ratio: 0.0, sortino_ratio: 0.0, calmar_ratio: 0.0,
            recovery_factor: 0.0, max_consecutive_wins: 0, max_consecutive_losses: 0,
            avg_trade_duration: 0.0,
        }
    }
}
