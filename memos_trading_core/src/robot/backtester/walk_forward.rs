// walk_forward.rs - Kayan Pencere (Walk-Forward) Analiz Motoru

use serde::{Deserialize, Serialize};
use rayon::prelude::*; // Paralel işleme desteği
use crate::core::types::Candle;
use crate::robot::backtester::{Backtester, BacktestConfig, BacktestResult};

// --- 1. YAPILANDIRMA VE SONUÇ MODELLERİ ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardConfig {
    pub in_sample_bars: usize,
    pub out_of_sample_bars: usize,
    pub step_bars: usize,
    pub initial_balance: f64,
    pub strategy_name: String,
    pub symbol: String,
    pub interval: String,
    pub commission_pct: f64,
}

impl Default for WalkForwardConfig {
    fn default() -> Self {
        Self {
            in_sample_bars: 200,
            out_of_sample_bars: 50,
            step_bars: 50,
            initial_balance: 10_000.0,
            strategy_name: "RSI".to_owned(),
            symbol: "BTCUSDT".to_owned(),
            interval: "1h".to_owned(),
            commission_pct: 0.001,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowResult {
    pub window_idx: usize,
    pub in_sample_range: (usize, usize),
    pub oos_range: (usize, usize),
    pub best_tp_pct: f64,
    pub best_sl_pct: f64,
    pub oos_metrics: BacktestMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BacktestMetrics {
    pub trades: usize,
    pub win_rate: f64,
    pub pnl_pct: f64,
    pub profit_factor: f64,
    pub max_dd_pct: f64,
    pub sharpe: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardResult {
    pub config: WalkForwardConfig,
    pub windows: Vec<WindowResult>,
    pub avg_oos_pnl_pct: f64,
    pub avg_oos_sharpe: f64,
    pub consistency_score: f64, // Kârlı pencere oranı
}

// --- 2. ANALİZ MOTORU ---

pub struct WalkForwardTester {
    pub config: WalkForwardConfig,
}

impl WalkForwardTester {
    pub fn new(config: WalkForwardConfig) -> Self {
        Self { config }
    }

    /// Ana Walk-Forward döngüsü
    pub fn run(&self, candles: &[Candle]) -> Option<WalkForwardResult> {
        let total = candles.len();
        let window_size = self.config.in_sample_bars + self.config.out_of_sample_bars;
        if total < window_size { return None; }

        // Pencereleri önceden tanımla (Allocation-optimized)
        let mut window_definitions = Vec::new();
        let mut start = 0;
        while start + window_size <= total {
            window_definitions.push(start);
            start += self.config.step_bars;
        }

        // PARALEL İŞLEME: Her pencereyi farklı CPU çekirdeğinde analiz et
        let windows: Vec<WindowResult> = window_definitions.par_iter().enumerate().map(|(idx, &start)| {
            let is_end = start + self.config.in_sample_bars;
            let oos_end = is_end + self.config.out_of_sample_bars;

            let in_sample = &candles[start..is_end];
            let oos = &candles[is_end..oos_end];

            // 1. In-Sample: En iyi parametreleri bul (Eğitim)
            let (best_tp, best_sl) = self.quick_optimize(in_sample);

            // 2. Out-of-Sample: Parametreleri test et (Validasyon)
            let metrics = self.run_backtest(oos, best_tp, best_sl);

            WindowResult {
                window_idx: idx,
                in_sample_range: (start, is_end),
                oos_range: (is_end, oos_end),
                best_tp_pct: best_tp,
                best_sl_pct: best_sl,
                oos_metrics: metrics,
            }
        }).collect();

        if windows.is_empty() { return None; }

        self.finalize_report(windows)
    }

    /// In-Sample optimizasyonu: Grid Search (Hafifletilmiş)
    fn quick_optimize(&self, candles: &[Candle]) -> (f64, f64) {
        let tp_grid = [2.5, 5.0, 7.5, 10.0, 15.0];
        let sl_grid = [1.0, 2.0, 3.0, 4.0, 5.0];

        let mut best_params = (5.0, 2.0);
        let mut best_score = f64::NEG_INFINITY;

        for &tp in &tp_grid {
            for &sl in &sl_grid {
                if tp <= sl { continue; }
                let res = self.run_backtest(candles, tp, sl);
                
                // Kompozit Skor: Sharpe %40 + PnL %35 + WinRate %25
                let score = (res.sharpe * 0.40) + (res.pnl_pct * 0.35) + (res.win_rate * 0.0025);
                if score > best_score {
                    best_score = score;
                    best_params = (tp, sl);
                }
            }
        }
        best_params
    }

    /// Alt-Backtest çalıştırıcı (Zero-Panic)
    fn run_backtest(&self, candles: &[Candle], tp: f64, sl: f64) -> BacktestMetrics {
        let cfg = BacktestConfig {
            symbol: self.config.symbol.clone(),
            interval: self.config.interval.clone(),
            initial_balance: self.config.initial_balance,
            max_position_size: 1.0,
            take_profit_pct: tp,
            stop_loss_pct: sl,
            strategy_name: self.config.strategy_name.clone(),
            commission_pct: self.config.commission_pct,
            ..Default::default()
        };

        match Backtester::new(cfg).run(candles) {
            Ok(r) => BacktestMetrics {
                trades: r.total_trades,
                win_rate: r.win_rate,
                pnl_pct: r.total_pnl_pct,
                profit_factor: r.profit_factor,
                max_dd_pct: r.max_drawdown_pct,
                sharpe: r.sharpe_ratio,
            },
            Err(_) => BacktestMetrics::default(),
        }
    }

    fn finalize_report(&self, windows: Vec<WindowResult>) -> Option<WalkForwardResult> {
        let n = windows.len() as f64;
        let profitable_count = windows.iter().filter(|w| w.oos_metrics.pnl_pct > 0.0).count();

        Some(WalkForwardResult {
            avg_oos_pnl_pct: windows.iter().map(|w| w.oos_metrics.pnl_pct).sum::<f64>() / n,
            avg_oos_sharpe: windows.iter().map(|w| w.oos_metrics.sharpe).sum::<f64>() / n,
            consistency_score: profitable_count as f64 / n,
            config: self.config.clone(),
            windows,
        })
    }
}
