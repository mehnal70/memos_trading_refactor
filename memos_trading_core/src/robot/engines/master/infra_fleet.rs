// src/robot/engines/master/infra_fleet.rs — Altyapı filosu: spawn_* arka plan task'ları
// Faz 1 modülerleştirme: master.rs'ten taşındı (davranış birebir korunur).
use super::*;

impl Engine {

    /// 🛠️ INFRASTRUCTURE FLEET: Global servisleri non-blocking olarak yönetir.
    pub(crate) async fn spawn_infrastructure_fleet(state: Arc<Mutex<AppState>>) {
        log::info!("⚡ Srivastava Altyapı Filosu sevk ediliyor...");
        push_state_log(&state, "⚡ Altyapı filosu sevk edildi: snapshot(5s) · heartbeat-file(60s) · heartbeat(1s) · phase(2s) · price-poll(5s) · trigger(250ms) · scheduler(60s) · psync(30s) · ws-user-data · balance-sync(5dk)".into());

        // ── Task 0: MissionControl snapshot yazıcısı — her 5 sn'de bir tam state'i
        //    data/mission_control.json'a atomik (tmp+rename) yazar. Headless mod ve
        //    Android/web istemcileri tek gerçek kaynak olarak bu dosyayı okur.
        //    SNAPSHOT_WRITER_DISABLE=1 ise atlanır.
        let snapshot_disabled = std::env::var("SNAPSHOT_WRITER_DISABLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !snapshot_disabled {
            let snapshot_path = std::env::var("MISSION_CONTROL_SNAPSHOT_PATH")
                .unwrap_or_else(|_| "data/mission_control.json".to_string());
            let snapshot_secs: u64 = std::env::var("MISSION_CONTROL_SNAPSHOT_SECS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(5).max(1);
            crate::robot::infra::snapshot_writer::spawn_snapshot_writer(
                Arc::clone(&state), snapshot_path, snapshot_secs,
            );
        } else if let Ok(mut st) = state.lock() {
            st.push_log("📤 Snapshot writer devre dışı (SNAPSHOT_WRITER_DISABLE)".into());
        }

        // ── Task 0b: Heartbeat yazıcısı — her dakika equity/açık/kapalı/anomali/faz
        //    metriklerini logs/heartbeat.jsonl'e append'ler. RAM'deki "💓 Devriye"
        //    logu uçunca post-mortem ve equity zaman serisi bu dosyadan replay edilir.
        //    HEARTBEAT_WRITER_DISABLE=1 ise atlanır.
        let heartbeat_disabled = std::env::var("HEARTBEAT_WRITER_DISABLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !heartbeat_disabled {
            let heartbeat_path = std::env::var("HEARTBEAT_PATH")
                .unwrap_or_else(|_| "logs/heartbeat.jsonl".to_string());
            let heartbeat_secs: u64 = std::env::var("HEARTBEAT_SECS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(60).max(1);
            crate::robot::infra::heartbeat_writer::spawn_heartbeat_writer(
                Arc::clone(&state), heartbeat_path, heartbeat_secs,
            );
        } else if let Ok(mut st) = state.lock() {
            st.push_log("💓 Heartbeat writer devre dışı (HEARTBEAT_WRITER_DISABLE)".into());
        }

        // ── Task 1: Heartbeat — her saniye main_loop step'ini canlı tut, overdue'ya bak.
        // Anomali eşiği: ana döngü 500 ms hedefli; 5 sn'den uzun sessizlik DataStall sayılır.
        let st_hb = Arc::clone(&state);
        tokio::spawn(async move {
            use crate::robot::data_pipeline::{StepStatus, AnomalySeverity, AnomalyKind};
            use std::time::{SystemTime, UNIX_EPOCH};
            loop {
                let now_epoch = SystemTime::now().duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs()).unwrap_or(0);

                let stop = {
                    let st = match st_hb.lock() { Ok(s) => s, Err(_) => break };
                    if st.app_stop_signal.load(Ordering::Relaxed) { true } else {
                        let last_tick = st.fleet.last_loop_tick.load(Ordering::Relaxed);
                        // Daha hiç tick yazılmadıysa overdue hesaplama, sadece "warming up" göster
                        let overdue = if last_tick == 0 { 0 }
                                      else { now_epoch.saturating_sub(last_tick).saturating_sub(1) };

                        if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                            let status = if last_tick == 0 { StepStatus::Idle } else { StepStatus::Running };
                            pipe.record_step("main_loop", status, last_tick, overdue);
                            if overdue > 5 {
                                pipe.push_anomaly(
                                    AnomalySeverity::Warning,
                                    AnomalyKind::DataStall,
                                    format!("main_loop gecikti: +{}s", overdue),
                                );
                            }
                        }
                        false
                    }
                };
                if stop { break; }
                sleep(Duration::from_secs(1)).await;
            }
        });

        // ── Task 3: Fiyat poll — aktif semboller için REST üzerinden son fiyatı çek
        let st_px = Arc::clone(&state);
        tokio::spawn(async move {
            use crate::robot::data_fetcher::binance::BinanceFetcher;
            use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
            use crate::robot::data_pipeline::{StepStatus, AnomalySeverity, AnomalyKind};
            let fetcher = BinanceFetcher::new();
            let started_at = std::time::Instant::now();
            let poll_secs = 5_u64;
            // İlk başarılı çekimde özet log'u TUI'ye düşür (sonrasında sessiz, sadece anomalide konuşur).
            let mut first_summary_pending = true;
            let mut last_error_summary_at: u64 = 0;
            // Aynı mesaj içeriği art arda spam'lamasın — son özet bellekte tutulur;
            // sonraki çağrıda içerik birebir aynıysa atılır. BEATUSDT/BLESSUSDT gibi
            // kalıcı geçersiz sembollerin TUI'de tekrar tekrar görünmesini engeller.
            let mut last_error_summary_msg: String = String::new();

            loop {
                let (symbols, interval, stop) = {
                    let st = match st_px.lock() { Ok(s) => s, Err(_) => break };
                    if st.app_stop_signal.load(Ordering::Relaxed) {
                        (vec![], String::new(), true)
                    } else {
                        // Canlı feed'i olmayan borsa sembolleri (örn. BIST) Binance API'ye
                        // gönderilmez ("Veri Format Hatası" → ApiError anomaly). Market-agnostik
                        // tek nokta: RuntimeTuning.symbol_eligible_for_live.
                        let tuning = Arc::clone(&st.tuning);
                        let eligible = |s: &str| tuning.symbol_eligible_for_live(s);

                        let mut syms: Vec<String> = vec![];
                        if eligible(&st.config.symbol) { syms.push(st.config.symbol.clone()); }
                        if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                            for w in orch.get_worker_status() {
                                if !eligible(&w.symbol) { continue; }
                                if !syms.contains(&w.symbol) { syms.push(w.symbol); }
                            }
                        }
                        // Yetim pozisyonları da kapsa: orchestrator worker'ı kalmamış ama
                        // hâlâ açık pozisyon olan semboller — yoksa current_price hep entry'de
                        // takılı kalır, PnL=0 görünür, SL/TP denetimi yapılamaz.
                        if let Ok(positions) = st.finance.live_positions.read() {
                            for sym in positions.keys() {
                                if !eligible(sym) { continue; }
                                if !syms.contains(sym) { syms.push(sym.clone()); }
                            }
                        }
                        (syms, st.config.interval.clone(), false)
                    }
                };
                if stop { break; }

                let mut new_prices: Vec<(String, f64)> = Vec::with_capacity(symbols.len());
                let mut errors: Vec<(String, String)> = Vec::new();
                // PRICE_POLL_MAX_CANDLE_AGE_SECS: Binance 1m kline endpoint düşük
                // likiditeli sembollerde (örn. BTCUSDC) saatler/günler önceki
                // candle'ı döndürebiliyor. live_price'a stale değer yazılırsa
                // open_paper_position entry o stale fiyatla açılır, sonra
                // gerçek fiyatla kapanır → sahte PnL döngüsü (BTCUSDC 24h
                // auditte 86 trade ile $3500+ sahte kâr basmıştı).
                // Eşik default 300sn = 5dk × interval; 1m bar için 5 tane mum.
                let max_candle_age: i64 = env_parse("PRICE_POLL_MAX_CANDLE_AGE_SECS", 300);
                let mut stale_skipped: Vec<String> = Vec::new();
                for sym in &symbols {
                    if sym.is_empty() { continue; }
                    match fetcher.fetch_latest(sym, &interval, 1).await {
                        Ok(candles) => {
                            if let Some(last) = candles.last() {
                                if last.close <= 0.0 { continue; }
                                let age = (chrono::Utc::now() - last.timestamp).num_seconds();
                                if max_candle_age > 0 && age > max_candle_age {
                                    stale_skipped.push(format!("{}({}s)", sym, age));
                                    continue;
                                }
                                new_prices.push((sym.clone(), last.close));
                            }
                        }
                        Err(e) => errors.push((sym.clone(), e)),
                    }
                }
                if !stale_skipped.is_empty()
                    && log_throttle_should_emit("price_poll", "stale_candle", 300)
                {
                    log::warn!(
                        "price_poll: {} sembol için stale candle (>{}sn) — live_price güncellenmedi: {}",
                        stale_skipped.len(), max_candle_age, stale_skipped.join(","),
                    );
                }

                let now_secs = started_at.elapsed().as_secs();
                // TUI log paneli için özet (kilit aç/kapatmadan önce hazırla)
                let summary_msg: Option<String> = if first_summary_pending && !new_prices.is_empty() {
                    first_summary_pending = false;
                    Some(format!(
                        "📡 Price-poll çalışıyor: {} sembolden ilk fiyatlar alındı ({})",
                        new_prices.len(),
                        new_prices.iter().map(|(s, p)| format!("{}={:.2}", s, p))
                            .take(3).collect::<Vec<_>>().join(" · "),
                    ))
                } else if !errors.is_empty()
                    && now_secs.saturating_sub(last_error_summary_at) >= 300 {
                    // Throttle 300sn (5dk) — kalıcı kötü sembolün 30sn'de bir
                    // tekrarlanmasının operatör değeri yok. + dedupe: özet
                    // metni bir öncekiyle aynıysa skip (hata seti değişmemişse).
                    let msg = format!(
                        "⚠️ Price-poll: {}/{} sembolde hata. Örn: {}",
                        errors.len(), symbols.len(),
                        errors.first().map(|(s, e)| format!("{}: {}", s, e)).unwrap_or_default(),
                    );
                    if msg == last_error_summary_msg {
                        None
                    } else {
                        last_error_summary_at = now_secs;
                        last_error_summary_msg = msg.clone();
                        Some(msg)
                    }
                } else { None };

                // `now_secs` task başlangıcından elapsed; record_step ise bridge.rs
                // tarafından epoch saniye olarak değerlendiriliyor (now_epoch - last_run).
                // İki ayrı semantik ayağı karıştırmamak için record_step çağrısına ayrı
                // bir `now_epoch_secs` geç — yaş gösterimi doğru olur.
                let now_epoch_secs: u64 = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                if let Ok(mut st) = st_px.lock() {
                    if let Ok(mut prices) = st.fleet.live_price.write() {
                        for (sym, px) in &new_prices { prices.insert(sym.clone(), *px); }
                    }
                    if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                        // Status mantığı: hepsi başarısızsa Failed (gerçek hata),
                        // en az 1 başarılı ise Done (kalıcı kötü sembol bütün
                        // step'i Failed göstermemeli — BEATUSDT/BLESSUSDT gibi
                        // 1 hatalı 7 sağlıklı durumda step Done görünmeli).
                        // Hatalı semboller anomaly listesine ayrı yazılır.
                        let status = if new_prices.is_empty() && !errors.is_empty() {
                            StepStatus::Failed
                        } else {
                            StepStatus::Done
                        };
                        pipe.record_step("price_poll", status, now_epoch_secs, 0);
                        for (sym, e) in &errors {
                            pipe.push_anomaly(
                                AnomalySeverity::Warning,
                                AnomalyKind::ApiError,
                                format!("fiyat çekme hatası ({}): {}", sym, e),
                            );
                        }
                    }
                    if let Some(msg) = summary_msg { st.push_log(msg); }
                }

                sleep(Duration::from_secs(poll_secs)).await;
            }
        });

        // ── Task 4: Trigger handler — AtomicBool'larını dinler ve karşılayan job'u tetikler.
        let st_trig = Arc::clone(&state);
        tokio::spawn(async move {
            use crate::robot::data_pipeline::StepStatus;
            loop {
                let (fired, stop) = {
                    let st = match st_trig.lock() { Ok(s) => s, Err(_) => break };
                    if st.app_stop_signal.load(Ordering::Relaxed) {
                        (vec![], true)
                    } else {
                        let mut fired = Vec::new();
                        for (name, flag) in st.fleet.triggers.iter() {
                            if flag.swap(false, Ordering::AcqRel) {
                                fired.push(name.clone());
                            }
                        }
                        (fired, false)
                    }
                };
                if stop { break; }

                if !fired.is_empty() {
                    // Aşağıdaki record_step çağrıları için epoch saniye. Bridge.rs
                    // "X saniye önce" yaşı bu epoch'tan hesaplar.
                    let now_secs: u64 = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

                    for name in &fired {
                        let label = format!("trigger:{}", name);

                        // Tetik bağlamını state'ten oku (kaynak: manuel/anomali/scheduler vb.)
                        if let Ok(mut st) = st_trig.lock() {
                            let n_anom = st.guardian.live_pipeline.read()
                                .map(|p| p.anomalies.len()).unwrap_or(0);
                            let context = if n_anom > 0 { "otonom (anomali)" } else { "manuel veya zamanlı" };
                            let detail = match name.as_str() {
                                "ml"       => "GBT modeli yeniden eğitilecek, best_params güncellenecek",
                                "backtest" => "Walk-forward grid search çalışacak, aktif strateji yeniden seçilecek",
                                "download" => "Pipeline mum tamamlama görevi koşacak",
                                "screener" => "Sembol tarayıcı yeniden çalışacak",
                                _ => "Bilinmeyen tetikleyici",
                            };
                            st.push_log(format!("🎮 Tetik [{}] ⇒ {}: {}", context, name, detail));
                            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                                pipe.record_step(label.clone(), StepStatus::Running, now_secs, 0);
                            }
                        }

                        let state_clone = Arc::clone(&st_trig);
                        let trigger_name = name.clone();

                        tokio::spawn(async move {
                            let mut final_status = StepStatus::Done;
                            match trigger_name.as_str() {
                                "ml" => {
                                    // Faz 7 (Optimize): ML retrain de bir optimization
                                    // işidir — GBT modeli + best_params'ı günceller.
                                    // Backtest gibi periyodik (~30dk), bu yüzden TUI
                                    // pipeline timeline'ında 7. madde Done görüntüsü
                                    // ml retrain başarısından da gelir (backtest 2sa'lık
                                    // cron'u beklemeden).
                                    Self::mark_pipeline_stage(
                                        &state_clone,
                                        crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                        crate::robot::data_pipeline::StepStatus::Running,
                                    );
                                    let st_for_job = Arc::clone(&state_clone);
                                    let out = tokio::task::spawn_blocking(move || {
                                        Self::run_ml_retrain_job(&st_for_job)
                                    }).await;
                                    match out {
                                        Ok(Ok(())) => {
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Done,
                                            );
                                        }
                                        Ok(Err(e)) => {
                                            log::warn!("🧠 ML retrain başarısız: {}", e);
                                            if let Ok(mut st) = state_clone.lock() {
                                                st.push_log(format!("❌ ML Retrain başarısız: {}", e));
                                            }
                                            final_status = StepStatus::Failed;
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Failed,
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!("🧠 ML retrain join hatası: {}", e);
                                            final_status = StepStatus::Failed;
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Failed,
                                            );
                                        }
                                    }
                                },
                                "backtest" => {
                                    // Faz 7 (Optimize) Running: walk-forward başlıyor.
                                    Self::mark_pipeline_stage(
                                        &state_clone,
                                        crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                        crate::robot::data_pipeline::StepStatus::Running,
                                    );
                                    let st_for_job = Arc::clone(&state_clone);
                                    let out = tokio::task::spawn_blocking(move || {
                                        Self::run_backtest_job(&st_for_job)
                                    }).await;
                                    match out {
                                        Ok(Ok(())) => {
                                            // ─── Faz 7 (Optimize): walk-forward backtest tamamlandı,
                                            // best_params/strategy_selector güncellendi.
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Done,
                                            );
                                        }
                                        Ok(Err(e)) => {
                                            log::warn!("🔬 Backtest başarısız: {}", e);
                                            if let Ok(mut st) = state_clone.lock() {
                                                st.push_log(format!("❌ Backtest başarısız: {}", e));
                                            }
                                            final_status = StepStatus::Failed;
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Failed,
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!("🔬 Backtest join hatası: {}", e);
                                            final_status = StepStatus::Failed;
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Failed,
                                            );
                                        }
                                    }
                                },
                                "download" => {
                                    log::info!("🌐 E2: Veri Akış Hattı (Data Pipeline) mum tamamlama görevine başladı...");
                                    let st_for_dl = Arc::clone(&state_clone);
                                    if let Err(e) = Self::run_download_job(&st_for_dl).await {
                                        log::warn!("🌐 Download başarısız: {}", e);
                                        if let Ok(mut st) = state_clone.lock() {
                                            st.push_log(format!("❌ Download başarısız: {}", e));
                                        }
                                        final_status = StepStatus::Failed;
                                    }
                                },
                                "screener" => {
                                    log::info!("🔭 E2: Sembol tarayıcısı başladı (otonom multi-symbol seçimi)");
                                    let st_for_scr = Arc::clone(&state_clone);
                                    let out = tokio::task::spawn_blocking(move || {
                                        Self::run_screener_job(&st_for_scr)
                                    }).await;
                                    match out {
                                        Ok(Ok(())) => {}
                                        Ok(Err(e)) => {
                                            log::warn!("🔭 Screener başarısız: {}", e);
                                            if let Ok(mut st) = state_clone.lock() {
                                                st.push_log(format!("❌ Screener başarısız: {}", e));
                                            }
                                            final_status = StepStatus::Failed;
                                        }
                                        Err(e) => {
                                            log::warn!("🔭 Screener join hatası: {}", e);
                                            final_status = StepStatus::Failed;
                                        }
                                    }
                                },
                                _ => {
                                    log::warn!("⚠️ Bilinmeyen tetikleyici sinyali alındı: {}", trigger_name);
                                    final_status = StepStatus::Skipped;
                                }
                            }

                            if let Ok(st) = state_clone.lock() {
                                if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                                    pipe.record_step(label, final_status, now_secs, 0);
                                }
                            }
                        });

                    }
                }

                sleep(Duration::from_millis(250)).await;
            }
        });

        // ── Task 2: Phase tracker — fleet.phase değişimini pipeline step'i olarak işle.
        let st_pipe = Arc::clone(&state);
        tokio::spawn(async move {
            use crate::robot::data_pipeline::StepStatus;
            let mut last_phase = String::new();
            loop {
                let (current_phase, stop) = {
                    let st = match st_pipe.lock() { Ok(s) => s, Err(_) => break };
                    if st.app_stop_signal.load(Ordering::Relaxed) {
                        (String::new(), true)
                    } else {
                        (st.fleet.phase.clone(), false)
                    }
                };
                if stop { break; }

                if current_phase != last_phase {
                    // record_step epoch saniye bekler; bridge "now - last_run" yaşı bundan
                    // hesaplar. Elapsed semantiği vereyim diye eski hesap "1779…" anomalisi
                    // yaratıyordu.
                    let now_epoch_secs: u64 = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    if let Ok(st) = st_pipe.lock() {
                        if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                            pipe.record_step(
                                format!("phase:{}", current_phase),
                                StepStatus::Done,
                                now_epoch_secs,
                                0,
                            );
                        }
                    }
                    last_phase = current_phase;
                }
                sleep(Duration::from_secs(2)).await;
            }
        });

        // ── Task 5: Scheduler — config'deki periyodlara göre download/backtest tetiği.
        //
        //   download_enabled  + download_every_mins   → "download" trigger pulse
        //   pipeline_enabled  + pipeline_every_mins   → "backtest" trigger pulse
        //
        // Boot sonrası WARMUP (30 sn) bittiğinde bir ilk download tetiği atılır ki
        // TUI hemen mum verisiyle dolsun. Backtest yalnız periyot dolunca tetiklenir.
        let st_sched = Arc::clone(&state);
        tokio::spawn(async move {
            const WARMUP_SECS: u64 = 30;
            const CHECK_EVERY_SECS: u64 = 60; // dakika hassasiyeti yeter

            // İlk fırlatma noktaları — sürekli her N dakikada bir tetikleme için Instant.
            // last_*_at başlangıçta None ⇒ ilk turda warmup sonrası tetiklenir.
            let mut last_download_at: Option<std::time::Instant> = None;
            let mut last_backtest_at: Option<std::time::Instant> = None;
            let mut last_screener_at: Option<std::time::Instant> = None;
            let mut last_ml_at: Option<std::time::Instant> = None;
            let mut warmup_done = false;

            // Screener tetik aralığı env'le ayarlanır; config struct'a alan eklemeden
            // davranış aktivleştirilir. Default 30 dk; 0 → screener fire kapalı.
            let screener_enabled = std::env::var("SCHEDULER_SCREENER_ENABLED")
                .map(|v| v != "false" && v != "0").unwrap_or(true);
            let screener_period: u64 = env_parse("SCHEDULER_SCREENER_EVERY_MINS", 30);

            // ML periyodik fallback: drift-only fire (intelligence_hub) düşük drift'te
            // hiç eğitim yapmıyordu → kullanıcı "hareketlenme yok" diyor. Periyodik
            // pulse ile en azından N dakikada bir yeniden eğitim garanti edilir.
            // Drift cooldown (cefc955) zaten arka arkaya çakışmayı önler.
            let ml_periodic_enabled = std::env::var("SCHEDULER_ML_ENABLED")
                .map(|v| v != "false" && v != "0").unwrap_or(true);
            let ml_period: u64 = env_parse("SCHEDULER_ML_EVERY_MINS", 120);

            // Backtest cadence — screener/ml gibi env-ayarlı (eskiden tek
            // config.pipeline_every_mins=120 sabitiyle gelir, env override yoktu →
            // backtest scheduler'ın tek env-dışı görevi idi). SCHEDULER_BACKTEST_EVERY_MINS
            // set ise config'i geçersiz kılar. SCHEDULER_BACKTEST_WARMUP=1 → boot
            // warmup'ında bir kez tetikle (ops/test: ilk periyodu beklemeden çalıştır).
            let bt_period_override: Option<u64> = std::env::var("SCHEDULER_BACKTEST_EVERY_MINS")
                .ok().and_then(|s| s.parse().ok());
            let bt_warmup = env_truthy("SCHEDULER_BACKTEST_WARMUP");

            sleep(Duration::from_secs(WARMUP_SECS)).await; // boot warmup

            loop {
                let stop = st_sched.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                // Config okumayı kısa kilit altında yap
                let (dl_enabled, dl_period, bt_enabled, bt_period) = {
                    let st = match st_sched.lock() { Ok(s) => s, Err(_) => break };
                    (st.config.download_enabled, st.config.download_every_mins,
                     st.config.pipeline_enabled,
                     bt_period_override.unwrap_or(st.config.pipeline_every_mins))
                };

                let now = std::time::Instant::now();

                // İlk warmup turu: download_enabled ise hemen bir kerelik tetik bas
                // ki kullanıcı TUI'ye baktığında veri akışı görünür olsun. Screener
                // de aynı turda kısa bir gecikme sonrası (download bittikten sonra
                // pool'un dolu olması için scheduler'ın bir sonraki turunda) fire eder.
                if !warmup_done {
                    warmup_done = true;
                    if dl_enabled {
                        if let Ok(st) = st_sched.lock() {
                            if let Some(t) = st.fleet.triggers.get("download") {
                                t.store(true, Ordering::Relaxed);
                            }
                        }
                        if let Ok(mut st) = st_sched.lock() {
                            st.push_log("⏰ Scheduler: warmup tamamlandı → ilk download tetiği".into());
                        }
                        last_download_at = Some(now);
                    }
                    // Boot ML warmup tetiği: anomaly bazlı tetik artık schema guard
                    // sayesinde tetiklenmiyor (DataIngest Failed yok), bu yüzden
                    // ilk run'da 120dk beklemeden GBT'yi cold-start eğitelim.
                    // SCHEDULER_ML_WARMUP_SKIP=1 ile bu tetik kapatılabilir.
                    let skip_warmup_ml = env_truthy("SCHEDULER_ML_WARMUP_SKIP");
                    if ml_periodic_enabled && !skip_warmup_ml {
                        if let Ok(st) = st_sched.lock() {
                            if let Some(t) = st.fleet.triggers.get("ml") {
                                t.store(true, Ordering::Relaxed);
                            }
                        }
                        if let Ok(mut st) = st_sched.lock() {
                            st.push_log("⏰ Scheduler: warmup → ilk ML retrain tetiği (GBT cold-train)".into());
                        }
                        last_ml_at = Some(now);
                    }
                    // Backtest warmup tetiği (opt-in): ilk periyodu beklemeden bir kez
                    // çalıştır. Backtest geçmiş mumları kullanır (taze real-time veri
                    // gerekmez), DB'deki tarihsel seri yeterli. last_backtest_at warmup'a
                    // set edilir → periyot sayacı buradan başlar, hemen tekrar fire etmez.
                    if bt_enabled && bt_warmup {
                        if let Ok(st) = st_sched.lock() {
                            if let Some(t) = st.fleet.triggers.get("backtest") {
                                t.store(true, Ordering::Relaxed);
                            }
                        }
                        if let Ok(mut st) = st_sched.lock() {
                            st.push_log("⏰ Scheduler: warmup → ilk backtest tetiği (SCHEDULER_BACKTEST_WARMUP)".into());
                        }
                        last_backtest_at = Some(now);
                    }
                }

                // Periyodik download tetiği
                if dl_enabled && dl_period > 0 {
                    let due = match last_download_at {
                        Some(t) => now.duration_since(t) >= Duration::from_secs(dl_period * 60),
                        None    => true,
                    };
                    if due {
                        if let Ok(st) = st_sched.lock() {
                            if let Some(t) = st.fleet.triggers.get("download") {
                                t.store(true, Ordering::Relaxed);
                            }
                        }
                        if let Ok(mut st) = st_sched.lock() {
                            st.push_log(format!(
                                "⏰ Scheduler: periyodik download tetiği (her {} dk)", dl_period,
                            ));
                        }
                        last_download_at = Some(now);
                    }
                }

                // Periyodik backtest tetiği
                if bt_enabled && bt_period > 0 {
                    let due = match last_backtest_at {
                        Some(t) => now.duration_since(t) >= Duration::from_secs(bt_period * 60),
                        None    => false, // boot'tan hemen sonra backtest çalıştırma; ilk periyodu bekle
                    };
                    if due {
                        if let Ok(st) = st_sched.lock() {
                            if let Some(t) = st.fleet.triggers.get("backtest") {
                                t.store(true, Ordering::Relaxed);
                            }
                        }
                        if let Ok(mut st) = st_sched.lock() {
                            st.push_log(format!(
                                "⏰ Scheduler: periyodik backtest tetiği (her {} dk)", bt_period,
                            ));
                        }
                        last_backtest_at = Some(now);
                    } else if last_backtest_at.is_none() {
                        // İlk kez: bu turun zamanını kayıt et ki periyot hesabı başlasın
                        last_backtest_at = Some(now);
                    }
                }

                // Periyodik screener tetiği — orchestrator havuzuna otonom sembol akışı.
                // İlk tur: warmup turunu zaten geçtikten sonraki ilk check'te fire eder
                // (last_screener_at hâlâ None ise due=true). Sonraki turlarda screener_period.
                if screener_enabled && screener_period > 0 {
                    let due = match last_screener_at {
                        Some(t) => now.duration_since(t) >= Duration::from_secs(screener_period * 60),
                        None    => true,
                    };
                    if due {
                        if let Ok(st) = st_sched.lock() {
                            if let Some(t) = st.fleet.triggers.get("screener") {
                                t.store(true, Ordering::Relaxed);
                            }
                        }
                        if let Ok(mut st) = st_sched.lock() {
                            st.push_log(format!(
                                "⏰ Scheduler: periyodik screener tetiği (her {} dk)", screener_period,
                            ));
                        }
                        last_screener_at = Some(now);
                    }
                }

                // Periyodik ML fallback tetiği — drift-only fire'a ek garanti.
                // İlk tur: backtest gibi periyodu bekler (boot anında hemen retrain
                // çalıştırmak için yeterli veri olmayabilir).
                if ml_periodic_enabled && ml_period > 0 {
                    let due = match last_ml_at {
                        Some(t) => now.duration_since(t) >= Duration::from_secs(ml_period * 60),
                        None    => false,
                    };
                    if due {
                        if let Ok(st) = st_sched.lock() {
                            if let Some(t) = st.fleet.triggers.get("ml") {
                                t.store(true, Ordering::Relaxed);
                            }
                        }
                        if let Ok(mut st) = st_sched.lock() {
                            st.push_log(format!(
                                "⏰ Scheduler: periyodik ML retrain tetiği (her {} dk)", ml_period,
                            ));
                        }
                        last_ml_at = Some(now);
                    } else if last_ml_at.is_none() {
                        last_ml_at = Some(now);
                    }
                }

                sleep(Duration::from_secs(CHECK_EVERY_SECS)).await;
            }
        });

        // ── Task 6: Protection sync (sadece Live mode'da çalışır).
        //
        // Her 30 sn'de bir, açık Live pozisyonların borsadaki SL+TP emirlerini sorgular.
        // Bir taraf tetiklenmişse (0 veya 1 açık emir görülür), local pozisyonu kapatır
        // ve orphan kalan diğer emri cancel eder. Bot ölmese bile bu döngü hız kazandırır:
        // SL borsa tarafında tetiklenir → 30 sn içinde local arşive geçer.
        let st_psync = Arc::clone(&state);
        tokio::spawn(async move {
            const SYNC_EVERY_SECS: u64 = 30;
            loop {
                let stop = st_psync.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                // Live executor + aktif sembol listesi — kısa kilit altında
                let (executor, db_path, interval, active_symbols, live_dry_run) = {
                    let st = match st_psync.lock() { Ok(s) => s, Err(_) => break };
                    let executor = st.live_executor.clone();
                    let active: Vec<String> = st.finance.live_positions.read()
                        .map(|m| m.keys().cloned().collect()).unwrap_or_default();
                    (executor, st.config.db_path.clone(), st.config.interval.clone(),
                     active, st.live_dry_run)
                };

                // Yalnız Live mode + dry-run değil
                if let (Some(exec), false) = (executor, live_dry_run) {
                    for symbol in &active_symbols {
                        match exec.get_open_orders(symbol).await {
                            Ok(orders) => {
                                let n = orders.len();
                                // Pozisyon açıldığında 2 koruma emiri verildiği için < 2 demek tetiklendi.
                                if n < 2 {
                                    if let Ok(mut st) = st_psync.lock() {
                                        st.push_log(format!(
                                            "🔄 [SYNC] {} açık emir={} (<2) → SL/TP tetiklenmiş, local pozisyon kapatılıyor",
                                            symbol, n,
                                        ));
                                    }
                                    // Orphan emri (varsa) temizle
                                    if n == 1 {
                                        let _ = exec.cancel_all_orders(symbol).await;
                                    }
                                    // Mum verisini al ve local pozisyonu strateji-sinyal sebebiyle kapat
                                    if let Ok(candles) = crate::persistence::reader::read_candles(
                                        &db_path, symbol, &interval, 5,
                                    ) {
                                        if !candles.is_empty() {
                                            // close_paper_position'ı Live emir ile değil, sadece
                                            // local tarafı kapatacak şekilde çağırırız. Live close
                                            // zaten zaten 0 pozisyon dönecek (Binance "Pozisyon kapalı"
                                            // hatası). Sebebi: SL ve TP zaten tetiklendi.
                                            Self::close_paper_position(
                                                &st_psync, symbol, &candles, ExitReason::TrailingStop,
                                            ).await;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                if let Ok(mut st) = st_psync.lock() {
                                    st.push_log(format!(
                                        "⚠️ [SYNC] {} get_open_orders hatası: {:?}", symbol, e,
                                    ));
                                }
                            }
                        }
                    }
                }

                sleep(Duration::from_secs(SYNC_EVERY_SECS)).await;
            }
        });

        // ── Task 7: WebSocket userDataStream (Live mode + non-dry-run).
        //
        // psync (30s polling) çok yavaş. WS sayesinde fill event'i milisaniye
        // mertebesinde yakalanır → diğer koruma emri anında cancel, local pozisyon
        // anında arşivlenir. Bağlantı hatası olursa exponential backoff ile yeniden
        // dener; reconnect başarısız olsa bile psync task hâlâ yedek olarak çalışır.
        Self::spawn_user_data_stream(Arc::clone(&state));

        // ── Task 8: Hesap bakiye senkronu (Live mode + non-dry-run).
        //
        // Her 5 dk borsa bakiyesini çeker ve AppState'in equity'siyle karşılaştırır.
        // %1+ fark → repair_log + uyarı (bot ↔ borsa para sayımı ayrışmış).
        // Senkron için BALANCE_SYNC_EVERY_SECS env'i ile aralık ayarlanabilir.
        Self::spawn_balance_sync(Arc::clone(&state));

        // ── Task 9: Daily/weekly trade summary raporu (mode-agnostik).
        //
        // closed_trades üzerinden o günün ve haftanın özetini her 5 dk yeniden
        // hesaplayıp data/reports/ altına atomik JSON olarak yazar. Geçmiş raporlar
        // dokunulmaz (dosya adı tarih/hafta bazlı). Paper & Live ortak çalışır.
        // TRADE_REPORT_DIR ve TRADE_REPORT_EVERY_SECS env'leri ayarı geçer.
        let reports_dir = std::env::var("TRADE_REPORT_DIR")
            .unwrap_or_else(|_| "data/reports".to_owned());
        let report_every = std::env::var("TRADE_REPORT_EVERY_SECS")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(300);
        crate::robot::infra::reporting::trade_summary::spawn_trade_summary(
            Arc::clone(&state), reports_dir, report_every,
        );

        // ── Task 10: Periyodik S/R bölge tespiti (mode-agnostik).
        //
        // Aktif sembol seti × son N candle → SrDetector::detect → fleet.live_sr_zones.
        // TUI "Market Gözetimi" (tuş 5) ve Engine'in S/R bağlam sorgusu bu state'i
        // okuyor; daha önce dolduran bir bağlantı yoktu, panel boş kalıyordu.
        // SR_UPDATER_DISABLE=1 ile kapatılabilir, SR_UPDATE_EVERY_SECS (default 30)
        // ile aralık ayarlanır.
        Self::spawn_sr_updater(Arc::clone(&state));

        // ── Task 11: Trail-feedback processor (Phase C otonom katman).
        //
        // PENDING_TRAIL_OBS kuyruğundaki TRAILING_STOP gözlemlerini 60sn olgunlaştıktan
        // sonra evalue eder: exit_price ile mevcut live_price karşılaştırılarak
        // "early exit" (trail çok sıkıydı) veya "right exit" tespit edilir; sonuç
        // ParameterStore.record_trailing_outcome'a aktarılır. Pencere dolunca patch
        // uygulanır (per sym+strateji target_override).
        Self::spawn_trail_feedback_processor(Arc::clone(&state));

        // ScalpSwing A4: periyodik auto_tune. Her N sn'de bir
        // brain.scalp_swing_stats okunur, auto_tune Scalp ve Swing kanalları
        // için ayrı ayrı çağrılır; bounds dahilinde değişiklikler config'e
        // yazılır. SCALP_SWING_TUNE_EVERY_SECS env yoksa 300sn (5dk) default.
        Self::spawn_scalp_swing_tuner(Arc::clone(&state));
    }

    /// ScalpSwing A4: periyodik tuner task. scalp_swing_stats'i okuyup
    /// `auto_tune(stats, Scalp, cfg)` ve `auto_tune(stats, Swing, cfg)`
    /// çağırır; değişiklikleri config'e yazar + log push. Stop signal
    /// veya SCALP_SWING_TUNE_DISABLE=1 ile devre dışı bırakılır.
    pub(crate) fn spawn_scalp_swing_tuner(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            if std::env::var("SCALP_SWING_TUNE_DISABLE").ok().as_deref() == Some("1") {
                push_state_log(&state, "🎚️ ScalpSwing tuner: DISABLE=1, task pasif".into());
                return;
            }
            let every_secs: u64 = std::env::var("SCALP_SWING_TUNE_EVERY_SECS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(300);

            // Tuner devrede olduğunu boot'ta operatöre bildir — aksi halde
            // "tuner çalışmadı mı?" şüphesi oluşuyordu (summary boş olunca
            // hiç log atılmıyor → görünürlük kayboluyordu).
            push_state_log(&state, format!(
                "🎚️ ScalpSwing tuner devrede (periyot={}sn, min trade=5)",
                every_secs,
            ));

            // İlk turda warmup için kısa bekle — boot anında stats boş olur.
            tokio::time::sleep(std::time::Duration::from_secs(every_secs.min(30))).await;

            loop {
                let stop = state.lock()
                    .map(|s| s.app_stop_signal.load(Ordering::Relaxed))
                    .unwrap_or(true);
                if stop { break; }

                // 1) Stats + cfg snapshot (tek kısa mutex; sleep'ten önce drop).
                let cfg_stats = {
                    if let Ok(st) = state.lock() {
                        let cfg = st.brain.scalp_swing_config.read().ok().map(|c| c.clone());
                        let stats = st.brain.scalp_swing_stats.read().ok().map(|t| t.clone());
                        cfg.zip(stats)
                    } else { None }
                };

                // 2) auto_tune çağrıları + yazma (mutex'ler block içinde).
                let mut summary: Vec<String> = Vec::new();
                if let Some((mut cfg, stats)) = cfg_stats {
                    if cfg.autonomous_tuning {
                        if stats.scalp.total_closed >= 5 {
                            summary.extend(crate::robot::scalp_swing::auto_tune(
                                &stats.scalp,
                                crate::robot::scalp_swing::TradeType::Scalp,
                                &mut cfg,
                            ));
                        }
                        if stats.swing.total_closed >= 5 {
                            summary.extend(crate::robot::scalp_swing::auto_tune(
                                &stats.swing,
                                crate::robot::scalp_swing::TradeType::Swing,
                                &mut cfg,
                            ));
                        }
                        // A6: Otonom kanal-kapama. 20+ trade'lik yeterli sample
                        // varsa ve win_rate < 0.30 ise kanal kalıcı olarak
                        // pasifleştirilir → ScalpSwing fırsatı üretmeyi durur.
                        // Operatör override: cfg.scalp_enabled/swing_enabled
                        // manuel re-enable (UI veya config dosyası).
                        if cfg.scalp_enabled
                            && stats.scalp.total_closed >= 20
                            && stats.scalp.win_rate() < 0.30
                        {
                            cfg.scalp_enabled = false;
                            summary.push(format!(
                                "SCP Auto-Disabled (wr={:.2}, n={})",
                                stats.scalp.win_rate(), stats.scalp.total_closed,
                            ));
                        }
                        if cfg.swing_enabled
                            && stats.swing.total_closed >= 20
                            && stats.swing.win_rate() < 0.30
                        {
                            cfg.swing_enabled = false;
                            summary.push(format!(
                                "SWG Auto-Disabled (wr={:.2}, n={})",
                                stats.swing.win_rate(), stats.swing.total_closed,
                            ));
                        }
                        if !summary.is_empty() {
                            if let Ok(st) = state.lock() {
                                if let Ok(mut w) = st.brain.scalp_swing_config.write() {
                                    *w = cfg;
                                }
                            }
                        }
                    }
                }

                // 3) Log (yine kısa scope).
                if !summary.is_empty() {
                    push_state_log(&state, format!(
                        "🎚️ ScalpSwing tuner: {} ayar uygulandı → {}",
                        summary.len(), summary.join(", "),
                    ));
                }

                tokio::time::sleep(std::time::Duration::from_secs(every_secs)).await;
            }
        });
    }

    /// Phase C processor: olgunlaşmış trailing observation'ları evalue edip
    /// ParameterStore'a feedback yansıtır. Her 10sn'de bir tarama yapar — ne çok
    /// agresif (kuyruk hızla büyür mü kontrol), ne de çok az (60sn olgunluk için yeter).
    pub(crate) fn spawn_trail_feedback_processor(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            const MATURE_SECS: u64 = 60;
            const POLL_SECS:   u64 = 10;
            const STALE_SECS:  u64 = 300; // 5dk: live_price yoksa gözlem düşer

            loop {
                let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                // Olgun + stale ayrımı yapıp tek scope'lu queue manipülasyonu.
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let mature: Vec<crate::robot::parameters::PendingTrailObservation> = {
                    if let Ok(mut q) = trail_obs_queue().lock() {
                        let mut keep = std::collections::VecDeque::with_capacity(q.len());
                        let mut take = Vec::new();
                        while let Some(o) = q.pop_front() {
                            if now.saturating_sub(o.exit_epoch) >= STALE_SECS {
                                continue; // 5dk üstü → düşür
                            }
                            if o.is_mature(MATURE_SECS) { take.push(o); }
                            else { keep.push_back(o); }
                        }
                        *q = keep;
                        take
                    } else { Vec::new() }
                };

                // Evalue + record — tek state.lock altında batch.
                if !mature.is_empty() {
                    if let Ok(st) = state.lock() {
                        let live_price = st.fleet.live_price.read().ok().map(|g| g.clone()).unwrap_or_default();
                        if let Ok(mut params) = st.brain.parameters.write() {
                            for obs in &mature {
                                let cur = match live_price.get(&obs.symbol).copied() {
                                    Some(v) if v > 0.0 => v,
                                    _ => continue, // fiyat yok → atla (kuyruktan zaten alındı, drop edilir)
                                };
                                let was_early = obs.evaluate(cur);
                                let changed = params.record_trailing_outcome(&obs.symbol, &obs.strategy, was_early);
                                if let Some(new_target) = changed {
                                    log::info!(
                                        "🎯 Trail feedback patch: {} ({}) → target={:.2}%",
                                        obs.symbol, obs.strategy, new_target,
                                    );
                                }
                            }
                        }
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
            }
        });
    }

    /// 📐 Periyodik S/R updater — aktif sembol setini gezer, son 200 candle üzerinden
    /// `SrDetector::detect` çağırıp `fleet.live_sr_zones` HashMap'ini günceller.
    ///
    /// Aktif sembol seti: `config.symbol` + `config.pinned_symbols` + orchestrator
    /// worker'ları (yinelemeler atılır). DB'de yeterli candle yoksa sembol atlanır.
    /// İlk turda warmup yok — bot ilk açıldığında TUI hemen dolu görünür.
    pub(crate) fn spawn_sr_updater(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            if std::env::var("SR_UPDATER_DISABLE").ok().as_deref() == Some("1") {
                push_state_log(&state, "📐 SR updater: SR_UPDATER_DISABLE=1, task pasif".into());
                return;
            }
            // Faz 2: interval ParameterStore'dan okunur. SR_UPDATE_EVERY_SECS env'i
            // store.from_env'de boot anında zaten alındı; runtime'da brain.parameters
            // güncellenirse bu task da sonraki turda yeni aralığı görür.
            let interval_secs: u64 = state.lock().ok()
                .and_then(|st| st.brain.parameters.read().ok().map(|p| p.sr_update_every_secs))
                .unwrap_or(30);
            let detector = crate::robot::sr_detector::SrDetector::new(
                crate::robot::sr_detector::SrDetectorConfig::default()
            );
            let mut first_run_logged = false;

            loop {
                let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                // 1) Aktif sembolleri topla. Canlı feed'i olmayan borsa sembolleri
                // (örn. eski BIST mumları DB'de durur) SR/Market Gözetimi'ne girmesin →
                // market-agnostik tek nokta: RuntimeTuning.symbol_eligible_for_live.
                let (db_path, interval, symbols) = {
                    let st = match state.lock() { Ok(s) => s, Err(_) => break };
                    let tuning = Arc::clone(&st.tuning);
                    let eligible = |s: &str| tuning.symbol_eligible_for_live(s);

                    let mut symbols: Vec<String> = vec![];
                    if eligible(&st.config.symbol)
                        && !st.config.symbol.is_empty()
                        && !symbols.contains(&st.config.symbol)
                    {
                        symbols.push(st.config.symbol.clone());
                    }
                    for s in &st.config.pinned_symbols {
                        if !eligible(s) { continue; }
                        if !symbols.contains(s) { symbols.push(s.clone()); }
                    }
                    if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                        for w in orch.get_worker_status() {
                            if !eligible(&w.symbol) { continue; }
                            if !symbols.contains(&w.symbol) { symbols.push(w.symbol); }
                        }
                    }
                    (st.config.db_path.clone(), st.config.interval.clone(), symbols)
                };

                // 2) Her sembol için candles oku, SR detect — IO/CPU lock dışında yapılır.
                let mut zones_map: std::collections::HashMap<String, Vec<crate::robot::sr_detector::SrZone>>
                    = Default::default();
                let mut total_zones = 0usize;
                for sym in &symbols {
                    if let Ok(candles) = crate::persistence::reader::read_candles(&db_path, sym, &interval, 200) {
                        // Detect lookback=5 default; en az ~11 candle gerekir, güvenli alt sınır 20.
                        if candles.len() >= 20 {
                            let zones = detector.detect(&candles);
                            if !zones.is_empty() {
                                total_zones += zones.len();
                                zones_map.insert(sym.clone(), zones);
                            }
                        }
                    }
                }

                // 3) Yaz — kısa scope'lu write lock.
                if let Ok(st) = state.lock() {
                    if let Ok(mut guard) = st.fleet.live_sr_zones.write() {
                        *guard = zones_map;
                    }
                }

                if !first_run_logged {
                    push_state_log(&state, format!(
                        "📐 SR updater: {} sembol, {} bölge, her {}sn",
                        symbols.len(), total_zones, interval_secs,
                    ));
                    first_run_logged = true;
                }

                sleep(Duration::from_secs(interval_secs)).await;
            }
        });
    }

    /// 💰 Periyodik hesap bakiye senkronu — Live mode için.
    ///
    /// İki katmanlı karar:
    ///   - Mismatch %1+ tek seferlik gözlem → ⚠️ uyarı (henüz onarım yok)
    ///   - Mismatch N kez (default 3) ardışık → 🩹 otomatik onarım (equity = borsa)
    /// Eşik altına döner dönmez sayaç sıfırlanır.
    pub(crate) fn spawn_balance_sync(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            let interval_secs: u64 = std::env::var("BALANCE_SYNC_EVERY_SECS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(300);
            let mismatch_pct_threshold: f64 = std::env::var("BALANCE_MISMATCH_PCT")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
            // Otomatik onarım için ardışık gözlem eşiği. 0 → autofix kapalı.
            let autofix_after_n: u32 = std::env::var("BALANCE_AUTOFIX_AFTER_N_OBS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(3);
            let autofix_enabled: bool = std::env::var("BALANCE_AUTOFIX_ENABLED")
                .map(|v| v != "false" && v != "0").unwrap_or(true);

            // Sadece Live + non-dry-run modunda çalış
            let (executor, dry_run) = {
                let st = match state.lock() { Ok(s) => s, Err(_) => return };
                (st.live_executor.clone(), st.live_dry_run)
            };
            let executor = match executor {
                Some(e) if !dry_run => e,
                _ => {
                    push_state_log(&state, "💰 Balance sync: Paper/DryRun mod, task pasif".into());
                    return;
                }
            };

            // İlk turda 30 sn warmup (boot anomalilerinden kaçınmak için)
            sleep(Duration::from_secs(30)).await;

            let mut consecutive_mismatch: u32 = 0;

            loop {
                let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                match executor.get_balance().await {
                    Ok(exchange_balance) => {
                        let local_equity = state.lock().map(|s| s.finance.equity).unwrap_or(0.0);
                        let diff = (exchange_balance - local_equity).abs();
                        let pct = if local_equity.abs() > f64::EPSILON {
                            (diff / local_equity) * 100.0
                        } else { 0.0 };

                        if pct > mismatch_pct_threshold {
                            // Eşik aşıldı → mismatch sayacı bir artar
                            consecutive_mismatch = consecutive_mismatch.saturating_add(1);

                            // Önce uyarı log'u + Telegram (BALANCE-MISMATCH key ile throttle)
                            if let Ok(mut st) = state.lock() {
                                st.push_alert(
                                    "BALANCE-MISMATCH",
                                    crate::robot::infra::telegram_notifier::Severity::Warning,
                                    format!(
                                        "[BALANCE-MISMATCH] borsa=${:.2} local=${:.2} fark=${:.2} ({:.2}%) > {:.2}% (gözlem #{} / {})",
                                        exchange_balance, local_equity, diff, pct, mismatch_pct_threshold,
                                        consecutive_mismatch, autofix_after_n,
                                    ),
                                );
                                st.guardian.repair_log.push_back(format!(
                                    "[{}] mismatch obs#{}: exchange=${:.2} local=${:.2} ({:.2}%)",
                                    chrono::Local::now().format("%H:%M:%S"),
                                    consecutive_mismatch, exchange_balance, local_equity, pct,
                                ));
                                while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
                            }

                            // Autofix tetikleyici: N ardışık gözlem
                            if autofix_enabled && autofix_after_n > 0 && consecutive_mismatch >= autofix_after_n {
                                // Otomatik onarım: local equity'yi borsaya hizala
                                if let Ok(mut st) = state.lock() {
                                    let old_equity = st.finance.equity;
                                    let delta = exchange_balance - old_equity;
                                    st.finance.equity = exchange_balance;
                                    // peak_equity revize: yeni equity peak'in üzerindeyse güncelle
                                    if exchange_balance > st.finance.peak_equity {
                                        st.finance.peak_equity = exchange_balance;
                                    }
                                    st.push_alert(
                                        "BALANCE-AUTOFIX",
                                        crate::robot::infra::telegram_notifier::Severity::Critical,
                                        format!(
                                            "[BALANCE-AUTOFIX] {} ardışık mismatch sonrası onarım: ${:.2} → ${:.2} (Δ={:+.2})",
                                            consecutive_mismatch, old_equity, exchange_balance, delta,
                                        ),
                                    );
                                    st.guardian.repair_log.push_back(format!(
                                        "[{}] AUTOFIX: equity ${:.2} → ${:.2} (Δ={:+.2})",
                                        chrono::Local::now().format("%H:%M:%S"),
                                        old_equity, exchange_balance, delta,
                                    ));
                                    while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
                                }
                                consecutive_mismatch = 0; // sayaç reset
                            }
                        } else {
                            // Eşik altına düştü → sayacı toparla
                            if consecutive_mismatch > 0 {
                                push_state_log(&state, format!(
                                    "💰 [BALANCE-SYNC] mismatch toparlandı (sayaç sıfırlandı): borsa=${:.2} ≈ local=${:.2}",
                                    exchange_balance, local_equity,
                                ));
                            } else if let Ok(mut st) = state.lock() {
                                st.push_log(format!(
                                    "💰 [BALANCE-SYNC] borsa=${:.2} ≈ local=${:.2} (fark {:.2}%, eşik altı)",
                                    exchange_balance, local_equity, pct,
                                ));
                            }
                            consecutive_mismatch = 0;
                        }
                    }
                    Err(e) => {
                        push_state_log(&state, format!("⚠️ [BALANCE-SYNC] get_balance hatası: {:?}", e));
                    }
                }

                sleep(Duration::from_secs(interval_secs)).await;
            }
        });
    }

    /// 🛰️ WebSocket userDataStream task'ı — Live mode fill event'leri için.
    pub(crate) fn spawn_user_data_stream(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            use futures::StreamExt;
            use tokio_tungstenite::{connect_async, tungstenite::Message};

            // Sadece Live + non-dry-run modunda çalış
            let (executor, dry_run) = {
                let st = match state.lock() { Ok(s) => s, Err(_) => return };
                (st.live_executor.clone(), st.live_dry_run)
            };
            let executor = match executor {
                Some(e) if !dry_run => e,
                _ => {
                    push_state_log(&state, "🛰️ WS userDataStream: Paper/DryRun mod, task pasif".into());
                    return;
                }
            };

            // Reconnect döngüsü
            let mut backoff_secs: u64 = 5;
            loop {
                let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                // 1. listenKey al
                let listen_key = match executor.create_listen_key().await {
                    Ok(k) => k,
                    Err(e) => {
                        push_state_log(&state, format!(
                            "🛰️ WS listenKey hatası: {:?} (backoff={}s)", e, backoff_secs,
                        ));
                        sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(60);
                        continue;
                    }
                };
                let ws_url = executor.user_data_stream_url(&listen_key);
                push_state_log(&state, format!("🛰️ WS userDataStream bağlanıyor: {}", ws_url));

                // 2. WS bağlan
                let (ws_stream, _) = match connect_async(&ws_url).await {
                    Ok(p) => p,
                    Err(e) => {
                        if let Ok(mut st) = state.lock() {
                            st.push_alert(
                                "WS-CONNECT-FAIL",
                                crate::robot::infra::telegram_notifier::Severity::Warning,
                                format!(
                                    "[WS-CONNECT-FAIL] userDataStream bağlanılamadı: {:?} (backoff={}s)",
                                    e, backoff_secs,
                                ),
                            );
                        }
                        sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(60);
                        continue;
                    }
                };
                push_state_log(&state, "🛰️ WS userDataStream bağlı ✓ — fill event'leri dinleniyor".into());
                backoff_secs = 5; // başarılı bağlantı, backoff reset

                // 3. Keepalive timer (30 dk'da bir listenKey yenile)
                let ka_exec = Arc::clone(&executor);
                let ka_state = Arc::clone(&state);
                let ka_key = listen_key.clone();
                let keepalive_handle = tokio::spawn(async move {
                    loop {
                        sleep(Duration::from_secs(30 * 60)).await;
                        let stop = ka_state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                        if stop { break; }
                        if let Err(e) = ka_exec.keepalive_listen_key(&ka_key).await {
                            if let Ok(mut st) = ka_state.lock() {
                                st.push_log(format!("🛰️ WS keepalive hatası: {:?}", e));
                            }
                            break;
                        }
                    }
                });

                // 4. Mesaj döngüsü
                let (_write, mut read) = ws_stream.split();
                while let Some(msg) = read.next().await {
                    let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                    if stop { break; }
                    match msg {
                        Ok(Message::Text(txt)) => {
                            Self::handle_user_data_event(&state, &txt).await;
                        }
                        Ok(Message::Ping(p)) => { let _ = p; /* yanıt tungstenite tarafında otomatik */ }
                        Ok(Message::Close(_)) => {
                            push_state_log(&state, "🛰️ WS sunucu Close gönderdi — yeniden bağlanılacak".into());
                            break;
                        }
                        Err(e) => {
                            push_state_log(&state, format!("🛰️ WS okuma hatası: {:?} — reconnect", e));
                            break;
                        }
                        _ => {}
                    }
                }

                keepalive_handle.abort();
                sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(60);
            }
        });
    }
}
