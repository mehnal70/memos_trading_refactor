// src/robot/engines/master.rs - Master Engine Otonom İnfaz Merkezi
// Srivastava ATP - İşlevsel Çarklar Odası (Unified Master Engine - Final Safe Compilation)

use crate::prelude::*;
use super::base::{EngineConfig, TradingEngine};
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use tokio::time::{sleep, Duration};

// Projedeki gerçek tiplerin ve trait'lerin bağlanması için ön hazırlık (Agnostik Katman)
pub struct MLModel;
pub struct Monitor;
pub trait MarketRegimeDetector {}
pub trait StrategyLifecycleManager {}

pub struct Engine {
    pub config: EngineConfig,
    pub ml_model: Option<MLModel>,
    pub monitor: Option<Monitor>,
    pub last_cycle_at: std::time::Instant,
    pub regime_detector: Box<dyn MarketRegimeDetector + Send>,
    pub strategy_manager: Box<dyn StrategyLifecycleManager + Send>,
}

/// Bir pozisyonun kapanış sebebi — `ClosedTradeModel.exit_reason` string'i bu enum'dan üretilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    StopLoss,
    TakeProfit,
    TrailingStop,
    Breakeven,
    StrategySignal,
}

impl ExitReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ExitReason::StopLoss        => "STOP_LOSS",
            ExitReason::TakeProfit      => "TAKE_PROFIT",
            ExitReason::TrailingStop    => "TRAILING_STOP",
            ExitReason::Breakeven       => "BREAKEVEN",
            ExitReason::StrategySignal  => "STRATEGY_SIGNAL",
        }
    }
    pub fn emoji(self) -> &'static str {
        match self {
            ExitReason::StopLoss        => "🔻",
            ExitReason::TakeProfit      => "🎯",
            ExitReason::TrailingStop    => "🪤",
            ExitReason::Breakeven       => "⚖️",
            ExitReason::StrategySignal  => "🏁",
        }
    }
}

impl Engine {
    /// 🚀 ANA OTONOM DÖNGÜ (Engine Garnizonu Girişi)
    pub async fn run_autonomous_loop(state: Arc<Mutex<AppState>>) {
        log::info!("🚀 Master Engine Ateşlendi. Otonom devriye başlatıldı.");
        // Engine ateşlendi mesajını TUI log paneline de düşür + Booting fazı
        if let Ok(mut st) = state.lock() {
            st.fleet.phase = "Booting".into();
            st.push_log("🚀 Master Engine ateşlendi. Otonom devriye başladı.".into());
            // Pipeline timeline'ında 7 kanonik fazı baştan Idle olarak göster —
            // ilk cycle henüz çalışmadan bile TUI'de doğru sıralı görünür.
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                pipe.init_canon_stages();
            }
        }

        // 1. INFRASTRUCTURE FLEET (WS, Diagnostic, Pipeline)
        Self::spawn_infrastructure_fleet(Arc::clone(&state)).await;

        // Ana döngü heartbeat'i (TUI log paneline periyodik canlılık mesajı, her 30 sn'de bir)
        let mut tick_count: u64 = 0;
        loop {
            // Çıkış kontrolü + heartbeat tick mühürlemesi (her tur)
            let is_stop = {
                let mut st = state.lock().unwrap();
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs()).unwrap_or(0);
                st.fleet.last_loop_tick.store(now, Ordering::Relaxed);
                // Her turun başında fazı taze "Scanning"e çevir (execute_trade_cycle ve
                // perform_anomaly_recovery aksiyon yaparsa kendi içinde Executing/Recovering yazar).
                st.fleet.phase = "Scanning".into();
                st.app_stop_signal.load(Ordering::Relaxed)
            };
            if is_stop { break; }
            tick_count += 1;

            // Snapshot üretimi
            let snap = {
                let st = state.lock().unwrap();
                crate::core::bridge::get_snapshot(&st)
            };

            // 2. İNFAZ DÖNGÜSÜ (ML + Q-Table + Risk)
            Self::execute_trade_cycle(&state, &snap).await;

            // 3. SAVUNMA (Anomali Onarımı) — aktif anomali varsa phase = Recovering
            Self::perform_anomaly_recovery(&state, &snap);

            // 4a. Equity tarihçesi: her 5 turda bir (≈2.5 sn) push edilir; sparkline ve drawdown.
            if tick_count % 5 == 0 {
                if let Ok(mut st) = state.lock() {
                    let equity = st.finance.equity;
                    if equity > st.finance.peak_equity { st.finance.peak_equity = equity; }
                    if let Ok(mut hist) = st.finance.equity_history.write() {
                        hist.push_back(equity);
                        while hist.len() > 120 { hist.pop_front(); }
                    }
                }
            }

            // 4b. IntelligenceHub güncellemesi: her 20 turda (≈10 sn)
            //  - FeatureVector çıkar → DriftDetector.update
            //  - should_retrain ise ml trigger pulse'u
            //  - tick_evolution (controller içinde N cycle'da bir gerçek evrim)
            if tick_count % 20 == 0 {
                Self::tick_intelligence_hub(&state).await;
            }

            // 4. Periyodik canlılık logu: her ~30 sn'de bir TUI log paneline kalp atışı.
            // (500 ms × 60 tur = 30 s). İlk turu da yakala: tick_count == 1.
            if tick_count == 1 || tick_count % 60 == 0 {
                if let Ok(mut st) = state.lock() {
                    let n_open = st.finance.live_positions.read().map(|p| p.len()).unwrap_or(0);
                    let n_closed = st.finance.live_closed_trades.read().map(|t| t.len()).unwrap_or(0);
                    let n_anom = st.guardian.live_pipeline.read().map(|p| p.anomalies.len()).unwrap_or(0);
                    let equity = st.finance.equity;
                    st.push_log(format!(
                        "💓 Devriye #{} | Equity: {:.2} | Açık: {} | Kapalı: {} | Anomali: {}",
                        tick_count, equity, n_open, n_closed, n_anom,
                    ));
                }
            }

