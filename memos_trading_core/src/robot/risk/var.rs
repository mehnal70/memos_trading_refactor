// risk_analysis.rs - Value at Risk ve Monte Carlo Stres Testi Motoru
use crate::prelude::*;
#[derive(Default)] pub struct VarEngine;
impl VarEngine { pub fn check_exposure(&self, _snap: &MissionControl) -> bool { true } }

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::time::SystemTime;

// --- 1. VALUE AT RISK (VaR) MODELLERİ ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VaRMethod {
    Historical,
    Parametric,
    MonteCarlo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueAtRisk {
    pub method: VaRMethod,
    pub confidence_level: f64,
    pub time_horizon: usize,
    pub var_amount: f64,
    pub var_pct: f64,
    pub cvar: Option<f64>,
}

impl ValueAtRisk {
    /// Historical VaR: Geçmiş getiri dağılımının yüzdelik dilimine göre risk ölçer.
    pub fn historical(returns: &[f64], confidence_level: f64, position_value: f64) -> Self {
        if returns.is_empty() { return Self::empty(VaRMethod::Historical, confidence_level); }
        
        let mut sorted = returns.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        
        let index = ((1.0 - confidence_level) * sorted.len() as f64).ceil() as usize;
        let index = index.min(sorted.len() - 1);
        
        let worst_return = sorted[index];
        
        // CVaR (Expected Shortfall): VaR eşiğinin ötesindeki kayıpların ortalaması
        let cvar = if index > 0 {
            let tail_avg = sorted[..index].iter().sum::<f64>() / index as f64;
            Some(position_value * tail_avg.abs())
        } else { None };
        
        Self {
            method: VaRMethod::Historical,
            confidence_level,
            time_horizon: 1,
            var_amount: position_value * worst_return.abs(),
            var_pct: worst_return.abs() * 100.0,
            cvar,
        }
    }
    
    /// Parametric VaR: Normal dağılım varsayımıyla (Mean/StdDev) risk ölçer.
    pub fn parametric(returns: &[f64], confidence_level: f64, position_value: f64) -> Self {
        let n = returns.len();
        if n < 2 { return Self::empty(VaRMethod::Parametric, confidence_level); }
        
        let mean = returns.iter().sum::<f64>() / n as f64;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n as f64;
        let std_dev = variance.sqrt();
        
        // Z-score (Güven aralıkları için sabitler)
        let z_score = match confidence_level {
            x if x >= 0.99 => 2.326,
            x if x >= 0.95 => 1.645,
            x if x >= 0.90 => 1.282,
            _ => 1.0,
        };
        
        let var_return = mean - (z_score * std_dev);
        
        Self {
            method: VaRMethod::Parametric,
            confidence_level,
            time_horizon: 1,
            var_amount: (position_value * var_return.abs()).max(0.0),
            var_pct: var_return.abs() * 100.0,
            cvar: None,
        }
    }

    fn empty(method: VaRMethod, confidence: f64) -> Self {
        Self { method, confidence_level: confidence, time_horizon: 1, var_amount: 0.0, var_pct: 0.0, cvar: None }
    }
}

// --- 2. MONTE CARLO SİMÜLASYONU ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonteCarloResult {
    pub n_simulations: usize,
    pub n_trades: usize,
    pub initial_balance: f64,
    pub ruin_threshold: f64,
    pub final_balance_p5: f64,
    pub final_balance_p25: f64,
    pub final_balance_p50: f64,
    pub final_balance_p75: f64,
    pub final_balance_p95: f64,
    pub max_dd_p50: f64,
    pub max_dd_p95: f64,
    pub ruin_probability: f64,
    pub expected_return_pct: f64,
    pub positive_scenario_pct: f64,
}

pub struct MonteCarloSimulator {
    pub n_simulations: usize,
    pub ruin_threshold: f64,
    pub seed: Option<u64>,
}

impl Default for MonteCarloSimulator {
    fn default() -> Self {
        Self { n_simulations: 1000, ruin_threshold: 0.50, seed: None }
    }
}

impl MonteCarloSimulator {
    /// Bootstrap metoduyla trade permütasyonlarını simüle eder.
    pub fn run(&self, trade_pnls: &[f64], initial_balance: f64) -> Option<MonteCarloResult> {
        if trade_pnls.is_empty() || initial_balance <= 0.0 { return None; }
        
        let n = trade_pnls.len();
        let ruin_floor = initial_balance * (1.0 - self.ruin_threshold);
        
        // Hızlı PRNG: LCG (Linear Congruential Generator)
        let mut rng_state = self.seed.unwrap_or_else(|| {
            SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64).unwrap_or(42)
        });

        let mut final_balances = Vec::with_capacity(self.n_simulations);
        let mut max_drawdowns = Vec::with_capacity(self.n_simulations);
        let mut ruin_count = 0u64;

        for _ in 0..self.n_simulations {
            let mut balance = initial_balance;
            let mut peak = initial_balance;
            let mut max_dd = 0.0;
            let mut is_ruined = false;

            for _ in 0..n {
                // LCG Next
                rng_state = rng_state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                let idx = (rng_state as usize) % n;
                
                balance += trade_pnls[idx];
                peak = peak.max(balance);
                
                let dd = (peak - balance) / peak * 100.0;
                max_dd = f64::max(max_dd, dd);
                
                if balance <= ruin_floor { is_ruined = true; break; }
            }

            if is_ruined { ruin_count += 1; }
            final_balances.push(balance);
            max_drawdowns.push(max_dd);
        }

        // İstatistiksel dilimleme için sıralama
        final_balances.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        max_drawdowns.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let get_pct = |v: &[f64], p: f64| -> f64 {
            let idx = ((p / 100.0) * (v.len() - 1) as f64).round() as usize;
            v[idx.min(v.len() - 1)]
        };

        let p50_balance = get_pct(&final_balances, 50.0);
        let positive_scenarios = final_balances.iter().filter(|&&b| b > initial_balance).count();

        Some(MonteCarloResult {
            n_simulations: self.n_simulations,
            n_trades: n,
            initial_balance,
            ruin_threshold: self.ruin_threshold,
            final_balance_p5: get_pct(&final_balances, 5.0),
            final_balance_p25: get_pct(&final_balances, 25.0),
            final_balance_p50: p50_balance,
            final_balance_p75: get_pct(&final_balances, 75.0),
            final_balance_p95: get_pct(&final_balances, 95.0),
            max_dd_p50: get_pct(&max_drawdowns, 50.0),
            max_dd_p95: get_pct(&max_drawdowns, 95.0),
            ruin_probability: ruin_count as f64 / self.n_simulations as f64,
            expected_return_pct: (p50_balance - initial_balance) / initial_balance * 100.0,
            positive_scenario_pct: (positive_scenarios as f64 / self.n_simulations as f64) * 100.0,
        })
    }
}

// --- 3. RİSK LİMİT KONTROLLERİ ---

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaRLimits {
    pub daily_var_limit_pct: f64,
    pub weekly_var_limit_pct: f64,
    pub position_limit_pct: f64,
    pub max_leverage: f64,
}

impl VaRLimits {
    pub fn is_position_ok(&self, pct: f64, leverage: f64) -> bool {
        pct <= self.position_limit_pct && leverage <= self.max_leverage
    }
    
    pub fn is_daily_var_ok(&self, var_pct: f64) -> bool {
        var_pct <= self.daily_var_limit_pct
    }
}
