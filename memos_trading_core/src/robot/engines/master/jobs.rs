// src/robot/engines/master/jobs.rs — Periyodik işler: ML/screener/backtest/download + anomali onarımı + intel hub
// Faz 1 modülerleştirme: master.rs'ten taşındı (davranış birebir korunur).
use super::*;

impl Engine {

    /// 🧠 IntelligenceHub periyodik tick: drift hesabı + evrim + retrain kararı.
    ///
    /// Akış:
    /// 1. Aktif sembolün son 200 mumundan FeatureVector çıkar
    /// 2. hub.drift_detector.update(fv) → drift_score güncellenir
    /// 3. brain.drift_history'e push (TUI snapshot için)
    /// 4. hub.should_retrain(drift_score) true ise: triggers["ml"].store(true) ve repair_log
    /// 5. hub.tick_evolution() — controller cycle_id'yi artırır, periyot dolduğunda evolve
    pub(crate) async fn tick_intelligence_hub(state: &Arc<Mutex<AppState>>) {
        use crate::robot::ml_engine::feature_extractor::FeatureVector;

        // 1) Mum verisini ve aktif sembolü al (kilit kısa)
        let (symbol, interval, db_path) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            (st.config.symbol.clone(), st.config.interval.clone(), st.config.db_path.clone())
        };

        // Mum varsa drift güncellemesi yap; yoksa sadece evrim tick'i çalışır.
        let candles_opt = crate::persistence::reader::read_candles(&db_path, &symbol, &interval, 200).ok();
        let fv: Option<FeatureVector> = candles_opt.as_ref()
            .filter(|c| c.len() >= 50)
            .map(|c| crate::robot::ml_engine::feature_extractor::FeatureExtractor::extract(c));

        // 2-5) Hub mutasyonu — tek mutex altında
        let mut st = match state.lock() { Ok(s) => s, Err(_) => return };
        // Drift-tetikli retrain cooldown: sürekli yüksek drift'te her tick'te
        // yeni trigger basılmasın diye ML_DRIFT_COOLDOWN_SECS (default 600)
        // boyunca bekle. Cooldown=0 → her tick fire (testing modu).
        let ml_drift_cooldown: u64 = std::env::var("ML_DRIFT_COOLDOWN_SECS").ok()
            .and_then(|s| s.parse::<u64>().ok()).unwrap_or(600);
        let mut should_retrain = false;
        let mut armed = true;
        let mut drift_score = 0.0;
        let mut controller_cycle = 0u64;
        let mut evolved = false;
        if let Ok(mut hub) = st.brain.intelligence_hub.write() {
            if let Some(ref fv) = fv {
                hub.drift_detector.update(fv);
                drift_score = hub.drift_detector.drift_score;
                hub.drift_history.push_back(drift_score);
                while hub.drift_history.len() > 100 { hub.drift_history.pop_front(); }
                should_retrain = hub.should_retrain(drift_score);
                armed = hub.drift_retrain_armed(ml_drift_cooldown);
                if should_retrain && armed {
                    hub.mark_drift_retrain_fired();
                }
            }

            // Evrim tick — mum olsa da olmasa da controller cycle ilerler
            hub.controller.begin_cycle();
            controller_cycle = hub.controller.cycle_id;
            if hub.controller.should_evolve() {
                hub.controller.evolve_population();
                evolved = true;
            }
        }
        // brain'in dış drift_history aynalaması (TUI/AI Center için)
        st.brain.drift_history.push_back(drift_score);
        while st.brain.drift_history.len() > 100 { st.brain.drift_history.pop_front(); }

