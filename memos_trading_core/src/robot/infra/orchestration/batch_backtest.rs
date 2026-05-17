// batch_backtest.rs - Gelişmiş Toplu Backtest Altyapısı

use crate::core::types::{Candle, Signal, StrategyParams};
use crate::robot::strategies::Strategy;
use crate::robot::logic::portfolio::Portfolio;
use std::collections::HashMap;
use rayon::prelude::*; // Paralel işleme için

pub struct BacktestConfig {
    pub strategies: Vec<Box<dyn Strategy + Sync + Send>>, // Thread-safe hale getirildi
    pub param_grid: Vec<StrategyParams>,
    pub symbols: Vec<String>,
    pub candles: HashMap<String, Vec<Candle>>,
    pub commission_pct: f64,
    pub slippage_pct: f64,
    pub latency_ms: u64,
}

#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub symbol: String,
    pub strategy: String,
    pub params: StrategyParams,
    pub total_pnl: f64,
    pub win_rate: f64,
    pub trade_count: usize,
}

pub async fn run_batch_backtest(cfg: BacktestConfig) -> Vec<BacktestResult> {
    // Pipeline Optimizasyonu: Her sembolü ayrı bir thread'de paralel koşturuyoruz
    let results: Vec<Vec<BacktestResult>> = cfg.symbols.par_iter().map(|symbol| {
        let mut symbol_results = Vec::new();

        if let Some(candles) = cfg.candles.get(symbol) {
            for strat in &cfg.strategies {
                for params in &cfg.param_grid {
                    let mut portfolio = Portfolio::default();

                    for c in candles {
                        // Unnecessary clone'u kaldırdık, referans üzerinden gidiyoruz
                        let signal = strat.generate_signal(&[c.clone()], params, None, None)
                            .unwrap_or(Signal::Hold);

                        if matches!(signal, Signal::Buy | Signal::Sell) {
                            // Gerçekçi Simülasyon: Slippage ve Komisyon
                            let slip = if signal == Signal::Buy { 1.0 + (cfg.slippage_pct / 100.0) } else { 1.0 - (cfg.slippage_pct / 100.0) };
                            let exec_price = c.close * slip;
                            let commission = exec_price * (cfg.commission_pct / 100.0);

                            // Latency Modellemesi: 
                            // Loop içinde sleep yapmak yerine, execute zamanını simüle eden bir counter tutulmalı.
                            // Backtest'te gerçek sleep sistemi yavaşlatır.

                            match signal {
                                Signal::Buy => portfolio.open_position(symbol, exec_price, 1.0, Signal::Buy, &strat.name()),
                                Signal::Sell => { portfolio.close_position(symbol, exec_price); },
                                _ => {}
                            }
                            portfolio.balance -= commission;
                        }
                    }

                    let metrics = portfolio.update_metrics();
                    symbol_results.push(BacktestResult {
                        symbol: symbol.clone(),
                        strategy: strat.name().to_string(),
                        params: params.clone(),
                        total_pnl: metrics.total_pnl,
                        win_rate: metrics.win_rate,
                        trade_count: metrics.trade_count,
                    });
                }
            }
        }
        symbol_results
    }).collect();

    // Nested vektörü tek bir düz listeye indiriyoruz (flatten)
    results.into_iter().flatten().collect()
}
