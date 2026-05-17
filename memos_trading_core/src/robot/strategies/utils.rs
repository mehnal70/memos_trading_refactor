// robot/strategies/utils.rs - Tüm stratejilerin paylaştığı yardımcılar
//
// `htf_trend_filter`: Üst zaman dilimi (HTF) filtresi — strateji sinyalini onaylar/geçersizleştirir.
// `grid_search_optimization`: Parametre uzayında brute-force optimizer (async).

use crate::core::indicators::CoreIndicatorEngine;
use crate::core::types::{Candle, Signal, StrategyParams};

/// HTF (Higher Timeframe) trend filtresi — Buy/Sell sinyalini üst dilimdeki MA durumuyla doğrular.
/// Üst dilim ters yöndeyse sinyali Hold'a çevirir; aksi halde olduğu gibi geçirir.
pub fn htf_trend_filter(
    signal: Signal,
    htf_candles: Option<&[Candle]>,
    fast: usize,
    slow: usize,
    name: &str,
) -> Signal {
    if matches!(signal, Signal::Hold) { return signal; }

    let htf = match htf_candles {
        Some(h) if h.len() >= slow => h,
        _ => return signal,
    };

    let fast_ma = CoreIndicatorEngine::sma(htf, fast);
    let slow_ma = CoreIndicatorEngine::sma(htf, slow);
    if fast_ma == 0.0 || slow_ma == 0.0 { return signal; }

    match signal {
        Signal::Buy if fast_ma < slow_ma => {
            log::info!("[{}] MTF Filter: Bearish HTF -> Hold", name);
            Signal::Hold
        }
        Signal::Sell if fast_ma > slow_ma => {
            log::info!("[{}] MTF Filter: Bullish HTF -> Hold", name);
            Signal::Hold
        }
        other => other,
    }
}

/// Brute-force parametre tarayıcı — verilen grid'in tüm kombinasyonlarını dener,
/// `evaluate_fn`'nin en yüksek skoru üreten parametre setini döner.
pub async fn grid_search_optimization<F>(
    candles: &[Candle],
    param_grid: Vec<(String, Vec<f64>)>,
    evaluate_fn: F,
) -> Option<(StrategyParams, f64)>
where F: Fn(&[Candle], &StrategyParams) -> f64 {
    use itertools::Itertools;

    let keys: Vec<_> = param_grid.iter().map(|(k, _)| k.as_str()).collect();
    let value_lists: Vec<_> = param_grid.iter().map(|(_, v)| v.clone()).collect();

    value_lists.into_iter().multi_cartesian_product()
        .map(|combo| {
            let mut p = StrategyParams::default();
            for (i, val) in combo.iter().enumerate() {
                match keys[i] {
                    "fast"        => p.fast = Some(*val as usize),
                    "slow"        => p.slow = Some(*val as usize),
                    "period"      => p.period = Some(*val as usize),
                    "overbought"  => p.overbought = Some(*val),
                    "oversold"    => p.oversold = Some(*val),
                    _ => {}
                }
            }
            let score = evaluate_fn(candles, &p);
            (p, score)
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
}
