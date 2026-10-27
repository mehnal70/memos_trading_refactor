// src/robot/engines/master/jobs_screener.rs — Screener job: sembol havuzu skor + seçim.
// Faz 2 modülerleştirme: jobs.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

impl Engine {
    pub(crate) fn run_screener_job(
        state: &Arc<Mutex<AppState>>,
    ) -> std::result::Result<(), String> {
        use crate::robot::screener::{score_symbol, select_top_n_diff, HtfBias, ScreenerScore};

        log::info!("🔭 E2: Screener çalışıyor...");

        let (
            db_path,
            market,
            interval,
            pinned,
            blocked,
            active_strategy,
            capital,
            max_workers,
            current_workers,
            multi_tf_enabled,
            multi_tf_min,
            data_gate_enabled,
            health_th,
            edge_symbols,
        ) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let strat = st
                .brain
                .live_strategy
                .read()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "MA_CROSSOVER".to_string());
            let strat = if strat.eq_ignore_ascii_case("default")
                || strat.eq_ignore_ascii_case("auto")
                || strat.is_empty()
            {
                "MA_CROSSOVER".to_string()
            } else {
                strat
            };
            let (max_w, current) = st
                .fleet
                .symbol_orchestrator
                .read()
                .ok()
                .map(|o| {
                    let cur: Vec<String> = o.workers.keys().cloned().collect();
                    (o.max_workers, cur)
                })
                .unwrap_or((16, vec![]));
            let (mtf_on, mtf_min, edges) = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| {
                    (
                        p.multi_tf.enabled,
                        p.multi_tf.min_required,
                        p.symbol_strategy
                            .keys()
                            .cloned()
                            .collect::<std::collections::HashSet<String>>(),
                    )
                })
                .unwrap_or((
                    true,
                    crate::robot::data_pipeline::HTF_MIN_REQUIRED,
                    Default::default(),
                ));
            (
                st.config.db_path.clone(),
                st.config.market.clone(),
                st.config.interval.clone(),
                st.config.pinned_symbols.clone(),
                st.config.blocked_symbols.clone(),
                strat,
                st.finance.equity.max(1.0),
                max_w,
                current,
                mtf_on,
                mtf_min,
                st.tuning.data_gate_enabled,
                st.tuning.health_thresholds(),
                edges,
            )
        };

        let top_n: usize = env_parse("SCREENER_TOP_N", 8);
        let limit: usize = env_parse("SCREENER_CANDLE_LIMIT", 500);
        let min_volume: f64 = env_parse("SCREENER_MIN_VOLUME", 0.0);
        let min_score: f64 = env_parse("SCREENER_MIN_SCORE", 0.0);
        let extras: Vec<String> = std::env::var("SCREENER_EXTRA_SYMBOLS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let htf_bias_delta: f64 = env_parse("SCREENER_HTF_BIAS", 0.2);
        let htf_aware = multi_tf_enabled && htf_bias_delta > 0.0;
        let edge_bonus_delta: f64 = env_parse("SCREENER_EDGE_BONUS", 0.3);
        let edge_aware = edge_bonus_delta > 0.0 && !edge_symbols.is_empty();
        let mut pool: Vec<String> = std::env::var("SCREENER_SYMBOLS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|symbol| !symbol.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .filter(|symbols: &Vec<String>| !symbols.is_empty())
            .unwrap_or_else(|| {
                crate::persistence::reader::list_symbols_for_market(
                    &db_path,
                    Some(&market),
                    Some(&interval),
                )
                .unwrap_or_default()
            });
        for e in extras {
            if !pool.contains(&e) {
                pool.push(e);
            }
        }
        let blocked_n_before = pool.len();
        pool.retain(|s| !blocked.iter().any(|b| b.eq_ignore_ascii_case(s)));
        let blocked_filtered = blocked_n_before.saturating_sub(pool.len());
        if blocked_filtered > 0 {
            push_state_log(
                state,
                format!(
                    "🚫 Screener: {} engellenmiş sembol havuzdan çıkarıldı (blocked_symbols)",
                    blocked_filtered,
                ),
            );
        }
        if pool.is_empty() {
            push_state_log(state, format!(
                "🔭 Screener: havuz boş (market={} interval={} için DB'de sembol yok ve SCREENER_EXTRA_SYMBOLS verilmedi)",
                market, interval,
            ));
            return Ok(());
        }

        push_state_log(state, format!(
            "🔭 Screener: havuz={} aday (market={} interval={}), top_n={} max_workers={} strateji={} htf_bias={} edge_bonus={}",
            pool.len(), market, interval, top_n, max_workers, active_strategy,
            if htf_aware { format!("±{:.2}", htf_bias_delta) } else { "kapalı".to_string() },
            if edge_aware { format!("+{:.2}×{}", edge_bonus_delta, edge_symbols.len()) } else { "kapalı".to_string() },
        ));

        use rayon::prelude::*;
        let unhealthy_skips = std::sync::atomic::AtomicUsize::new(0);
        let mut scored: Vec<(String, ScreenerScore)> = pool
            .par_iter()
            .filter_map(|sym| {
                let candles =
                    Self::read_candles_routed(state, &db_path, sym, &interval, limit).ok()?;
                if candles.len() < 50 {
                    return None;
                }
                if data_gate_enabled {
                    let h = crate::robot::data_pipeline::CandleHealth::from_candles(
                        &candles, &interval,
                    );
                    if !h.is_healthy(&health_th, &interval) {
                        unhealthy_skips.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        return None;
                    }
                }
                let htf_vec = if htf_aware {
                    let account_profile = state
                        .lock()
                        .ok()
                        .map(|st| st.config.venue_profile.as_str().to_string())
                        .unwrap_or_else(|| "default".to_string());
                    match Self::resolve_read_ns(state, sym) {
                        Some((ex, mk, bare)) => {
                            crate::robot::data_pipeline::load_htf_candles_ns_for_profile(
                                &db_path,
                                &bare,
                                &interval,
                                &ex,
                                &mk,
                                &account_profile,
                                multi_tf_min,
                            )
                        }
                        None => crate::robot::data_pipeline::load_htf_candles(
                            &db_path,
                            sym,
                            &interval,
                            &market,
                            multi_tf_min,
                        ),
                    }
                } else {
                    Vec::new()
                };
                let htf_slice = if htf_vec.is_empty() {
                    None
                } else {
                    Some(htf_vec.as_slice())
                };
                let edge_bonus = if edge_aware && edge_symbols.contains(sym) {
                    edge_bonus_delta
                } else {
                    0.0
                };
                let s = score_symbol(
                    &candles,
                    &active_strategy,
                    4.0,
                    2.0,
                    0.3,
                    capital,
                    htf_slice,
                    htf_bias_delta,
                    edge_bonus,
                );
                if s.avg_volume < min_volume {
                    return None;
                }
                return Some((sym.clone(), s));
            })
            .collect();

        let n_unhealthy = unhealthy_skips.load(std::sync::atomic::Ordering::Relaxed);
        if n_unhealthy > 0 {
            push_state_log(state, format!(
                "🩺 Veri-sağlık kapısı: {} sembol elendi (bayat/sparse/gappy; min_rows={} max_gap={:.0}% max_stale={}bar)",
                n_unhealthy, health_th.min_rows, health_th.max_gap_pct, health_th.max_stale_bars,
            ));
        }

        scored.sort_by(|a, b| {
            b.1.composite
                .partial_cmp(&a.1.composite)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let below_floor = if min_score > 0.0 {
            scored
                .iter()
                .filter(|(_, s)| s.sharpe <= 0.0 || s.base_composite < min_score)
                .count()
        } else {
            0
        };
        let diff = select_top_n_diff(
            &current_workers,
            &pinned,
            &scored,
            top_n,
            max_workers,
            min_score,
        );

        let (added_ok, removed_ok) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let mut orch = st
                .fleet
                .symbol_orchestrator
                .write()
                .map_err(|e| format!("orchestrator lock: {}", e))?;
            let mut added = 0usize;
            for sym in &diff.to_add {
                if orch.register(sym, &market, &interval).is_some() {
                    added += 1;
                }
            }
            let mut removed = 0usize;
            for sym in &diff.to_remove {
                if orch.stop_symbol(sym) {
                    removed += 1;
                }
            }
            (added, removed)
        };

        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🔭 Screener ✓ skorlandı={} seçilen={} eklendi={} düşürüldü={}{}",
                scored.len(),
                diff.selected.len(),
                added_ok,
                removed_ok,
                if min_score > 0.0 {
                    format!(
                        " · edge-taban={:.2} (altı eleme={})",
                        min_score, below_floor
                    )
                } else {
                    String::new()
                },
            ));
            let top_brief: Vec<String> = scored
                .iter()
                .take(5)
                .map(|(name, s)| {
                    let b = match s.htf_bias {
                        HtfBias::Bullish => "↑",
                        HtfBias::Bearish => "↓",
                        HtfBias::Neutral => "·",
                    };
                    let edge = if s.edge_bonus > 0.0 { "🌱" } else { "" };
                    format!(
                        "{name}{edge}(c={:.2}{b} sh={:.2} wr={:.0}% n={})",
                        s.composite, s.sharpe, s.win_rate, s.trades
                    )
                })
                .collect();
            if !top_brief.is_empty() {
                st.push_log(format!("🔭 Top: {}", top_brief.join(" | ")));
            }
            if !diff.to_add.is_empty() {
                st.push_log(format!("🔭 + {}", diff.to_add.join(", ")));
            }
            if !diff.to_remove.is_empty() {
                st.push_log(format!("🔭 − {}", diff.to_remove.join(", ")));
            }
        }

        return Ok(());
    }
}
