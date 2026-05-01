// Value at Risk (VaR) + Monte Carlo Simülatörü
//
// Srivastava mimarisi: Worst-case loss scenarios + Bootstrap trade permütasyonu

use serde::{Serialize, Deserialize};

/// Value at Risk hesaplama yöntemi
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VaRMethod {
    /// Historical simulation
    Historical,
    /// Parametric (variance-covariance)
    Parametric,
    /// Monte Carlo
    MonteCarlo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueAtRisk {
    /// VaR method
    pub method: VaRMethod,
    
    /// Güven seviyesi (örnek: 0.95 = %95)
    pub confidence_level: f64,
    
    /// Time horizon (gün)
    pub time_horizon: usize,
    
    /// VaR miktarı (para birimi)
    pub var_amount: f64,
    
    /// VaR yüzdesi (%)
    pub var_pct: f64,
    
    /// Conditional VaR (CVaR) - tail risk
    pub cvar: Option<f64>,
}

impl ValueAtRisk {
    /// Historical VaR hesapla
    pub fn historical(returns: &[f64], confidence_level: f64, position_value: f64) -> Self {
        let mut sorted = returns.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        
        let index = ((1.0 - confidence_level) * sorted.len() as f64).ceil() as usize;
        let index = index.min(sorted.len() - 1);
        
        let worst_return = sorted[index];
        let var_amount = position_value * worst_return.abs();
        let var_pct = worst_return.abs() * 100.0;
        
        // CVaR = avg of worst returns beyond VaR
        let cvar = if index > 0 {
            let tail_avg: f64 = sorted[0..index].iter().sum::<f64>() / index as f64;
            Some(position_value * tail_avg.abs())
        } else {
            None
        };
        
        Self {
            method: VaRMethod::Historical,
            confidence_level,
            time_horizon: 1,
            var_amount,
            var_pct,
            cvar,
        }
    }
    
    /// Parametric VaR (Gaussian assumption)
    pub fn parametric(
        returns: &[f64],
        confidence_level: f64,
        position_value: f64,
    ) -> Self {
        if returns.len() < 2 {
            return Self {
                method: VaRMethod::Parametric,
                confidence_level,
                time_horizon: 1,
                var_amount: 0.0,
                var_pct: 0.0,
                cvar: None,
            };
        }
        
        // Mean ve Std Dev
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance = returns
            .iter()
            .map(|r| (r - mean).powi(2))
            .sum::<f64>() / returns.len() as f64;
        let std_dev = variance.sqrt();
        
        // Z-score for confidence level
        let z_score = match confidence_level {
            x if x >= 0.99 => 2.326,  // 99%
            x if x >= 0.95 => 1.645,  // 95%
            x if x >= 0.90 => 1.282,  // 90%
            _ => 0.0,
        };
        
        let var_return = mean - (z_score * std_dev);
        let var_amount = (position_value * var_return.abs()).max(0.0);
        let var_pct = var_return.abs() * 100.0;
        
        Self {
            method: VaRMethod::Parametric,
            confidence_level,
            time_horizon: 1,
            var_amount,
            var_pct,
            cvar: None,
        }
    }
}

/// VaR Limits Checker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaRLimits {
    /// Daily VaR limiti (%)
    pub daily_var_limit_pct: f64,
    
    /// Weekly VaR limiti (%)
    pub weekly_var_limit_pct: f64,
    
    /// Position size limiti (%)
    pub position_limit_pct: f64,
    
    /// Max leverage
    pub max_leverage: f64,
}

impl Default for VaRLimits {
    fn default() -> Self {
        Self {
            daily_var_limit_pct: 2.0,    // Günde max %2 risk
            weekly_var_limit_pct: 5.0,   // Haftada max %5 risk
            position_limit_pct: 10.0,    // Her pozisyon max %10
            max_leverage: 3.0,           // Max 3x leverage
        }
    }
}

impl VaRLimits {
    pub fn is_position_ok(&self, position_pct: f64, leverage: f64) -> bool {
        position_pct <= self.position_limit_pct && leverage <= self.max_leverage
    }
    
    pub fn is_daily_var_ok(&self, daily_var_pct: f64) -> bool {
        daily_var_pct <= self.daily_var_limit_pct
    }
    
    pub fn is_weekly_var_ok(&self, weekly_var_pct: f64) -> bool {
        weekly_var_pct <= self.weekly_var_limit_pct
    }
}

