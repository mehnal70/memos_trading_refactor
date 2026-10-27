// src/robot/engines/master/jobs_backtest.rs — Backtest job (walk-forward grid + otonom strateji seçimi).
// Faz 2 modülerleştirme: jobs.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

impl Engine {
    /// 🔬 Backtest (Faz 4 - "backtest" trigger):
    /// Daha geniş bir grid ile composite score'u en yüksek olan profili seçer ve
    /// brain.live_strategy'i otonom değiştirir.
    pub(crate) fn run_backtest_job(
        state: &Arc<Mutex<AppState>>,
    ) -> std::result::Result<(), String> {
        log::info!("🔬 E2: Walk-Forward Backtest başlatıldı (grid: 6×4×3)...");

        let (symbol, interval, db_path, capital, use_htf, _market, data_gate, health_th) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            // Backtest, canlının multi-TF'ini aynalasın: multi_tf.enabled açıksa WF
            // seçimi + param araması da HTF filtresini görür (canlı ile tek-davranış).
            let use_htf = st
                .brain
                .parameters
                .read()
                .map(|p| p.multi_tf.enabled)
                .unwrap_or(false);
            (
                st.config.symbol.clone(),
                st.config.interval.clone(),
                st.config.db_path.clone(),
                st.finance.equity,
                use_htf,
                st.config.market.clone(),
                st.tuning.data_gate_enabled,
                st.tuning.health_thresholds(),
            )
        };

        // Giriş kalitesi edge filtresi (#4): backtest, canlı process_symbol_cycle'ın
        // `edge < edge_threshold ⇒ REDDEDİLDİ` hunisini aynalasın → param araması
        // zayıf/ters-momentum girişlerini canlıyla aynı eler (özellikle 1m'de aşırı-işlem
        // + komisyon erozyonunu keser). Env BACKTEST_EDGE_FILTER (bkz parse_edge_filter).
        // Default (unset): canlıyı aynala → Some(dynamic_edge_threshold(0)) = 0.20
        // (use_htf'in "canlıyı aynala" deseniyle aynı). 0/false → kapat (legacy).
        // A/B (bt_ab_entry_quality, gerçek DB): 1m'de net kazanç (n %13-28↓, PF
        // 1.65→1.91), 1h'de backtest dürüstleşir (canlının reddettiği zayıf-edge
        // girişleri elenir). Daha katı için BACKTEST_EDGE_FILTER=0.35.
        const EDGE_FILTER_DEFAULT_ON: bool = true;
        let on_value = Self::dynamic_edge_threshold(0.0);
        let edge_min = parse_edge_filter(
            std::env::var("BACKTEST_EDGE_FILTER").ok(),
            if EDGE_FILTER_DEFAULT_ON {
                Some(on_value)
            } else {
                None
            },
            on_value,
        );

