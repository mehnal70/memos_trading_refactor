// src/robot/engines/master/jobs.rs — Periyodik işler: intel hub tick + anomali onarımı + sembol-statü + ML retrain.
// Faz 2 modülerleştirme: screener/backtest/download job'ları kardeş dosyalara ayrıldı (davranış birebir).
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
        let ml_drift_cooldown: u64 = env_parse("ML_DRIFT_COOLDOWN_SECS", 600);
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
        let cooldown_secs: u64 = env_parse("ANOMALY_ML_TRIGGER_COOLDOWN_SECS", 300);
        let now_secs = crate::core::time::now_epoch_secs();
        let last_fired = ANOMALY_ML_LAST_TRIGGER_EPOCH.load(Ordering::Relaxed);
        let armed = last_fired == 0 || now_secs.saturating_sub(last_fired) >= cooldown_secs;

        // ML retrain'i tetikle (zaten ml job'u kendi loglarını basacak)
        // push_log + repair_log SADECE armed iken yazılır — cooldown'da spam yapmaz.
        // Daha önce her cycle (500ms) "ML retrain tetiklendi" log basıyordu ama cooldown
        // gerçek tetiği bastırıyordu → mesaj yanıltıcı + olay günlüğü doluyor.
        if armed {
            if let Some(t) = st.fleet.triggers.get("ml") { t.store(true, Ordering::Relaxed) }
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
        let gbt_window_bars: usize = env_parse("GBT_WINDOW_BARS", 50);
        let gbt_forward_bars: usize = env_parse("GBT_FORWARD_BARS", 5);

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
}