// ─── Monte Carlo Simülatörü ────────────────────────────────────────────────────
//
// Backtest trade serisini N kez rastgele permüte eder (bootstrap).
// Her permütasyonda:
//   - Bakiye seyrini yeniden hesaplar
//   - Max drawdown'ı ölçer
//   - Nihai bakiyeyi kaydeder
//
// Çıktı: %5/%25/%50/%75/%95 yüzdelik dilimleri + ruin olasılığı
//
// "Ruin" eşiği: başlangıç sermayesinin %X'ini kaybetmek (varsayılan %50).
// Bootstrap (yerine koyarak örnekleme) kullanılır — gerçek piyasa korelasyonunu
// bozmadan sıra bağımlılığını test eder.

/// Monte Carlo simülasyon sonucu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonteCarloResult {
    /// Kaç simülasyon çalıştırıldı
    pub n_simulations: usize,
    /// Orijinal trade sayısı
    pub n_trades: usize,
    /// Başlangıç bakiyesi
    pub initial_balance: f64,
    /// Ruin eşiği (örn: 0.50 = %50 kayıp = ruin)
    pub ruin_threshold: f64,

    // ── Nihai bakiye dağılımı ──────────────────────────────────────────────
    pub final_balance_p5:  f64,   // En kötü %5 senaryonun medyanı
    pub final_balance_p25: f64,
    pub final_balance_p50: f64,   // Medyan senaryo
    pub final_balance_p75: f64,
    pub final_balance_p95: f64,   // En iyi %5 senaryonun medyanı

    // ── Max drawdown dağılımı ──────────────────────────────────────────────
    pub max_dd_p50: f64,          // Medyan senaryoda beklenen max drawdown (%)
    pub max_dd_p95: f64,          // Kötü senaryolarda max drawdown (%)

    // ── Ruin istatistiği ──────────────────────────────────────────────────
    pub ruin_probability: f64,    // 0.0–1.0 arası; 0.05 = %5 ruin riski

    // ── Getiri istatistiği ────────────────────────────────────────────────
    pub expected_return_pct: f64, // Medyan getiri %
    pub positive_scenario_pct: f64, // Kârlı biten simülasyon oranı
}

/// Monte Carlo simülatörü.
/// Trade PnL serisini bootstrapla ve dağılımı hesapla.
pub struct MonteCarloSimulator {
    /// Kaç simülasyon çalıştırılacak (önerilen: 1000–5000)
    pub n_simulations: usize,
    /// Ruin eşiği: başlangıç sermayesinin bu oranı kaybedilince "ruin" sayılır
    pub ruin_threshold: f64,
    /// Rastgele tohum — None ise her çalıştırmada farklı sonuç
    pub seed: Option<u64>,
}

impl Default for MonteCarloSimulator {
    fn default() -> Self {
        Self { n_simulations: 1000, ruin_threshold: 0.50, seed: None }
    }
}

impl MonteCarloSimulator {
    pub fn new(n_simulations: usize) -> Self {
        Self { n_simulations, ..Default::default() }
    }

