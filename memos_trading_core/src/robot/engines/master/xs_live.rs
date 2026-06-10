// src/robot/engines/master/xs_live.rs — Kesitsel (cross-sectional) MOMENTUM adanmış mod (ince sarmalayıcı).
//
// XS_LIVE_ENABLED iken sepet sembolleri SADECE momentum kitabıyla (market-nötr long/short) yönetilir.
// Tüm kitap makinesi `book_core` ORTAK motorunda (xs⊕carry DRY): burada yalnız momentum'a özgü kısım
// kalır — sinyal (kapalı-bar fiyat getirisi), tag, kadans (bar-başına=1). Skorlama backtest çekirdeğiyle
// BİT-AYNI. [[project_xs_momentum]] [[feedback_autonomy_first]] [[feedback_modular_dry_perf]]
use super::*;

/// Momentum kitabı pozisyonlarının strateji/trade_type etiketi — açılışta mühürlenir, kapanış +
/// komisyon muhasebesi bununla XS pozisyonunu tanır (maker icra: USE_LIMIT_ENTRY iken maker oranı).
pub(crate) const XS_STRATEGY_TAG: &str = "XS_MOMENTUM";

/// SAF: son kapanıştan lookback-bar geriye momentum sinyali = close[t]/close[t−lb]−1. Yetersiz → None.
pub(crate) fn latest_signal(closes: &[f64], lookback: usize) -> Option<f64> {
    let n = closes.len();
    if n <= lookback {
        return None;
    }
    let (c0, cl) = (closes[n - 1], closes[n - 1 - lookback]);
    if cl > 0.0 && c0 > 0.0 {
        Some(c0 / cl - 1.0)
    } else {
        None
    }
}

impl Engine {
    /// Kesitsel MOMENTUM adanmış mod cycle adımı: `book_core::process_book` ortak motorunu momentum
    /// sinyali + bar-başına kadans (rebalance_min_bars=1) ile çağırır. Mod kapalı/sepet yetersiz → no-op.
    pub(crate) async fn process_xs_book(state: &Arc<Mutex<AppState>>) {
        let (cfg, tuning, db_path, lookback, closed_only) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let xs = match st.brain.parameters.read().ok().map(|p| p.xs_live.clone()) {
                Some(x) if x.enabled && x.top_k >= 1 && x.symbols.len() >= 2 * x.top_k => x,
                _ => return,
            };
            let cfg = super::book_core::BookConfig {
                symbols: xs.symbols.clone(),
                interval: xs.interval.clone(),
                lookback: xs.lookback,
                top_k: xs.top_k,
                exit_buffer: xs.exit_buffer,
                momentum: xs.momentum,
                position_pct: xs.position_pct,
                leverage: xs.leverage,
                regime_gate: xs.regime_gate,
                max_drawdown_pct: xs.max_drawdown_pct,
                cb_cooldown_secs: xs.cb_cooldown_secs,
                take_profit_pct: xs.take_profit_pct,
                tp_cooldown_secs: xs.tp_cooldown_secs,
                rebalance_min_bars: 1, // momentum: bar-başına (bar-içi rank churn'ü kapanır)
            };
            (cfg, Arc::clone(&st.tuning), st.config.db_path.clone(),
             xs.lookback, st.tuning.signal_closed_bar_only)
        };

        let interval_secs =
            crate::robot::data_pipeline::DataNormalizer::parse_interval(&cfg.interval) as i64;
        // 📐 Momentum kesitsel sinyal üreteci: her sembolün KAPALI-BAR penceresinden fiyat getirisi
        // (forming barı dışla; live=backtest). Escape: SIGNAL_CLOSED_BAR_ONLY=0. [[project_closed_bar_signal]]
        let signal_source = move |candles_map: &std::collections::HashMap<String, Vec<Candle>>| {
            momentum_signals(candles_map, interval_secs, closed_only, lookback)
        };

        Self::process_book(
            state, &cfg, super::book_core::BookKind::Momentum, &tuning, &db_path, signal_source,
        ).await;
    }
}

/// SAF-yardımcı: candles_map → momentum kesitsel sinyalleri (kapalı-bar fiyat getirisi). xs_live ve
/// blend_live ORTAK kullanır (DRY) → tek-kaynak momentum sinyali. Skoru olmayan sembol vec'e girmez.
pub(crate) fn momentum_signals(
    candles_map: &std::collections::HashMap<String, Vec<Candle>>,
    interval_secs: i64, closed_only: bool, lookback: usize,
) -> Vec<(String, f64)> {
    let now = chrono::Utc::now();
    candles_map.iter().filter_map(|(sym, c)| {
        let sig_c = super::loop_core::closed_bar_window(c, interval_secs, closed_only, now);
        let closes: Vec<f64> = sig_c.iter().map(|k| k.close).collect();
        latest_signal(&closes, lookback).map(|s| (sym.clone(), s))
    }).collect()
}

#[cfg(test)]
mod xs_live_tests {
    use super::*;

    #[test]
    fn latest_signal_basic() {
        assert!((latest_signal(&[100.0, 110.0, 121.0], 2).unwrap() - 0.21).abs() < 1e-9); // 121/100−1
        assert_eq!(latest_signal(&[100.0, 110.0], 5), None, "yetersiz mum → None");
    }
}