        match (should_retrain, armed) {
            (true, true) => {
                if let Some(t) = st.fleet.triggers.get("ml") {
                    t.store(true, Ordering::Relaxed);
                }
                st.push_log(format!(
                    "🧠 Hub: drift={:.3} eşik aşıldı ⇒ ml retrain tetiklendi (cycle={})",
                    drift_score, controller_cycle,
                ));
                // Repair log'a da düşür — kaynak "drift" olarak işaretli.
                st.guardian.repair_log.push_back(format!(
                    "[{}] hub: drift-driven retrain (drift={:.3}, cooldown={}s)",
                    chrono::Local::now().format("%H:%M:%S"), drift_score, ml_drift_cooldown,
                ));
                while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
            }
            (true, false) => {
                // Drift devam ediyor ama cooldown'da — tek satır bilgi log'u
                // (her tick spam'lemesin diye sadece guardian repair_log'a yaz,
                // ana UI log'una basma; orada görünmemesi normal akış).
                st.guardian.repair_log.push_back(format!(
                    "[{}] hub: drift={:.3} eşik aşıldı ama cooldown'da ({}s) — fire atlandı",
                    chrono::Local::now().format("%H:%M:%S"), drift_score, ml_drift_cooldown,
                ));
                while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
            }
            _ => { /* drift normal — sessiz */ }
        }
        if evolved {
            st.push_log(format!(
                "🧬 Hub evrim: popülasyon evrimleştirildi (cycle={})", controller_cycle,
            ));
        }
    }

    /// 🛡️ ANOMALİ ONARIMI: aktif anomali sayısı > 0 ise ML retrain tetiklenir,
    /// anomali türleri ve onarım kaydı guardian.repair_log'a düşürülür.
    ///
    /// Cooldown (`ANOMALY_ML_TRIGGER_COOLDOWN_SECS`, default 300sn):
    /// Anomali aralıksız sürüyorsa (ör. recovery edilen pasif semboller
    /// nedeniyle DataIngest Failed) her 500ms ML trigger spam'i oluşurdu.
    /// Cooldown süresince ML trigger ATIlMAZ; phase ve log girdisi yine yazılır.
    pub(crate) fn perform_anomaly_recovery(state: &Arc<Mutex<AppState>>, snap: &MissionControl) {
        if snap.active_anomalies == 0 { return; }
        let mut st = state.lock().unwrap();
        // Phase precedence: Booting/Executing > Recovering > Scanning.
        // Aynı tick'te trade yapıldıysa (execute_trade_cycle phase'i Executing'e
        // yazmışsa) onu ezme — operatör Executing'i görmeli. Stale ApiError
        // anomaly'leri (BEATUSDT/BLESSUSDT) sürekli Recovering basıp 1540
        // tick'te sadece 1 Executing görünmesine yol açıyordu.
        if !matches!(st.fleet.phase.as_str(), "Executing" | "Booting") {
            st.fleet.phase = "Recovering".into();
        }

        // Anomalilerin özetini çıkar (severity sayım)
        let mut warning_n = 0u32;
        let mut critical_n = 0u32;
        let mut kinds: Vec<String> = Vec::new();
        for a in &snap.anomalies {
            if a.severity.contains("Critical") { critical_n += 1; }
            else { warning_n += 1; }
            if !kinds.contains(&a.kind) { kinds.push(a.kind.clone()); }
        }

        // Cooldown denetimi: bir önceki ML trigger'dan beri yeterli süre geçti mi?
        let cooldown_secs = std::env::var("ANOMALY_ML_TRIGGER_COOLDOWN_SECS")
            .ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(300);
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let last_fired = ANOMALY_ML_LAST_TRIGGER_EPOCH.load(Ordering::Relaxed);
        let armed = last_fired == 0 || now_secs.saturating_sub(last_fired) >= cooldown_secs;

        // ML retrain'i tetikle (zaten ml job'u kendi loglarını basacak)
        // push_log + repair_log SADECE armed iken yazılır — cooldown'da spam yapmaz.
        // Daha önce her cycle (500ms) "ML retrain tetiklendi" log basıyordu ama cooldown
        // gerçek tetiği bastırıyordu → mesaj yanıltıcı + olay günlüğü doluyor.
        if armed {
            st.fleet.triggers.get("ml").map(|t| t.store(true, Ordering::Relaxed));
            ANOMALY_ML_LAST_TRIGGER_EPOCH.store(now_secs, Ordering::Relaxed);

            st.push_log(format!(
                "🚨 Anomali onarımı: {} aktif ({} kritik / {} uyarı), türler: {} ⇒ ML retrain tetiklendi",
                snap.active_anomalies, critical_n, warning_n,
                kinds.join(","),
            ));

            // Repair log: onarım adımının izi
            let repair_entry = format!(
                "[{}] auto-fix: ml-retrain dispatched (anomaly_count={})",
                chrono::Local::now().format("%H:%M:%S"), snap.active_anomalies,
            );
            st.guardian.repair_log.push_back(repair_entry);
            if st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
        }
    }

    /// 🗂️ Sembol-statü registry refresh: Binance exchangeInfo'dan tüm sembollerin
    /// `status`'ünü (TRADING/BREAK/HALT…) çeker, cache'i günceller + DB'ye persist eder.
    /// `symbol_eligible_for_live` bunu okur → halted/delisted (ALPACAUSDT BREAK gibi)
    /// otoritatif dışlanır; de/re-list otomatik yansır. Public endpoint (key gerekmez,
    /// paper modda da çalışır). Scheduler periyodik çağırır + boot warmup'ta bir kez.
    pub(crate) async fn run_symbol_status_refresh(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        let (market, db_path) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            (st.config.market.clone(), st.config.db_path.clone())
        };
        let url = if market.eq_ignore_ascii_case("futures") {
            "https://fapi.binance.com/fapi/v1/exchangeInfo"
        } else {
            "https://api.binance.com/api/v3/exchangeInfo"
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| format!("client: {}", e))?;
        let v: serde_json::Value = client.get(url).send().await
            .map_err(|e| format!("exchangeInfo fetch: {}", e))?
            .json().await
            .map_err(|e| format!("exchangeInfo parse: {}", e))?;
        let arr = v.get("symbols").and_then(|s| s.as_array())
            .ok_or_else(|| "exchangeInfo: symbols dizisi yok".to_string())?;
        let mut entries: Vec<(String, String)> = Vec::with_capacity(arr.len());
        for s in arr {
            if let (Some(sym), Some(status)) = (
                s.get("symbol").and_then(|x| x.as_str()),
                s.get("status").and_then(|x| x.as_str()),
            ) {
                entries.push((sym.to_string(), status.to_string()));
            }
        }
        if entries.is_empty() {
            return Err("exchangeInfo: 0 sembol döndü".to_string());
        }
        let n = entries.len();
        let n_break = entries.iter().filter(|(_, s)| s != "TRADING").count();

        // Hot-path eligibility'nin okuduğu cache'i güncelle.
        set_symbol_statuses(&entries);

        // DB'ye persist (restart hydrate için) — senkron, spawn_blocking.
        let db_clone = db_path.clone();
        let entries_clone = entries.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(conn) = rusqlite::Connection::open(&db_clone) {
                let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                if let Err(e) = crate::persistence::writer::save_symbol_statuses(&conn, &entries_clone) {
                    log::warn!("symbol_status persist: {:?}", e);
                }
            }
        }).await;

        push_state_log(state, format!(
            "🗂️ Sembol statü registry ✓ {} sembol ({} TRADING-dışı dışlandı)",
            n, n_break,
        ));
        Ok(())
    }

    /// 🧠 ML Retrain (Faz 4 - "ml" trigger):
    /// Aktif sembolde ParameterOptimizer.random_search çalıştırır, en iyi TP/SL/PS setini
    /// brain.best_params'a yazar ve config/best_params.json'a atomik mühürler.
    pub(crate) fn run_ml_retrain_job(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        log::info!("🧠 E2: ML Retrain başlatıldı (random search 60 iter)...");

        // 1) Çalışma sembolü ve mum kuyruğu — kilidi job boyunca tutmuyoruz.
        let (symbol, interval, db_path, capital) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            (st.config.symbol.clone(), st.config.interval.clone(),
             st.config.db_path.clone(), st.finance.equity)
        };

        push_state_log(state, format!(
            "🧠 ML Retrain başladı: sembol={} aralık={} kapital=${:.0}",
            symbol, interval, capital,
        ));

        let candles = crate::persistence::reader::read_candles(&db_path, &symbol, &interval, 1000)
            .map_err(|e| format!("read_candles: {}", e))?;
        if candles.len() < 50 {
            return Err(format!("yetersiz mum verisi: {} mum", candles.len()));
        }

        let strategy_name = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.brain.live_strategy.read().map(|s| s.clone()).unwrap_or_else(|_| "MA_CROSSOVER".into())
        };
        let strategy_name = if strategy_name.eq_ignore_ascii_case("default")
                              || strategy_name.eq_ignore_ascii_case("auto")
                              || strategy_name.is_empty() {
            "MA_CROSSOVER".to_string()
        } else { strategy_name };

        push_state_log(state, format!(
            "🧠 ML Retrain: {} mum yüklendi, strateji={}, random search 60 iter çalışıyor...",
            candles.len(), strategy_name,
        ));

        // Giriş kalitesi edge filtresi (#4): ML retrain'in TP/SL/PS aramasının da
        // canlı edge hunisini görmesi için (best_params canlıya gider). run_backtest_job
        // ile aynı env + default. bkz parse_edge_filter.
        let edge_min = parse_edge_filter(
            std::env::var("BACKTEST_EDGE_FILTER").ok(),
            Some(Self::dynamic_edge_threshold(0.0)),
            Self::dynamic_edge_threshold(0.0),
        );
        let opt = crate::robot::backtester::parameter_optimizer::ParameterOptimizer::new(
            symbol.clone(), interval.clone(), capital, strategy_name.clone(),
        ).with_edge_min_score(edge_min);
        let result = opt.random_search(&candles, 60)
            .map_err(|e| format!("random_search: {:?}", e))?;

        // 2) brain.best_params + ParameterStore.trade_risk'e yaz + ml_confidence güncelle.
        //    best_params HashMap legacy okuyucular için kalır; store yeni canonical
        //    kaynaktır (engine cycle pozisyon açılışta önce store'a bakar).
        let conf = (result.best_result.sharpe_ratio / 3.0).clamp(0.0, 1.0);
        {
            let mut st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.brain.best_params.insert("take_profit_pct".into(), result.best_parameters.take_profit_pct);
            st.brain.best_params.insert("stop_loss_pct".into(),   result.best_parameters.stop_loss_pct);
            st.brain.best_params.insert("max_position_size".into(), result.best_parameters.max_position_size);
            st.brain.best_params.insert("ml_score".into(), result.best_result.sharpe_ratio);
            if let Ok(mut params) = st.brain.parameters.write() {
                params.apply_optimization(
                    result.best_parameters.take_profit_pct,
                    result.best_parameters.stop_loss_pct,
                    result.best_parameters.max_position_size,
                );
            }
            st.brain.ml_confidence = conf;
            st.brain.hyperopt_score = result.best_result.sharpe_ratio;
            st.push_log(format!(
                "🧠 ML Retrain ✓ {} TP={:.2}% SL={:.2}% PS={:.2} | Sharpe={:.2} ({} kombinasyon)",
                strategy_name,
                result.best_parameters.take_profit_pct,
                result.best_parameters.stop_loss_pct,
                result.best_parameters.max_position_size,
                result.best_result.sharpe_ratio,
                result.total_tested,
            ));
        }

        // 2b) GBT (Gradient Boosted Trees) eğitimi — cycle başına dinamik
        //     ml_confidence için. build_training_set forward-return işaretini
        //     hedef alır; gbt_grid_search hyperparam'i seçer; final model
        //     `IntelligenceHub.gbt`'ye yazılır. Yetersiz veri/eğitim
        //     başarısızlığı sessiz fallback: statik ml_confidence yolunda kalınır.
        let gbt_window_bars: usize = std::env::var("GBT_WINDOW_BARS").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(50);
        let gbt_forward_bars: usize = std::env::var("GBT_FORWARD_BARS").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(5);

        // 🌐 HEDEF MİMARİ: GBT regime AI'ı GENİŞ TF'de (4h/1d) çalışmalı → modeli de
        // HTF mumlarıyla eğit (regime_for_cycle skoru aynı TF'den besler, train/infer
        // tutarlı). multi_tf açık + GBT_TRAIN_HTF (default true) + HTF DB'de yeterli ise
        // HTF; değilse base TF'ye düş (eski davranış). HTF mumları candles tablosunda
        // gerçek interval string'iyle ("4h") durur → doğrudan read_candles.
        let multi_tf_enabled = state.lock().ok()
            .and_then(|st| st.brain.parameters.read().ok().map(|p| p.multi_tf.enabled))
            .unwrap_or(true);
        let gbt_train_htf = !matches!(
            std::env::var("GBT_TRAIN_HTF").ok().as_deref(),
            Some("0") | Some("false") | Some("off"),
        );
        let htf_interval = crate::robot::data_pipeline::orchestrator::DataPipeline
            ::get_htf_interval(&interval);
        // build_training_set(window+forward) için ≥ ~20 örnek → en az ~window+forward+20 mum.
        const GBT_HTF_MIN_BARS: usize = 80;
        let htf_candles: Vec<crate::core::types::Candle> =
            if gbt_train_htf && multi_tf_enabled && htf_interval != interval {
                crate::persistence::reader::read_candles(&db_path, &symbol, htf_interval, 2000)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
        let (train_slice, train_interval): (&[crate::core::types::Candle], &str) =
            if htf_candles.len() >= GBT_HTF_MIN_BARS {
                (&htf_candles, htf_interval)
            } else {
                (&candles, interval.as_str())
            };
        let gbt_ds = crate::robot::ml_engine::build_training_set(
            train_slice, gbt_window_bars, gbt_forward_bars,
        );
        if gbt_ds.len() < 20 {
            push_state_log(state, format!(
                "🌲 GBT atlandı: yetersiz training örneği ({} < 20)", gbt_ds.len(),
            ));
        } else {
            use crate::robot::ml_engine::{gbt_grid_search, GradientBoostedTrees};
            let tune = gbt_grid_search(&gbt_ds);
            let (n_est, lr, depth, oos_acc) = match tune {
                Some(r) => (r.n_estimators, r.learning_rate, r.max_depth, r.oos_accuracy),
                None    => (5, 0.10, 3, f64::NAN),
            };
            let mut gbt = GradientBoostedTrees::new(n_est, lr, depth);
            gbt.train(&gbt_ds);
            let ready = gbt.is_ready();
            if ready {
                if let Ok(mut st) = state.lock() {
                    if let Ok(mut hub) = st.brain.intelligence_hub.write() {
                        hub.gbt = gbt;
                        // Hangi TF'de eğitildi → regime_for_cycle skoru aynı TF'den besler.
                        hub.gbt_trained_interval = Some(train_interval.to_string());
                    }
                    let acc_str = if oos_acc.is_nan() { "-".into() }
                                  else { format!("{:.1}%", oos_acc) };
                    st.push_log(format!(
                        "🌲 GBT ✓ TF={train_interval} n_est={n_est} lr={lr:.2} depth={depth} | OOS acc={acc_str} | {} örnek",
                        gbt_ds.len(),
                    ));
                }
            } else if let Ok(mut st) = state.lock() {
                st.push_log("🌲 GBT eğitim başarısız (is_ready=false)".into());
            }
        }

        // 3) Diske atomik mühürle (BestParamsCache → seal_config_to_disk).
        let snapshot = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.brain.best_params.clone()
        };
        crate::persistence::writer::seal_config_to_disk("config/best_params.json", &snapshot)
            .map_err(|e| format!("seal: {:?}", e))?;
        Ok(())
    }

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
             max_workers, current_workers, multi_tf_enabled, multi_tf_min) = {
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
            )
        };

        // 2) Env override'ları.
        let top_n: usize = std::env::var("SCREENER_TOP_N").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(8);
        let limit: usize = std::env::var("SCREENER_CANDLE_LIMIT").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(500);
        let min_volume: f64 = std::env::var("SCREENER_MIN_VOLUME").ok()
            .and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        let extras: Vec<String> = std::env::var("SCREENER_EXTRA_SYMBOLS").ok()
            .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default();
        // HTF bias delta — 0 veya multi_tf kapalıysa HTF yüklemeden saf tek-TF sıralama.
        let htf_bias_delta: f64 = std::env::var("SCREENER_HTF_BIAS").ok()
            .and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.2);
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
        let mut scored: Vec<(String, ScreenerScore)> = pool.par_iter().filter_map(|sym| {
            let candles = crate::persistence::reader::read_candles(&db_path, sym, &interval, limit).ok()?;
            if candles.len() < 50 { return None; }
            let htf_vec = if htf_aware {
                crate::robot::data_pipeline::load_htf_candles(&db_path, sym, &interval, multi_tf_min)
            } else {
                Vec::new()
            };
            let htf_slice = if htf_vec.is_empty() { None } else { Some(htf_vec.as_slice()) };
            let s = score_symbol(&candles, &active_strategy, 4.0, 2.0, 0.3, capital, htf_slice, htf_bias_delta);
            if s.avg_volume < min_volume { return None; }
            Some((sym.clone(), s))
        }).collect();

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
        let wf_is   = std::env::var("WALK_FORWARD_IS_BARS").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(200);
        let wf_oos  = std::env::var("WALK_FORWARD_OOS_BARS").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(50);
        let wf_step = std::env::var("WALK_FORWARD_STEP_BARS").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(50);
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
                let n_iters: usize = std::env::var("BACKTEST_STRATEGY_PARAM_ITERS").ok()
                    .and_then(|s| s.parse().ok()).unwrap_or(40);
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
        let regime_min_samples: usize = std::env::var("REGIME_MIN_SAMPLES").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(2);
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
                "sealed_at": chrono::Utc::now().to_rfc3339(),
            })
        };
        crate::persistence::writer::seal_config_to_disk("config/active_profile.json", &snapshot)
            .map_err(|e| format!("seal: {:?}", e))?;
        Ok(())
    }

    /// 🌐 Data Pipeline Download (Faz 4 - "download" trigger):
    /// Aktif sembol filosundaki her sembol için BinanceFetcher ile son N mumu çekip
    /// SQLite'a (candles_{symbol}_{interval} tablosu) yazar.
    ///
    /// Akış:
    /// 1. State'ten sembol listesi + interval + db_path + limit topla
    /// 2. Her sembol için fetch_latest çağır (paralel değil, sıralı — rate-limit dostu)
    /// 3. Başarılı çekimi spawn_blocking ile save_candle'a aktar (SQLite senkron API)
    /// 4. Her sembol için tek satır log + sonda toplam özet
    pub(crate) async fn run_download_job(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        use crate::robot::data_fetcher::binance::BinanceFetcher;
        use crate::robot::data_fetcher::market_fetcher::MarketFetcher;

        log::info!("🌐 E2: Data pipeline download başlatıldı...");

        // 1) Çalışma listesi — kilit kısa
        // Canlı feed'i olmayan borsa sembolleri (örn. BIST) download'a gönderilmez →
        // aksi halde "Veri Format Hatası" log kirliliği. Karar market-agnostik tek nokta:
        // RuntimeTuning.symbol_eligible_for_live (hydrate/price_poll/cycle ile aynı).
        let (symbols, interval, db_path, limit) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let tuning = Arc::clone(&st.tuning);
            let eligible = |s: &str| tuning.symbol_eligible_for_live(s);

            let mut syms: Vec<String> = vec![];
            if eligible(&st.config.symbol) { syms.push(st.config.symbol.clone()); }
            // SymbolOrchestrator + pinned
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for w in orch.get_worker_status() {
                    if !eligible(&w.symbol) { continue; }
                    if !syms.contains(&w.symbol) { syms.push(w.symbol); }
                }
            }
            for s in &st.config.pinned_symbols {
                if !eligible(s) { continue; }
                if !syms.contains(s) { syms.push(s.clone()); }
            }
            // Açık pozisyon sembolleri (recovery sonrası orchestrator register'ı
            // başarısız olursa veya trade pipeline-dışı bir sembolde açıldıysa
            // defensive olarak buradan da yakalıyoruz). Aksi halde stale candle
            // → price-sanity guard tetiklenir, pozisyon kapatılamaz.
            if let Ok(positions) = st.finance.live_positions.read() {
                for sym in positions.keys() {
                    if !eligible(sym) { continue; }
                    if !syms.contains(sym) { syms.push(sym.clone()); }
                }
            }
            syms.retain(|s| !s.is_empty());
            (syms, st.config.interval.clone(), st.config.db_path.clone(),
             st.config.download_candle_limit.max(50))
        };

        if symbols.is_empty() {
            return Err("indirilecek sembol yok (config.symbol + pinned + orchestrator boş)".into());
        }

        log::info!(
            "🌐 Download başladı: {} sembol × {} mum (interval={}) → {}",
            symbols.len(), limit, interval, symbols.join(","),
        );
        push_state_log(state, format!(
            "🌐 Download başladı: {} sembol × {} mum (interval={})",
            symbols.len(), limit, interval,
        ));

        // 2) Her sembol için sırayla mum çek + DB'ye yaz
        let fetcher = BinanceFetcher::new();
        let mut total_fetched = 0usize;
        let mut total_failed = 0usize;
        let mut per_symbol_summary: Vec<String> = Vec::new();

        for sym in &symbols {
            match fetcher.fetch_latest(sym, &interval, limit).await {
                Ok(candles) => {
                    // Başarılı fetch → delisted sayacını sıfırla (geçici hata
                    // sonrası sembol normalleştiyse yanlış pozitif olmasın).
                    delisted_record_success(sym);
                    let n = candles.len();
                    total_fetched += n;
                    // 3) SQLite yazımı senkron → spawn_blocking
                    let db_path_clone = db_path.clone();
                    let candles_clone = candles.clone();
                    // Yazımı gerçekten say + ilk hatayı yüzeye çıkar (eskiden `let _ =` ile
                    // yutuluyordu → şema uyumsuzluğunda sahte "✓ N mum yazıldı" basılıyordu).
                    let write_result = tokio::task::spawn_blocking(move || -> std::result::Result<(usize, Option<String>), String> {
                        let conn = rusqlite::Connection::open(&db_path_clone)
                            .map_err(|e| format!("db open: {}", e))?;
                        // WAL olsa da yazıcı-yazıcı çakışmasında anlık SQLITE_BUSY olabiliyor
                        // (snapshot/engine eşzamanlı yazımı) → busy_timeout ile bekle, "database
                        // is locked" ile mum düşürme.
                        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                        let mut written = 0usize;
                        let mut first_err: Option<String> = None;
                        for c in &candles_clone {
                            match crate::persistence::writer::save_candle(&conn, "binance", "spot", c) {
                                Ok(()) => written += 1,
                                Err(e) => if first_err.is_none() { first_err = Some(e.to_string()); },
                            }
                        }
                        Ok((written, first_err))
                    }).await;
                    match write_result {
                        // Fetch başarılı ama DB'ye HİÇ yazılamadıysa: bunu başarısızlık say,
                        // gerçek hatayı logla (sessiz veri donması yerine görünür sinyal).
                        Ok(Ok((0, err))) if n > 0 => {
                            total_failed += 1;
                            push_state_log(state, format!(
                                "    └─ {} ❌ fetch {} mum ama DB yazımı 0 (hata: {})",
                                sym, n, err.as_deref().unwrap_or("?"),
                            ));
                        }
                        Ok(Ok((written, err))) => {
                            let warn = match &err {
                                Some(e) => format!(" ⚠️ {} atlandı: {}", n.saturating_sub(written), e),
                                None => String::new(),
                            };
                            per_symbol_summary.push(format!("{}={}", sym, written));
                            // Otonom katman: sembol+interval bazlı noise floor hesabı.
                            // compute_symbol_stats min 64 candle istiyor (14 ATR + 50 sample);
                            // limit ≥50 garantili ama yetersizse None döner ve store
                            // güncellenmez → resolve_atr_mult fallback'e düşer.
                            if let Some(stats) = crate::robot::parameters::compute_symbol_stats(&candles) {
                                if let Ok(st) = state.lock() {
                                    if let Ok(mut params) = st.brain.parameters.write() {
                                        params.update_symbol_stats(sym, &interval, stats);
                                    }
                                }
                            }
                            push_state_log(state, format!("    └─ {} ✓ {} mum yazıldı{}", sym, written, warn));

                            // Multi-TF Faz B c2/c3: HTF (üst zaman dilimi) mumlarını da indir.
                            // get_htf_interval base ile aynıysa atla (1d → 1d). HTF fetch
                            // başarısızsa sessiz geç — htf_trend_filter eksiklikte
                            // pass-through yapar, cycle yine de döner.
                            // MULTI_TF_DOWNLOAD=false → HTF fetch skip (base interval yeterli).
                            let download_htf = state.lock().ok()
                                .and_then(|st| st.brain.parameters.read().ok()
                                    .map(|p| p.multi_tf.enabled && p.multi_tf.download_htf))
                                .unwrap_or(true);
                            let htf_interval = crate::robot::data_pipeline::DataPipeline::get_htf_interval(&interval);
                            if download_htf && htf_interval != interval {
                                let htf_limit = (limit / 4).max(50);
                                match fetcher.fetch_latest(sym, htf_interval, htf_limit).await {
                                    Ok(htf_candles) if !htf_candles.is_empty() => {
                                        let htf_n = htf_candles.len();
                                        let db2 = db_path.clone();
                                        let htf_clone = htf_candles.clone();
                                        let _ = tokio::task::spawn_blocking(move || {
                                            if let Ok(conn) = rusqlite::Connection::open(&db2) {
                                                let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                                                for c in &htf_clone {
                                                    let _ = crate::persistence::writer::save_candle(&conn, "binance", "spot", c);
                                                }
                                            }
                                        }).await;
                                        push_state_log(state, format!("        └─ {} HTF {} ✓ {} mum", sym, htf_interval, htf_n));
                                    }
                                    _ => {
                                        // HTF eksikliği fatal değil — loader fallback'i 1m varsa
                                        // CandleSynth ile in-memory üretebilir.
                                    }
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            total_failed += 1;
                            push_state_log(state, format!("    └─ {} ❌ yazma hatası: {}", sym, e));
                        }
                        Err(e) => {
                            total_failed += 1;
                            push_state_log(state, format!("    └─ {} ❌ blocking join hatası: {}", sym, e));
                        }
                    }
                }
                Err(e) => {
                    total_failed += 1;
                    log::warn!("🌐 Download fetch hatası: {} → {}", sym, e);
                    push_state_log(state, format!("    └─ {} ❌ fetch hatası: {}", sym, e));
                    // Delisted auto-detect: ardışık başarısızlık sayacı.
                    let n_fail = delisted_record_failure(sym);
                    let threshold = delisted_detection_threshold();
                    if threshold > 0 && n_fail >= threshold {
                        Self::purge_delisted_symbol(state, sym, n_fail);
                    }
                }
            }
        }

        // 4) Özet
        log::info!(
            "🌐 Download ✓ tamamlandı: {} mum (başarılı={}, başarısız={}) · {}",
            total_fetched,
            symbols.len() - total_failed,
            total_failed,
            per_symbol_summary.join(" · "),
        );
        if let Ok(mut st) = state.lock() {
            st.fleet.download_active = false;
            st.push_log(format!(
                "🌐 Download ✓ tamamlandı: {} mum (başarılı={}, başarısız={}) · {}",
                total_fetched,
                symbols.len() - total_failed,
                total_failed,
                per_symbol_summary.join(" · "),
            ));
        }

        if total_failed == symbols.len() {
            Err(format!("tüm {} sembolde indirme başarısız", symbols.len()))
        } else {
            Ok(())
        }
    }
}