    /// trade_pnls: backtest/gerçek işlemlerden gelen PnL listesi ($)
    /// initial_balance: simülasyon başlangıç bakiyesi
    pub fn run(&self, trade_pnls: &[f64], initial_balance: f64) -> Option<MonteCarloResult> {
        if trade_pnls.is_empty() || initial_balance <= 0.0 {
            return None;
        }
        let n = trade_pnls.len();
        let ruin_floor = initial_balance * (1.0 - self.ruin_threshold);

        // LCG tabanlı basit PRNG — rand crate bağımlılığı eklememek için
        let mut rng_state: u64 = self.seed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(42)
        });
        let lcg_next = |s: &mut u64| -> u64 {
            *s = s.wrapping_mul(6_364_136_223_846_793_005)
                   .wrapping_add(1_442_695_040_888_963_407);
            *s
        };

        let mut final_balances: Vec<f64> = Vec::with_capacity(self.n_simulations);
        let mut max_drawdowns: Vec<f64>  = Vec::with_capacity(self.n_simulations);
        let mut ruin_count = 0u64;

        for _ in 0..self.n_simulations {
            // Yerine koyarak örnekleme (bootstrap)
            let mut balance = initial_balance;
            let mut peak    = initial_balance;
            let mut max_dd  = 0.0f64;
            let mut ruined  = false;

            for _ in 0..n {
                let idx = (lcg_next(&mut rng_state) as usize) % n;
                balance += trade_pnls[idx];
                if balance > peak { peak = balance; }
                let dd = (peak - balance) / peak * 100.0;
                if dd > max_dd { max_dd = dd; }
                if balance <= ruin_floor { ruined = true; break; }
            }

            if ruined { ruin_count += 1; }
            final_balances.push(balance);
            max_drawdowns.push(max_dd);
        }

        final_balances.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        max_drawdowns .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let pct = |v: &[f64], p: f64| -> f64 {
            if v.is_empty() { return 0.0; }
            let idx = ((p / 100.0) * (v.len() - 1) as f64).round() as usize;
            v[idx.min(v.len() - 1)]
        };

        let p50_balance = pct(&final_balances, 50.0);
        let positive_count = final_balances.iter().filter(|&&b| b > initial_balance).count();

        Some(MonteCarloResult {
            n_simulations:        self.n_simulations,
            n_trades:             n,
            initial_balance,
            ruin_threshold:       self.ruin_threshold,
            final_balance_p5:     pct(&final_balances,  5.0),
            final_balance_p25:    pct(&final_balances, 25.0),
            final_balance_p50:    p50_balance,
            final_balance_p75:    pct(&final_balances, 75.0),
            final_balance_p95:    pct(&final_balances, 95.0),
            max_dd_p50:           pct(&max_drawdowns,  50.0),
            max_dd_p95:           pct(&max_drawdowns,  95.0),
            ruin_probability:     ruin_count as f64 / self.n_simulations as f64,
            expected_return_pct:  (p50_balance - initial_balance) / initial_balance * 100.0,
            positive_scenario_pct: positive_count as f64 / self.n_simulations as f64 * 100.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_historical_var() {
        let returns = vec![-0.05, -0.03, -0.02, 0.00, 0.01, 0.02, 0.03];
        let var = ValueAtRisk::historical(&returns, 0.95, 10000.0);
        
        assert!(var.var_amount > 0.0);
        assert!(var.var_pct > 0.0);
    }
    
    #[test]
    fn test_parametric_var() {
        let returns = vec![0.01, 0.015, 0.02, -0.005, 0.01];
        let var = ValueAtRisk::parametric(&returns, 0.95, 10000.0);
        
        assert!(var.var_amount > 0.0);
    }
    
    #[test]
    fn test_var_limits() {
        let limits = VaRLimits::default();

        assert!(limits.is_daily_var_ok(1.5));  // OK
        assert!(!limits.is_daily_var_ok(3.0)); // Fail

        assert!(limits.is_position_ok(8.0, 2.0)); // OK
        assert!(!limits.is_position_ok(15.0, 1.0)); // Fail
    }

    #[test]
    fn test_monte_carlo_basic() {
        // Karışık trade serisi: +10, -5, +8, -3, +12
        let pnls = vec![10.0, -5.0, 8.0, -3.0, 12.0, -4.0, 15.0, -6.0, 9.0, -2.0];
        let sim = MonteCarloSimulator { n_simulations: 500, ruin_threshold: 0.50, seed: Some(42) };
        let res = sim.run(&pnls, 1000.0).expect("simülasyon başarısız");

        assert_eq!(res.n_trades, 10);
        assert!(res.ruin_probability >= 0.0 && res.ruin_probability <= 1.0);
        assert!(res.final_balance_p5 <= res.final_balance_p50);
        assert!(res.final_balance_p50 <= res.final_balance_p95);
        assert!(res.max_dd_p50 >= 0.0 && res.max_dd_p50 <= 100.0);
        assert!(res.positive_scenario_pct >= 0.0 && res.positive_scenario_pct <= 100.0);
    }

    #[test]
    fn test_monte_carlo_all_losses_high_ruin() {
        // Sürekli kayıp — ruin olasılığı yüksek olmalı
        let pnls: Vec<f64> = vec![-50.0; 20];
        let sim = MonteCarloSimulator { n_simulations: 200, ruin_threshold: 0.50, seed: Some(7) };
        let res = sim.run(&pnls, 1000.0).expect("simülasyon başarısız");
        assert!(res.ruin_probability > 0.5, "sürekli kayıpta ruin > %50 bekleniyor");
    }

    #[test]
    fn test_monte_carlo_empty_returns_none() {
        let sim = MonteCarloSimulator::default();
        assert!(sim.run(&[], 1000.0).is_none());
        assert!(sim.run(&[10.0], 0.0).is_none());
    }
}
