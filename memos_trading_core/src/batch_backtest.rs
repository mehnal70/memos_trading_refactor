// batch_backtest.rs - Gelişmiş Toplu Backtest ve Simülasyon Altyapısı
// Çoklu strateji, parametre ve piyasa kombinasyonları için toplu test
// Slippage, komisyon, likidite ve latency modellemesi ile
// Türkçe açıklamalar ile

use crate::types::{Candle, Signal, StrategyParams};
use crate::strategies::Strategy;
use crate::portfolio::Portfolio;
use crate::robot::order_management::types::SlippageInfo;
use std::collections::HashMap;

pub struct BacktestConfig {
    pub strategies: Vec<Box<dyn Strategy>>,
    pub param_grid: Vec<StrategyParams>,
    pub symbols: Vec<String>,
    pub candles: HashMap<String, Vec<Candle>>, // symbol -> candles
    pub commission_pct: f64,
    pub slippage_pct: f64,
    pub latency_ms: u64,
}

pub struct BacktestResult {
    pub symbol: String,
    pub strategy: String,
    pub params: StrategyParams,
    pub total_pnl: f64,
    pub win_rate: f64,
    pub trade_count: usize,
}

pub async fn run_batch_backtest(cfg: BacktestConfig) -> Vec<BacktestResult> {
    let mut results = Vec::new();
    for symbol in &cfg.symbols {
        if let Some(candles) = cfg.candles.get(symbol) {
            for strat in &cfg.strategies {
                for params in &cfg.param_grid {
                    // Portföy başlat
                    let mut portfolio = Portfolio::default();
                    // Her bar için sinyal üret
                    for c in candles {
                        let signal = strat.generate_signal(&[c.clone()], params, None, None).unwrap_or(Signal::Hold);
                        // Emir simülasyonu: slippage, komisyon, latency
                        // (örnek: sadece market order, gerçekçi modelleme için genişletilebilir)
                        if let Signal::Buy | Signal::Sell = signal {
                            // Slippage uygula
                            let exec_price = c.close * (1.0 + cfg.slippage_pct / 100.0);
                            // Komisyon uygula
                            let commission = exec_price * cfg.commission_pct / 100.0;
                            // Latency simülasyonu (bekletme)
                            if cfg.latency_ms > 0 { tokio::time::sleep(std::time::Duration::from_millis(cfg.latency_ms)).await; }
                            // Pozisyon aç/kapat
                            if let Signal::Buy = signal {
                                portfolio.open_position(&symbol, exec_price, 1.0, Signal::Buy, &strat.name());
                            } else if let Signal::Sell = signal {
                                portfolio.close_position(&symbol, exec_price);
                            }
                            portfolio.balance -= commission;
                        }
                    }
                    let metrics = portfolio.update_metrics();
                    results.push(BacktestResult {
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
    }
    results
}