/// `BACKTEST_EDGE_FILTER` env'ini giriş-kalitesi edge eşiğine çözer (#4). Backtest'in
/// canlı `process_symbol_cycle` edge hunisini aynalamasını ayarlar:
///   - unset → `default` (job'ın kararı; canlıyı aynalamak için Some(on_value))
///   - "0"/"false"/"off"/"none" → `None` (filtre yok, legacy: her Buy'da açılış)
///   - "1"/"true"/"on" → `Some(on_value)` (canlı cold-start eşiği = dynamic_edge_threshold(0))
///   - geçerli pozitif float → `Some(f)` (daha katı/gevşek elle eşik)
///   - geçersiz metin → `default` (sessiz fallback)
/// Serbest fonksiyon → env'siz unit-test edilebilir.
pub(crate) fn parse_edge_filter(
    raw: Option<String>, default: Option<f64>, on_value: f64,
) -> Option<f64> {
    match raw {
        None => default,
        Some(v) => {
            let v = v.trim();
            if v.eq_ignore_ascii_case("0") || v.eq_ignore_ascii_case("false")
                || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("none") {
                None
            } else if v.eq_ignore_ascii_case("1") || v.eq_ignore_ascii_case("true")
                || v.eq_ignore_ascii_case("on") {
                Some(on_value)
            } else {
                match v.parse::<f64>() {
                    Ok(f) if f > 0.0 => Some(f),
                    Ok(_) => None,        // ≤0 → kapalı
                    Err(_) => default,    // çöp girdi → default
                }
            }
        }
    }
}

#[cfg(test)]
mod edge_filter_tests {
    use super::parse_edge_filter;

    #[test]
    fn unset_uses_default() {
        assert_eq!(parse_edge_filter(None, Some(0.20), 0.20), Some(0.20));
        assert_eq!(parse_edge_filter(None, None, 0.20), None);
    }

    #[test]
    fn off_tokens_disable() {
        for t in ["0", "false", "FALSE", "off", "none", "  off  "] {
            assert_eq!(parse_edge_filter(Some(t.into()), Some(0.20), 0.20), None, "token={t}");
        }
    }

    #[test]
    fn on_tokens_use_on_value() {
        for t in ["1", "true", "TRUE", "on"] {
            assert_eq!(parse_edge_filter(Some(t.into()), None, 0.20), Some(0.20), "token={t}");
        }
    }

    #[test]
    fn float_override() {
        assert_eq!(parse_edge_filter(Some("0.35".into()), None, 0.20), Some(0.35));
        assert_eq!(parse_edge_filter(Some("-1".into()), Some(0.20), 0.20), None); // ≤0 → kapalı
        assert_eq!(parse_edge_filter(Some("çöp".into()), Some(0.20), 0.20), Some(0.20)); // fallback
    }
}