        // Orderbook icrası (#c) — opt-in (default kapalı). BACKTEST_ORDERBOOK=liquid|illiquid
        // ile açılır → WF seçimi + param araması slippage'i de görür (canlı paper ile aynı motor).
        let orderbook_sim: Option<String> = std::env::var("BACKTEST_ORDERBOOK")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| {
                !s.is_empty()
                    && !s.eq_ignore_ascii_case("off")
                    && !s.eq_ignore_ascii_case("0")
                    && !s.eq_ignore_ascii_case("none")
            });

        push_state_log(
            state,
            format!(
            "🔬 Backtest başladı: sembol={} aralık={} kapital=${:.0} edge_filtre={} orderbook={}",
            symbol, interval, capital,
            edge_min.map(|t| format!("≥{:.2}", t)).unwrap_or_else(|| "kapalı".into()),
            orderbook_sim.as_deref().unwrap_or("kapalı"),
        ),
        );

        // Veri derinliği — env BACKTEST_CANDLE_LIMIT (default 5000). Sağlıklı
        // backtest için istatistiksel anlamlı işlem + çok-rejim kapsama gerekir;
        // eski sabit 1500 sığdı (1m'de ~1 gün, tek rejim). TF'e göre öneri:
        // 1h≈6-12 ay (4.3k-8.8k), 1m sadece infaz simülasyonu (fee/gürültü baskın).
        let candle_limit: usize = env_parse("BACKTEST_CANDLE_LIMIT", 5000usize).max(300);

        // Walk-Forward konfigürasyonu — env'den override edilebilir.
        // Varsayılan IS=200 / OOS=50 / step=50. Daha derin veride IS/OOS'u TF'e
        // ölçeklemek için WALK_FORWARD_* env'leri kullanılır (örn. 1m'de IS≥1000).
        let wf_is: usize = env_parse("WALK_FORWARD_IS_BARS", 200);
        let wf_oos: usize = env_parse("WALK_FORWARD_OOS_BARS", 50);
        let wf_step: usize = env_parse("WALK_FORWARD_STEP_BARS", 50);
        let wf_min = wf_is + wf_oos;

        // Route-farkında okuma: global `market` yerine sembolün venue namespace'i.
        // (Çok-venue profilde global market MT5 sembolünü yanlış kovada aratıyordu.)
        let candles = Self::read_candles_routed(state, &db_path, &symbol, &interval, candle_limit)
            .map_err(|e| format!("read_candles: {}", e))?;
        if candles.len() < wf_min {
            return Err(format!(
                "yetersiz mum verisi: {} mum (walk-forward için ≥{} gerekli)",
                candles.len(),
                wf_min,
            ));
        }
        // Faz 3 veri-sağlık kapısı: bayat/gappy veride backtest = yanıltıcı verdikt → atla.
        if data_gate {
            let h = crate::robot::data_pipeline::CandleHealth::from_candles(&candles, &interval);
            if !h.is_healthy(&health_th, &interval) {
                return Err(format!(
                    "veri-sağlık kapısı: {} {} atlandı (satır={} gap={:.0}% bayat={}s)",
                    symbol, interval, h.rows, h.gap_pct, h.stale_secs,
                ));
            }
        }

        // ─── Canlı ÇIKIŞ MODELİ (TP/SL re-opt) ────────────────────────────────
        // R/R asimetrisinin asıl kaynağı: TP/SL eskiden trailing'siz optimize ediliyordu
        // (BacktestConfig Default → atr_trail_mult=None), ama canlıda çıkışların çoğu
        // TRAILING_STOP ile oluyor → seçilen TP nadiren ateşlenip realized R/R çöküyordu.
        // Artık strateji seçimi + TP/SL/PS araması + yön A/B'si canlının uyguladığı
        // trailing + breakeven ile BİRLİKTE çalışır. Temsili trail mult canlı
        // resolve_atr_mult ile aynı: default_target / serinin_noise_floor, clamp[1.5,30]
        // (per-rejim trail A/B ayrıca ekseni override eder; bu base/temsili değerdir).
        const EXIT_BREAKEVEN_RR: f64 = 1.0;
        let exit_trail_mult: f64 = {
            let default_target = state
                .lock()
                .ok()
                .and_then(|st| {
                    st.brain
                        .parameters
                        .read()
                        .ok()
                        .map(|p| p.target_trail_pct_for_strategy("default"))
                })
                .unwrap_or(0.7);
            match crate::robot::parameters::window_noise_floor_pct(&candles) {
                Some(nf) if nf > 0.0 => (default_target / nf).clamp(1.5, 30.0),
                _ => 2.0,
            }
        };
        push_state_log(
            state,
            format!(
            "🪤 Çıkış modeli (TP/SL re-opt): trail≈{:.1}×ATR + breakeven@RR {:.1} (canlı-temsili)",
            exit_trail_mult, EXIT_BREAKEVEN_RR,
        ),
        );

        // Aday strateji pool'u StrategyRegistry'den otomatik genişler (Faz 4 c2):
        // yeni strateji default_registry()'ye eklendiğinde backtest_job ekstra
        // değişiklik gerektirmez. Alias'lar dahil edilmez (canonical_pool).
        let strat_pool: Vec<String> = super::enforce_trend_strict_pool(
            crate::robot::strategies::default_registry().canonical_pool(),
        );
        if super::trend_strict_mode_enabled() {
            push_state_log(
                state,
                format!(
                    "📏 Trend-Strict aktif: strateji havuzu={}",
                    strat_pool.join(","),
                ),
            );
        }
        let est_windows = candles.len().saturating_sub(wf_min) / wf_step.max(1) + 1;
        push_state_log(
            state,
            format!(
            "🔬 Backtest (Walk-Forward): {} mum, {} strateji × ~{} pencere (IS={} OOS={} step={})",
            candles.len(), strat_pool.len(), est_windows, wf_is, wf_oos, wf_step,
        ),
        );

        // ─── 1) Strateji aday seçimi: her aday için Walk-Forward → OOS sharpe + tutarlılık ───
        //
        // wf_score = avg_oos_sharpe * 0.7 + consistency * 0.3
        // (consistency = kârlı OOS pencerelerinin oranı 0..1)
        // OOS metrikleri overfitting'i engeller; in-sample sharpe'a göre seçim yapılmıyor.
        use crate::robot::backtester::walk_forward::{WalkForwardConfig, WalkForwardTester};
        const WF_CONSISTENCY_WEIGHT: f64 = 0.3;
        const WF_LOG_TOP_N: usize = 5;
        const WF_LOG_MIN_ABS_SCORE: f64 = 0.02;

        // Env-tabanlı eliminasyon katmanı (opt-in/opt-out): aday stratejiler canlıya
        // aday olmadan önce minimum OOS kalitesi eşiğini geçmeli. Tüm adaylar elenirse
        // davranış bozulmaması için legacy fallback (en iyi skor) devreye girer.
        let elim_enabled = !matches!(
            std::env::var("BACKTEST_ELIMINATION_ENABLED")
                .ok()
                .as_deref(),
            Some("0") | Some("false") | Some("off")
        );
        let elim_min_consistency =
            (env_parse::<f64>("BACKTEST_ELIM_MIN_CONSISTENCY_PCT", 8.0).clamp(0.0, 100.0)) / 100.0;
        let elim_min_windows: usize = env_parse("BACKTEST_ELIM_MIN_WINDOWS", 24usize).max(1);
        let elim_min_oos_sharpe: f64 = env_parse("BACKTEST_ELIM_MIN_OOS_SHARPE", -0.05f64);
        let elim_min_score: f64 = env_parse("BACKTEST_ELIM_MIN_SCORE", 0.0f64);
        let elim_min_trades: usize = env_parse("BACKTEST_ELIM_MIN_TRADES", 40usize).max(1);
        let elim_max_binom_pvalue: f64 =
            env_parse("BACKTEST_ELIM_MAX_BINOM_PVALUE", 0.20f64).clamp(0.0, 1.0);
        let elim_bootstrap_enabled = !matches!(
            std::env::var("BACKTEST_ELIM_BOOTSTRAP_ENABLED")
                .ok()
                .as_deref(),
            Some("0") | Some("false") | Some("off")
        );
        let elim_bootstrap_samples: usize =
            env_parse("BACKTEST_ELIM_BOOTSTRAP_SAMPLES", 300usize).max(20);
        let elim_bootstrap_ci_pct: f64 =
            env_parse("BACKTEST_ELIM_BOOTSTRAP_CI_PCT", 5.0f64).clamp(0.1, 49.9);
        let elim_bootstrap_min_mean_pnl_pct: f64 =
            env_parse("BACKTEST_ELIM_BOOTSTRAP_MIN_MEAN_PNL_PCT", -0.10f64);

        // Binom tek-yanlı anlamlılık p-değeri (H0: pencere başına karlı olma olasılığı 0.5).
        let binom_tail_pvalue = |n: usize, k: usize| -> f64 {
            if n == 0 {
                return 1.0;
            }
            let kk = k.min(n);
            let mut pmf = 0.5_f64.powi(n as i32); // pmf(0)
            let mut cdf_below = 0.0_f64; // P(X <= k-1)
            for i in 0..kk {
                cdf_below += pmf;
                pmf *= (n - i) as f64 / (i + 1) as f64;
            }
            (1.0 - cdf_below).clamp(0.0, 1.0)
        };

        // OOS pencere ortalama PnL% için basit bootstrap alt güven sınırı.
        let bootstrap_mean_ci_lower = |vals: &[f64], samples: usize, tail_pct: f64| -> f64 {
            if vals.is_empty() {
                return f64::NEG_INFINITY;
            }
            let n = vals.len();
            if n == 1 {
                return vals[0];
            }
            let mut state: u64 = 0x9E37_79B9_7F4A_7C15u64 ^ (n as u64 * 0xA24B_AED4_963E_E407u64);
            let mut means: Vec<f64> = Vec::with_capacity(samples);
            for _ in 0..samples {
                let mut sum = 0.0_f64;
                for _ in 0..n {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let idx = (state as usize) % n;
                    sum += vals[idx];
                }
                means.push(sum / n as f64);
            }
            means.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let q = (tail_pct / 100.0).clamp(0.001, 0.499);
            let idx = ((q * samples as f64).floor() as usize).min(samples.saturating_sub(1));
            means[idx]
        };

        push_state_log(state, format!(
            "🧭 nested-WF L1(model): elim={} minC={:.0}% minW={} minTr={} minSh={:.2} minS={:.2} maxP={:.2} bs={} ci{}%>= {:+.2}%",
            if elim_enabled { "on" } else { "off" },
            elim_min_consistency * 100.0,
            elim_min_windows,
            elim_min_trades,
            elim_min_oos_sharpe,
            elim_min_score,
            elim_max_binom_pvalue,
            if elim_bootstrap_enabled { "on" } else { "off" },
            elim_bootstrap_ci_pct,
            elim_bootstrap_min_mean_pnl_pct,
        ));

        let mut best_wf: Option<(
            String,
            f64,
            crate::robot::backtester::walk_forward::WalkForwardResult,
        )> = None;
        let mut best_wf_fallback: Option<(
            String,
            f64,
            crate::robot::backtester::walk_forward::WalkForwardResult,
        )> = None;
        let mut wf_log_rows: Vec<(String, f64, f64, f64, usize, usize, f64, f64, bool)> =
            Vec::new();
        let mut eliminated_count = 0usize;

        for name in &strat_pool {
            let wf_cfg = WalkForwardConfig {
                in_sample_bars: wf_is,
                out_of_sample_bars: wf_oos,
                step_bars: wf_step,
                initial_balance: capital,
                strategy_name: name.clone(),
                symbol: symbol.clone(),
                interval: interval.clone(),
                commission_pct: 0.001,
                use_htf,
                edge_min_score: edge_min,
                orderbook_sim: orderbook_sim.clone(),
                atr_trail_mult: Some(exit_trail_mult),
                breakeven_at_rr: Some(EXIT_BREAKEVEN_RR),
                pyramiding_enabled: true,
            };
            let Some(wf_res) = WalkForwardTester::new(wf_cfg).run(&candles) else {
                push_state_log(state, format!("🔬   aday {} → WF sonuç alınamadı", name));
                continue;
            };

            let wf_score = wf_res.avg_oos_sharpe * (1.0 - WF_CONSISTENCY_WEIGHT)
                + wf_res.consistency_score * WF_CONSISTENCY_WEIGHT;
            let windows = wf_res.windows.len();
            let windows_with_trades = wf_res
                .windows
                .iter()
                .filter(|w| w.oos_metrics.trades > 0)
                .count();
            let profitable_windows = wf_res
                .windows
                .iter()
                .filter(|w| w.oos_metrics.trades > 0 && w.oos_metrics.pnl_pct > 0.0)
                .count();
            let total_trades: usize = wf_res.windows.iter().map(|w| w.oos_metrics.trades).sum();
            let pvalue = binom_tail_pvalue(windows_with_trades, profitable_windows);
            let oos_window_pnls: Vec<f64> = wf_res
                .windows
                .iter()
                .filter(|w| w.oos_metrics.trades > 0)
                .map(|w| w.oos_metrics.pnl_pct)
                .collect();
            let bootstrap_ci_lower = if elim_bootstrap_enabled {
                bootstrap_mean_ci_lower(
                    &oos_window_pnls,
                    elim_bootstrap_samples,
                    elim_bootstrap_ci_pct,
                )
            } else {
                f64::NEG_INFINITY
            };
            let passes_elimination = !elim_enabled
                || (wf_res.consistency_score >= elim_min_consistency
                    && windows >= elim_min_windows
                    && total_trades >= elim_min_trades
                    && wf_res.avg_oos_sharpe >= elim_min_oos_sharpe
                    && wf_score >= elim_min_score
                    && pvalue <= elim_max_binom_pvalue
                    && (!elim_bootstrap_enabled
                        || bootstrap_ci_lower >= elim_bootstrap_min_mean_pnl_pct));
            wf_log_rows.push((
                name.clone(),
                wf_res.avg_oos_sharpe,
                wf_res.consistency_score,
                wf_score,
                windows,
                total_trades,
                pvalue,
                bootstrap_ci_lower,
                passes_elimination,
            ));
            if best_wf_fallback
                .as_ref()
                .map(|(_, s, _)| *s)
                .unwrap_or(f64::NEG_INFINITY)
                < wf_score
            {
                best_wf_fallback = Some((name.clone(), wf_score, wf_res.clone()));
            }
            if passes_elimination {
                if best_wf
                    .as_ref()
                    .map(|(_, s, _)| *s)
                    .unwrap_or(f64::NEG_INFINITY)
                    < wf_score
                {
                    best_wf = Some((name.clone(), wf_score, wf_res));
                }
            } else {
                eliminated_count += 1;
            }
        }

        if elim_enabled {
            push_state_log(state, format!(
                "🧹 eliminasyon: minC={:.0}% minW={} minTr={} minSh={:.2} minS={:.2} maxP={:.2} bs={} ci{}%>= {:+.2}% ⇒ elenen={}/{}",
                elim_min_consistency * 100.0,
                elim_min_windows,
                elim_min_trades,
                elim_min_oos_sharpe,
                elim_min_score,
                elim_max_binom_pvalue,
                if elim_bootstrap_enabled { "on" } else { "off" },
                elim_bootstrap_ci_pct,
                elim_bootstrap_min_mean_pnl_pct,
                eliminated_count,
                wf_log_rows.len(),
            ));
        }

        if !wf_log_rows.is_empty() {
            wf_log_rows.sort_by(|a, b| {
                b.3.partial_cmp(&a.3)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            });
            let meaningful: Vec<String> = wf_log_rows
                .iter()
                .filter(
                    |(_, sharpe, consistency, score, _, trades, pvalue, ci, pass)| {
                        score.abs() >= WF_LOG_MIN_ABS_SCORE
                            || sharpe.abs() >= WF_LOG_MIN_ABS_SCORE
                            || *consistency > 0.0
                            || *trades > 0
                            || pvalue.is_finite()
                            || ci.is_finite()
                            || *pass
                    },
                )
                .take(WF_LOG_TOP_N)
                .map(
                    |(name, sharpe, consistency, score, windows, trades, pvalue, ci, pass)| {
                        let tag = if *pass { "✓" } else { "✖" };
                        format!(
                            "{}{}[Sh={:.2} C={:.0}% W={} Tr={} p={:.2} CI={:+.2}% S={:.3}]",
                            tag,
                            name,
                            sharpe,
                            consistency * 100.0,
                            windows,
                            trades,
                            pvalue,
                            ci,
                            score,
                        )
                    },
                )
                .collect();
            if meaningful.is_empty() {
                push_state_log(
                    state,
                    format!(
                        "🔬 aday taraması: {} strateji değerlendirildi, anlamlı ayrışma yok",
                        wf_log_rows.len(),
                    ),
                );
            } else {
                push_state_log(
                    state,
                    format!(
                        "🔬 aday taraması top{} / {}: {}",
                        meaningful.len(),
                        wf_log_rows.len(),
                        meaningful.join(" · "),
                    ),
                );
            }
        }

        if best_wf.is_none() {
            if let Some((name, score, res)) = best_wf_fallback {
                push_state_log(
                    state,
                    format!(
                        "⚠️ eliminasyon tüm adayları eledi; fallback: {} (S={:.3})",
                        name, score,
                    ),
                );
                best_wf = Some((name, score, res));
            }
        }

        let (best_name, best_wf_score, best_wf_res) =
            best_wf.ok_or_else(|| "Hiçbir strateji walk-forward sonuç üretemedi".to_string())?;

        // ─── 2) Kazanan strateji için PS dahil final parametre optimizasyonu (tüm veri) ───
        //
        // Walk-Forward'da quick_optimize sadece TP/SL üzerinde tarıyor; pozisyon boyutu
        // (PS) burada belirlenir ki best_params üç ekseni de kapsasın.
        let final_opt = crate::robot::backtester::parameter_optimizer::ParameterOptimizer::new(
            symbol.clone(),
            interval.clone(),
            capital,
            best_name.clone(),
        )
        .with_edge_min_score(edge_min)
        .with_orderbook_sim(orderbook_sim.clone())
        .with_exit_model(Some(exit_trail_mult), Some(EXIT_BREAKEVEN_RR));
        push_state_log(
            state,
            format!(
                "🧭 nested-WF L2(param): optimize '{}' (TP/SL/PS) tüm veri üstünde başlıyor",
                best_name,
            ),
        );
        let final_res = final_opt
            .optimize_parallel(
                &candles,
                (2.0, 8.0, 1.0), // TP %2 → %8, step 1
                (1.0, 4.0, 1.0), // SL %1 → %4, step 1
                (0.1, 0.4, 0.1), // PS  0.1 → 0.4
            )
            .map_err(|e| format!("final optimize_parallel: {:?}", e))?;

        // ─── 2b) Kazanan stratejinin YAPISAL parametreleri (param_spec araması) ───
        //
        // Faz 1b: strateji KENDİ param_spec()'ini bildirir; HyperOpt::spec_search bu
        // uzaydan örnekler. Bulunan en iyi set ParameterStore.strategy_params'a yazılır
        // ve canlı cycle generate_signal'a verilir (eskiden her zaman default geçiliyordu).
        // Yapısal paramı olmayan strateji (PRICE_ACTION/FUNDING) → spec boş → None.
        let best_strategy_params = {
            let specs = crate::robot::strategies::default_registry()
                .make(&best_name)
                .param_spec();
            if specs.is_empty() {
                None
            } else {
                let n_iters: usize = env_parse("BACKTEST_STRATEGY_PARAM_ITERS", 40);
                let bt_cfg = crate::robot::backtester::BacktestConfig {
                    symbol: symbol.clone(),
                    interval: interval.clone(),
                    initial_balance: capital,
                    max_position_size: final_res.best_parameters.max_position_size,
                    take_profit_pct: final_res.best_parameters.take_profit_pct,
                    stop_loss_pct: final_res.best_parameters.stop_loss_pct,
                    strategy_name: best_name.clone(),
                    strategy_params: None,
                    commission_pct: 0.001,
                    breakeven_at_rr: Some(EXIT_BREAKEVEN_RR),
                    atr_trail_mult: Some(exit_trail_mult),
                    partial_tp_ratio: None,
                    position_profile: Some("trend-pyramiding".into()),
                    security_profile: None,
                    use_htf,
                    edge_min_score: edge_min,
                    orderbook_sim: orderbook_sim.clone(),
                    regime_gate: Default::default(),
                    direction: Default::default(),
                    regime_style_fit: false, // otonom backtest yolu nötr; A/B ayrı testte ölçülür
                    atr_sl_mult: None,
                    atr_tp_mult: None,
                    vol_target_pct: None,
                };
                crate::robot::ml_engine::hyperopt::HyperOpt::spec_search(
                    &candles,
                    &specs,
                    n_iters,
                    &bt_cfg,
                    Some(12345),
                )
                .map(|r| (r.best_params, r.best_score, r.combinations_tested))
            }
        };

        // ─── 3) Rejim-bazlı parametre katmanları ──────────────────────────
        //
        // Her WF penceresinin OOS dilimi `classify_regime` ile sınıflandırılır;
        // rejim başına ortanca TP/SL hesaplanır (PS final_res'ten — global).
        // Sonuç ParameterStore.regime_overrides'a yazılır → engine cycle
        // rejime özgü TradeRiskParams ile çalışır (Faz 2 c4 + Faz 3 patch
        // kanalı). REGIME_MIN_SAMPLES env ile override edilebilir, default 2.
        let regime_min_samples: usize = env_parse("REGIME_MIN_SAMPLES", 2);
        let regime_agg = crate::robot::backtester::walk_forward::aggregate_windows_by_regime(
            &candles,
            &best_wf_res.windows,
            |oos_slice| Self::classify_regime(oos_slice).as_str().to_string(),
            regime_min_samples,
        );

        // ─── 3b) Per-rejim YÖN disiplini A/B (otonom RegimePolicy) ───────────
        // Her rejimin OOS pencerelerinde LongOnly vs RegimeDirectional backtest →
        // kazanan regime_overrides[regime].policy'ye yazılır, canlı cycle o rejimde
        // regime_directional_for ile okur (env yerine veri-temelli, per-rejim otonom).
        let dir_ab_base = crate::robot::backtester::BacktestConfig {
            symbol: symbol.clone(),
            interval: interval.clone(),
            initial_balance: capital,
            max_position_size: final_res.best_parameters.max_position_size,
            take_profit_pct: final_res.best_parameters.take_profit_pct,
            stop_loss_pct: final_res.best_parameters.stop_loss_pct,
            strategy_name: best_name.clone(),
            strategy_params: best_strategy_params.as_ref().map(|(sp, _, _)| *sp),
            commission_pct: 0.001,
            use_htf,
            edge_min_score: edge_min,
            // Canlı çıkış modeli — yön A/B'si de trailing'i görür (trail A/B base'i
            // bunu miras alır, ekseni kendi override eder).
            atr_trail_mult: Some(exit_trail_mult),
            breakeven_at_rr: Some(EXIT_BREAKEVEN_RR),
            ..Default::default()
        };
        let regime_dir_map = crate::robot::backtester::walk_forward::evaluate_regime_direction(
            &candles,
            &best_wf_res.windows,
            |oos_slice| Self::classify_regime(oos_slice).as_str().to_string(),
            &dir_ab_base,
            regime_min_samples,
        );

        // ─── 3b-2) Per-rejim TRAILING hedef A/B (R/R asimetrisi lever'ı) ──────────
        // Canlı çıkışların çoğu TRAILING_STOP ile oluyor; R/R'yi trail SIKILIĞI
        // belirliyor (sıkı trail kazancı erken keser). Her rejimin OOS pencerelerinde
        // target_trail_pct adaylarını canlı resolve_atr_mult formülüyle BİREBİR
        // (target/pencere_noise_floor → mult) skorla; kazanan
        // regime_overrides[regime].target_trail_pct'e yazılır, canlı
        // resolve_atr_mult_for_regime numerator'da okur (per-sembol mikro-yapı korunur).
        // Base canlı çıkışı modeller (breakeven_at_rr=1.0) → ölçüm sadık.
        const TRAIL_CANDIDATES: [f64; 8] = [0.5, 0.7, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0];
        let trail_ab_base = crate::robot::backtester::BacktestConfig {
            breakeven_at_rr: Some(1.0),
            ..dir_ab_base.clone()
        };
        let regime_trail_map = crate::robot::backtester::walk_forward::evaluate_regime_trail(
            &candles,
            &best_wf_res.windows,
            |oos_slice| Self::classify_regime(oos_slice).as_str().to_string(),
            &trail_ab_base,
            &TRAIL_CANDIDATES,
            regime_min_samples,
        );

        // ─── 3b-3) ÇIKIŞ-MODELİ TARAMASI (teşhis: trailing mi suçlu, edge mi yok) ──
        // Yalnız ÇIKIŞ ekseni değişir (giriş/strateji/param sabit = global best). Üç
        // model OOS pencerelerinde havuzlanıp karşılaştırılır:
        //   • trailing yok (TP/SL) · baseline (~temsili mult) · gevşek (target=5.0)
        // Okuma: "trailing yok/gevşek" baseline'ı belirgin geçiyorsa → trailing edge'i
        // yiyor (gevşet/kapat). Üçü de ~0 sharpe/PF~1 → girişlerde edge yok (çıkış ayarı
        // kurtarmaz). active_profile.json["exit_sweep"]'e mühürlenir.
        // Teşhis taramaları TÜM seride (tek uzun pencere) koşar — 50-barlık WF OOS
        // pencereleri (üstelik use_htf'te HTF kapısı kısa pencerede bar bulamaz) işlem
        // üretemeyecek kadar kısa (filtre kapalıyken bile 0 işlem gözlendi). Tam seri =
        // işlem-bol + canlı HTF davranışına yakın. In-sample iyimser ama "edge VAR MI"
        // sorusu için yeterli: çok işlemde bile PF<1 ise edge yok demektir.
        let full_window = vec![crate::robot::backtester::walk_forward::WindowResult {
            window_idx: 0,
            in_sample_range: (0, 0),
            oos_range: (0, candles.len()),
            best_tp_pct: final_res.best_parameters.take_profit_pct,
            best_sl_pct: final_res.best_parameters.stop_loss_pct,
            oos_metrics: Default::default(),
        }];
        let loose_mult = {
            let nf = crate::robot::parameters::window_noise_floor_pct(&candles);
            match nf {
                Some(n) if n > 0.0 => (5.0 / n).clamp(1.5, 30.0),
                _ => exit_trail_mult.max(5.0),
            }
        };
        let exit_models: Vec<(String, Option<f64>, Option<f64>)> = vec![
            ("trailing-yok".into(), None, Some(EXIT_BREAKEVEN_RR)),
            (
                format!("baseline-{:.1}x", exit_trail_mult),
                Some(exit_trail_mult),
                Some(EXIT_BREAKEVEN_RR),
            ),
            (
                format!("gevsek-{:.1}x", loose_mult),
                Some(loose_mult),
                Some(EXIT_BREAKEVEN_RR),
            ),
        ];
        let exit_sweep = crate::robot::backtester::walk_forward::evaluate_exit_models(
            &candles,
            &full_window,
            &dir_ab_base,
            &exit_models,
        );

        // ─── 3b-4) EDGE-FİLTRE TARAMASI (teşhis: huni mi sıkı, sinyal mi kötü) ─────
        // Yalnız GİRİŞ HUNİSİ (edge_min_score) değişir (çıkış/strateji/param sabit; base'in
        // canlı çıkış modeli korunur). Çıkış taraması trailing'i eledi → asıl şüpheli giriş.
        // Okuma: gevşek eşik işlemi belirgin artırıp PF'i 1+ yapıyorsa → huni çok sıkıymış
        // (gevşet). İşlem artıyor ama PF<1 kalıyorsa → sinyalde edge yok (strateji/feature işi).
        // active_profile.json["edge_sweep"]'e mühürlenir.
        let edge_thresholds: Vec<Option<f64>> = vec![None, Some(0.10), Some(0.20), Some(0.30)];
        let edge_sweep = crate::robot::backtester::walk_forward::evaluate_edge_filters(
            &candles,
            &full_window,
            &dir_ab_base,
            &edge_thresholds,
        );

        // ─── 3c) POOL-WIDE otonom INTERVAL seçimi (hafif OOS skoru) ──────────────
        // Her pool sembolü için aday TF'ler (auto_interval_candidates TEK KAYNAK, default
        // 15m,1h,4h,1d — WF-onaylı edge çoğu 1d'de) arasında HAFİF skor: wf_oos_windows +
        // score_config_over_windows (param SABİT = global best → interval ekseni izole + ucuz;
        // per-pencere param re-opt YOK). OBJEKTİF pooled Profit Factor (ham PnL değil) → gürültü
        // TF'i (1m, PF≈0.01) yüksek PnL ile seçilmez; R/R'si sağlam TF kazanır ([[project_rr_trail_ab]]
        // lever A). evaluate_symbol_interval + pick_best_with_margin yeniden kullanılır (DRY).
        // Mevcut TF'i IV_MARGIN (PF birimi) ile geçmeyen değişmez (flip-flop). Chicken-egg:
        // yeterli mumu olmayan aday atlanır. [[project_adaptive_regime]] [[feedback_modular_dry_perf]].
        let auto_iv_candidates: Vec<String> = super::auto_interval_candidates();
        let min_iv_bars = wf_is + wf_oos;
        const IV_MARGIN: f64 = 0.05; // PF birimi: yeni TF mevcudu ≥0.05 PF geçmeli (histerezis)
                                     // Sabit skorlama şablonu (global best strateji+param; yalnız symbol/interval değişir).
        let iv_base = crate::robot::backtester::BacktestConfig {
            initial_balance: capital,
            max_position_size: final_res.best_parameters.max_position_size,
            take_profit_pct: final_res.best_parameters.take_profit_pct,
            stop_loss_pct: final_res.best_parameters.stop_loss_pct,
            strategy_name: best_name.clone(),
            strategy_params: best_strategy_params.as_ref().map(|(sp, _, _)| *sp),
            commission_pct: 0.001,
            use_htf,
            edge_min_score: edge_min,
            ..Default::default()
        };
        // Pool sembolleri + mevcut interval/strateji map'leri (eligible; tek kısa lock).
        let (iv_pool, iv_current, strat_current) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let tuning = Arc::clone(&st.tuning);
            let mut pool: Vec<String> = Vec::new();
            let add = |s: &str, p: &mut Vec<String>| {
                if !s.is_empty() && tuning.symbol_eligible_for_live(s) && !p.iter().any(|x| x == s)
                {
                    p.push(s.to_string());
                }
            };
            add(&symbol, &mut pool);
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for w in orch.get_worker_status() {
                    add(&w.symbol, &mut pool);
                }
            }
            let pinned = st.config.pinned_symbols.clone();
            for s in &pinned {
                add(s, &mut pool);
            }
            let (cur_iv, cur_strat) = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| (p.symbol_interval.clone(), p.symbol_strategy.clone()))
                .unwrap_or_default();
            (pool, cur_iv, cur_strat)
        };
        // Per-symbol STRATEJİ A/B histerezisi + atama eşiği (PF birimi). edge_scan bulgusu:
        // farklı semboller farklı strateji ister (BTC→ICT_COMPOSITE 1.53 vs EMA 0.68).
        const STRAT_MARGIN: f64 = 0.10; // yeni strateji mevcudu ≥0.10 PF geçmeli (flip-flop koruması)
        const STRAT_MIN_SCORE: f64 = 1.0; // yalnız PF≥1.0 (gerçek edge) per-symbol atanır; aksi global/auto

        // Lock DIŞI ağır hesap; sonra toplu yazım.
        let mut iv_results: Vec<(String, String)> = Vec::new(); // (symbol, chosen)
        let mut iv_log: Vec<String> = Vec::new();
        // (symbol, karar): Some(strateji)=ata, None=KALDIR (çürüyen seed/keşif → global/auto'ya dön).
        let mut strat_results: Vec<(String, Option<String>)> = Vec::new();
        let mut strat_log: Vec<String> = Vec::new();
        for sym in &iv_pool {
            let cur = iv_current
                .get(sym)
                .cloned()
                .unwrap_or_else(|| interval.clone());
            let (chosen, scored) = crate::robot::backtester::walk_forward::evaluate_symbol_interval(
                &auto_iv_candidates,
                |tf| Self::read_candles_routed(state, &db_path, sym, tf, 5000).unwrap_or_default(),
                |tf, cand_candles| {
                    if cand_candles.len() < min_iv_bars {
                        return None;
                    }
                    let windows = crate::robot::backtester::walk_forward::wf_oos_windows(
                        cand_candles.len(),
                        wf_is,
                        wf_oos,
                        wf_step,
                    );
                    if windows.is_empty() {
                        return None;
                    }
                    let mut cfg = iv_base.clone();
                    cfg.symbol = sym.clone();
                    cfg.interval = tf.to_string(); // HTF agregasyonu doğru TF'den türesin
                    Some(
                        crate::robot::backtester::walk_forward::score_config_over_windows(
                            &cfg,
                            cand_candles,
                            &windows,
                        ),
                    )
                },
                Some(&cur),
                IV_MARGIN,
            );
            if let Some(best) = &chosen {
                if best != &cur {
                    iv_results.push((sym.clone(), best.clone()));
                }
                if !scored.is_empty() {
                    let sc: Vec<String> =
                        scored.iter().map(|(c, s)| format!("{c}={s:.1}")).collect();
                    iv_log.push(format!("{sym}→{best}[{}]", sc.join(",")));
                }
            }

            // ─── Per-symbol STRATEJİ seçimi (online self-discovery) ──────────────
            // Sembolün EFEKTİF interval'inde (yeni seçildiyse o, yoksa mevcut) tüm strateji
            // havuzunu pooled-PF ile skorla; param SABİT (iv_base = global best → strateji ekseni
            // izole + ucuz). Yalnız PF≥STRAT_MIN_SCORE kazanan + margin'i geçen atanır.
            let eff_iv = chosen.clone().unwrap_or_else(|| cur.clone());
            let strat_candles =
                Self::read_candles_routed(state, &db_path, sym, &eff_iv, 5000).unwrap_or_default();
            if strat_candles.len() >= min_iv_bars {
                let windows = crate::robot::backtester::walk_forward::wf_oos_windows(
                    strat_candles.len(),
                    wf_is,
                    wf_oos,
                    wf_step,
                );
                if !windows.is_empty() {
                    let cur_strat = strat_current.get(sym).map(|s| s.as_str());
                    use crate::robot::backtester::walk_forward::StrategyChoice;
                    let (s_choice, s_scored) =
                        crate::robot::backtester::walk_forward::evaluate_symbol_strategy(
                            &strat_pool,
                            |name| {
                                let mut cfg = iv_base.clone();
                                cfg.symbol = sym.clone();
                                cfg.interval = eff_iv.clone();
                                cfg.strategy_name = name.to_string();
                                Some(crate::robot::backtester::walk_forward::score_config_over_windows(
                                &cfg, &strat_candles, &windows))
                            },
                            cur_strat,
                            STRAT_MARGIN,
                            STRAT_MIN_SCORE,
                        );
                    // En iyi 3 skor (havuz büyük olabilir) — gözlemlenebilirlik.
                    let top3 = || {
                        let mut top = s_scored.clone();
                        top.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        top.into_iter()
                            .take(3)
                            .map(|(c, s)| format!("{c}={s:.2}"))
                            .collect::<Vec<_>>()
                            .join(",")
                    };
                    match s_choice {
                        StrategyChoice::Assign(best) => {
                            strat_results.push((sym.clone(), Some(best.clone())));
                            strat_log.push(format!("{sym}@{eff_iv}→{best}[{}]", top3()));
                        }
                        // Çürüme: mevcut atama min_score'u kaybetti + edge'li aday yok → KALDIR.
                        StrategyChoice::Demote => {
                            strat_results.push((sym.clone(), None));
                            strat_log.push(format!("{sym}@{eff_iv}→⊘demote[{}]", top3()));
                        }
                        StrategyChoice::Keep => {}
                    }
                }
            }
        }

        // brain.live_strategy + best_params + ParameterStore.trade_risk güncellenir.
        {
            let mut st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            if let Ok(mut s) = st.brain.live_strategy.write() {
                *s = best_name.clone();
            }
            st.brain.best_params.insert(
                "take_profit_pct".into(),
                final_res.best_parameters.take_profit_pct,
            );
            st.brain.best_params.insert(
                "stop_loss_pct".into(),
                final_res.best_parameters.stop_loss_pct,
            );
            st.brain.best_params.insert(
                "max_position_size".into(),
                final_res.best_parameters.max_position_size,
            );
            st.brain
                .best_params
                .insert("wf_score".into(), best_wf_score);
            st.brain
                .best_params
                .insert("oos_sharpe".into(), best_wf_res.avg_oos_sharpe);
            st.brain
                .best_params
                .insert("oos_consistency".into(), best_wf_res.consistency_score);
            if let Ok(mut params) = st.brain.parameters.write() {
                params.apply_optimization(
                    final_res.best_parameters.take_profit_pct,
                    final_res.best_parameters.stop_loss_pct,
                    final_res.best_parameters.max_position_size,
                );
                // Rejim katmanları — PS global, TP/SL rejime özgü + otonom yön policy.
                for (regime, agg) in &regime_agg {
                    let trade_risk = crate::robot::parameters::TradeRiskParams {
                        take_profit_pct: agg.median_tp_pct,
                        stop_loss_pct: agg.median_sl_pct,
                        max_position_size: final_res.best_parameters.max_position_size,
                    };
                    let mut patch =
                        crate::robot::parameters::RegimePatch::empty().with_trade_risk(trade_risk);
                    // Per-rejim yön disiplini A/B kazananı (varsa) → RegimePolicy.
                    if let Some(&directional) = regime_dir_map.get(regime) {
                        patch = patch.with_policy(crate::robot::parameters::RegimePolicy {
                            regime_directional: Some(directional),
                        });
                    }
                    // Per-rejim trailing hedef A/B kazananı (varsa) → target_trail_pct.
                    if let Some(&trail_pct) = regime_trail_map.get(regime) {
                        patch = patch.with_trail_target(trail_pct);
                    }
                    params.set_regime_patch(regime.clone(), patch);
                }
                // Kazanan stratejinin yapısal (indikatör) parametreleri — Faz 1b.
                if let Some((sp, _, _)) = &best_strategy_params {
                    params.set_strategy_params(best_name.clone(), *sp);
                }
                // Pool-wide otonom interval kazananları (evaluate_symbol_interval seçti;
                // yalnız mevcuttan FARKLI olanlar iv_results'a girdi → toplu yaz).
                for (sym, iv) in &iv_results {
                    params.symbol_interval.insert(sym.clone(), iv.clone());
                }
                // Pool-wide otonom STRATEJİ kararları: Some→ata (PF≥1.0 + margin), None→KALDIR
                // (çürüyen seed/keşif → sembol global/auto'ya döner).
                for (sym, strat) in &strat_results {
                    match strat {
                        Some(s) => {
                            params.symbol_strategy.insert(sym.clone(), s.clone());
                        }
                        None => {
                            params.symbol_strategy.remove(sym);
                        }
                    }
                }
            }
            // Pool-wide otonom interval özeti — gözlemlenebilirlik (değişenler + tüm skorlar).
            if !iv_log.is_empty() {
                let iv_preview = iv_log
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · ");
                st.push_log(format!(
                    "📐 auto-interval ({} sembol değerlendirildi, {} değişti): {}{}",
                    iv_log.len(),
                    iv_results.len(),
                    iv_preview,
                    if iv_log.len() > 4 {
                        format!(" · +{} daha", iv_log.len() - 4)
                    } else {
                        String::new()
                    },
                ));
            }
            // Pool-wide otonom strateji özeti (per-symbol: hangi sembol edge aldı/çürüyüp demote oldu).
            if !strat_log.is_empty() {
                let strat_preview = strat_log
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · ");
                st.push_log(format!(
                    "🧠 auto-strateji ({} sembol karar, {} değişti): {}{}",
                    strat_log.len(),
                    strat_results.len(),
                    strat_preview,
                    if strat_log.len() > 4 {
                        format!(" · +{} daha", strat_log.len() - 4)
                    } else {
                        String::new()
                    },
                ));
            }
            // hyperopt_score WF skoruna mühürlenir — UI/legacy okuyucular için
            // overfitting-koruyucu seçim ölçütü.
            st.brain.hyperopt_score = best_wf_score;
            st.push_log(format!(
                "🔬 Backtest ✓ aktif '{}' (WF skor={:.3} | OOS Sharpe={:.2} Tutarlılık={:.0}% | final TP={:.1}% SL={:.1}% PS={:.2})",
                best_name, best_wf_score,
                best_wf_res.avg_oos_sharpe,
                best_wf_res.consistency_score * 100.0,
                final_res.best_parameters.take_profit_pct,
                final_res.best_parameters.stop_loss_pct,
                final_res.best_parameters.max_position_size,
            ));
            // Yapısal parametre araması özeti (Faz 1b).
            match &best_strategy_params {
                Some((sp, score, tested)) => {
                    let params_str: String = sp.iter()
                        .map(|(k, v)| format!("{}={:.3}", k, v))
                        .collect::<Vec<_>>()
                        .join(" ");
                    st.push_log(format!(
                        "🧩 '{}' param_spec ({} kombinasyon, skor={:.3}): {}",
                        best_name, tested, score, params_str,
                    ))
                }
                None => st.push_log(format!(
                    "🧩 '{}' yapısal parametre uzayı boş veya arama sonuç vermedi → default paramlar",
                    best_name,
                )),
            }
            // Rejim katmanları log'una tek satırlık özet.
            if regime_agg.is_empty() {
                st.push_log(
                    "🎚  Rejim katmanı yazılmadı — min örneklem altında veya sınıflandırma boş"
                        .into(),
                );
            } else {
                let mut entries: Vec<String> = regime_agg
                    .iter()
                    .map(|(r, a)| {
                        format!(
                            "{r}(n={}) TP={:.1}% SL={:.1}% dir={} trail={}",
                            a.sample_count,
                            a.median_tp_pct,
                            a.median_sl_pct,
                            match regime_dir_map.get(r) {
                                Some(true) => "RD",
                                Some(false) => "long",
                                None => "—",
                            },
                            match regime_trail_map.get(r) {
                                Some(t) => format!("{:.1}%", t),
                                None => "—".into(),
                            },
                        )
                    })
                    .collect();
                entries.sort();
                st.push_log(format!(
                    "🎚  Rejim katmanları yazıldı: {}",
                    entries.join(" | ")
                ));
            }
            // Çıkış-modeli taraması özeti (teşhis): model · işlem · win% · beklenti · PF · sharpe.
            if !exit_sweep.is_empty() {
                let line: Vec<String> = exit_sweep
                    .iter()
                    .map(|s| {
                        format!(
                            "{}[n={} wr={:.0}% E={:+.3} PF={:.2} Sh={:+.2}]",
                            s.label,
                            s.total_trades,
                            s.win_rate * 100.0,
                            s.expectancy,
                            s.profit_factor,
                            s.sharpe,
                        )
                    })
                    .collect();
                st.push_log(format!("🪤 Çıkış taraması: {}", line.join(" · ")));
            }
            // Edge-filtre taraması özeti (teşhis): eşik · işlem · win% · beklenti · PF · sharpe.
            if !edge_sweep.is_empty() {
                let line: Vec<String> = edge_sweep
                    .iter()
                    .map(|s| {
                        format!(
                            "{}[n={} wr={:.0}% E={:+.3} PF={:.2} Sh={:+.2}]",
                            s.label,
                            s.total_trades,
                            s.win_rate * 100.0,
                            s.expectancy,
                            s.profit_factor,
                            s.sharpe,
                        )
                    })
                    .collect();
                st.push_log(format!("🚪 Edge-filtre taraması: {}", line.join(" · ")));
            }
        }

        // Profil de diske mühürlenir.
        let regime_breakdown: serde_json::Value = regime_agg
            .iter()
            .map(|(r, a)| {
                (
                    r.clone(),
                    serde_json::json!({
                        "median_tp_pct": a.median_tp_pct,
                        "median_sl_pct": a.median_sl_pct,
                        "sample_count": a.sample_count,
                        "regime_directional": regime_dir_map.get(r).copied(),
                        "target_trail_pct": regime_trail_map.get(r).copied(),
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into();
        // Havuzlanmış varyant istatistiklerini JSON'a çevirir (exit_sweep + edge_sweep DRY).
        let sweep_json = |sweep: &[crate::robot::backtester::walk_forward::ExitModelStats]| {
            sweep.iter().map(|s| serde_json::json!({
                "label": s.label,
                "total_trades": s.total_trades,
                "win_rate": s.win_rate,
                "avg_win": s.avg_win,
                "avg_loss": s.avg_loss,
                "expectancy": s.expectancy,
                "profit_factor": if s.profit_factor.is_finite() { serde_json::json!(s.profit_factor) } else { serde_json::json!("inf") },
                "sharpe": s.sharpe,
                "total_pnl": s.total_pnl,
            })).collect::<Vec<_>>()
        };
        let snapshot = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            serde_json::json!({
                "active_strategy": best_name,
                "params": st.brain.best_params,
                "selection_layers": {
                    "model_selection": {
                        "method": "walk_forward",
                        "elimination_enabled": elim_enabled,
                        "min_consistency_pct": elim_min_consistency * 100.0,
                        "min_windows": elim_min_windows,
                        "min_trades": elim_min_trades,
                        "min_oos_sharpe": elim_min_oos_sharpe,
                        "min_score": elim_min_score,
                        "max_binom_pvalue": elim_max_binom_pvalue,
                        "bootstrap_enabled": elim_bootstrap_enabled,
                        "bootstrap_samples": elim_bootstrap_samples,
                        "bootstrap_ci_pct": elim_bootstrap_ci_pct,
                        "bootstrap_min_mean_pnl_pct": elim_bootstrap_min_mean_pnl_pct,
                    },
                    "parameter_selection": {
                        "method": "global_grid_optimize_parallel",
                        "axes": ["tp_pct", "sl_pct", "position_size"],
                    },
                    "final_report": {
                        "method": "sealed_active_profile",
                    }
                },
                "wf_score": best_wf_score,
                "oos_sharpe": best_wf_res.avg_oos_sharpe,
                "oos_consistency": best_wf_res.consistency_score,
                "oos_windows": best_wf_res.windows.len(),
                "in_sample_bars": wf_is,
                "out_of_sample_bars": wf_oos,
                "step_bars": wf_step,
                "regime_overrides": regime_breakdown,
                "regime_min_samples": regime_min_samples,
                "auto_interval_pool": iv_results.iter()
                    .map(|(sym, iv)| (sym.clone(), serde_json::json!(iv))).collect::<serde_json::Map<_, _>>(),
                "auto_interval_evaluated": iv_log,
                "exit_sweep": sweep_json(&exit_sweep),
                "edge_sweep": sweep_json(&edge_sweep),
                "sealed_at": chrono::Utc::now().to_rfc3339(),
            })
        };

        Self::log_aux_research_backtest_shadow(
            state,
            &db_path,
            capital,
            &best_name,
            use_htf,
            edge_min,
            orderbook_sim.clone(),
            final_res.best_parameters.take_profit_pct,
            final_res.best_parameters.stop_loss_pct,
            final_res.best_parameters.max_position_size,
            wf_is,
            wf_oos,
            wf_step,
        );

        crate::persistence::writer::seal_config_to_disk("config/active_profile.json", &snapshot)
            .map_err(|e| format!("seal: {:?}", e))?;
        Ok(())
    }
}
