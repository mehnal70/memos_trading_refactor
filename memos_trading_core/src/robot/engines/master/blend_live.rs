// src/robot/engines/master/blend_live.rs — İKİ-FAKTÖR HARMAN adanmış mod (Faz 2, ince sarmalayıcı).
//
// BLEND_LIVE_ENABLED iken sepet sembolleri TEK market-nötr kitapla yönetilir; sıralama skoru iki dik
// faktörün KESİTSEL z-score harmanı: w_mom·z(momentum) + w_carry·z(carry). Doğrulanmış optimal
// carry-ağırlıklı (w_carry≈0.6) → birleşik Sharpe 0.91, NW-t 2.49, +%27 tekilden ([[project_funding_carry]]).
// Tek kitap → tek-pozisyon/sembol invariantı temiz, değişken-ağırlık GEREKMEZ (eşit-ağırlık 1/k korunur):
// seçenek B, net-ağırlıktan (C) daha az invaziv, ölçülen edge'e ayrık-sepetten (A) yakın. Sinyaller
// momentum_signals (xs_live) + carry_signals (carry_live) ORTAK helper'larından → sıfır kod tekrarı,
// harman yalnız z-score birleştirme. [[feedback_modular_dry_perf]]
use super::*;

/// Harman kitabı pozisyonlarının strateji/trade_type etiketi (maker icra + kapanış muhasebesi bununla tanır).
pub(crate) const BLEND_STRATEGY_TAG: &str = "BLEND_FACTOR";

impl Engine {
    /// İKİ-FAKTÖR HARMAN adanmış mod cycle adımı: `book_core::process_book` ortak motorunu z-score harman
    /// sinyal üreteci + carry-dominant kadans (rebalance_bars) ile çağırır. Mod kapalı/sepet yetersiz → no-op.
    pub(crate) async fn process_blend_book(state: &Arc<Mutex<AppState>>) {
        let (cfg, tuning, db_path, mom_lb, carry_lb, fmax, flimit, wm, wc, closed_only) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let bl = match st.brain.parameters.read().ok().map(|p| p.blend_live.clone()) {
                Some(x) if x.enabled && x.top_k >= 1 && x.symbols.len() >= 2 * x.top_k => x,
                _ => return,
            };
            let cfg = super::book_core::BookConfig {
                symbols: bl.symbols.clone(),
                interval: bl.interval.clone(),
                lookback: bl.mom_lookback, // teşhis logu (momentum bacağı)
                top_k: bl.top_k,
                exit_buffer: bl.exit_buffer,
                momentum: true, // harman skoru yüksek = long (z_m yüksek momentum + z_c düşük funding)
                position_pct: bl.position_pct,
                leverage: bl.leverage,
                regime_gate: bl.regime_gate,
                max_drawdown_pct: bl.max_drawdown_pct,
                cb_cooldown_secs: bl.cb_cooldown_secs,
                take_profit_pct: bl.take_profit_pct,
                tp_cooldown_secs: bl.tp_cooldown_secs,
                // Harman carry-ağırlıklı (düşük-turnover faktör baskın) → carry kadansı: ≥14 bar.
                rebalance_min_bars: bl.rebalance_bars.max(1),
            };
            (cfg, Arc::clone(&st.tuning), st.config.db_path.clone(),
             bl.mom_lookback, bl.carry_lookback, bl.funding_max_age_secs, bl.funding_limit,
             bl.weight_momentum, bl.weight_carry, st.tuning.signal_closed_bar_only)
        };

        let interval_secs = crate::robot::data_pipeline::DataNormalizer::parse_interval(&cfg.interval)
            .max(1) as i64;
        let db_path_sig = db_path.clone();
        // Kesitsel HARMAN üreteci: iki dik faktörü ORTAK helper'lardan al, z-score'la birleştir (DRY).
        // Yalnız her iki faktörde de skoru olan semboller harmana girer (blend_zscores kesişimi).
        let signal_source = move |candles_map: &std::collections::HashMap<String, Vec<Candle>>| {
            let mom = super::xs_live::momentum_signals(candles_map, interval_secs, closed_only, mom_lb);
            let carry = super::carry_live::carry_signals(
                candles_map, &db_path_sig, carry_lb, interval_secs, fmax, flimit);
            super::book_core::blend_zscores(&mom, &carry, wm, wc)
        };

        Self::process_book(
            state, &cfg, super::book_core::BookKind::Blend, &tuning, &db_path, signal_source,
        ).await;
    }
}
