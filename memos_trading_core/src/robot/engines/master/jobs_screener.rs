// src/robot/engines/master/jobs_screener.rs — Screener job: sembol havuzu skor + seçim.
// Faz 2 modülerleştirme: jobs.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

impl Engine {

    /// 🔭 Sembol tarayıcısı ("screener" trigger):
    ///
    /// 1) Aday havuzu: SQLite `list_symbols(db_path)` + `SCREENER_EXTRA_SYMBOLS`
    ///    env (virgülle ayrılmış). Pinned semboller her durumda korunur.
    /// 2) Her aday için `score_symbol` ile aktif strateji + sabit varsayılan
    ///    TP/SL/PS kullanarak hızlı backtest → composite skor.
    /// 3) `select_top_n_diff` orchestrator'ın mevcut worker listesi ile
    ///    karşılaştırıp eklenecek/düşürülecek sembolleri çıkartır.
    /// 4) `register` / `stop_symbol` çağrılarıyla uygulanır; özet log basılır.
    ///
    /// Env override:
    ///   - `SCREENER_TOP_N`           (default 8)
    ///   - `SCREENER_CANDLE_LIMIT`    (default 500)
    ///   - `SCREENER_MIN_VOLUME`      (default 0.0)
    ///   - `SCREENER_EXTRA_SYMBOLS`   (örn. "BNBUSDT,ADAUSDT")
    ///   - `SCREENER_HTF_BIAS`        (default 0.2) — sembol SEÇİMİNE üst-TF trend
    ///     hizasını additif katar (boğa +, ayı −). multi_tf.enabled=false veya
    ///     0.0 → kapalı (saf tek-TF backtest sıralaması, legacy davranış).
    pub(crate) fn run_screener_job(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        use crate::robot::screener::{score_symbol, select_top_n_diff, HtfBias, ScreenerScore};

        log::info!("🔭 E2: Screener çalışıyor...");

        // 1) State'ten yapı yapısı + kapasite + pinned + strateji + blocked + multi-TF.
        let (db_path, market, interval, pinned, blocked, active_strategy, capital,
             max_workers, current_workers, multi_tf_enabled, multi_tf_min,
             data_gate_enabled, health_th) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let strat = st.brain.live_strategy.read()
                .map(|s| s.clone()).unwrap_or_else(|_| "MA_CROSSOVER".to_string());
            let strat = if strat.eq_ignore_ascii_case("default")
                       || strat.eq_ignore_ascii_case("auto")
                       || strat.is_empty() { "MA_CROSSOVER".to_string() } else { strat };
            let (max_w, current) = st.fleet.symbol_orchestrator.read().ok().map(|o| {
                let cur: Vec<String> = o.workers.keys().cloned().collect();
                (o.max_workers, cur)
            }).unwrap_or((16, vec![]));
            // Multi-TF gate: sinyal yolundakiyle aynı param kaynağı (ParameterStore).
            let (mtf_on, mtf_min) = st.brain.parameters.read().ok()
                .map(|p| (p.multi_tf.enabled, p.multi_tf.min_required))
                .unwrap_or((true, crate::robot::data_pipeline::HTF_MIN_REQUIRED));
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
            )
        };

        // 2) Env override'ları.
        let top_n: usize = env_parse("SCREENER_TOP_N", 8);
        let limit: usize = env_parse("SCREENER_CANDLE_LIMIT", 500);
        let min_volume: f64 = env_parse("SCREENER_MIN_VOLUME", 0.0);
        let extras: Vec<String> = std::env::var("SCREENER_EXTRA_SYMBOLS").ok()
            .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default();
        // HTF bias delta — 0 veya multi_tf kapalıysa HTF yüklemeden saf tek-TF sıralama.
        let htf_bias_delta: f64 = env_parse("SCREENER_HTF_BIAS", 0.2);
        let htf_aware = multi_tf_enabled && htf_bias_delta > 0.0;

        // 3) Aday havuzu — config.market + config.interval'a uyan SQLite sembolleri
        //    + env extras (dedupe). Market segmentasyonu sayesinde örn. futures
        //    profilinde BIST sembolleri (AKBNK, AGHOL...) havuza girmez; crypto
        //    profilinde de aksi geçerli. blocked_symbols filtresi havuz oluşumunda
        //    uygulanır → engellenmiş semboller skorlama yapmadan elenir.
        let mut pool: Vec<String> = crate::persistence::reader::list_symbols_for_market(
            &db_path, Some(&market), Some(&interval),
        ).unwrap_or_default();
        for e in extras {
            if !pool.contains(&e) { pool.push(e); }
        }
        let blocked_n_before = pool.len();
        pool.retain(|s| !blocked.iter().any(|b| b.eq_ignore_ascii_case(s)));
        let blocked_filtered = blocked_n_before.saturating_sub(pool.len());
        if blocked_filtered > 0 {
            push_state_log(state, format!(
                "🚫 Screener: {} engellenmiş sembol havuzdan çıkarıldı (blocked_symbols)",
                blocked_filtered,
            ));
        }
        if pool.is_empty() {
            push_state_log(state, format!(
                "🔭 Screener: havuz boş (market={} interval={} için DB'de sembol yok ve SCREENER_EXTRA_SYMBOLS verilmedi)",
                market, interval,
            ));
            return Ok(());
        }

        push_state_log(state, format!(
            "🔭 Screener: havuz={} aday (market={} interval={}), top_n={} max_workers={} strateji={} htf_bias={}",
            pool.len(), market, interval, top_n, max_workers, active_strategy,
            if htf_aware { format!("±{:.2}", htf_bias_delta) } else { "kapalı".to_string() },
        ));

        // 4) Her aday için skor (paralel — rayon).
        //    htf_aware ise her aday için HTF mumları yüklenip composite'e trend
        //    hizası katılır (sinyal yoluyla aynı load_htf_candles + SMA(10/30)).
        use rayon::prelude::*;
        // Faz 3 veri-sağlık kapısı: bayat/sparse/gappy sembol×interval havuza girmez.
        let unhealthy_skips = std::sync::atomic::AtomicUsize::new(0);
        let mut scored: Vec<(String, ScreenerScore)> = pool.par_iter().filter_map(|sym| {
            let candles = crate::persistence::reader::read_candles_market(&db_path, sym, &interval, &market, limit).ok()?;
            if candles.len() < 50 { return None; }
            if data_gate_enabled {
                let h = crate::robot::data_pipeline::CandleHealth::from_candles(&candles, &interval);
                if !h.is_healthy(&health_th, &interval) {
                    unhealthy_skips.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return None;
                }
            }
            let htf_vec = if htf_aware {
                crate::robot::data_pipeline::load_htf_candles(&db_path, sym, &interval, &market, multi_tf_min)
            } else {
                Vec::new()
            };
            let htf_slice = if htf_vec.is_empty() { None } else { Some(htf_vec.as_slice()) };
            let s = score_symbol(&candles, &active_strategy, 4.0, 2.0, 0.3, capital, htf_slice, htf_bias_delta);
            if s.avg_volume < min_volume { return None; }
            Some((sym.clone(), s))
        }).collect();

        let n_unhealthy = unhealthy_skips.load(std::sync::atomic::Ordering::Relaxed);
        if n_unhealthy > 0 {
            push_state_log(state, format!(
                "🩺 Veri-sağlık kapısı: {} sembol elendi (bayat/sparse/gappy; min_rows={} max_gap={:.0}% max_stale={}bar)",
                n_unhealthy, health_th.min_rows, health_th.max_gap_pct, health_th.max_stale_bars,
            ));
        }

        // 5) Composite skoruna göre sıralı (büyükten küçüğe).
        scored.sort_by(|a, b| b.1.composite.partial_cmp(&a.1.composite)
            .unwrap_or(std::cmp::Ordering::Equal));

        // 6) Selection delta.
        let diff = select_top_n_diff(&current_workers, &pinned, &scored, top_n, max_workers);

        // 7) Orchestrator'a uygula.
        let (added_ok, removed_ok) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let mut orch = st.fleet.symbol_orchestrator.write()
                .map_err(|e| format!("orchestrator lock: {}", e))?;
            let mut added = 0usize;
            for sym in &diff.to_add {
                if orch.register(sym, &market, &interval).is_some() { added += 1; }
            }
            let mut removed = 0usize;
            for sym in &diff.to_remove {
                if orch.stop_symbol(sym) { removed += 1; }
            }
            (added, removed)
        };

        // 8) Telemetri — özet + top 5 ayrıntı.
        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🔭 Screener ✓ skorlandı={} seçilen={} eklendi={} düşürüldü={}",
                scored.len(), diff.selected.len(), added_ok, removed_ok,
            ));
            let top_brief: Vec<String> = scored.iter().take(5)
                .map(|(name, s)| {
                    let b = match s.htf_bias {
                        HtfBias::Bullish => "↑",
                        HtfBias::Bearish => "↓",
                        HtfBias::Neutral => "·",
                    };
                    format!(
                        "{name}(c={:.2}{b} sh={:.2} wr={:.0}% n={})",
                        s.composite, s.sharpe, s.win_rate, s.trades,
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

        Ok(())
    }
}