            sleep(Duration::from_millis(500)).await;
        }

        if let Ok(mut st) = state.lock() {
            st.fleet.phase = "Stopped".into();
            st.push_log("🛑 Master Engine devriyesi durduruldu.".into());
        }
    }

    /// 🛠️ INFRASTRUCTURE FLEET: Global servisleri non-blocking olarak yönetir.
    async fn spawn_infrastructure_fleet(state: Arc<Mutex<AppState>>) {
        log::info!("⚡ Srivastava Altyapı Filosu sevk ediliyor...");
        if let Ok(mut st) = state.lock() {
            st.push_log("⚡ Altyapı filosu sevk edildi: snapshot(5s) · heartbeat-file(60s) · heartbeat(1s) · phase(2s) · price-poll(5s) · trigger(250ms) · scheduler(60s) · psync(30s) · ws-user-data · balance-sync(5dk)".into());
        }

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

            loop {
                let (symbols, interval, stop) = {
                    let st = match st_px.lock() { Ok(s) => s, Err(_) => break };
                    if st.app_stop_signal.load(Ordering::Relaxed) {
                        (vec![], String::new(), true)
                    } else {
                        let mut syms: Vec<String> = vec![st.config.symbol.clone()];
                        if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                            for w in orch.get_worker_status() {
                                if !syms.contains(&w.symbol) { syms.push(w.symbol); }
                            }
                        }
                        (syms, st.config.interval.clone(), false)
                    }
                };
                if stop { break; }

                let mut new_prices: Vec<(String, f64)> = Vec::with_capacity(symbols.len());
                let mut errors: Vec<(String, String)> = Vec::new();
                for sym in &symbols {
                    if sym.is_empty() { continue; }
                    match fetcher.fetch_latest(sym, &interval, 1).await {
                        Ok(candles) => {
                            if let Some(last) = candles.last() {
                                if last.close > 0.0 { new_prices.push((sym.clone(), last.close)); }
                            }
                        }
                        Err(e) => errors.push((sym.clone(), e)),
                    }
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
                    && now_secs.saturating_sub(last_error_summary_at) >= 30 {
                    last_error_summary_at = now_secs;
                    Some(format!(
                        "⚠️ Price-poll: {}/{} sembolde hata. Örn: {}",
                        errors.len(), symbols.len(),
                        errors.first().map(|(s, e)| format!("{}: {}", s, e)).unwrap_or_default(),
                    ))
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
                        let status = if errors.is_empty() { StepStatus::Done } else { StepStatus::Failed };
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
                                    let st_for_job = Arc::clone(&state_clone);
                                    let out = tokio::task::spawn_blocking(move || {
                                        Self::run_ml_retrain_job(&st_for_job)
                                    }).await;
                                    match out {
                                        Ok(Ok(())) => {}
                                        Ok(Err(e)) => {
                                            log::warn!("🧠 ML retrain başarısız: {}", e);
                                            if let Ok(mut st) = state_clone.lock() {
                                                st.push_log(format!("❌ ML Retrain başarısız: {}", e));
                                            }
                                            final_status = StepStatus::Failed;
                                        }
                                        Err(e) => {
                                            log::warn!("🧠 ML retrain join hatası: {}", e);
                                            final_status = StepStatus::Failed;
                                        }
                                    }
                                },
                                "backtest" => {
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
            let mut warmup_done = false;

            sleep(Duration::from_secs(WARMUP_SECS)).await; // boot warmup

            loop {
                let stop = st_sched.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                // Config okumayı kısa kilit altında yap
                let (dl_enabled, dl_period, bt_enabled, bt_period) = {
                    let st = match st_sched.lock() { Ok(s) => s, Err(_) => break };
                    (st.config.download_enabled, st.config.download_every_mins,
                     st.config.pipeline_enabled, st.config.pipeline_every_mins)
                };

                let now = std::time::Instant::now();

                // İlk warmup turu: download_enabled ise hemen bir kerelik tetik bas
                // ki kullanıcı TUI'ye baktığında veri akışı görünür olsun.
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
    }

    /// 📐 Periyodik S/R updater — aktif sembol setini gezer, son 200 candle üzerinden
    /// `SrDetector::detect` çağırıp `fleet.live_sr_zones` HashMap'ini günceller.
    ///
    /// Aktif sembol seti: `config.symbol` + `config.pinned_symbols` + orchestrator
    /// worker'ları (yinelemeler atılır). DB'de yeterli candle yoksa sembol atlanır.
    /// İlk turda warmup yok — bot ilk açıldığında TUI hemen dolu görünür.
    fn spawn_sr_updater(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            if std::env::var("SR_UPDATER_DISABLE").ok().as_deref() == Some("1") {
                if let Ok(mut st) = state.lock() {
                    st.push_log("📐 SR updater: SR_UPDATER_DISABLE=1, task pasif".into());
                }
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

                // 1) Aktif sembolleri topla.
                let (db_path, interval, symbols) = {
                    let st = match state.lock() { Ok(s) => s, Err(_) => break };
                    let mut symbols: Vec<String> = vec![];
                    if !st.config.symbol.is_empty() && !symbols.contains(&st.config.symbol) {
                        symbols.push(st.config.symbol.clone());
                    }
                    for s in &st.config.pinned_symbols {
                        if !symbols.contains(s) { symbols.push(s.clone()); }
                    }
                    if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                        for w in orch.get_worker_status() {
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
                    if let Ok(mut st) = state.lock() {
                        st.push_log(format!(
                            "📐 SR updater: {} sembol, {} bölge, her {}sn",
                            symbols.len(), total_zones, interval_secs,
                        ));
                    }
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
    fn spawn_balance_sync(state: Arc<Mutex<AppState>>) {
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
                    if let Ok(mut st) = state.lock() {
                        st.push_log("💰 Balance sync: Paper/DryRun mod, task pasif".into());
                    }
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
                                if let Ok(mut st) = state.lock() {
                                    st.push_log(format!(
                                        "💰 [BALANCE-SYNC] mismatch toparlandı (sayaç sıfırlandı): borsa=${:.2} ≈ local=${:.2}",
                                        exchange_balance, local_equity,
                                    ));
                                }
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
                        if let Ok(mut st) = state.lock() {
                            st.push_log(format!("⚠️ [BALANCE-SYNC] get_balance hatası: {:?}", e));
                        }
                    }
                }

                sleep(Duration::from_secs(interval_secs)).await;
            }
        });
    }

    /// 🛰️ WebSocket userDataStream task'ı — Live mode fill event'leri için.
    fn spawn_user_data_stream(state: Arc<Mutex<AppState>>) {
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
                    if let Ok(mut st) = state.lock() {
                        st.push_log("🛰️ WS userDataStream: Paper/DryRun mod, task pasif".into());
                    }
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
                        if let Ok(mut st) = state.lock() {
                            st.push_log(format!(
                                "🛰️ WS listenKey hatası: {:?} (backoff={}s)", e, backoff_secs,
                            ));
                        }
                        sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(60);
                        continue;
                    }
                };
                let ws_url = executor.user_data_stream_url(&listen_key);
                if let Ok(mut st) = state.lock() {
                    st.push_log(format!("🛰️ WS userDataStream bağlanıyor: {}", ws_url));
                }

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
                if let Ok(mut st) = state.lock() {
                    st.push_log("🛰️ WS userDataStream bağlı ✓ — fill event'leri dinleniyor".into());
                }
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
                            if let Ok(mut st) = state.lock() {
                                st.push_log("🛰️ WS sunucu Close gönderdi — yeniden bağlanılacak".into());
                            }
                            break;
                        }
                        Err(e) => {
                            if let Ok(mut st) = state.lock() {
                                st.push_log(format!("🛰️ WS okuma hatası: {:?} — reconnect", e));
                            }
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

    /// userDataStream'den gelen JSON event'i parse et: FILLED, PARTIALLY_FILLED,
    /// REJECTED, EXPIRED durumlarını ayrıştırıp ilgili işleyiciyi çağırır.
    /// NEW ve CANCELED sessizce yutulur (normal yaşam döngüsü).
    /// `pub` çünkü entegrasyon testlerinde gerçek JSON'la uçtan uca doğrulanır.
    pub async fn handle_user_data_event(state: &Arc<Mutex<AppState>>, raw: &str) {
        let v: serde_json::Value = match serde_json::from_str(raw) {
            Ok(v) => v, Err(_) => return,
        };

        // Spot: executionReport     → X=status, s=symbol, q=orig_qty, z=cum_qty, l=last_qty,
        //                             S=side, i=orderId, r=rejection reason ("NONE" yok ise)
        // Futures: ORDER_TRADE_UPDATE → o.X=status, o.s=symbol, o.q=orig_qty, o.z=cum_qty,
        //                             o.l=last_qty, o.S=side, o.i=orderId
        let event_type = v.get("e").and_then(|x| x.as_str()).unwrap_or("").to_owned();
        let parse_f = |o: &serde_json::Value, k: &str| -> f64 {
            o.get(k).and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0)
        };
        let parse_s = |o: &serde_json::Value, k: &str| -> String {
            o.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned()
        };
        let parse_id = |o: &serde_json::Value, k: &str| -> String {
            // orderId tipik olarak sayı; bazen string de gelir. İkisini de yakalayalım.
            o.get(k).map(|x| match x {
                serde_json::Value::String(s) => s.clone(),
                _ => x.to_string(),
            }).unwrap_or_default()
        };
        // L = bu event'te dolan kısmın ortalama fiyatı (last_filled_price).
        let (status, symbol, orig_qty, cum_qty, last_qty, last_price, side, order_id, reject_reason) =
            match event_type.as_str() {
                "executionReport" => (
                    parse_s(&v, "X"), parse_s(&v, "s"),
                    parse_f(&v, "q"), parse_f(&v, "z"), parse_f(&v, "l"), parse_f(&v, "L"),
                    parse_s(&v, "S"), parse_id(&v, "i"), parse_s(&v, "r"),
                ),
                "ORDER_TRADE_UPDATE" => {
                    let o = v.get("o").cloned().unwrap_or_default();
                    (
                        parse_s(&o, "X"), parse_s(&o, "s"),
                        parse_f(&o, "q"), parse_f(&o, "z"), parse_f(&o, "l"), parse_f(&o, "L"),
                        parse_s(&o, "S"), parse_id(&o, "i"), parse_s(&o, "r"),
                    )
                }
                _ => return, // diğer event'ler ignored (account update vb.)
            };

        match status.as_str() {
            "FILLED" => Self::process_user_fill_status(state, &status, &symbol).await,
            "PARTIALLY_FILLED" =>
                Self::process_partial_fill(
                    state, &symbol, &side, orig_qty, cum_qty, last_qty, last_price,
                ).await,
            "REJECTED" | "EXPIRED" =>
                Self::process_order_anomaly(
                    state, &status, &symbol, &side, &order_id, orig_qty, &reject_reason,
                ).await,
            _ => {} // NEW, CANCELED, TRADE → sessiz (normal yaşam döngüsü)
        }
    }

    /// 🛑 REJECTED / EXPIRED — emir borsada açılamadı/iptal oldu.
    /// Sebep çoğunlukla LOT_SIZE / MIN_NOTIONAL / INSUFFICIENT_BALANCE / GTX-as-taker.
    /// `apply_filters` ön kontrolüyle önlenmesi gerekiyordu; yine de düşerse hem
    /// push_log hem repair_log'a yazılır (kullanıcı görsün ve operatör doğrulasın).
    async fn process_order_anomaly(
        state: &Arc<Mutex<AppState>>,
        status: &str,
        symbol: &str,
        side: &str,
        order_id: &str,
        orig_qty: f64,
        reject_reason: &str,
    ) {
        if symbol.is_empty() { return; }
        // Spot'ta `r="NONE"` gelirse sebep yok demektir. Boş veya NONE olan değeri gizle.
        let reason_part = if reject_reason.is_empty() || reject_reason == "NONE" {
            String::new()
        } else {
            format!(" · sebep={}", reject_reason)
        };
        let side_part = if side.is_empty() { String::new() } else { format!(" {}", side) };
        let id_part = if order_id.is_empty() { String::new() } else { format!(" order={}", order_id) };

        if let Ok(mut st) = state.lock() {
            // Telegram: REJECTED → Critical, EXPIRED → Warning. Throttle key sembol+status.
            let severity = if status == "REJECTED" {
                crate::robot::infra::telegram_notifier::Severity::Critical
            } else {
                crate::robot::infra::telegram_notifier::Severity::Warning
            };
            let key = format!("WS-{}-{}", status, symbol);
            st.push_alert(
                &key,
                severity,
                format!(
                    "[WS-{}] {}{} qty={:.4}{}{}",
                    status, symbol, side_part, orig_qty, id_part, reason_part,
                ),
            );
            st.guardian.repair_log.push_back(format!(
                "[{}] {}: {}{} qty={:.4}{}{}",
                chrono::Local::now().format("%H:%M:%S"),
                status, symbol, side_part, orig_qty, id_part, reason_part,
            ));
            while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
        }
    }

    /// 🌓 PARTIAL fill — emirin bir bölümü dolu. İki tür var:
    ///
    /// **ENTRY partial** (side pozisyonun yönüyle aynı: LONG için BUY, SHORT için SELL):
    ///   - Local qty `cum_qty`'e hizalanır (gerçekte bu kadar tutuyoruz).
    ///   - Sadece komisyon equity'den düşülür; realize PnL yok.
    ///
    /// **CLOSE partial** (side pozisyonu kapatıyor: LONG için SELL, SHORT için BUY):
    ///   - Local qty bu event'te kapanan kadar (`last_qty`) azalır.
    ///   - Realize PnL = (last_price − entry_price) × last_qty × yön; equity'e işlenir.
    ///   - Komisyon ayrıca düşülür; live_execution_costs.commission_usd büyür.
    ///
    /// `pub` çünkü entegrasyon testleri (partial fill PnL muhasebesi) bunu doğrular.
    pub async fn process_partial_fill(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        side: &str,
        orig_qty: f64,
        cum_qty: f64,
        last_qty: f64,
        last_price: f64,
    ) {
        if symbol.is_empty() || orig_qty <= 0.0 || last_qty <= 0.0 { return; }
        let fill_pct = (cum_qty / orig_qty * 100.0).clamp(0.0, 100.0);
        const COMMISSION_RATE: f64 = 0.001; // 0.1% — open/close ile aynı

        // 1. Pozisyonu oku, entry vs close sınıflandır.
        let (is_long, entry_price, current_price, local_qty_before) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let positions = match st.finance.live_positions.read() { Ok(p) => p, Err(_) => return };
            match positions.get(symbol) {
                Some(pos) => (pos.is_long, pos.entry_price, pos.current_price, pos.qty),
                None => return, // bot bilmediği bir sembol için event aldı
            }
        };
        let is_closing = (is_long && side == "SELL") || (!is_long && side == "BUY");

        // 2. Fiyat 0 ise (executor pratikte 0 dönmez ama defensive guard) realize PnL
        //    hesaplayamayız; o zaman sadece qty güncelle ve log at — komisyon da
        //    notional 0 olur. Çağıran zaten WS payload'unu doğrudan veriyor.
        let trade_notional = last_qty * last_price;
        let commission = trade_notional * COMMISSION_RATE;
        let realized_pnl = if is_closing && last_price > 0.0 {
            crate::core::math::calculate_pnl(entry_price, last_price, last_qty, is_long)
        } else { 0.0 };

        // 3. State mutation — pozisyon qty + equity + execution_costs.
        let new_qty: Option<f64> = if let Ok(mut st) = state.lock() {
            let mutated = if let Ok(mut positions) = st.finance.live_positions.write() {
                positions.get_mut(symbol).map(|pos| {
                    if is_closing {
                        pos.qty = (pos.qty - last_qty).max(0.0);
                    } else {
                        // Entry partial: cum kadar gerçekten tutuyoruz
                        pos.qty = cum_qty;
                    }
                    pos.qty
                })
            } else { None };

            if mutated.is_some() {
                if let Ok(mut costs) = st.finance.live_execution_costs.write() {
                    costs.commission_usd += commission;
                    costs.total_cost_usd += commission;
                }
                // Realize PnL sadece kapanış partial'inde; ENTRY partial'de equity
                // sadece komisyon kadar azalır (notional henüz realize değil).
                if is_closing {
                    st.finance.equity += realized_pnl - commission;
                } else {
                    st.finance.equity -= commission;
                }
            }
            mutated
        } else { None };

        // 4. Log + audit.
        if let Some(new_q) = new_qty {
            let kind_tag = if is_closing { "CLOSE" } else { "ENTRY" };
            let pnl_part = if is_closing {
                format!(" · pnl=${:+.2}", realized_pnl)
            } else { String::new() };
            if let Ok(mut st) = state.lock() {
                st.push_log(format!(
                    "🌓 [WS-PARTIAL-{}] {} %{:.1} ({} {:.4} @ {:.4}) · qty {:.4} → {:.4}{} · fee=${:.4}",
                    kind_tag, symbol, fill_pct, side, last_qty, last_price,
                    local_qty_before, new_q, pnl_part, commission,
                ));
            }

            // 5. Anomali tespiti → Telegram push_alert.
            //    Üç kriter; her biri farklı throttle anahtarına bağlandı, sembol başına
            //    bağımsız cooldown takip eder (BTCUSDT'nin uyarısı ETHUSDT'yi susturmaz).
            Self::detect_partial_fill_anomalies(
                state, symbol, side, fill_pct, orig_qty, cum_qty,
                last_qty, last_price, local_qty_before, entry_price,
                current_price, is_closing, is_long,
            );
        }
    }

    /// Partial fill anomalilerini değerlendirir; eşik aşıldığında push_alert atar.
    ///
    /// 3 kriter:
    ///   - OVERFILL (Critical): `last_qty > local_qty_before * 1.001`. Borsa local
    ///     pozisyondan fazla doldurmuş → bot ↔ borsa qty ayrışması. equity ve risk
    ///     hesabı bozulur; muhasebe için kritik.
    ///   - CUM_INCONSISTENT (Warning): `cum_qty > orig_qty * 1.001`. Borsa toplam
    ///     fill'i emrin orig_qty'sinden büyük raporladı; payload tutarsızlığı.
    ///   - SLIPPAGE (Warning): adverse fiyat sapması eşiği aştı. Beklenen referans
    ///     CLOSE partial'de pozisyonun `current_price`'ı, ENTRY partial'de
    ///     `entry_price`. Eşik env `PARTIAL_FILL_MAX_SLIPPAGE_PCT` (default 1.0%).
    ///     `side` bot tarafından bakılır: BUY → daha pahalıya alındı, SELL → daha
    ///     ucuza satıldı negatif.
    #[allow(clippy::too_many_arguments)]
    fn detect_partial_fill_anomalies(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        side: &str,
        fill_pct: f64,
        orig_qty: f64,
        cum_qty: f64,
        last_qty: f64,
        last_price: f64,
        local_qty_before: f64,
        entry_price: f64,
        current_price: f64,
        is_closing: bool,
        is_long: bool,
    ) {
        use crate::robot::infra::telegram_notifier::Severity;
        // Faz 2: sabit eşikler yerine ParameterStore'dan oku (HyperOpt/manuel update
        // runtime'da değişiklik yapabilsin). Lock alınamazsa legacy default fallback.
        let pf = state.lock().ok()
            .and_then(|st| st.brain.parameters.read().ok().map(|p| p.partial_fill))
            .unwrap_or_default();

        // 1) OVERFILL: borsa pozisyondan fazla doldurmuş (close partial için anlamlı).
        //    Entry partial'de cum henüz local'in üstüne çıkamaz tanım gereği, ama yine
        //    de defensive olarak kontrol ediyoruz.
        if local_qty_before > 0.0
            && last_qty > local_qty_before * (1.0 + pf.overfill_tolerance)
        {
            let key = format!("PARTIAL-ANOMALY-OVERFILL-{}", symbol);
            let msg = format!(
                "[PARTIAL-ANOMALY-OVERFILL] {} side={} last_qty={:.6} > local_qty={:.6} \
                 (cum={:.6}/orig={:.6}) — bot↔borsa qty ayrışması",
                symbol, side, last_qty, local_qty_before, cum_qty, orig_qty,
            );
            if let Ok(mut st) = state.lock() {
                st.push_alert(&key, Severity::Critical, msg);
            }
        }

        // 2) CUM tutarsız: borsa cum'u emrin orig_qty'sinden büyük raporladı.
        if cum_qty > orig_qty * (1.0 + pf.cum_tolerance) {
            let key = format!("PARTIAL-ANOMALY-CUM-{}", symbol);
            let msg = format!(
                "[PARTIAL-ANOMALY-CUM] {} cum={:.6} > orig={:.6} (%{:.1}) — borsa payload tutarsız",
                symbol, cum_qty, orig_qty, fill_pct,
            );
            if let Ok(mut st) = state.lock() {
                st.push_alert(&key, Severity::Warning, msg);
            }
        }

        // 3) SLIPPAGE: bot tarafına göre adverse fiyat sapması.
        if last_price > 0.0 {
            let expected = if is_closing { current_price } else { entry_price };
            if expected > 0.0 {
                let adverse_pct = match side {
                    "BUY"  => (last_price - expected) / expected * 100.0,
                    "SELL" => (expected - last_price) / expected * 100.0,
                    _ => 0.0,
                };
                let threshold_pct = pf.max_slippage_pct;
                if adverse_pct > threshold_pct {
                    let kind = if is_closing { "CLOSE" } else { "ENTRY" };
                    let key = format!("PARTIAL-ANOMALY-SLIPPAGE-{}-{}", kind, symbol);
                    let dir = if is_long { "LONG" } else { "SHORT" };
                    let msg = format!(
                        "[PARTIAL-ANOMALY-SLIPPAGE] {} {} {} side={} fill@{:.6} \
                         vs beklenen {:.6} → adverse %{:.3} (eşik %{:.2})",
                        symbol, dir, kind, side, last_price, expected, adverse_pct, threshold_pct,
                    );
                    if let Ok(mut st) = state.lock() {
                        st.push_alert(&key, Severity::Warning, msg);
                    }
                }
            }
        }
    }

    /// FILLED event'inin tek/uniform işleyicisi (spot + futures için ortak).
    async fn process_user_fill_status(state: &Arc<Mutex<AppState>>, status: &str, symbol: &str) {
        if status != "FILLED" || symbol.is_empty() { return; }

        let (executor, db_path, interval, has_local) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let has = st.finance.live_positions.read().map(|p| p.contains_key(symbol)).unwrap_or(false);
            (st.live_executor.clone(), st.config.db_path.clone(),
             st.config.interval.clone(), has)
        };
        if !has_local { return; } // bot bilmediği bir sembol için event aldı

        if let Some(exec) = executor {
            let _ = exec.cancel_all_orders(symbol).await;
            if let Ok(mut st) = state.lock() {
                st.push_log(format!(
                    "🛰️ [WS-FILL] {} FILLED yakalandı → orphan emirler temizlendi, local pozisyon kapatılıyor",
                    symbol,
                ));
            }
        }

        if let Ok(candles) = crate::persistence::reader::read_candles(&db_path, symbol, &interval, 5) {
            if !candles.is_empty() {
                Self::close_paper_position(state, symbol, &candles, ExitReason::TrailingStop).await;
            }
        }
    }

    /// ⚔️ STRATEJİK İNFAZ: Pozisyonların güncel fiyatla PnL'ini günceller ve sinyal avı yapar.
    ///
    /// Akış (Faz 3): live_price → mark-to-market → strateji seçimi (brain.live_strategy)
    /// → edge skoru (signal × ml_confidence) → RiskManager zinciri (Guardrails+Kelly+VaR+Gate)
    /// → aç/kapat kararı.
    async fn execute_trade_cycle(state: &Arc<Mutex<AppState>>, snap: &MissionControl) {
        // 1) Mark-to-market: aktif pozisyonların current_price'ı güncel.
        let (candidates, db_path, interval, live_strategy, ml_confidence) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let price_map = st.fleet.live_price.read().ok().map(|g| g.clone()).unwrap_or_default();
            if let Ok(mut positions) = st.finance.live_positions.write() {
                for pos in positions.values_mut() {
                    if let Some(&live) = price_map.get(&pos.symbol) {
                        if live > 0.0 { pos.current_price = live; }
                    }
                }
            }
            let mut candidates = Vec::new();
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for worker in orch.get_worker_status() {
                    candidates.push(worker.symbol.clone());
                }
            }
            let live_strategy = st.brain.live_strategy.read()
                .map(|s| s.clone()).unwrap_or_else(|_| "MA_CROSSOVER".to_string());
            (candidates, st.config.db_path.clone(), st.config.interval.clone(),
             live_strategy, st.brain.ml_confidence)
        };

        // 2) Paralel sembol infazı — her sembol için ayrı tokio task. State Arc<Mutex> üzerinden
        //    paylaşılır; lock contention'ı kısa tutmak için her closure içinde minimal scope kullanılır.
        //
        // Sıralı→paralel kazanımı: 100 sembol × 5 ms DB read = 500 ms (sıralı) ≈ 30-80 ms (paralel,
        // Tokio multi-thread). State mutex contention 100 sembolde de tolere edilebilir çünkü
        // her sembol için tek kısa açış+kapanış+okuma turu yapılır.
        let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::with_capacity(candidates.len());
        for symbol in candidates {
            let state_clone = Arc::clone(state);
            let db_path_c = db_path.clone();
            let interval_c = interval.clone();
            let live_strategy_c = live_strategy.clone();
            let snap_clone = snap.clone();
            handles.push(tokio::spawn(async move {
                Self::process_symbol_cycle(
                    &state_clone, &symbol, &db_path_c, &interval_c,
                    &live_strategy_c, ml_confidence, &snap_clone,
                ).await;
            }));
        }
        // Tüm sembollerin tamamlanmasını bekle (timeout yok — her biri kısa).
        for h in handles { let _ = h.await; }
    }

    /// Bir sembol için tam tur: exit denetimi → strateji üretimi → edge filtresi →
    /// risk gate → pozisyon aç/kapat. `execute_trade_cycle` her sembol için bu fonksiyonu
    /// paralel spawn eder.
    async fn process_symbol_cycle(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        db_path: &str,
        interval: &str,
        live_strategy: &str,
        ml_confidence: f64,
        snap: &MissionControl,
    ) {
        use crate::robot::data_pipeline::canon::PipelineStage;
        use crate::robot::data_pipeline::StepStatus;
        let risk_manager = crate::robot::risk::RiskManager::new();

        // Tek sembol için iş bloğu — orijinal `for symbol in candidates` gövdesinin içeriği.
        // Aşağıda `continue` yerine `return` kullanılır (kısa devre tek sembolde).
        {
            // ─── Faz 1 (DataIngest): SQLite'tan son 200 candle ────────────
            let candles = match crate::persistence::reader::read_candles(db_path, symbol, interval, 200) {
                Ok(c) if !c.is_empty() => {
                    Self::mark_pipeline_stage(state, PipelineStage::DataIngest, StepStatus::Done);
                    c
                }
                _ => {
                    Self::mark_pipeline_stage(state, PipelineStage::DataIngest, StepStatus::Failed);
                    return;
                }
            };

            // === 1.5) AÇIK POZİSYON İSE: önce SL/TP/Trailing/Breakeven denetle ===
            let live_price = candles.last().map(|c| c.close).unwrap_or(0.0);
            let atr_value  = Self::calc_atr(&candles, 14);
            let exit_reason = {
                let st = match state.lock() { Ok(s) => s, Err(_) => return };
                let atr_mult = st.brain.best_params.get("pos_atr_trail_mult").copied().unwrap_or(2.0);
                let be_rr    = st.brain.best_params.get("pos_breakeven_at_rr").copied().unwrap_or(1.0);
                let reason_opt = if let Ok(mut positions) = st.finance.live_positions.write() {
                    if let Some(pos) = positions.get_mut(symbol) {
                        pos.current_price = live_price;
                        Self::check_exit_conditions(pos, live_price, atr_value, atr_mult, be_rr)
                    } else { None }
                } else { None };
                reason_opt
            };
            if let Some(reason) = exit_reason {
                if let Ok(mut st) = state.lock() {
                    st.push_log(format!(
                        "{} {} {} koşulu tetiklendi @ {:.4}",
                        reason.emoji(), symbol, reason.as_str(), live_price,
                    ));
                }
                Self::close_paper_position(state, symbol, &candles, reason).await;
                return; // bu sembolde tur bitti, yeniden açılış aynı turda denenmesin
            }

            // 3) Strateji seçimi: brain.live_strategy "Default"/"AUTO" ise rejime göre otonom seç.
            let strategy_name = if live_strategy.eq_ignore_ascii_case("default")
                                  || live_strategy.eq_ignore_ascii_case("auto")
                                  || live_strategy.is_empty() {
                let sel = crate::robot::ml_engine::strategy_selector::StrategySelector::new();
                sel.select_best(&candles, &crate::core::types::StrategyParams::default()).to_string()
            } else {
                live_strategy.to_string()
            };

            // Savunma rejimleri (IDLE_PROTECT vb.) — kural IdleStrategyPolicy'de;
            // master.rs ve RoboticTradeExecutor aynı kararı tek source of truth
            // üzerinden okur (Faz 4 c4).
            if !crate::robot::execution::IdleStrategyPolicy
                .evaluate_name(Some(&strategy_name))
                .is_allow()
            {
                return;
            }

            let strategy = crate::robot::logic::optimizer::make_strategy_pub(&strategy_name);
            let strat_params = crate::core::types::StrategyParams::default();

            // ─── Faz 3 (StrategyEval): sinyal üretimi ─────────────────────
            let signal = match strategy.generate_signal(&candles, &strat_params, None, None) {
                Ok(s) => {
                    Self::mark_pipeline_stage(state, PipelineStage::StrategyEval, StepStatus::Done);
                    s
                }
                Err(e) => {
                    Self::mark_pipeline_stage(state, PipelineStage::StrategyEval, StepStatus::Failed);
                    if let Some(logger) = state.lock().ok().and_then(|s| s.trading_logger.clone()) {
                        let ev = crate::robot::infra::logger::TradeEvent::error(
                            &format!("{} sinyal üretim hatası: {:?}", symbol, e),
                        );
                        let _ = logger.log_event(&ev);
                    }
                    return;
                }
            };

            // ─── Faz 2 (FeatureExtract): edge skoru + ATR + (gerek olursa) ML feature.
            // compute_edge_score momentum'u ATR ile normalize edip ML confidence ile harmanlar;
            // başka indikatör/feature hesapları (S/R, regime classify) cycle dışında periyodik
            // task'larda yapılır. Bu nokta tek atımlık feature extraction'ı temsil eder.
            //
            // GBT inference (cycle başına dinamik ml_confidence): IntelligenceHub.gbt
            // hazırsa son ~50 mumdan FeatureVector → predict_confidence(fv, signal)
            // hibrit conf üretir; yoksa eski statik brain.ml_confidence yolu.
            // Sinyal yönü + GBT skoru uyumu cycle bazında değişir → her sembolde
            // farklı conf üretebilir.
            let ml_conf_used: f64 = {
                let gbt_conf = if candles.len() >= 30 {
                    let tail = &candles[candles.len().saturating_sub(200)..];
                    let fv = crate::robot::ml_engine::FeatureExtractor::extract(tail);
                    state.lock().ok().and_then(|st| {
                        st.brain.intelligence_hub.read().ok()
                            .and_then(|hub| hub.predict_confidence(&fv, &signal))
                    })
                } else { None };
                gbt_conf.unwrap_or(ml_confidence)
            };

            let edge = Self::compute_edge_score(&candles, &signal, ml_conf_used);
            Self::mark_pipeline_stage(state, PipelineStage::FeatureExtract, StepStatus::Done);
            // ML henüz hazır değilse (cold-start) gevşek eşik; modele güven arttıkça katılaşır.
            // Faz 2 c4: edge_threshold rejim-bazlı override'a açık.
            // Faz 3 c1: rejim ilk kez görülüyorsa adaptive heuristic patch otomatik.
            // Faz 3 c3: rejim drift değişimi → ekstra savunmacı tighten + bildirim.
            let regime = Self::classify_regime(&candles);
            Self::ensure_regime_patch(state, regime.as_str());
            Self::observe_regime_drift(state, regime.as_str());
            let edge_threshold = state.lock().ok()
                .and_then(|st| st.brain.parameters.read().ok()
                    .map(|p| p.edge_threshold_for(regime.as_str(), ml_conf_used)))
                .unwrap_or_else(|| Self::dynamic_edge_threshold(ml_conf_used));
            // Aday log eşiği: kabul edilen edge'in %75'inin altındaki sinyaller spam sayılır.
            let edge_log_floor = edge_threshold * 0.75;

            // Pozisyonun yönü: None = pozisyon yok, Some(true) = LONG, Some(false) = SHORT.
            // Yön bilgisi kritik; aksi halde aynı yöndeki tekrar sinyalleri pozisyonu kapatır
            // ve aç/kapa döngüsü oluşur (komisyon erozyonu).
            let pos_dir: Option<bool> = {
                let st = match state.lock() { Ok(s) => s, Err(_) => return };
                st.finance.live_positions.read().ok()
                    .and_then(|p| p.get(symbol).map(|pos| pos.is_long))
            };

            let signal_label = match signal {
                Signal::Buy => "BUY", Signal::Sell => "SELL", Signal::Hold => "HOLD",
            };

            // SIGNAL eventi: yalnız Buy/Sell için logla (HOLD spam yapmasın).
            if matches!(signal, Signal::Buy | Signal::Sell) {
                if let Some(logger) = state.lock().ok().and_then(|s| s.trading_logger.clone()) {
                    let ev = crate::robot::infra::logger::TradeEvent::signal(symbol, signal, live_price);
                    let _ = logger.log_event(&ev);
                }
            }

            match (signal, pos_dir) {
                // Pozisyon yokken: yalnız yüksek edge'de açılış denenir.
                (crate::core::types::Signal::Buy, None) | (crate::core::types::Signal::Sell, None) => {
                    if edge < edge_threshold {
                        // Spam'i kısmak için sadece eşiğe yakın aday sinyalleri logla.
                        if edge >= edge_log_floor {
                            if let Ok(mut st) = state.lock() {
                                st.push_log(format!(
                                    "📊 {} {} edge={:.2} eşik={:.2} ⇒ REDDEDİLDİ (zayıf edge, strat={})",
                                    symbol, signal_label, edge, edge_threshold, strategy_name,
                                ));
                            }
                        }
                        return;
                    }
                    // ─── Faz 4 (RiskGate): Guardrails + Kelly + VaR + Gate ───
                    // Notional yaklaşımı: equity'nin %10'u (RiskGatePolicy ayrıca clamp eder).
                    let req_notional = snap.finance.total_equity * 0.10;
                    let decision = risk_manager.authorize(&signal, snap, edge, req_notional);
                    if let crate::robot::risk::risk_gate::RiskDecision::Deny { reasons, enter_safe_mode, halt } = decision {
                        Self::mark_pipeline_stage(state, PipelineStage::RiskGate, StepStatus::Skipped);
                        let mode = if halt { "HALT" }
                            else if enter_safe_mode { "SAFE-MODE" }
                            else { "DENY" };
                        let block_reason = format!("[{}] {}", mode, reasons.join(" · "));
                        if let Ok(mut st) = state.lock() {
                            st.push_log(format!(
                                "🛡️ {} {} edge={:.2} ✓ ama RiskManager [{}]: {}",
                                symbol, signal_label, edge, mode, reasons.join(" · "),
                            ));
                        }
                        if let Some(logger) = state.lock().ok().and_then(|s| s.trading_logger.clone()) {
                            let ev = crate::robot::infra::logger::TradeEvent::risk_block(
                                &block_reason, symbol,
                            );
                            let _ = logger.log_event(&ev);
                        }
                        return;
                    }
                    Self::mark_pipeline_stage(state, PipelineStage::RiskGate, StepStatus::Done);
                    if let Ok(mut st) = state.lock() {
                        st.push_log(format!(
                            "📊 {} {} edge={:.2} ✓ + risk ✓ ⇒ POZİSYON AÇILIYOR (strat={})",
                            symbol, signal_label, edge, strategy_name,
                        ));
                    }
                    Self::open_paper_position(state, symbol, &signal, &candles).await;
                }
                // Pozisyon varken TERS yönde sinyal → kapanış (edge filtresi gevşek).
                // Long + Sell ya da Short + Buy: trend dönmüş demektir.
                (crate::core::types::Signal::Sell, Some(true))
                | (crate::core::types::Signal::Buy,  Some(false)) => {
                    if let Ok(mut st) = state.lock() {
                        st.push_log(format!(
                            "🔄 {} açık pozisyon + ters {} sinyali (edge={:.2}) ⇒ KAPANIŞ",
                            symbol, signal_label, edge,
                        ));
                    }
                    Self::close_paper_position(state, symbol, &candles, ExitReason::StrategySignal).await;
                }
                // Aynı yöndeki tekrar sinyaller: pozisyon zaten o yönde, dokunma.
                // (Aksi halde aç/kapa döngüsü ve komisyon erozyonu doğar.)
                (crate::core::types::Signal::Buy,  Some(true))
                | (crate::core::types::Signal::Sell, Some(false)) => {}
                _ => {}
            }
        }
    }

    /// Edge skoru: son 20 mumun momentum gücü (ATR'ye göre normalize) ile ML confidence ortalaması.
    /// Sinyal yönü momentum ile uyumlu değilse ceza uygulanır.
    ///
    /// Momentum gücü = |ham getiri / ATR%|, 1.0'a clamp'lenir. Yani 20 mum içinde fiyatın ATR'nin
    /// en az 1 katı yön yapması "tam güç" sayılır. Ham getiriyi kullanmak yerine ATR normalizasyonu
    /// 1m gibi düşük volatilite timeframe'lerinde edge'in pratik olarak ölçülebilir kalmasını sağlar.
    fn compute_edge_score(candles: &[Candle], signal: &Signal, ml_confidence: f64) -> f64 {
        if candles.len() < 20 { return 0.0; }
        let recent = &candles[candles.len() - 20..];
        let first = recent.first().map(|c| c.close).unwrap_or(0.0);
        let last  = recent.last().map(|c| c.close).unwrap_or(0.0);
        if first <= 0.0 || last <= 0.0 { return 0.0; }
        let mom = ((last - first) / first).clamp(-1.0, 1.0); // göreli getiri

        // Momentum'u ATR%'ye göre normalize et: kaç ATR yön yapıldı?
        let atr = Self::calc_atr(candles, 14);
        let atr_pct = if last > 0.0 { (atr / last).max(1e-6) } else { 1e-6 };
        let mom_strength = if atr_pct > 1e-6 {
            (mom.abs() / atr_pct).clamp(0.0, 1.0)
        } else {
            mom.abs().clamp(0.0, 1.0)
        };

        let dir_match = match signal {
            Signal::Buy  if mom > 0.0  => 1.0,
            Signal::Sell if mom < 0.0  => 1.0,
            Signal::Hold               => 0.0,
            _                          => 0.4, // ters yön sinyali → ciddi ceza
        };
        let ml = ml_confidence.clamp(0.0, 1.0);
        // ML henüz hazır değilse (0.0) momentum tek başına baskın olsun.
        let ml_w = if ml < f64::EPSILON { 0.0 } else { 0.5 };
        let mom_w = 1.0 - ml_w;
        (dir_match * (mom_strength * mom_w + ml * ml_w)).clamp(0.0, 1.0)
    }

    /// Dinamik edge eşiği: ML modeli henüz hazır değilken (confidence ≈ 0) momentum tek başına
    /// taşıyıcı, bu yüzden daha gevşek eşik. ML hazırlandıkça daha katı bir filtreye geçilir.
    fn dynamic_edge_threshold(ml_confidence: f64) -> f64 {
        if ml_confidence < 0.05 { 0.20 }
        else if ml_confidence < 0.30 { 0.35 }
        else { 0.55 }
    }

    /// Faz 3 c3: rejim drift gözlemi. Önceki cycle'dan farklı bir rejime
    /// geçildiyse store kendi içinde patch'i bir basamak daha sıkılaştırır;
    /// burada push_alert ile kullanıcıya bildirim gönderiyoruz (Telegram + UI log).
    /// İlk gözlem değişim sayılmaz (cold start).
    fn observe_regime_drift(state: &Arc<Mutex<AppState>>, regime: &str) {
        let drift = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            st.brain.parameters.write().ok().map(|mut p| p.observe_regime(regime)).unwrap_or(false)
        };
        if !drift { return; }
        if let Ok(mut st) = state.lock() {
            let key = format!("REGIME-DRIFT-{}", regime);
            let msg = format!(
                "🌪️ Rejim drift: '{}' → patch sıkılaştırıldı (savunmacı duruş)",
                regime,
            );
            st.push_alert(
                &key,
                crate::robot::infra::telegram_notifier::Severity::Warning,
                msg,
            );
        }
    }

    /// Faz 3 c1: ilk kez görülen rejim için ParameterStore'da otomatik heuristic
    /// patch yerleştirir. Patch zaten varsa (manuel ya da önceki cycle'da yazıldı)
    /// dokunmaz — HyperOpt veya manuel override'ın ezilmesini önler.
    /// Boş heuristic patch ise (Weak* / Unknown) hiç yazılmaz.
    fn ensure_regime_patch(state: &Arc<Mutex<AppState>>, regime: &str) {
        let needs_patch = state.lock().ok()
            .and_then(|st| st.brain.parameters.read().ok()
                .map(|p| !p.regime_overrides.contains_key(regime)))
            .unwrap_or(false);
        if !needs_patch { return; }

        let patch = crate::robot::parameters::adaptive::default_patch_for_regime(regime);
        if patch.is_empty() { return; }

        if let Ok(mut st) = state.lock() {
            if let Ok(mut params) = st.brain.parameters.write() {
                params.set_regime_patch(regime, patch);
            }
            st.push_log(format!(
                "📐 Rejim '{}' ilk kez görüldü → adaptive patch uygulandı (Faz 3)",
                regime,
            ));
        }
    }

    /// Pipeline canon aşamasını "bitti" olarak işaretler ve Failed/Skipped
    /// durumlarda otomatik bir Anomaly emit eder (TUI Pipeline sekmesindeki
    /// "🛡️ Aktif Anomaliler" panelinde gözükür).
    ///
    /// - `Done` → sadece chain_steps güncellenir.
    /// - `Failed` → Critical anomaly + chain_steps.
    /// - `Skipped` → Warning anomaly + chain_steps (örn. RiskGate Deny; bot
    ///   sinyali bilerek reddetti, kullanıcının görmesi faydalı).
    /// - diğer (Idle/Running) → sadece chain_steps; anomaly üretilmez.
    ///
    /// state.lock() + live_pipeline.write() kısa scope'lu; lock contention yok.
    fn mark_pipeline_stage(
        state: &Arc<Mutex<AppState>>,
        stage: crate::robot::data_pipeline::canon::PipelineStage,
        status: crate::robot::data_pipeline::StepStatus,
    ) {
        use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity, StepStatus};
        if let Ok(st) = state.lock() {
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                pipe.mark_stage_completed(stage, status);
                let (severity, kind, msg) = match status {
                    StepStatus::Failed => (
                        AnomalySeverity::Critical,
                        AnomalyKind::DataStall,
                        format!("{} fazı başarısız: cycle bu aşamada koptu", stage.label()),
                    ),
                    StepStatus::Skipped => (
                        AnomalySeverity::Warning,
                        AnomalyKind::RiskBreach,
                        format!("{} fazı atlandı: koruma/red akışı tetiklendi", stage.label()),
                    ),
                    _ => return, // Done / Idle / Running anomaly üretmez
                };
                pipe.push_anomaly(severity, kind, msg);
            }
        }
    }

    /// 🧬 FAZ F3: OTONOM POZİSYON AÇILIŞ MOTORU (Paper + Live dispatcher)
    /// Kelly oranı, brain.ml_confidence ve loss_streak ile dinamik tahsisat yapar.
    /// Live executor bağlıysa ve dry-run değilse: gerçek market order gönderir.
    async fn open_paper_position(state: &Arc<Mutex<AppState>>, symbol: &str, signal: &Signal, candles: &[Candle]) {
        use crate::robot::risk::kelly::KellyCriterion;
        let last_candle = match candles.last() { Some(c) => c, None => return };

        let is_long = matches!(signal, Signal::Buy);
        let entry = last_candle.close;
        let atr = Self::calc_atr(candles, 14);
        let regime = Self::classify_regime(candles);
        let pos_id = crate::core::types::PositionId::new();
        let pos_id_str = pos_id.to_string();

        // Tüm sync hesap + state okumaları tek mutex skopunda — guard async sınırını geçemez.
        struct OpenPlan {
            new_pos: PositionModel,
            alloc_capital: f64,
            qty_val: f64,
            kelly_fraction: f64,
            risk_appetite: f64,
            ml_conf: f64,
            tp_pct: f64,
            sl_pct: f64,
            strategy_name: String,
            live_executor: Option<Arc<crate::robot::engines::binance_executor::BinanceFuturesExecutor>>,
            live_dry_run: bool,
            live_max_notional: f64,
            atr_mult: f64,
        }
        let plan: Option<OpenPlan> = {
            let mut st = state.lock().unwrap();
            st.fleet.phase = "Executing".into();

            let risk_appetite = st.finance.calculate_risk_appetite();
            let ml_conf = st.brain.ml_confidence;
            let loss_streak = st.finance.live_closed_trades.read()
                .map(|tr| tr.iter().rev().take(5).filter(|t| t.pnl < 0.0).count())
                .unwrap_or(0);
            let (wins, losses, sum_win, sum_loss) = st.finance.live_closed_trades.read().map(|tr| {
                let mut w = 0u32; let mut l = 0u32; let mut sw = 0.0f64; let mut sl = 0.0f64;
                for t in tr.iter().rev().take(50) {
                    if t.pnl > 0.0 { w += 1; sw += t.pnl; }
                    else if t.pnl < 0.0 { l += 1; sl += -t.pnl; }
                }
                (w, l, sw, sl)
            }).unwrap_or((0, 0, 0.0, 0.0));
            let total = (wins + losses) as f64;
            let win_prob = if total > 0.0 { wins as f64 / total } else { 0.5 };
            let avg_win = if wins > 0 { sum_win / wins as f64 } else { 1.0 };
            let avg_loss = if losses > 0 { sum_loss / losses as f64 } else { 1.0 };
            let kelly = KellyCriterion::calculate(win_prob, avg_win, avg_loss);

            let base_alloc = st.finance.equity * 0.10 * risk_appetite;
            let alloc_capital = kelly.calculate_dynamic_scale(base_alloc, loss_streak, ml_conf)
                .max(base_alloc * 0.25);
            let qty_val = (alloc_capital / entry).max(0.0);
            if qty_val <= 0.0 { return; }

            // Faz 2 c4: TP/SL artık rejim-bazlı override'a açık. Store'da o rejim için
            // patch varsa onun trade_risk'i, yoksa base trade_risk kullanılır.
            // HyperOpt rejim-aware tuning yaptıkça (Faz 3'te) patch'leri besleyecek.
            let (tp_pct, sl_pct) = st.brain.parameters.read()
                .map(|p| {
                    let tr = p.trade_risk_for(regime.as_str());
                    (tr.take_profit_pct, tr.stop_loss_pct)
                })
                .unwrap_or((3.0, 1.5));
            let tp_pct = tp_pct.max(0.1);
            let sl_pct = sl_pct.max(0.1);
            let (stop_loss, take_profit) = if is_long {
                (entry * (1.0 - sl_pct / 100.0), entry * (1.0 + tp_pct / 100.0))
            } else {
                (entry * (1.0 + sl_pct / 100.0), entry * (1.0 - tp_pct / 100.0))
            };
            let atr_mult = st.brain.best_params.get("pos_atr_trail_mult").copied().unwrap_or(2.0);
            let trailing_stop = if is_long { entry - atr * atr_mult }
                                else       { entry + atr * atr_mult };
            let strategy_name = st.brain.live_strategy.read()
                .map(|s| s.clone()).unwrap_or_else(|_| "MA_CROSSOVER".into());
            let new_pos = PositionModel {
                pos_id: pos_id_str.clone(),
                symbol: symbol.to_string(),
                entry_price: entry, current_price: entry,
                qty: qty_val, leverage: 1.0,
                trade_type: if is_long { "LONG".into() } else { "SHORT".into() },
                is_long,
                opened_at: chrono::Utc::now().to_rfc3339(),
                stop_loss, take_profit, trailing_stop,
                max_favorable_price: entry,
                breakeven_activated: false,
            };
            Some(OpenPlan {
                new_pos, alloc_capital, qty_val,
                kelly_fraction: kelly.kelly_fraction, risk_appetite, ml_conf,
                tp_pct, sl_pct, strategy_name,
                live_executor: st.live_executor.clone(),
                live_dry_run: st.live_dry_run,
                live_max_notional: st.live_max_notional_usd,
                atr_mult,
            })
        }; // st burada drop
        let plan = match plan { Some(p) => p, None => return };

        // 💱 LIVE Mode dispatcher (3 koşullu onay zinciri):
        let live_executor = plan.live_executor.clone();
        let live_dry_run = plan.live_dry_run;
        let live_max_notional = plan.live_max_notional;
        let alloc_capital = plan.alloc_capital;
        let qty_val = plan.qty_val;
        let new_pos = plan.new_pos.clone();
        let kelly_fraction = plan.kelly_fraction;
        let risk_appetite = plan.risk_appetite;
        let ml_conf = plan.ml_conf;
        let tp_pct = plan.tp_pct;
        let sl_pct = plan.sl_pct;
        let strategy_name = plan.strategy_name.clone();
        let atr_mult = plan.atr_mult;

        let mut live_order_id: Option<String> = None;
        // Filtre sonrası qty ve SL/TP fiyatları burada güncellenir; local pozisyon
        // (new_pos) borsaya gönderilen değerle birebir eşleşsin diye mutable.
        let mut qty_val = qty_val;
        let mut new_pos = new_pos;
        if let Some(executor) = live_executor.as_ref() {
            let side = if is_long { "BUY" } else { "SELL" };
            if alloc_capital > live_max_notional {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🛑 LIVE veto: notional ${:.2} > tavan ${:.2} ({} {} iptal edildi)",
                        alloc_capital, live_max_notional, symbol, side,
                    ));
                }
                return;
            }

            // 🧮 ExchangeInfo filtre kontrolü (LOT_SIZE / MIN_NOTIONAL / PRICE_FILTER).
            // qty stepSize'a aşağı yuvarlanır, qty*price minNotional altındaysa emir
            // gönderilmeden veto edilir → Binance -1013 reddini önler.
            match executor.apply_filters(symbol, qty_val, entry).await {
                Ok(rounded) => {
                    if (rounded - qty_val).abs() > f64::EPSILON {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "🧮 [LIVE-FILTER] {} qty {:.8} → {:.8} (stepSize'a yuvarlandı)",
                                symbol, qty_val, rounded,
                            ));
                        }
                        qty_val = rounded;
                        new_pos.qty = rounded;
                    }
                    if let Ok(map) = executor.filters.read() {
                        if let Some(f) = map.get(symbol) {
                            let sl_r = f.round_price(new_pos.stop_loss);
                            let tp_r = f.round_price(new_pos.take_profit);
                            new_pos.stop_loss = sl_r;
                            new_pos.take_profit = tp_r;
                            new_pos.trailing_stop = f.round_price(new_pos.trailing_stop);
                        }
                    }
                }
                Err(reason) => {
                    if let Ok(mut st2) = state.lock() {
                        st2.push_log(format!(
                            "🛑 [LIVE-FILTER-VETO] {} {} reddedildi: {}",
                            symbol, side, reason,
                        ));
                        st2.guardian.repair_log.push_back(format!(
                            "[{}] LOT_SIZE/MIN_NOTIONAL veto: {} {} ({})",
                            chrono::Local::now().format("%H:%M:%S"), symbol, side, reason,
                        ));
                        while st2.guardian.repair_log.len() > 100 { st2.guardian.repair_log.pop_front(); }
                    }
                    return;
                }
            }

            if live_dry_run {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🟡 [LIVE-DRY-RUN] {} {} {:.4} @ {:.2} (${:.2}) → emir gönderilmedi",
                        symbol, side, qty_val, entry, alloc_capital,
                    ));
                }
            } else {
                match executor.place_market_order(symbol, side, qty_val).await {
                    Ok(resp) => {
                        live_order_id = resp.get("orderId").map(|v| v.to_string());
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "💱 [LIVE] {} {} {:.4} @ {:.2} (${:.2}) ✓ order={}",
                                symbol, side, qty_val, entry, alloc_capital,
                                live_order_id.as_deref().unwrap_or("?"),
                            ));
                        }

                        // 🛡️ Borsa-tarafı koruma: SL ve TP emirlerini hemen yerleştir.
                        // Bot ölse / network kopsa bile pozisyon korumalı kalır.
                        let pos_sl = new_pos.stop_loss;
                        let pos_tp = new_pos.take_profit;
                        let (sl_res, tp_res) = executor.place_protection_orders(
                            symbol, is_long, qty_val, pos_sl, pos_tp,
                        ).await;
                        let sl_id = sl_res.as_ref().ok()
                            .and_then(|r| r.get("orderId").map(|v| v.to_string()));
                        let tp_id = tp_res.as_ref().ok()
                            .and_then(|r| r.get("orderId").map(|v| v.to_string()));
                        let sl_status = match &sl_res {
                            Ok(_)  => format!("SL ✓ ({})", sl_id.as_deref().unwrap_or("?")),
                            Err(e) => format!("SL ❌ {:?}", e),
                        };
                        let tp_status = match &tp_res {
                            Ok(_)  => format!("TP ✓ ({})", tp_id.as_deref().unwrap_or("?")),
                            Err(e) => format!("TP ❌ {:?}", e),
                        };
                        // Order ID eşlemesini state'e mühürle (cancel için audit trail).
                        if let Ok(mut st2) = state.lock() {
                            if let Ok(mut map) = st2.finance.live_orders.write() {
                                map.insert(symbol.to_string(), crate::core::model::LiveOrderRefs {
                                    entry_order_id: live_order_id.clone(),
                                    sl_order_id: sl_id.clone(),
                                    tp_order_id: tp_id.clone(),
                                    placed_at: chrono::Utc::now().to_rfc3339(),
                                });
                            }
                            st2.push_log(format!(
                                "🛡️ [LIVE-PROTECT] {} @ SL={:.4} TP={:.4} · {} · {}",
                                symbol, pos_sl, pos_tp, sl_status, tp_status,
                            ));
                        }

                        // Kritik uyarı: SL emri başarısızsa pozisyon korumasız — emergency.
                        if sl_res.is_err() {
                            if let Ok(mut st2) = state.lock() {
                                st2.push_alert(
                                    "LIVE-EMERGENCY",
                                    crate::robot::infra::telegram_notifier::Severity::Critical,
                                    format!(
                                        "[LIVE-EMERGENCY] {} SL emri verilemedi → pozisyon acil kapatılıyor",
                                        symbol,
                                    ),
                                );
                                st2.guardian.repair_log.push_back(format!(
                                    "[{}] live SL hatası: {} emergency close",
                                    chrono::Local::now().format("%H:%M:%S"), symbol,
                                ));
                                while st2.guardian.repair_log.len() > 100 { st2.guardian.repair_log.pop_front(); }
                            }
                            // Hemen pozisyonu kapat — koruma sağlanamadı.
                            let _ = executor.close_position(symbol).await;
                            return;
                        }
                    }
                    Err(e) => {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "❌ [LIVE] {} {} emir hatası: {:?} — pozisyon kaydedilmedi",
                                symbol, side, e,
                            ));
                        }
                        return; // Live emir başarısızsa paper'ı da çalıştırma.
                    }
                }
            }
        }

        // Mutex'i geri al
        let mut st = state.lock().unwrap();

        let new_pos_for_log = new_pos.clone();
        {
            if let Ok(mut positions) = st.finance.live_positions.write() {
                positions.insert(symbol.to_string(), new_pos);
            }
        }

        // Komisyon (0.1%) live_execution_costs'a yazılır.
        let commission = alloc_capital * 0.001;
        if let Ok(mut costs) = st.finance.live_execution_costs.write() {
            costs.commission_usd += commission;
            costs.total_cost_usd += commission;
            costs.trade_count    += 1;
        }

        // IntelligenceHub.track_trade — kapanışta learn_from_exit ile eşleşecek.
        if let Ok(mut hub) = st.brain.intelligence_hub.write() {
            hub.track_trade(pos_id, regime, strategy_name.clone());
        }

        let mode_tag = if live_order_id.is_some() { "LIVE" }
                       else if live_executor.is_some() && live_dry_run { "DRY-RUN" }
                       else { "PAPER" };
        let pos_for_log = new_pos_for_log;
        st.push_log(format!(
            "🚀 [{}-{}] {} açıldı @ {:.2} | Qty={:.4} ${:.2} | SL={:.2} TP={:.2} Trail={:.2} (ATR={:.4} ×{:.1})",
            mode_tag,
            if is_long { "BUY" } else { "SELL" },
            symbol, entry, qty_val, alloc_capital,
            pos_for_log.stop_loss, pos_for_log.take_profit, pos_for_log.trailing_stop, atr, atr_mult,
        ));
        st.push_log(format!(
            "    └─ Kelly f*={:.3} · risk_iştah={:.2} · ML={:.2} · TP%={:.2} SL%={:.2} · Rejim={} · Strat={}",
            kelly_fraction, risk_appetite, ml_conf, tp_pct, sl_pct,
            regime.as_str(), strategy_name,
        ));
        // 📝 Periyodik dosya logu: TRADE_OPEN. Logger Arc'ını lock altında clone'la,
        // unlock sonrası IO yap.
        let logger_for_event = st.trading_logger.clone();
        let equity_now = st.finance.equity;
        drop(st);
        if let Some(logger) = logger_for_event {
            let ev = crate::robot::infra::logger::TradeEvent::trade_open(
                symbol, &strategy_name, is_long, entry, qty_val, equity_now,
            );
            let _ = logger.log_event(&ev);
        }
        let _ = live_order_id; // ileride pos_id ↔ order_id eşlemesi için saklanabilir

        // ─── Faz 5 (Execute): pozisyon başarıyla mühürlendi ───────────────
        Self::mark_pipeline_stage(
            state,
            crate::robot::data_pipeline::canon::PipelineStage::Execute,
            crate::robot::data_pipeline::StepStatus::Done,
        );
    }

    /// 🧠 IntelligenceHub periyodik tick: drift hesabı + evrim + retrain kararı.
    ///
    /// Akış:
    /// 1. Aktif sembolün son 200 mumundan FeatureVector çıkar
    /// 2. hub.drift_detector.update(fv) → drift_score güncellenir
    /// 3. brain.drift_history'e push (TUI snapshot için)
    /// 4. hub.should_retrain(drift_score) true ise: triggers["ml"].store(true) ve repair_log
    /// 5. hub.tick_evolution() — controller cycle_id'yi artırır, periyot dolduğunda evolve
    async fn tick_intelligence_hub(state: &Arc<Mutex<AppState>>) {
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

    /// 🌐 Mum dizisinden evolution::MarketRegime çıkar (IntelligenceHub'a yöne duyarlı sinyal).
    /// AdxRegime'i momentumla zenginleştirir.
    fn classify_regime(candles: &[Candle]) -> crate::evolution::MarketRegime {
        use crate::evolution::MarketRegime;
        use crate::robot::logic::market_regime::{detect_adx_regime, AdxRegime};
        if candles.len() < 20 { return MarketRegime::Unknown; }
        let adx = detect_adx_regime(candles);
        let recent = &candles[candles.len() - 20..];
        let first = recent.first().map(|c| c.close).unwrap_or(0.0);
        let last  = recent.last().map(|c| c.close).unwrap_or(0.0);
        if first <= 0.0 { return MarketRegime::Unknown; }
        let mom_pct = (last - first) / first * 100.0;
        match adx {
            AdxRegime::Volatile => MarketRegime::HighVolatility,
            AdxRegime::Ranging  => MarketRegime::Ranging,
            AdxRegime::Trending if mom_pct >  2.0 => MarketRegime::StrongUptrend,
            AdxRegime::Trending if mom_pct >  0.0 => MarketRegime::WeakUptrend,
            AdxRegime::Trending if mom_pct < -2.0 => MarketRegime::StrongDowntrend,
            AdxRegime::Trending                   => MarketRegime::WeakDowntrend,
            AdxRegime::Neutral if mom_pct.abs() < 0.5 => MarketRegime::LowVolatility,
            AdxRegime::Neutral                        => MarketRegime::Unknown,
        }
    }

    /// 📏 ATR (Average True Range) — son N mum üzerinde Wilder-style.
    /// Tarihçe yetersizse 0.0 döner (trailing devre dışı sayılır).
    fn calc_atr(candles: &[Candle], period: usize) -> f64 {
        let n = candles.len();
        if n < period + 1 { return 0.0; }
        let slice = &candles[n - period - 1..];
        let mut sum = 0.0;
        for w in slice.windows(2) {
            let prev = &w[0]; let cur = &w[1];
            let h_l = cur.high - cur.low;
            let h_pc = (cur.high - prev.close).abs();
            let l_pc = (cur.low  - prev.close).abs();
            sum += h_l.max(h_pc).max(l_pc);
        }
        sum / period as f64
    }

    /// 🛡️ POZİSYON ÇIKIŞ KONTROLÜ: Açık her pozisyon için SL/TP/Trailing/Breakeven
    /// koşullarını sırasıyla denetler. Tetiklenmişse Some(ExitReason) döner ve
    /// pozisyonun max_favorable_price / breakeven_activated / trailing_stop alanlarını
    /// günceller.
    pub fn check_exit_conditions(
        position: &mut PositionModel,
        last_price: f64,
        atr: f64,
        atr_trail_mult: f64,
        breakeven_rr: f64,
    ) -> Option<ExitReason> {
        if last_price <= 0.0 { return None; }

        // 1) Favorable price güncellemesi (long en yüksek, short en düşük)
        if position.is_long {
            if last_price > position.max_favorable_price { position.max_favorable_price = last_price; }
        } else {
            if position.max_favorable_price == 0.0 || last_price < position.max_favorable_price {
                position.max_favorable_price = last_price;
            }
        }

        // 2) SL — statik (breakeven aktifse SL = entry'e taşınmış olur).
        if position.stop_loss > 0.0 {
            if position.is_long && last_price <= position.stop_loss {
                return Some(if position.breakeven_activated { ExitReason::Breakeven }
                            else { ExitReason::StopLoss });
            }
            if !position.is_long && last_price >= position.stop_loss {
                return Some(if position.breakeven_activated { ExitReason::Breakeven }
                            else { ExitReason::StopLoss });
            }
        }

        // 3) TP — statik.
        if position.take_profit > 0.0 {
            if position.is_long && last_price >= position.take_profit {
                return Some(ExitReason::TakeProfit);
            }
            if !position.is_long && last_price <= position.take_profit {
                return Some(ExitReason::TakeProfit);
            }
        }

        // 4) Breakeven aktivasyonu — TP'nin yarısına ulaştığında SL'i entry'e taşı.
        //    breakeven_rr: ROE eşiği (örn. 1.0 = RR 1:1, yani SL kadar kazanç).
        if !position.breakeven_activated && position.entry_price > 0.0 && position.stop_loss > 0.0 {
            let risk = (position.entry_price - position.stop_loss).abs();
            if risk > 0.0 {
                let gain = if position.is_long { last_price - position.entry_price }
                           else                 { position.entry_price - last_price };
                if gain >= risk * breakeven_rr {
                    position.breakeven_activated = true;
                    position.stop_loss = position.entry_price; // SL'i entry'e taşı
                }
            }
        }

        // 5) Trailing stop — ATR × mult uzaklıkta, sadece elverişli yönde kayar.
        if atr > 0.0 && atr_trail_mult > 0.0 {
            let delta = atr * atr_trail_mult;
            if position.is_long {
                let new_trail = position.max_favorable_price - delta;
                if new_trail > position.trailing_stop { position.trailing_stop = new_trail; }
                if position.trailing_stop > 0.0 && last_price <= position.trailing_stop {
                    return Some(ExitReason::TrailingStop);
                }
            } else {
                let new_trail = position.max_favorable_price + delta;
                if position.trailing_stop == 0.0 || new_trail < position.trailing_stop {
                    position.trailing_stop = new_trail;
                }
                if position.trailing_stop > 0.0 && last_price >= position.trailing_stop {
                    return Some(ExitReason::TrailingStop);
                }
            }
        }

        None
    }

    /// 🧬 FAZ F3: OTONOM POZİSYON KAPATMA MOTORU (Paper + Live dispatcher)
    async fn close_paper_position(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        candles: &[Candle],
        reason: ExitReason,
    ) {
        let last_candle = match candles.last() { Some(c) => c, None => return };

        // Mutex guard'ı async sınırını geçemez (MutexGuard !Send). Tüm sync iş bu skopta:
        let (target_pos, live_executor, live_dry_run, mode_tag) = {
            let mut st = state.lock().unwrap();
            st.fleet.phase = "Executing".into();
            let target_pos = if let Ok(mut positions) = st.finance.live_positions.write() {
                positions.remove(symbol)
            } else { None };
            let exec = st.live_executor.clone();
            let dry = st.live_dry_run;
            let tag = if exec.is_some() && !dry { "LIVE" }
                      else if exec.is_some() && dry { "DRY-RUN" }
                      else { "PAPER" };
            (target_pos, exec, dry, tag)
        }; // st burada otomatik drop olur

        if let Some(executor) = live_executor.as_ref() {
            if live_dry_run {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🟡 [LIVE-DRY-RUN] {} close ({:?}) → emir gönderilmedi", symbol, reason,
                    ));
                }
            } else {
                // 1. Bekleyen koruma emirlerini hedefli olarak iptal et.
                //    live_orders map'inden SL ve TP order_id'leri okunur; sadece bu emirler
                //    cancel edilir (paralel sembollerdeki orphan'lar etkilenmesin).
                //    Map'te kayıt yoksa fallback: cancel_all_orders (eski davranış).
                let refs = state.lock().ok()
                    .and_then(|s| s.finance.live_orders.read().ok()
                        .and_then(|m| m.get(symbol).cloned()));

                let cancel_result = if let Some(refs) = refs {
                    let mut summary: Vec<String> = Vec::new();
                    if let Some(sl_id_str) = refs.sl_order_id.as_deref() {
                        if let Ok(id) = sl_id_str.trim_matches('"').parse::<u64>() {
                            match executor.cancel_order(symbol, id).await {
                                Ok(_) => summary.push(format!("SL#{} ✓", id)),
                                Err(e) => summary.push(format!("SL#{} ❌ {:?}", id, e)),
                            }
                        }
                    }
                    if let Some(tp_id_str) = refs.tp_order_id.as_deref() {
                        if let Ok(id) = tp_id_str.trim_matches('"').parse::<u64>() {
                            match executor.cancel_order(symbol, id).await {
                                Ok(_) => summary.push(format!("TP#{} ✓", id)),
                                Err(e) => summary.push(format!("TP#{} ❌ {:?}", id, e)),
                            }
                        }
                    }
                    Some(summary)
                } else {
                    None
                };

                match cancel_result {
                    Some(summary) if !summary.is_empty() => {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "🧹 [LIVE] {} hedefli iptal: {}", symbol, summary.join(" · "),
                            ));
                        }
                    }
                    _ => {
                        // Fallback: order_id eşlemesi yoksa cancel_all (geriye uyum)
                        match executor.cancel_all_orders(symbol).await {
                            Ok(_) => {
                                if let Ok(mut st2) = state.lock() {
                                    st2.push_log(format!(
                                        "🧹 [LIVE] {} cancel_all (id yok, geniş iptal)", symbol,
                                    ));
                                }
                            }
                            Err(e) => {
                                if let Ok(mut st2) = state.lock() {
                                    st2.push_log(format!(
                                        "⚠️ [LIVE] {} cancel_all_orders hatası: {:?} (orphan SL/TP olabilir)",
                                        symbol, e,
                                    ));
                                }
                            }
                        }
                    }
                }
                // Eşlemeyi temizle (pozisyon artık yok).
                if let Ok(st2) = state.lock() {
                    if let Ok(mut map) = st2.finance.live_orders.write() {
                        map.remove(symbol);
                    }
                }
                // 2. Pozisyonu market emir ile kapat.
                match executor.close_position(symbol).await {
                    Ok(resp) => {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "💱 [LIVE] {} close ({:?}) ✓ order={}",
                                symbol, reason,
                                resp.get("orderId").map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
                            ));
                        }
                    }
                    Err(e) => {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "❌ [LIVE] {} close hatası: {:?} — paper tarafı yine de kapanacak",
                                symbol, e,
                            ));
                        }
                    }
                }
            }
        }

        let mut st = state.lock().unwrap();

        if let Some(pos) = target_pos {
            // Çıkış fiyatı: SL/TP/Trailing'de kapanma seviyesi pos'taki değer; aksi son mum kapanışı.
            let exit_price = match reason {
                ExitReason::StopLoss | ExitReason::Breakeven => pos.stop_loss,
                ExitReason::TakeProfit                       => pos.take_profit,
                ExitReason::TrailingStop                     => pos.trailing_stop,
                ExitReason::StrategySignal                   => last_candle.close,
            };
            let exit_price = if exit_price > 0.0 { exit_price } else { last_candle.close };

            let pnl_val = crate::core::math::calculate_pnl(pos.entry_price, exit_price, pos.qty, pos.is_long);
            // Çıkış komisyonu (0.1%) — exit notional üzerinden
            let exit_commission = (exit_price * pos.qty) * 0.001;
            if let Ok(mut costs) = st.finance.live_execution_costs.write() {
                costs.commission_usd += exit_commission;
                costs.total_cost_usd += exit_commission;
            }
            st.finance.equity += pnl_val - exit_commission;

            let pnl_pct_val = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                (pnl_val / (pos.entry_price * pos.qty)) * 100.0
            } else { 0.0 };

            let closed_trade = ClosedTradeModel {
                symbol: symbol.to_string(),
                is_long: pos.is_long,
                exit_reason: reason.as_str().to_string(),
                pnl: pnl_val,
                pnl_pct: pnl_pct_val,
                closed_at: chrono::Utc::now().to_rfc3339(),
                opened_at: pos.opened_at.clone(),
            };

            // [DÜZELTME]: Arşiv listesine itme işlemi izole skopa alındı
            {
                if let Ok(mut closed_list) = st.finance.live_closed_trades.write() {
                    closed_list.push(closed_trade.clone());
                }
            }

            st.push_log(format!(
                "{} [{}-CLOSE/{}] {} kapatıldı @ {:.2} (entry={:.2}) | Net PnL: {:.2} USDT ({:+.2}%)",
                reason.emoji(), mode_tag, reason.as_str(), symbol, exit_price, pos.entry_price, pnl_val, pnl_pct_val,
            ));

            // ─── Faz 6 (Learn): IntelligenceHub.learn_from_exit ─────────────
            // track_trade'de açılışta hangi rejim/strateji ile mühürlediysek,
            // kazanç/kayıp uçtan uca o eşleştirmeye gider.
            let mut learn_recorded = false;
            if !pos.pos_id.is_empty() {
                let pid = crate::core::types::PositionId::from_str_or_new(&pos.pos_id);
                let mut hub_summary: Option<(usize, String)> = None;
                if let Ok(mut hub) = st.brain.intelligence_hub.write() {
                    hub.learn_from_exit(pid, pnl_pct_val);
                    hub_summary = Some((hub.controller.consecutive_failures, hub.controller.state.to_string()));
                    learn_recorded = true;
                }
                if let Some((cf, controller_state)) = hub_summary {
                    st.push_log(format!(
                        "🧠 Hub öğrendi: pos_id={}… pnl={:+.2}% · ardışık kayıp={} · controller={}",
                        &pos.pos_id[..pos.pos_id.len().min(8)],
                        pnl_pct_val, cf, controller_state,
                    ));
                }
            }
            // Learn mark — hub.write() gerçekten çalıştıysa Done; aksi halde Skipped
            // (eski/legacy pos_id eşleşmedi). Helper yerine inline yazıyoruz çünkü
            // `st` lock zaten elde; relock yapmak gereksiz. Skipped durumunda
            // helper'la aynı anomaly emit edilir (TUI Anomaliler paneline düşer).
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                use crate::robot::data_pipeline::{canon::PipelineStage, StepStatus,
                    AnomalyKind, AnomalySeverity};
                let status = if learn_recorded { StepStatus::Done } else { StepStatus::Skipped };
                pipe.mark_stage_completed(PipelineStage::Learn, status);
                if matches!(status, StepStatus::Skipped) {
                    pipe.push_anomaly(
                        AnomalySeverity::Warning,
                        AnomalyKind::RiskBreach,
                        format!("{} fazı atlandı: pos_id eşleşmedi (legacy pozisyon)", PipelineStage::Learn.label()),
                    );
                }
            }

            // 📝 Periyodik dosya logu: TRADE_CLOSE. Logger Arc'ını clone'la, IO için
            // mutex'i bırakmadan önce gerekli alanları kopyala.
            let logger_for_event = st.trading_logger.clone();
            let equity_now = st.finance.equity;
            let strategy_name = st.brain.live_strategy.read()
                .map(|s| s.clone()).unwrap_or_else(|_| "?".to_string());

            drop(st); // Q-Table alt işçisi çağrılmadan önce ana kilit tamamen imha edilir (Fail-Safe)

            if let Some(logger) = logger_for_event {
                let ev = crate::robot::infra::logger::TradeEvent::trade_close(
                    symbol, &strategy_name, pos.is_long, exit_price, pos.qty,
                    pnl_val, equity_now, reason.as_str(),
                );
                let _ = logger.log_event(&ev);
            }
            Self::update_cognitive_memory(state, &closed_trade);

            // ─── Faz 3 c2: rejim-bazlı trade feedback rafinasyonu ───────────
            // Kapanış candles'tan anlık rejimi hesapla; ParameterStore'a pnl_pct'yi
            // bildir. Yeterli veri biriktiyse (WINDOW=10) ve win_rate eşiği (0.40)
            // altına düştüyse o rejim için patch otomatik sıkılaştırılır.
            let regime_at_close = Self::classify_regime(candles);
            let regime_key = regime_at_close.as_str().to_string();
            let tightened = {
                let st = state.lock().ok();
                let mut tightened = false;
                if let Some(st) = st {
                    if let Ok(mut params) = st.brain.parameters.write() {
                        tightened = params.apply_trade_feedback(&regime_key, pnl_pct_val);
                    }
                }
                tightened
            };
            if tightened {
                if let Ok(mut st) = state.lock() {
                    st.push_log(format!(
                        "🛡️ Adaptive: rejim '{}' düşük win-rate → patch sıkılaştırıldı",
                        regime_key,
                    ));
                }
            }

            // ─── Faz 5 (Execute): kapanış icrası tamamlandı ─────────────────
            // Açılış open_paper_position'da işaretleniyor; kapanış da bir Execute
            // adımı sayılır (cancel_orders + arşivleme + reward feedback).
            Self::mark_pipeline_stage(
                state,
                crate::robot::data_pipeline::canon::PipelineStage::Execute,
                crate::robot::data_pipeline::StepStatus::Done,
            );
        }
    }


    /// 🧠 BİLİŞSEL HAFIZA: Q-Table ödül/ceza sistemi.
    pub fn update_cognitive_memory(state: &Arc<Mutex<AppState>>, last_trade: &ClosedTradeModel) {
        let mut st = state.lock().unwrap();
        let reward = crate::core::math::calculate_trade_reward(last_trade.pnl_pct, 0, 0.0);
        st.push_log(format!("🧠 Tecrübe Mühürlendi: {} | Ödül: {:.2}", last_trade.symbol, reward));
    }

    /// 🛡️ ANOMALİ ONARIMI: aktif anomali sayısı > 0 ise ML retrain tetiklenir,
    /// anomali türleri ve onarım kaydı guardian.repair_log'a düşürülür.
    fn perform_anomaly_recovery(state: &Arc<Mutex<AppState>>, snap: &MissionControl) {
        if snap.active_anomalies == 0 { return; }
        let mut st = state.lock().unwrap();
        st.fleet.phase = "Recovering".into();

        // Anomalilerin özetini çıkar (severity sayım)
        let mut warning_n = 0u32;
        let mut critical_n = 0u32;
        let mut kinds: Vec<String> = Vec::new();
        for a in &snap.anomalies {
            if a.severity.contains("Critical") { critical_n += 1; }
            else { warning_n += 1; }
            if !kinds.contains(&a.kind) { kinds.push(a.kind.clone()); }
        }

        // ML retrain'i tetikle (zaten ml job'u kendi loglarını basacak)
        st.fleet.triggers.get("ml").map(|t| t.store(true, Ordering::Relaxed));

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

    /// 🧠 ML Retrain (Faz 4 - "ml" trigger):
    /// Aktif sembolde ParameterOptimizer.random_search çalıştırır, en iyi TP/SL/PS setini
    /// brain.best_params'a yazar ve config/best_params.json'a atomik mühürler.
    fn run_ml_retrain_job(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        log::info!("🧠 E2: ML Retrain başlatıldı (random search 60 iter)...");

        // 1) Çalışma sembolü ve mum kuyruğu — kilidi job boyunca tutmuyoruz.
        let (symbol, interval, db_path, capital) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            (st.config.symbol.clone(), st.config.interval.clone(),
             st.config.db_path.clone(), st.finance.equity)
        };

        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🧠 ML Retrain başladı: sembol={} aralık={} kapital=${:.0}",
                symbol, interval, capital,
            ));
        }

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

        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🧠 ML Retrain: {} mum yüklendi, strateji={}, random search 60 iter çalışıyor...",
                candles.len(), strategy_name,
            ));
        }

        let opt = crate::robot::backtester::parameter_optimizer::ParameterOptimizer::new(
            symbol.clone(), interval.clone(), capital, strategy_name.clone(),
        );
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
        let gbt_ds = crate::robot::ml_engine::build_training_set(
            &candles, gbt_window_bars, gbt_forward_bars,
        );
        if gbt_ds.len() < 20 {
            if let Ok(mut st) = state.lock() {
                st.push_log(format!(
                    "🌲 GBT atlandı: yetersiz training örneği ({} < 20)", gbt_ds.len(),
                ));
            }
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
                    }
                    let acc_str = if oos_acc.is_nan() { "-".into() }
                                  else { format!("{:.1}%", oos_acc) };
                    st.push_log(format!(
                        "🌲 GBT ✓ n_est={n_est} lr={lr:.2} depth={depth} | OOS acc={acc_str} | {} örnek",
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

    /// 🔬 Backtest (Faz 4 - "backtest" trigger):
    /// Daha geniş bir grid ile composite score'u en yüksek olan profili seçer ve
    /// brain.live_strategy'i otonom değiştirir.
    fn run_backtest_job(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        log::info!("🔬 E2: Walk-Forward Backtest başlatıldı (grid: 6×4×3)...");

        let (symbol, interval, db_path, capital) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            (st.config.symbol.clone(), st.config.interval.clone(),
             st.config.db_path.clone(), st.finance.equity)
        };

        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🔬 Backtest başladı: sembol={} aralık={} kapital=${:.0}",
                symbol, interval, capital,
            ));
        }

        // Walk-Forward konfigürasyonu — env'den override edilebilir.
        // Varsayılan IS=200 / OOS=50 / step=50: 1500 mumda ~26 pencere.
        let wf_is   = std::env::var("WALK_FORWARD_IS_BARS").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(200);
        let wf_oos  = std::env::var("WALK_FORWARD_OOS_BARS").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(50);
        let wf_step = std::env::var("WALK_FORWARD_STEP_BARS").ok()
            .and_then(|s| s.parse::<usize>().ok()).unwrap_or(50);
        let wf_min  = wf_is + wf_oos;

        let candles = crate::persistence::reader::read_candles(&db_path, &symbol, &interval, 1500)
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
        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🔬 Backtest (Walk-Forward): {} mum, {} strateji × ~{} pencere (IS={} OOS={} step={})",
                candles.len(), strat_pool.len(), est_windows, wf_is, wf_oos, wf_step,
            ));
        }

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
            };
            let Some(wf_res) = WalkForwardTester::new(wf_cfg).run(&candles) else {
                if let Ok(mut st) = state.lock() {
                    st.push_log(format!("🔬   aday {} → WF sonuç alınamadı", name));
                }
                continue;
            };

            let wf_score = wf_res.avg_oos_sharpe * (1.0 - WF_CONSISTENCY_WEIGHT)
                         + wf_res.consistency_score * WF_CONSISTENCY_WEIGHT;
            if let Ok(mut st) = state.lock() {
                st.push_log(format!(
                    "🔬   aday {} → OOS Sharpe={:.2} Tutarlılık={:.0}% ({} pencere) skor={:.3}",
                    name, wf_res.avg_oos_sharpe,
                    wf_res.consistency_score * 100.0,
                    wf_res.windows.len(),
                    wf_score,
                ));
            }
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
        );
        let final_res = final_opt.optimize_parallel(
            &candles,
            (2.0, 8.0, 1.0),       // TP %2 → %8, step 1
            (1.0, 4.0, 1.0),       // SL %1 → %4, step 1
            (0.1, 0.4, 0.1),       // PS  0.1 → 0.4
        ).map_err(|e| format!("final optimize_parallel: {:?}", e))?;

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
                // Rejim katmanları — PS global, TP/SL rejime özgü.
                for (regime, agg) in &regime_agg {
                    let trade_risk = crate::robot::parameters::TradeRiskParams {
                        take_profit_pct:   agg.median_tp_pct,
                        stop_loss_pct:     agg.median_sl_pct,
                        max_position_size: final_res.best_parameters.max_position_size,
                    };
                    params.set_regime_patch(
                        regime.clone(),
                        crate::robot::parameters::RegimePatch::empty().with_trade_risk(trade_risk),
                    );
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
            // Rejim katmanları log'una tek satırlık özet.
            if regime_agg.is_empty() {
                st.push_log(
                    "🎚  Rejim katmanı yazılmadı — min örneklem altında veya sınıflandırma boş".into(),
                );
            } else {
                let mut entries: Vec<String> = regime_agg.iter()
                    .map(|(r, a)| format!(
                        "{r}(n={}) TP={:.1}% SL={:.1}%",
                        a.sample_count, a.median_tp_pct, a.median_sl_pct,
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
    async fn run_download_job(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        use crate::robot::data_fetcher::binance::BinanceFetcher;
        use crate::robot::data_fetcher::market_fetcher::MarketFetcher;

        log::info!("🌐 E2: Data pipeline download başlatıldı...");

        // 1) Çalışma listesi — kilit kısa
        let (symbols, interval, db_path, limit) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let mut syms: Vec<String> = vec![st.config.symbol.clone()];
            // SymbolOrchestrator + pinned
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for w in orch.get_worker_status() {
                    if !syms.contains(&w.symbol) { syms.push(w.symbol); }
                }
            }
            for s in &st.config.pinned_symbols {
                if !syms.contains(s) { syms.push(s.clone()); }
            }
            syms.retain(|s| !s.is_empty());
            (syms, st.config.interval.clone(), st.config.db_path.clone(),
             st.config.download_candle_limit.max(50))
        };

        if symbols.is_empty() {
            return Err("indirilecek sembol yok (config.symbol + pinned + orchestrator boş)".into());
        }

        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🌐 Download başladı: {} sembol × {} mum (interval={})",
                symbols.len(), limit, interval,
            ));
        }

        // 2) Her sembol için sırayla mum çek + DB'ye yaz
        let fetcher = BinanceFetcher::new();
        let mut total_fetched = 0usize;
        let mut total_failed = 0usize;
        let mut per_symbol_summary: Vec<String> = Vec::new();

        for sym in &symbols {
            match fetcher.fetch_latest(sym, &interval, limit).await {
                Ok(candles) => {
                    let n = candles.len();
                    total_fetched += n;
                    // 3) SQLite yazımı senkron → spawn_blocking
                    let db_path_clone = db_path.clone();
                    let candles_clone = candles.clone();
                    let write_result = tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
                        let conn = rusqlite::Connection::open(&db_path_clone)
                            .map_err(|e| format!("db open: {}", e))?;
                        for c in &candles_clone {
                            let _ = crate::persistence::writer::save_candle(&conn, "binance", "spot", c);
                        }
                        Ok(())
                    }).await;
                    match write_result {
                        Ok(Ok(())) => {
                            per_symbol_summary.push(format!("{}={}", sym, n));
                            if let Ok(mut st) = state.lock() {
                                st.push_log(format!("    └─ {} ✓ {} mum yazıldı", sym, n));
                            }
                        }
                        Ok(Err(e)) => {
                            total_failed += 1;
                            if let Ok(mut st) = state.lock() {
                                st.push_log(format!("    └─ {} ❌ yazma hatası: {}", sym, e));
                            }
                        }
                        Err(e) => {
                            total_failed += 1;
                            if let Ok(mut st) = state.lock() {
                                st.push_log(format!("    └─ {} ❌ blocking join hatası: {}", sym, e));
                            }
                        }
                    }
                }
                Err(e) => {
                    total_failed += 1;
                    if let Ok(mut st) = state.lock() {
                        st.push_log(format!("    └─ {} ❌ fetch hatası: {}", sym, e));
                    }
                }
            }
        }

        // 4) Özet
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
