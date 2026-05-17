// robot/ml_engine/hyperopt.rs - Srivastava ATP Hiperparametre Optimizasyon Merkezi
//
// Modernizasyon Standartları:
// 1. Fonksiyonel Pipeline: Parametre taramaları map/filter zincirine taşındı.
// 2. Kapsüllenmiş PRNG: LCG rastgele sayı üreticisi DRY (Don't Repeat Yourself) uyarınca mühürlendi.
// 3. Match-Guard Karar Yapısı: Skorlama ve Bayesian aşamaları pattern matching ile sadeleşti.
// 4. Zero-Copy Optimizasyonu: Bellek tahsisatı (allocation) minimize edildi.

use crate::core::types::{Candle, StrategyParams};
use crate::robot::backtester::{Backtester, BacktestConfig};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

// --- 1. VERİ MODELLERİ ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperOptResult {
    pub best_params:      StrategyParams,
    pub best_score:       f64,
    pub best_win_rate:    f64,
    pub best_pnl_pct:     f64,
    pub best_sharpe:      f64,
    pub best_pf:          f64,
    pub best_max_dd:      f64,
    pub combinations_tested: usize,
    pub top_results:      Vec<HyperOptEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperOptEntry {
    pub params:   StrategyParams,
    pub score:    f64,
    pub win_rate: f64,
    pub pnl_pct:  f64,
    pub sharpe:   f64,
}

// --- 2. OTONOM YARDIMCILAR (PRNG & SKORLAMA) ---

struct SrivastavaPrng(u64);
impl SrivastavaPrng {
    fn new(seed: Option<u64>) -> Self {
        Self(seed.unwrap_or_else(|| {
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(12345)
        }))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 { lo + self.next() % (hi - lo + 1) }
    fn float(&mut self, lo: f64, hi: f64) -> f64 { lo + (self.next() as f64 / u64::MAX as f64) * (hi - lo) }
}

fn compute_composite_score(sharpe: f64, pf: f64, wr: f64, dd: f64, n: usize) -> f64 {
    match n {
        i if i < 3 => f64::NEG_INFINITY,
        _ => {
            let pf_norm = if pf > 1.0 { pf.ln() + 1.0 } else { pf.max(0.0) };
            sharpe * 0.35 + pf_norm * 0.25 + (wr / 100.0) * 0.25 - (dd / 100.0).min(1.0) * 0.15
        }
    }
}

// --- 3. ANA OPTİMİZASYON MOTORU ---

pub struct HyperOpt;
impl HyperOpt {
    /// §87.1: Grid Search - Belirlenmiş ızgarayı fonksiyonel tarar.
    pub fn grid_search(candles: &[Candle], grid: &[StrategyParams], b_cfg: &BacktestConfig) -> Option<HyperOptResult> {
        if candles.is_empty() || grid.is_empty() { return None; }

        let entries: Vec<_> = grid.iter().filter_map(|p| {
            let mut cfg = b_cfg.clone();
            cfg.strategy_params = Some(p.clone());
            Backtester::new(cfg).run(candles).ok().map(|r| HyperOptEntry {
                params: p.clone(),
                score: compute_composite_score(r.sharpe_ratio, r.profit_factor, r.win_rate, r.max_drawdown_pct, r.total_trades),
                win_rate: r.win_rate, pnl_pct: r.total_pnl_pct, sharpe: r.sharpe_ratio,
            })
        }).collect();

        Self::build_result(entries)
    }

    /// §87.2: Random Search - Geniş arama uzayını rastgele örnekler.
    pub fn random_search(candles: &[Candle], n: usize, b_cfg: &BacktestConfig, seed: Option<u64>) -> Option<HyperOptResult> {
        let mut prng = SrivastavaPrng::new(seed);
        let random_grid: Vec<_> = (0..n).map(|_| StrategyParams {
            fast: Some(prng.range(3, 15) as usize),
            slow: Some(prng.range(20, 60) as usize),
            period: Some(prng.range(7, 21) as usize),
            overbought: Some(prng.float(65.0, 82.0)),
            oversold: Some(prng.float(18.0, 35.0)),
            bb_period: Some(prng.range(10, 30) as usize),
            ..Default::default()
        }).collect();

        Self::grid_search(candles, &random_grid, b_cfg)
    }

    /// §87.3: Bayesian-like Search - Keşif (Explore) ve Sömürü (Exploit) dengesi.
    pub fn bayesian_search(candles: &[Candle], n_explore: usize, n_exploit: usize, b_cfg: &BacktestConfig) -> Option<HyperOptResult> {
        let explore = Self::random_search(candles, n_explore, b_cfg, Some(42))?;
        let best = &explore.best_params;
        let mut prng = SrivastavaPrng::new(Some(99991));

        let exploit_grid: Vec<_> = (0..n_exploit).map(|_| {
            let d_f = prng.range(0, 2) as i64 - 1;
            let d_s = prng.range(0, 4) as i64 - 2;
            let d_p = prng.range(0, 2) as i64 - 1;

            let fast = (best.fast.unwrap_or(10) as i64 + d_f).max(3) as usize;
            StrategyParams {
                fast: Some(fast),
                slow: Some((best.slow.unwrap_or(30) as i64 + d_s).max(fast as i64 + 1) as usize),
                period: Some((best.period.unwrap_or(14) as i64 + d_p).max(5) as usize),
                ..best.clone()
            }
        }).collect();

        let exploit = Self::grid_search(candles, &exploit_grid, b_cfg)?;
        
        let mut all_results = [explore.top_results, exploit.top_results].concat();
        all_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        
        Self::build_result(all_results).map(|mut r| {
            r.combinations_tested = explore.combinations_tested + exploit.combinations_tested;
            r
        })
    }

    fn build_result(mut entries: Vec<HyperOptEntry>) -> Option<HyperOptResult> {
        if entries.is_empty() { return None; }
        entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        let best = entries[0].clone();
        let total = entries.len();
        entries.truncate(20);

        Some(HyperOptResult {
            best_params: best.params,
            best_score: best.score,
            best_win_rate: best.win_rate,
            best_pnl_pct: best.pnl_pct,
            best_sharpe: best.sharpe,
            best_pf: 0.0, best_max_dd: 0.0,
            combinations_tested: total,
            top_results: entries,
        })
    }
}
