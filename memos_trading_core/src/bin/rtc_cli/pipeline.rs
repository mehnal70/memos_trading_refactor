//! Pipeline orchestration worker.
//!
//! Faz-bazlı pipeline state machine:
//! `Idle → Download → Backtest → MLTrain → P5Analysis → Done → Idle` (periyodik).
//!
//! Her aşama bir öncekinin tamamlanmasını bekler (timestamp değişimi ile tespiti).
//! Başlangıçta 15s sonra ilk çalıştırma; ardından `pipeline_every_mins` periyodik tekrar.
//!
//! Bu modül rtc_cli (bin) içinde alt modül olarak yer alır; `AppState`, `OtoConfig`,
//! `PipelinePhase` ve `spawn_p5_analysis` gibi parent bağımlılıklarına `super::` ile erişir.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use super::{AppState, OtoConfig, PipelinePhase, spawn_p5_analysis};

pub fn run_pipeline_worker(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
    config:      OtoConfig,
) {
    std::thread::spawn(move || {
        // Önceki aşamanın başladığı andaki referans zaman damgaları (değişim tespiti için)
        let mut ref_download_ts  = String::new();
        let mut ref_backtest_ts  = String::new();
        let mut ref_ml_ts        = String::new();
        let mut ref_p5_ts        = String::new();

        // Fase bazlı zaman aşımı sabitleri (saniye)
        // Download: büyük geçmiş çekme 10+ dk sürebilir; 15 dk güvenli timeout
        const TIMEOUT_DOWNLOAD:  u64 = 900;  // 15 dk
        const TIMEOUT_BACKTEST:  u64 = 300;  // 5 dk
        const TIMEOUT_ML:        u64 = 300;  // 5 dk
        const TIMEOUT_P5:        u64 = 600;  // 10 dk / sembol
        const POLL_SECS:         u64 = 5;

        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }
            std::thread::sleep(std::time::Duration::from_secs(POLL_SECS));
            if stop_signal.load(Ordering::Relaxed) { break; }

            // ── Pipeline devre dışı veya init tamamlanmadıysa bekle ─────────
            let init_done = {
                app_state.lock().ok()
                    .map(|st| st.init_complete.load(Ordering::Relaxed))
                    .unwrap_or(false)
            };
            if !init_done { continue; }

            // ── Mevcut faz ve tetik durumunu oku ───────────────────────────
            let (phase, triggered, enabled, next_at) = {
                if let Ok(st) = app_state.lock() {
                    let trig = st.pipeline.trigger.swap(false, Ordering::Relaxed);
                    (
                        st.pipeline.phase.clone(),
                        trig,
                        st.pipeline.enabled,
                        st.pipeline.next_run_at,
                    )
                } else { continue; }
            };

            if !enabled { continue; }

            let time_up = next_at <= std::time::Instant::now();

            match phase {
                // ── IDLE: tetik veya zaman gelince Download'a geç ───────────
                PipelinePhase::Idle => {
                    if !triggered && !time_up { continue; }

                    // Referans zaman damgalarını kaydet (değişim tespiti için)
                    if let Ok(st) = app_state.lock() {
                        ref_download_ts = st.last_download.clone().unwrap_or_default();
                        ref_backtest_ts = st.last_backtest_at.clone().unwrap_or_default();
                        ref_ml_ts       = st.last_ml_train.clone().unwrap_or_default();
                        ref_p5_ts       = st.p5_last_status.as_ref()
                            .map(|s| s.ts.clone()).unwrap_or_default();
                    }
                    // Download'u tetikle
                    if let Ok(mut st) = app_state.lock() {
                        st.download_trigger.store(true, Ordering::Relaxed);
                        st.pipeline.phase            = PipelinePhase::Download;
                        st.pipeline.phase_started_at = Some(std::time::Instant::now());
                        st.push_log("🔄 [Pipeline] ─── Başladı: ⬇ İndirme aşaması".to_string());
                    }
                }

                // ── DOWNLOAD: tamamlanınca Backtest'e geç ──────────────────
                PipelinePhase::Download => {
                    let (done, elapsed, still_active) = {
                        if let Ok(st) = app_state.lock() {
                            let changed = st.last_download.as_deref()
                                .unwrap_or("") != ref_download_ts.as_str();
                            let elapsed = st.pipeline.phase_started_at
                                .map(|t| t.elapsed().as_secs()).unwrap_or(0);
                            (changed, elapsed, st.download_active)
                        } else { (false, 0, false) }
                    };
                    // İndirme aktif çalışıyorsa bekle — ama timeout dolunca zorla geç
                    // (download thread panic'lerse still_active kalıcı true olabilir;
                    //  timeout override etmezsek pipeline sonsuza kilitlenir).
                    if still_active && elapsed < TIMEOUT_DOWNLOAD { continue; }
                    if !done && elapsed < TIMEOUT_DOWNLOAD { continue; }

                    // Timeout ile çıkıldıysa ve download hâlâ aktif görünüyorsa bayrağı sıfırla
                    if still_active {
                        if let Ok(mut st) = app_state.lock() {
                            st.download_active = false;
                        }
                    }
                    let reason = if done { "tamamlandı" } else { "zaman aşımı (download_active sıfırlandı)" };
                    if let Ok(mut st) = app_state.lock() {
                        ref_backtest_ts = st.last_backtest_at.clone().unwrap_or_default();
                        st.backtest_trigger.store(true, Ordering::Relaxed);
                        st.pipeline.phase            = PipelinePhase::Backtest;
                        st.pipeline.phase_started_at = Some(std::time::Instant::now());
                        st.push_log(format!("🔄 [Pipeline] İndirme {} → 🔬 Backtest", reason));
                    }
                }

                // ── BACKTEST: tamamlanınca ML'e geç ────────────────────────
                PipelinePhase::Backtest => {
                    let (done, elapsed) = {
                        if let Ok(st) = app_state.lock() {
                            let changed = st.last_backtest_at.as_deref()
                                .unwrap_or("") != ref_backtest_ts.as_str();
                            let elapsed = st.pipeline.phase_started_at
                                .map(|t| t.elapsed().as_secs()).unwrap_or(0);
                            (changed, elapsed)
                        } else { (false, 0) }
                    };
                    if !done && elapsed < TIMEOUT_BACKTEST { continue; }

                    let reason = if done { "tamamlandı" } else { "zaman aşımı" };
                    if let Ok(mut st) = app_state.lock() {
                        ref_ml_ts = st.last_ml_train.clone().unwrap_or_default();
                        st.ml_trigger.store(true, Ordering::Relaxed);
                        st.pipeline.phase            = PipelinePhase::MLTrain;
                        st.pipeline.phase_started_at = Some(std::time::Instant::now());
                        st.push_log(format!("🔄 [Pipeline] Backtest {} → 🧠 ML Eğitim", reason));
                    }
                }

                // ── ML TRAIN: tamamlanınca P5'e geç ────────────────────────
                PipelinePhase::MLTrain => {
                    let (done, elapsed) = {
                        if let Ok(st) = app_state.lock() {
                            let changed = st.last_ml_train.as_deref()
                                .unwrap_or("") != ref_ml_ts.as_str();
                            let elapsed = st.pipeline.phase_started_at
                                .map(|t| t.elapsed().as_secs()).unwrap_or(0);
                            (changed, elapsed)
                        } else { (false, 0) }
                    };
                    if !done && elapsed < TIMEOUT_ML { continue; }

                    let reason = if done { "tamamlandı" } else { "zaman aşımı" };
                    // P5 için sembol listesini hazırla: aktif + top_n aday
                    if let Ok(mut st) = app_state.lock() {
                        let (ae, am, asym, aint) = st.active_trade_target();
                        let top_n = st.pipeline.p5_top_n;
                        let mut syms: Vec<(String,String,String,String)> =
                            vec![(ae.clone(), am.clone(), asym.clone(), aint.clone())];
                        for cand in st.symbol_candidates.iter()
                            .filter(|c| c.score > 0.0 && c.symbol != asym)
                            .take(top_n.saturating_sub(1))
                        {
                            syms.push((
                                cand.exchange.clone(),
                                cand.market.clone(),
                                cand.symbol.clone(),
                                cand.interval.clone(),
                            ));
                        }
                        st.pipeline.p5_symbols  = syms.clone();
                        st.pipeline.p5_sym_idx  = 0;
                        st.pipeline.phase            = PipelinePhase::P5Analysis;
                        st.pipeline.phase_started_at = Some(std::time::Instant::now());
                        st.push_log(format!(
                            "🔄 [Pipeline] ML {} → 🐍 P5 Analiz ({} sembol)",
                            reason, syms.len()
                        ));
                    }
                    // İlk sembolü lock bırakıldıktan sonra başlat
                    {
                        let first = {
                            if let Ok(st) = app_state.lock() {
                                st.pipeline.p5_symbols.first().cloned()
                            } else { None }
                        };
                        if let Some((e0, m0, s0, i0)) = first {
                            ref_p5_ts = app_state.lock().ok()
                                .and_then(|st| st.p5_last_status.as_ref().map(|s| s.ts.clone()))
                                .unwrap_or_default();
                            spawn_p5_analysis(&app_state, &s0, &m0, &e0, &i0, &config.db_path);
                        }
                    }
                }

                // ── P5 ANALYSIS: her sembol tamamlanınca bir sonrakine geç ─
                PipelinePhase::P5Analysis => {
                    let (done, elapsed, sym_idx, total_syms) = {
                        if let Ok(st) = app_state.lock() {
                            let p5_done = st.p5_last_status.as_ref()
                                .map(|s| s.state == "done" || s.state == "error")
                                .unwrap_or(false);
                            let p5_changed = st.p5_last_status.as_ref()
                                .map(|s| s.ts != ref_p5_ts && (s.state == "done" || s.state == "error"))
                                .unwrap_or(false);
                            let elapsed = st.pipeline.phase_started_at
                                .map(|t| t.elapsed().as_secs()).unwrap_or(0);
                            (p5_done && p5_changed, elapsed, st.pipeline.p5_sym_idx, st.pipeline.p5_symbols.len())
                        } else { (false, 0, 0, 0) }
                    };

                    if !done && elapsed < TIMEOUT_P5 { continue; }

                    let next_idx = sym_idx + 1;
                    if next_idx < total_syms {
                        // Sonraki sembol için P5 başlat
                        let (e1, m1, s1, i1, db1) = {
                            if let Ok(st) = app_state.lock() {
                                let sym = &st.pipeline.p5_symbols[next_idx];
                                (sym.0.clone(), sym.1.clone(), sym.2.clone(),
                                 sym.3.clone(), config.db_path.clone())
                            } else { continue; }
                        };
                        if let Ok(mut st) = app_state.lock() {
                            st.pipeline.p5_sym_idx  = next_idx;
                            st.pipeline.phase_started_at = Some(std::time::Instant::now());
                            st.push_log(format!(
                                "🔄 [Pipeline] P5 {}/{} tamamlandı → {} başlatıldı",
                                sym_idx + 1, total_syms, s1
                            ));
                        }
                        ref_p5_ts = app_state.lock().ok()
                            .and_then(|st| st.p5_last_status.as_ref().map(|s| s.ts.clone()))
                            .unwrap_or_default();
                        spawn_p5_analysis(&app_state, &s1, &m1, &e1, &i1, &db1);
                    } else {
                        // Tüm semboller tamamlandı → Done
                        let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
                        if let Ok(mut st) = app_state.lock() {
                            let every    = st.pipeline.every_mins;
                            let runs     = st.pipeline.runs_completed + 1;
                            st.pipeline.phase          = PipelinePhase::Done;
                            st.pipeline.last_run_at    = Some(now_str.clone());
                            st.pipeline.runs_completed = runs;
                            st.pipeline.next_run_at    =
                                std::time::Instant::now() +
                                std::time::Duration::from_secs(every * 60);
                            st.push_log(format!(
                                "✅ [Pipeline] Tamamlandı ({} sembol, çalışma #{}) → sonraki: {} dk sonra",
                                total_syms, runs, every
                            ));
                        }
                    }
                }

                // ── DONE: periyodik tekrar için Idle'a dön ─────────────────
                PipelinePhase::Done => {
                    if time_up || triggered {
                        if let Ok(mut st) = app_state.lock() {
                            st.pipeline.phase = PipelinePhase::Idle;
                            // Idle → hemen bir sonraki POLL_SECS'te tetik alacak
                            st.pipeline.next_run_at = std::time::Instant::now();
                        }
                    }
                }
            }
        }
    });
}
