// src/robot/engines/master/jobs_backtest.rs — Backtest job (walk-forward grid + otonom strateji seçimi).
// Faz 2 modülerleştirme: jobs.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

impl Engine {

    /// 🔬 Backtest (Faz 4 - "backtest" trigger):
    /// Daha geniş bir grid ile composite score'u en yüksek olan profili seçer ve
    /// brain.live_strategy'i otonom değiştirir.
    pub(crate) fn run_backtest_job(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        log::info!("🔬 E2: Walk-Forward Backtest başlatıldı (grid: 6×4×3)...");

        let (symbol, interval, db_path, capital, use_htf, market, data_gate, health_th) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            // Backtest, canlının multi-TF'ini aynalasın: multi_tf.enabled açıksa WF
            // seçimi + param araması da HTF filtresini görür (canlı ile tek-davranış).
            let use_htf = st.brain.parameters.read().map(|p| p.multi_tf.enabled).unwrap_or(false);
            (st.config.symbol.clone(), st.config.interval.clone(),
             st.config.db_path.clone(), st.finance.equity, use_htf, st.config.market.clone(),
             st.tuning.data_gate_enabled, st.tuning.health_thresholds())
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

        let candles = crate::persistence::reader::read_candles_market(&db_path, &symbol, &interval, &market, candle_limit)
            .map_err(|e| format!("read_candles: {}", e))?;
        if candles.len() < wf_min {
            return Err(format!(
                "yetersiz mum verisi: {} mum (walk-forward için ≥{} gerekli)",
                candles.len(), wf_min,
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
            let default_target = state.lock().ok()
                .and_then(|st| st.brain.parameters.read().ok()
                    .map(|p| p.target_trail_pct_for_strategy("default")))
                .unwrap_or(0.7);
            match crate::robot::parameters::window_noise_floor_pct(&candles) {
                Some(nf) if nf > 0.0 => (default_target / nf).clamp(1.5, 30.0),
                _ => 2.0,
            }
        };
        push_state_log(state, format!(
            "🪤 Çıkış modeli (TP/SL re-opt): trail≈{:.1}×ATR + breakeven@RR {:.1} (canlı-temsili)",
            exit_trail_mult, EXIT_BREAKEVEN_RR,
        ));

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
                atr_trail_mult: Some(exit_trail_mult),
                breakeven_at_rr: Some(EXIT_BREAKEVEN_RR),
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
        ).with_edge_min_score(edge_min).with_orderbook_sim(orderbook_sim.clone())
         .with_exit_model(Some(exit_trail_mult), Some(EXIT_BREAKEVEN_RR));
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
                    breakeven_at_rr: Some(EXIT_BREAKEVEN_RR),
                    atr_trail_mult: Some(exit_trail_mult),
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
            match nf { Some(n) if n > 0.0 => (5.0 / n).clamp(1.5, 30.0), _ => exit_trail_mult.max(5.0) }
        };
        let exit_models: Vec<(String, Option<f64>, Option<f64>)> = vec![
            ("trailing-yok".into(),                       None,                  Some(EXIT_BREAKEVEN_RR)),
            (format!("baseline-{:.1}x", exit_trail_mult), Some(exit_trail_mult), Some(EXIT_BREAKEVEN_RR)),
            (format!("gevsek-{:.1}x", loose_mult),        Some(loose_mult),      Some(EXIT_BREAKEVEN_RR)),
        ];
        let exit_sweep = crate::robot::backtester::walk_forward::evaluate_exit_models(
            &candles, &full_window, &dir_ab_base, &exit_models,
        );

        // ─── 3b-4) EDGE-FİLTRE TARAMASI (teşhis: huni mi sıkı, sinyal mi kötü) ─────
        // Yalnız GİRİŞ HUNİSİ (edge_min_score) değişir (çıkış/strateji/param sabit; base'in
        // canlı çıkış modeli korunur). Çıkış taraması trailing'i eledi → asıl şüpheli giriş.
        // Okuma: gevşek eşik işlemi belirgin artırıp PF'i 1+ yapıyorsa → huni çok sıkıymış
        // (gevşet). İşlem artıyor ama PF<1 kalıyorsa → sinyalde edge yok (strateji/feature işi).
        // active_profile.json["edge_sweep"]'e mühürlenir.
        let edge_thresholds: Vec<Option<f64>> = vec![None, Some(0.10), Some(0.20), Some(0.30)];
        let edge_sweep = crate::robot::backtester::walk_forward::evaluate_edge_filters(
            &candles, &full_window, &dir_ab_base, &edge_thresholds,
        );

        // ─── 3c) POOL-WIDE otonom INTERVAL seçimi (hafif OOS skoru) ──────────────
        // Her pool sembolü için aday TF'ler (env AUTO_INTERVAL_CANDIDATES, default 15m,1h)
        // arasında HAFİF skor: wf_oos_windows + score_config_over_windows (param SABİT =
        // global best → interval ekseni izole + ucuz; per-pencere param re-opt YOK).
        // OBJEKTİF artık pooled Profit Factor (ham PnL değil) → gürültü TF'i (1m, PF≈0.01)
        // yüksek PnL ile seçilmez; R/R'si sağlam TF (1h) kazanır ([[project_rr_trail_ab]] lever A).
        // evaluate_symbol_interval + pick_best_with_margin yeniden kullanılır (DRY).
        // Mevcut TF'i IV_MARGIN (PF birimi) ile geçmeyen değişmez (flip-flop). Chicken-egg:
        // yeterli mumu olmayan aday atlanır. [[project_adaptive_regime]] [[feedback_modular_dry_perf]].
        let auto_iv_candidates: Vec<String> = std::env::var("AUTO_INTERVAL_CANDIDATES")
            .unwrap_or_else(|_| "15m,1h".into())
            .split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
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
            commission_pct: 0.001, use_htf, edge_min_score: edge_min,
            ..Default::default()
        };
        // Pool sembolleri + mevcut interval/strateji map'leri (eligible; tek kısa lock).
        let (iv_pool, iv_current, strat_current) = {
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
            let (cur_iv, cur_strat) = st.brain.parameters.read().ok()
                .map(|p| (p.symbol_interval.clone(), p.symbol_strategy.clone())).unwrap_or_default();
            (pool, cur_iv, cur_strat)
        };
        // Per-symbol STRATEJİ A/B histerezisi + atama eşiği (PF birimi). edge_scan bulgusu:
        // farklı semboller farklı strateji ister (BTC→ICT_COMPOSITE 1.53 vs EMA 0.68).
        const STRAT_MARGIN: f64 = 0.10;     // yeni strateji mevcudu ≥0.10 PF geçmeli (flip-flop koruması)
        const STRAT_MIN_SCORE: f64 = 1.0;   // yalnız PF≥1.0 (gerçek edge) per-symbol atanır; aksi global/auto

