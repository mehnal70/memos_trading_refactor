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
            st.push_log("⚡ Altyapı filosu sevk edildi: heartbeat(1s) · phase(2s) · price-poll(5s) · trigger(250ms) · scheduler(60s)".into());
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

                if let Ok(mut st) = st_px.lock() {
                    if let Ok(mut prices) = st.fleet.live_price.write() {
                        for (sym, px) in &new_prices { prices.insert(sym.clone(), *px); }
                    }
                    if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                        let status = if errors.is_empty() { StepStatus::Done } else { StepStatus::Failed };
                        pipe.record_step("price_poll", status, now_secs, 0);
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
            let started_at = std::time::Instant::now();
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
                    let now_secs = started_at.elapsed().as_secs();

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
                                        Ok(Ok(())) => {}
                                        Ok(Err(e)) => {
                                            log::warn!("🔬 Backtest başarısız: {}", e);
                                            if let Ok(mut st) = state_clone.lock() {
                                                st.push_log(format!("❌ Backtest başarısız: {}", e));
                                            }
                                            final_status = StepStatus::Failed;
                                        }
                                        Err(e) => {
                                            log::warn!("🔬 Backtest join hatası: {}", e);
                                            final_status = StepStatus::Failed;
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
            let started_at = std::time::Instant::now();
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
                    let now_secs = started_at.elapsed().as_secs();
                    if let Ok(st) = st_pipe.lock() {
                        if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                            pipe.record_step(
                                format!("phase:{}", current_phase),
                                StepStatus::Done,
                                now_secs,
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

        // 2) Risk yöneticisi (her tur taze policy; AppState'e taşımak istenirse daha sonra cache'lenir).
        let risk_manager = crate::robot::risk::RiskManager::new();

        for symbol in candidates {
            let candles = match crate::persistence::reader::read_candles(&db_path, &symbol, &interval, 200) {
                Ok(c) if !c.is_empty() => c,
                _ => continue,
            };

            // === 1.5) AÇIK POZİSYON İSE: önce SL/TP/Trailing/Breakeven denetle ===
            let live_price = candles.last().map(|c| c.close).unwrap_or(0.0);
            let atr_value  = Self::calc_atr(&candles, 14);
            let exit_reason = {
                let st = match state.lock() { Ok(s) => s, Err(_) => continue };
                let atr_mult = st.brain.best_params.get("pos_atr_trail_mult").copied().unwrap_or(2.0);
                let be_rr    = st.brain.best_params.get("pos_breakeven_at_rr").copied().unwrap_or(1.0);
                let reason_opt = if let Ok(mut positions) = st.finance.live_positions.write() {
                    if let Some(pos) = positions.get_mut(&symbol) {
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
                Self::close_paper_position(state, &symbol, &candles, reason);
                continue; // bu sembolde tur bitti, yeniden açılış aynı turda denenmesin
            }

            // 3) Strateji seçimi: brain.live_strategy "Default"/"AUTO" ise rejime göre otonom seç.
            let strategy_name = if live_strategy.eq_ignore_ascii_case("default")
                                  || live_strategy.eq_ignore_ascii_case("auto")
                                  || live_strategy.is_empty() {
                let sel = crate::robot::ml_engine::strategy_selector::StrategySelector::new();
                sel.select_best(&candles, &crate::core::types::StrategyParams::default()).to_string()
            } else {
                live_strategy.clone()
            };

            // "IDLE_PROTECT" / "IDLE" gibi savunma rejimlerinde sinyal üretme.
            if strategy_name.to_uppercase().starts_with("IDLE") { continue; }

            let strategy = crate::robot::logic::optimizer::make_strategy_pub(&strategy_name);
            let strat_params = crate::core::types::StrategyParams::default();

            let signal = match strategy.generate_signal(&candles, &strat_params, None, None) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // 4) Edge skoru: momentum + ML confidence uyumu. Yön uyuşmazsa edge düşer.
            let edge = Self::compute_edge_score(&candles, &signal, ml_confidence);
            const EDGE_THRESHOLD: f64 = 0.55;

            let has_position = {
                let st = match state.lock() { Ok(s) => s, Err(_) => continue };
                st.finance.live_positions.read().map(|p| p.contains_key(&symbol)).unwrap_or(false)
            };

            let signal_label = match signal {
                Signal::Buy => "BUY", Signal::Sell => "SELL", Signal::Hold => "HOLD",
            };

            match (signal, has_position) {
                // Pozisyon yokken: yalnız yüksek edge'de açılış denenir.
                (crate::core::types::Signal::Buy, false) | (crate::core::types::Signal::Sell, false) => {
                    if edge < EDGE_THRESHOLD {
                        // Spam'i kısmak için sadece eşiğe yakın aday sinyalleri logla (>= 0.40)
                        if edge >= 0.40 {
                            if let Ok(mut st) = state.lock() {
                                st.push_log(format!(
                                    "📊 {} {} edge={:.2} eşik={:.2} ⇒ REDDEDİLDİ (zayıf edge, strat={})",
                                    symbol, signal_label, edge, EDGE_THRESHOLD, strategy_name,
                                ));
                            }
                        }
                        continue;
                    }
                    let authorized = risk_manager.authorize(&signal, snap);
                    if !authorized {
                        if let Ok(mut st) = state.lock() {
                            st.push_log(format!(
                                "🛡️ {} {} edge={:.2} ✓ ama RiskManager VETO etti (Guardrails/Kelly/Gate/VaR)",
                                symbol, signal_label, edge,
                            ));
                        }
                        continue;
                    }
                    if let Ok(mut st) = state.lock() {
                        st.push_log(format!(
                            "📊 {} {} edge={:.2} ✓ + risk ✓ ⇒ POZİSYON AÇILIYOR (strat={})",
                            symbol, signal_label, edge, strategy_name,
                        ));
                    }
                    Self::open_paper_position(state, &symbol, &signal, &candles);
                }
                // Pozisyon varken ters sinyal → kapanış (edge filtresi gevşek).
                (crate::core::types::Signal::Sell, true) | (crate::core::types::Signal::Buy, true) => {
                    if let Ok(mut st) = state.lock() {
                        st.push_log(format!(
                            "🔄 {} açık pozisyon + {} sinyali (edge={:.2}) ⇒ KAPANIŞ",
                            symbol, signal_label, edge,
                        ));
                    }
                    Self::close_paper_position(state, &symbol, &candles, ExitReason::StrategySignal);
                }
                _ => {}
            }
        }
    }

    /// Edge skoru: son 20 mumun fiyat momentumu (-1..+1) ile ML confidence (0..1) ortalaması.
    /// Sinyal yönü momentum ile uyumlu değilse ceza uygulanır.
    fn compute_edge_score(candles: &[Candle], signal: &Signal, ml_confidence: f64) -> f64 {
        if candles.len() < 20 { return 0.0; }
        let recent = &candles[candles.len() - 20..];
        let first = recent.first().map(|c| c.close).unwrap_or(0.0);
        let last  = recent.last().map(|c| c.close).unwrap_or(0.0);
        if first <= 0.0 { return 0.0; }
        let mom = ((last - first) / first).clamp(-1.0, 1.0); // göreli getiri
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
        (dir_match * (mom.abs() * mom_w + ml * ml_w)).clamp(0.0, 1.0)
    }

    /// 🧬 FAZ F3: PAPER-MODE OTONOM POZİSYON AÇILIŞ MOTORU
    /// Kelly oranı, brain.ml_confidence ve loss_streak ile dinamik tahsisat yapar.
    fn open_paper_position(state: &Arc<Mutex<AppState>>, symbol: &str, signal: &Signal, candles: &[Candle]) {
        use crate::robot::risk::kelly::KellyCriterion;
        let last_candle = match candles.last() { Some(c) => c, None => return };
        let mut st = state.lock().unwrap();
        // Açılış aktif olduğu turda fazı "Executing" yap (bir sonraki tur Scanning'e döner).
        st.fleet.phase = "Executing".into();

        // Kasanın iştahı (drawdown ↑ ise küçülür) × Kelly dinamik ölçek
        let risk_appetite = st.finance.calculate_risk_appetite();
        let ml_conf = st.brain.ml_confidence;
        // Loss streak: son 5 kapanan işlem
        let loss_streak = st.finance.live_closed_trades.read()
            .map(|tr| tr.iter().rev().take(5).filter(|t| t.pnl < 0.0).count())
            .unwrap_or(0);

        // Win istatistikleri (varsayılan: 50/50, +1/-1 USD).
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

        // Temel tahsisat: equity'nin %10'u, sonra Kelly dinamik ölçek ile çarpılır.
        let base_alloc = st.finance.equity * 0.10 * risk_appetite;
        let alloc_capital = kelly.calculate_dynamic_scale(base_alloc, loss_streak, ml_conf)
            .max(base_alloc * 0.25);
        let qty_val = (alloc_capital / last_candle.close).max(0.0);
        if qty_val <= 0.0 { return; }

        let is_long = matches!(signal, Signal::Buy);
        let entry = last_candle.close;

        // SL/TP yüzdeleri: brain.best_params'tan oku (ML retrain bunu günceller).
        // Yoksa muhafazakar varsayılan: TP %3, SL %1.5 (RR ≈ 2.0).
        let tp_pct = st.brain.best_params.get("take_profit_pct").copied().unwrap_or(3.0).max(0.1);
        let sl_pct = st.brain.best_params.get("stop_loss_pct").copied().unwrap_or(1.5).max(0.1);
        let (stop_loss, take_profit) = if is_long {
            (entry * (1.0 - sl_pct / 100.0), entry * (1.0 + tp_pct / 100.0))
        } else {
            (entry * (1.0 + sl_pct / 100.0), entry * (1.0 - tp_pct / 100.0))
        };

        // Trailing stop için ilk seviye: ATR × atr_trail_mult kadar uzakta başlat.
        let atr = Self::calc_atr(candles, 14);
        let atr_mult = st.brain.best_params.get("pos_atr_trail_mult").copied().unwrap_or(2.0);
        let trailing_stop = if is_long { entry - atr * atr_mult }
                            else       { entry + atr * atr_mult };

        // IntelligenceHub eşlemesi: pos_id + market regime + strateji adı.
        let pos_id = crate::core::types::PositionId::new();
        let pos_id_str = pos_id.to_string();
        let regime = Self::classify_regime(candles);
        let strategy_name = st.brain.live_strategy.read()
            .map(|s| s.clone()).unwrap_or_else(|_| "MA_CROSSOVER".into());

        let new_pos = PositionModel {
            pos_id: pos_id_str.clone(),
            symbol: symbol.to_string(),
            entry_price: entry,
            current_price: entry,
            qty: qty_val,
            leverage: 1.0,
            trade_type: if is_long { "LONG".into() } else { "SHORT".into() },
            is_long,
            opened_at: chrono::Utc::now().to_rfc3339(),
            stop_loss,
            take_profit,
            trailing_stop,
            max_favorable_price: entry,
            breakeven_activated: false,
        };

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

        st.push_log(format!(
            "🚀 [PAPER-{}] {} açıldı @ {:.2} | Qty={:.4} ${:.2} | SL={:.2} TP={:.2} Trail={:.2} (ATR={:.4} ×{:.1})",
            if is_long { "BUY" } else { "SELL" },
            symbol, entry, qty_val, alloc_capital,
            stop_loss, take_profit, trailing_stop, atr, atr_mult,
        ));
        st.push_log(format!(
            "    └─ Kelly f*={:.3} · risk_iştah={:.2} · ML={:.2} · TP%={:.2} SL%={:.2} · Rejim={} · Strat={}",
            kelly.kelly_fraction, risk_appetite, ml_conf, tp_pct, sl_pct,
            regime.as_str(), strategy_name,
        ));
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
        let mut should_retrain = false;
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

        if should_retrain {
            if let Some(t) = st.fleet.triggers.get("ml") {
                t.store(true, Ordering::Relaxed);
            }
            st.push_log(format!(
                "🧠 Hub: drift={:.3} eşik aşıldı ⇒ ml retrain tetiklendi (cycle={})",
                drift_score, controller_cycle,
            ));
            // Repair log'a da düşür
            st.guardian.repair_log.push_back(format!(
                "[{}] hub: drift-driven retrain (drift={:.3})",
                chrono::Local::now().format("%H:%M:%S"), drift_score,
            ));
            while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
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

    /// 🧬 FAZ F3: PAPER-MODE OTONOM POZİSYON KAPATMA MOTORU (Borrow Checker Tahkimatlı)
    fn close_paper_position(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        candles: &[Candle],
        reason: ExitReason,
    ) {
        let last_candle = match candles.last() { Some(c) => c, None => return };
        let mut st = state.lock().unwrap();
        st.fleet.phase = "Executing".into();

        // [DÜZELTME]: Pozisyon silme (remove) işlemi kendi izole skopuna alındı
        let target_pos = {
            if let Ok(mut positions) = st.finance.live_positions.write() {
                positions.remove(symbol)
            } else {
                None
            }
        }; // Kilit koruyucu bu satırda düşer, hafıza çiti temizlenir.

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
            };

            // [DÜZELTME]: Arşiv listesine itme işlemi izole skopa alındı
            {
                if let Ok(mut closed_list) = st.finance.live_closed_trades.write() {
                    closed_list.push(closed_trade.clone());
                }
            }

            st.push_log(format!(
                "{} [PAPER-CLOSE/{}] {} kapatıldı @ {:.2} (entry={:.2}) | Net PnL: {:.2} USDT ({:+.2}%)",
                reason.emoji(), reason.as_str(), symbol, exit_price, pos.entry_price, pnl_val, pnl_pct_val,
            ));

            // IntelligenceHub.learn_from_exit — track_trade'de açılışta hangi rejim/strateji ile
            // mühürlediysek, kazanç/kayıp uçtan uca o eşleştirmeye gider.
            if !pos.pos_id.is_empty() {
                let pid = crate::core::types::PositionId::from_str_or_new(&pos.pos_id);
                let mut hub_summary: Option<(usize, String)> = None;
                if let Ok(mut hub) = st.brain.intelligence_hub.write() {
                    hub.learn_from_exit(pid, pnl_pct_val);
                    hub_summary = Some((hub.controller.consecutive_failures, hub.controller.state.to_string()));
                }
                if let Some((cf, controller_state)) = hub_summary {
                    st.push_log(format!(
                        "🧠 Hub öğrendi: pos_id={}… pnl={:+.2}% · ardışık kayıp={} · controller={}",
                        &pos.pos_id[..pos.pos_id.len().min(8)],
                        pnl_pct_val, cf, controller_state,
                    ));
                }
            }

            drop(st); // Q-Table alt işçisi çağrılmadan önce ana kilit tamamen imha edilir (Fail-Safe)
            Self::update_cognitive_memory(state, &closed_trade);
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

        // 2) brain.best_params'a yaz + ml_confidence güncelle (sharpe normalize).
        let conf = (result.best_result.sharpe_ratio / 3.0).clamp(0.0, 1.0);
        {
            let mut st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.brain.best_params.insert("take_profit_pct".into(), result.best_parameters.take_profit_pct);
            st.brain.best_params.insert("stop_loss_pct".into(),   result.best_parameters.stop_loss_pct);
            st.brain.best_params.insert("max_position_size".into(), result.best_parameters.max_position_size);
            st.brain.best_params.insert("ml_score".into(), result.best_result.sharpe_ratio);
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

        let candles = crate::persistence::reader::read_candles(&db_path, &symbol, &interval, 1500)
            .map_err(|e| format!("read_candles: {}", e))?;
        if candles.len() < 100 {
            return Err(format!("yetersiz mum verisi: {} mum", candles.len()));
        }

        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🔬 Backtest: {} mum yüklendi, 4 strateji × 18 parametre kombinasyonu = 72 senaryo",
                candles.len(),
            ));
        }

        // Çoklu stratejide aday seç — rejime göre.
        let strat_pool = ["MA_CROSSOVER", "SUPERTREND", "RSI", "MACD"];
        let mut best_overall: Option<(String, f64,
            crate::robot::backtester::parameter_optimizer::OptimizationResult)> = None;

        for name in &strat_pool {
            let opt = crate::robot::backtester::parameter_optimizer::ParameterOptimizer::new(
                symbol.clone(), interval.clone(), capital, (*name).to_string());
            // TP, SL, PositionSize gridleri
            let res = opt.optimize_parallel(
                &candles,
                (2.0, 8.0, 1.0),       // TP %2 → %8, step 1
                (1.0, 4.0, 1.0),       // SL %1 → %4, step 1
                (0.1, 0.4, 0.1),       // PS  0.1 → 0.4
            );
            if let Ok(r) = res {
                let score = r.best_result.sharpe_ratio;
                let win_rate = r.best_result.win_rate;
                let pf = r.best_result.profit_factor;
                if let Ok(mut st) = state.lock() {
                    st.push_log(format!(
                        "🔬   aday {} → Sharpe={:.2} WR={:.1}% PF={:.2}",
                        name, score, win_rate, pf,
                    ));
                }
                if best_overall.as_ref().map(|(_, s, _)| *s).unwrap_or(f64::NEG_INFINITY) < score {
                    best_overall = Some(((*name).to_string(), score, r));
                }
            } else if let Ok(mut st) = state.lock() {
                st.push_log(format!("🔬   aday {} → sonuç alınamadı", name));
            }
        }

        let (best_name, best_score, best_res) = best_overall
            .ok_or_else(|| "Hiçbir strateji aday sonuç üretemedi".to_string())?;

        // brain.live_strategy ve best_params güncellenir.
        {
            let mut st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            if let Ok(mut s) = st.brain.live_strategy.write() {
                *s = best_name.clone();
            }
            st.brain.best_params.insert("take_profit_pct".into(),
                best_res.best_parameters.take_profit_pct);
            st.brain.best_params.insert("stop_loss_pct".into(),
                best_res.best_parameters.stop_loss_pct);
            st.brain.best_params.insert("max_position_size".into(),
                best_res.best_parameters.max_position_size);
            st.brain.hyperopt_score = best_score;
            st.push_log(format!(
                "🔬 Backtest ✓ aktif strateji '{}' (Sharpe={:.2}, WR={:.1}%, PF={:.2})",
                best_name, best_score,
                best_res.best_result.win_rate, best_res.best_result.profit_factor,
            ));
        }

        // Profil de diske mühürlenir.
        let snapshot = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            serde_json::json!({
                "active_strategy": best_name,
                "params": st.brain.best_params,
                "score": best_score,
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
