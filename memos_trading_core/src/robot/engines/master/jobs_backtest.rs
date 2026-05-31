// src/robot/engines/master/jobs_backtest.rs — Backtest job (walk-forward grid + otonom strateji seçimi).
// Faz 2 modülerleştirme: jobs.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

impl Engine {

    /// 🔬 Backtest (Faz 4 - "backtest" trigger):
    /// Daha geniş bir grid ile composite score'u en yüksek olan profili seçer ve
    /// brain.live_strategy'i otonom değiştirir.
    pub(crate) fn run_backtest_job(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        log::info!("🔬 E2: Walk-Forward Backtest başlatıldı (grid: 6×4×3)...");

        let (symbol, interval, db_path, capital, use_htf) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            // Backtest, canlının multi-TF'ini aynalasın: multi_tf.enabled açıksa WF
            // seçimi + param araması da HTF filtresini görür (canlı ile tek-davranış).
            let use_htf = st.brain.parameters.read().map(|p| p.multi_tf.enabled).unwrap_or(false);
            (st.config.symbol.clone(), st.config.interval.clone(),
             st.config.db_path.clone(), st.finance.equity, use_htf)
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
            if EDGE_FILTER_DEFAULT_ON { Some(on_value) } else { None },
            on_value,
        );

        // Orderbook icrası (#c) — opt-in (default kapalı). BACKTEST_ORDERBOOK=liquid|illiquid
        // ile açılır → WF seçimi + param araması slippage'i de görür (canlı paper ile aynı motor).
        let orderbook_sim: Option<String> = std::env::var("BACKTEST_ORDERBOOK").ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("off")
                && !s.eq_ignore_ascii_case("0") && !s.eq_ignore_ascii_case("none"));

        push_state_log(state, format!(
            "🔬 Backtest başladı: sembol={} aralık={} kapital=${:.0} edge_filtre={} orderbook={}",
            symbol, interval, capital,
            edge_min.map(|t| format!("≥{:.2}", t)).unwrap_or_else(|| "kapalı".into()),
            orderbook_sim.as_deref().unwrap_or("kapalı"),
        ));

        // Veri derinliği — env BACKTEST_CANDLE_LIMIT (default 5000). Sağlıklı
        // backtest için istatistiksel anlamlı işlem + çok-rejim kapsama gerekir;
        // eski sabit 1500 sığdı (1m'de ~1 gün, tek rejim). TF'e göre öneri:
        // 1h≈6-12 ay (4.3k-8.8k), 1m sadece infaz simülasyonu (fee/gürültü baskın).
        let candle_limit: usize = env_parse("BACKTEST_CANDLE_LIMIT", 5000usize).max(300);

        // Walk-Forward konfigürasyonu — env'den override edilebilir.
        // Varsayılan IS=200 / OOS=50 / step=50. Daha derin veride IS/OOS'u TF'e
        // ölçeklemek için WALK_FORWARD_* env'leri kullanılır (örn. 1m'de IS≥1000).
        let wf_is:   usize = env_parse("WALK_FORWARD_IS_BARS", 200);
        let wf_oos:  usize = env_parse("WALK_FORWARD_OOS_BARS", 50);
        let wf_step: usize = env_parse("WALK_FORWARD_STEP_BARS", 50);
        let wf_min  = wf_is + wf_oos;

        let candles = crate::persistence::reader::read_candles(&db_path, &symbol, &interval, candle_limit)
            .map_err(|e| format!("read_candles: {}", e))?;
        if candles.len() < wf_min {
            return Err(format!(
                "yetersiz mum verisi: {} mum (walk-forward için ≥{} gerekli)",
                candles.len(), wf_min,
            ));
        }

        // Aday strateji pool'u StrategyRegistry'den otomatik genişler (Faz 4 c2):
        // yeni strateji default_registry()'ye eklendiğinde backtest_job ekstra
        // değişiklik gerektirmez. Alias'lar dahil edilmez (canonical_pool).
        let strat_pool: Vec<String> =
            crate::robot::strategies::default_registry().canonical_pool();
        let est_windows = candles.len().saturating_sub(wf_min) / wf_step.max(1) + 1;
        push_state_log(state, format!(
            "🔬 Backtest (Walk-Forward): {} mum, {} strateji × ~{} pencere (IS={} OOS={} step={})",
            candles.len(), strat_pool.len(), est_windows, wf_is, wf_oos, wf_step,
        ));

        // ─── 1) Strateji aday seçimi: her aday için Walk-Forward → OOS sharpe + tutarlılık ───
        //
        // wf_score = avg_oos_sharpe * 0.7 + consistency * 0.3
        // (consistency = kârlı OOS pencerelerinin oranı 0..1)
        // OOS metrikleri overfitting'i engeller; in-sample sharpe'a göre seçim yapılmıyor.
        use crate::robot::backtester::walk_forward::{WalkForwardConfig, WalkForwardTester};
        const WF_CONSISTENCY_WEIGHT: f64 = 0.3;

        let mut best_wf: Option<(String, f64,
            crate::robot::backtester::walk_forward::WalkForwardResult)> = None;

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
            };
            let Some(wf_res) = WalkForwardTester::new(wf_cfg).run(&candles) else {
                push_state_log(state, format!("🔬   aday {} → WF sonuç alınamadı", name));
                continue;
            };

            let wf_score = wf_res.avg_oos_sharpe * (1.0 - WF_CONSISTENCY_WEIGHT)
                         + wf_res.consistency_score * WF_CONSISTENCY_WEIGHT;
            push_state_log(state, format!(
                "🔬   aday {} → OOS Sharpe={:.2} Tutarlılık={:.0}% ({} pencere) skor={:.3}",
                name, wf_res.avg_oos_sharpe,
                wf_res.consistency_score * 100.0,
                wf_res.windows.len(),
                wf_score,
            ));
            if best_wf.as_ref().map(|(_, s, _)| *s).unwrap_or(f64::NEG_INFINITY) < wf_score {
                best_wf = Some((name.clone(), wf_score, wf_res));
            }
        }

        let (best_name, best_wf_score, best_wf_res) = best_wf
            .ok_or_else(|| "Hiçbir strateji walk-forward sonuç üretemedi".to_string())?;

        // ─── 2) Kazanan strateji için PS dahil final parametre optimizasyonu (tüm veri) ───
        //
        // Walk-Forward'da quick_optimize sadece TP/SL üzerinde tarıyor; pozisyon boyutu
        // (PS) burada belirlenir ki best_params üç ekseni de kapsasın.
        let final_opt = crate::robot::backtester::parameter_optimizer::ParameterOptimizer::new(
            symbol.clone(), interval.clone(), capital, best_name.clone(),
        ).with_edge_min_score(edge_min).with_orderbook_sim(orderbook_sim.clone());
        let final_res = final_opt.optimize_parallel(
            &candles,
            (2.0, 8.0, 1.0),       // TP %2 → %8, step 1
            (1.0, 4.0, 1.0),       // SL %1 → %4, step 1
            (0.1, 0.4, 0.1),       // PS  0.1 → 0.4
        ).map_err(|e| format!("final optimize_parallel: {:?}", e))?;

        // ─── 2b) Kazanan stratejinin YAPISAL parametreleri (param_spec araması) ───
        //
        // Faz 1b: strateji KENDİ param_spec()'ini bildirir; HyperOpt::spec_search bu
        // uzaydan örnekler. Bulunan en iyi set ParameterStore.strategy_params'a yazılır
        // ve canlı cycle generate_signal'a verilir (eskiden her zaman default geçiliyordu).
        // Yapısal paramı olmayan strateji (PRICE_ACTION/FUNDING) → spec boş → None.
        let best_strategy_params = {
            let specs = crate::robot::strategies::default_registry()
                .make(&best_name).param_spec();
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
                    breakeven_at_rr: Some(1.0),
                    atr_trail_mult: Some(2.0),
                    partial_tp_ratio: None,
                    position_profile: None,
                    security_profile: None,
                    use_htf,
                    edge_min_score: edge_min,
                    orderbook_sim: orderbook_sim.clone(),
                    regime_gate: Default::default(),
                    direction: Default::default(),
                    atr_sl_mult: None,
                    atr_tp_mult: None,
                    vol_target_pct: None,
                };
                crate::robot::ml_engine::hyperopt::HyperOpt::spec_search(
                    &candles, &specs, n_iters, &bt_cfg, Some(12345),
                ).map(|r| (r.best_params, r.best_score, r.combinations_tested))
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
            ..Default::default()
        };
        let regime_dir_map = crate::robot::backtester::walk_forward::evaluate_regime_direction(
            &candles,
            &best_wf_res.windows,
            |oos_slice| Self::classify_regime(oos_slice).as_str().to_string(),
            &dir_ab_base,
            regime_min_samples,
        );

        // ─── 3c) POOL-WIDE otonom INTERVAL seçimi (hafif OOS skoru) ──────────────
        // Her pool sembolü için aday TF'ler (env AUTO_INTERVAL_CANDIDATES, default 15m,1h)
        // arasında HAFİF skor: wf_oos_windows + score_config_over_windows (param SABİT =
        // global best → interval ekseni izole + ucuz; per-pencere param re-opt YOK).
        // evaluate_symbol_interval + pick_best_with_margin yeniden kullanılır (DRY).
        // Mevcut TF'i IV_MARGIN ile geçmeyen değişmez (flip-flop). Chicken-egg: yeterli
        // mumu olmayan aday atlanır. [[project_adaptive_regime]] [[feedback_modular_dry_perf]].
        let auto_iv_candidates: Vec<String> = std::env::var("AUTO_INTERVAL_CANDIDATES")
            .unwrap_or_else(|_| "15m,1h".into())
            .split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        let min_iv_bars = wf_is + wf_oos;
        const IV_MARGIN: f64 = 0.05;
        // Sabit skorlama şablonu (global best strateji+param; yalnız symbol/interval değişir).
        let iv_base = crate::robot::backtester::BacktestConfig {
            initial_balance: capital,
            max_position_size: final_res.best_parameters.max_position_size,
            take_profit_pct: final_res.best_parameters.take_profit_pct,
            stop_loss_pct: final_res.best_parameters.stop_loss_pct,
            strategy_name: best_name.clone(),
            strategy_params: best_strategy_params.as_ref().map(|(sp, _, _)| *sp),
            commission_pct: 0.001, use_htf, edge_min_score: edge_min,
            ..Default::default()
        };
        // Pool sembolleri + mevcut interval map (eligible; tek kısa lock).
        let (iv_pool, iv_current) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let tuning = Arc::clone(&st.tuning);
            let mut pool: Vec<String> = Vec::new();
            let add = |s: &str, p: &mut Vec<String>| {
                if !s.is_empty() && tuning.symbol_eligible_for_live(s)
                    && !p.iter().any(|x| x == s) { p.push(s.to_string()); }
            };
            add(&symbol, &mut pool);
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for w in orch.get_worker_status() { add(&w.symbol, &mut pool); }
            }
            let pinned = st.config.pinned_symbols.clone();
            for s in &pinned { add(s, &mut pool); }
            let cur = st.brain.parameters.read().ok()
                .map(|p| p.symbol_interval.clone()).unwrap_or_default();
            (pool, cur)
        };
        // Lock DIŞI ağır hesap; sonra toplu yazım.
        let mut iv_results: Vec<(String, String)> = Vec::new(); // (symbol, chosen)
        let mut iv_log: Vec<String> = Vec::new();
        for sym in &iv_pool {
            let cur = iv_current.get(sym).cloned().unwrap_or_else(|| interval.clone());
            let (chosen, scored) = crate::robot::backtester::walk_forward::evaluate_symbol_interval(
                &auto_iv_candidates,
                |tf| crate::persistence::reader::read_candles(&db_path, sym, tf, 5000).unwrap_or_default(),
                |tf, cand_candles| {
                    if cand_candles.len() < min_iv_bars { return None; }
                    let windows = crate::robot::backtester::walk_forward::wf_oos_windows(
                        cand_candles.len(), wf_is, wf_oos, wf_step);
                    if windows.is_empty() { return None; }
                    let mut cfg = iv_base.clone();
                    cfg.symbol = sym.clone();
                    cfg.interval = tf.to_string(); // HTF agregasyonu doğru TF'den türesin
                    Some(crate::robot::backtester::walk_forward::score_config_over_windows(
                        &cfg, cand_candles, &windows))
                },
                Some(&cur), IV_MARGIN,
            );
            if let Some(best) = chosen {
                if best != cur { iv_results.push((sym.clone(), best.clone())); }
                if !scored.is_empty() {
                    let sc: Vec<String> = scored.iter().map(|(c, s)| format!("{c}={s:.1}")).collect();
                    iv_log.push(format!("{sym}→{best}[{}]", sc.join(",")));
                }
            }
        }

        // brain.live_strategy + best_params + ParameterStore.trade_risk güncellenir.
        {
            let mut st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            if let Ok(mut s) = st.brain.live_strategy.write() {
                *s = best_name.clone();
            }
            st.brain.best_params.insert("take_profit_pct".into(),
                final_res.best_parameters.take_profit_pct);
            st.brain.best_params.insert("stop_loss_pct".into(),
                final_res.best_parameters.stop_loss_pct);
            st.brain.best_params.insert("max_position_size".into(),
                final_res.best_parameters.max_position_size);
            st.brain.best_params.insert("wf_score".into(), best_wf_score);
            st.brain.best_params.insert("oos_sharpe".into(), best_wf_res.avg_oos_sharpe);
            st.brain.best_params.insert("oos_consistency".into(), best_wf_res.consistency_score);
            if let Ok(mut params) = st.brain.parameters.write() {
                params.apply_optimization(
                    final_res.best_parameters.take_profit_pct,
                    final_res.best_parameters.stop_loss_pct,
                    final_res.best_parameters.max_position_size,
                );
                // Rejim katmanları — PS global, TP/SL rejime özgü + otonom yön policy.
                for (regime, agg) in &regime_agg {
                    let trade_risk = crate::robot::parameters::TradeRiskParams {
                        take_profit_pct:   agg.median_tp_pct,
                        stop_loss_pct:     agg.median_sl_pct,
                        max_position_size: final_res.best_parameters.max_position_size,
                    };
                    let mut patch = crate::robot::parameters::RegimePatch::empty()
                        .with_trade_risk(trade_risk);
                    // Per-rejim yön disiplini A/B kazananı (varsa) → RegimePolicy.
                    if let Some(&directional) = regime_dir_map.get(regime) {
                        patch = patch.with_policy(crate::robot::parameters::RegimePolicy {
                            regime_directional: Some(directional),
                        });
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
            }
            // Pool-wide otonom interval özeti — gözlemlenebilirlik (değişenler + tüm skorlar).
            if !iv_log.is_empty() {
                st.push_log(format!(
                    "📐 auto-interval ({} sembol değerlendirildi, {} değişti): {}",
                    iv_log.len(), iv_results.len(), iv_log.join(" · "),
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
                Some((sp, score, tested)) => st.push_log(format!(
                    "🧩 '{}' param_spec ({} kombinasyon, skor={:.3}): fast={:?} slow={:?} period={:?} std_dev={:?} ob={:?} os={:?}",
                    best_name, tested, score,
                    sp.fast, sp.slow, sp.period, sp.std_dev, sp.overbought, sp.oversold,
                )),
                None => st.push_log(format!(
                    "🧩 '{}' yapısal parametre uzayı boş veya arama sonuç vermedi → default paramlar",
                    best_name,
                )),
            }
            // Rejim katmanları log'una tek satırlık özet.
            if regime_agg.is_empty() {
                st.push_log(
                    "🎚  Rejim katmanı yazılmadı — min örneklem altında veya sınıflandırma boş".into(),
                );
            } else {
                let mut entries: Vec<String> = regime_agg.iter()
                    .map(|(r, a)| format!(
                        "{r}(n={}) TP={:.1}% SL={:.1}% dir={}",
                        a.sample_count, a.median_tp_pct, a.median_sl_pct,
                        match regime_dir_map.get(r) { Some(true) => "RD", Some(false) => "long", None => "—" },
                    ))
                    .collect();
                entries.sort();
                st.push_log(format!("🎚  Rejim katmanları yazıldı: {}", entries.join(" | ")));
            }
        }

        // Profil de diske mühürlenir.
        let regime_breakdown: serde_json::Value = regime_agg.iter()
            .map(|(r, a)| (r.clone(), serde_json::json!({
                "median_tp_pct": a.median_tp_pct,
                "median_sl_pct": a.median_sl_pct,
                "sample_count": a.sample_count,
                "regime_directional": regime_dir_map.get(r).copied(),
            })))
            .collect::<serde_json::Map<_, _>>()
            .into();
        let snapshot = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            serde_json::json!({
                "active_strategy": best_name,
                "params": st.brain.best_params,
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
                "sealed_at": chrono::Utc::now().to_rfc3339(),
            })
        };
        crate::persistence::writer::seal_config_to_disk("config/active_profile.json", &snapshot)
            .map_err(|e| format!("seal: {:?}", e))?;
        Ok(())
    }
}