        // Lock DIŞI ağır hesap; sonra toplu yazım.
        let mut iv_results: Vec<(String, String)> = Vec::new(); // (symbol, chosen)
        let mut iv_log: Vec<String> = Vec::new();
        let mut strat_results: Vec<(String, String)> = Vec::new(); // (symbol, chosen strateji)
        let mut strat_log: Vec<String> = Vec::new();
        for sym in &iv_pool {
            let cur = iv_current.get(sym).cloned().unwrap_or_else(|| interval.clone());
            let (chosen, scored) = crate::robot::backtester::walk_forward::evaluate_symbol_interval(
                &auto_iv_candidates,
                |tf| crate::persistence::reader::read_candles_market(&db_path, sym, tf, &market, 5000).unwrap_or_default(),
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
            if let Some(best) = &chosen {
                if best != &cur { iv_results.push((sym.clone(), best.clone())); }
                if !scored.is_empty() {
                    let sc: Vec<String> = scored.iter().map(|(c, s)| format!("{c}={s:.1}")).collect();
                    iv_log.push(format!("{sym}→{best}[{}]", sc.join(",")));
                }
            }

            // ─── Per-symbol STRATEJİ seçimi (online self-discovery) ──────────────
            // Sembolün EFEKTİF interval'inde (yeni seçildiyse o, yoksa mevcut) tüm strateji
            // havuzunu pooled-PF ile skorla; param SABİT (iv_base = global best → strateji ekseni
            // izole + ucuz). Yalnız PF≥STRAT_MIN_SCORE kazanan + margin'i geçen atanır.
            let eff_iv = chosen.clone().unwrap_or_else(|| cur.clone());
            let strat_candles = crate::persistence::reader::read_candles_market(&db_path, sym, &eff_iv, &market, 5000)
                .unwrap_or_default();
            if strat_candles.len() >= min_iv_bars {
                let windows = crate::robot::backtester::walk_forward::wf_oos_windows(
                    strat_candles.len(), wf_is, wf_oos, wf_step);
                if !windows.is_empty() {
                    let cur_strat = strat_current.get(sym).map(|s| s.as_str());
                    let (s_chosen, s_scored) = crate::robot::backtester::walk_forward::evaluate_symbol_strategy(
                        &strat_pool,
                        |name| {
                            let mut cfg = iv_base.clone();
                            cfg.symbol = sym.clone();
                            cfg.interval = eff_iv.clone();
                            cfg.strategy_name = name.to_string();
                            Some(crate::robot::backtester::walk_forward::score_config_over_windows(
                                &cfg, &strat_candles, &windows))
                        },
                        cur_strat, STRAT_MARGIN, STRAT_MIN_SCORE,
                    );
                    if let Some(best) = s_chosen {
                        if Some(best.as_str()) != cur_strat { strat_results.push((sym.clone(), best.clone())); }
                        if !s_scored.is_empty() {
                            // En iyi 3 skoru logla (havuz büyük olabilir).
                            let mut top = s_scored.clone();
                            top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                            let sc: Vec<String> = top.iter().take(3).map(|(c, s)| format!("{c}={s:.2}")).collect();
                            strat_log.push(format!("{sym}@{eff_iv}→{best}[{}]", sc.join(",")));
                        }
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
                // Pool-wide otonom STRATEJİ kazananları (evaluate_symbol_strategy, PF≥1.0 + margin).
                for (sym, strat) in &strat_results {
                    params.symbol_strategy.insert(sym.clone(), strat.clone());
                }
            }
            // Pool-wide otonom interval özeti — gözlemlenebilirlik (değişenler + tüm skorlar).
            if !iv_log.is_empty() {
                st.push_log(format!(
                    "📐 auto-interval ({} sembol değerlendirildi, {} değişti): {}",
                    iv_log.len(), iv_results.len(), iv_log.join(" · "),
                ));
            }
            // Pool-wide otonom strateji özeti (per-symbol edge: hangi sembol hangi stratejiyi seçti).
            if !strat_log.is_empty() {
                st.push_log(format!(
                    "🧠 auto-strateji ({} sembol edge-li, {} değişti): {}",
                    strat_log.len(), strat_results.len(), strat_log.join(" · "),
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
                        "{r}(n={}) TP={:.1}% SL={:.1}% dir={} trail={}",
                        a.sample_count, a.median_tp_pct, a.median_sl_pct,
                        match regime_dir_map.get(r) { Some(true) => "RD", Some(false) => "long", None => "—" },
                        match regime_trail_map.get(r) { Some(t) => format!("{:.1}%", t), None => "—".into() },
                    ))
                    .collect();
                entries.sort();
                st.push_log(format!("🎚  Rejim katmanları yazıldı: {}", entries.join(" | ")));
            }
            // Çıkış-modeli taraması özeti (teşhis): model · işlem · win% · beklenti · PF · sharpe.
            if !exit_sweep.is_empty() {
                let line: Vec<String> = exit_sweep.iter().map(|s| format!(
                    "{}[n={} wr={:.0}% E={:+.3} PF={:.2} Sh={:+.2}]",
                    s.label, s.total_trades, s.win_rate * 100.0,
                    s.expectancy, s.profit_factor, s.sharpe,
                )).collect();
                st.push_log(format!("🪤 Çıkış taraması: {}", line.join(" · ")));
            }
            // Edge-filtre taraması özeti (teşhis): eşik · işlem · win% · beklenti · PF · sharpe.
            if !edge_sweep.is_empty() {
                let line: Vec<String> = edge_sweep.iter().map(|s| format!(
                    "{}[n={} wr={:.0}% E={:+.3} PF={:.2} Sh={:+.2}]",
                    s.label, s.total_trades, s.win_rate * 100.0,
                    s.expectancy, s.profit_factor, s.sharpe,
                )).collect();
                st.push_log(format!("🚪 Edge-filtre taraması: {}", line.join(" · ")));
            }
        }

        // Profil de diske mühürlenir.
        let regime_breakdown: serde_json::Value = regime_agg.iter()
            .map(|(r, a)| (r.clone(), serde_json::json!({
                "median_tp_pct": a.median_tp_pct,
                "median_sl_pct": a.median_sl_pct,
                "sample_count": a.sample_count,
                "regime_directional": regime_dir_map.get(r).copied(),
                "target_trail_pct": regime_trail_map.get(r).copied(),
            })))
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
        crate::persistence::writer::seal_config_to_disk("config/active_profile.json", &snapshot)
            .map_err(|e| format!("seal: {:?}", e))?;
        Ok(())
    }
}
