// Scheduler Task Entegrasyon Testi
//
// Faz 2 Task 5: download/backtest tetiklerinin periyodik olarak otomatik atıldığını
// doğrular. Test'te warmup süresi sabit 30 sn → 35-40 sn beklemek gerek.
// Config: pipeline_every_mins = 1 (test için kısa), download_enabled = true,
//         download_every_mins = 999 (uzun → sadece warmup tetiği görelim).

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_fires_warmup_download_after_30s() {
    // Geçici DB — backtest job DB'ye dokunsa da problem olmaz
    let tmp_db = format!("/tmp/memos_sched_test_{}.db", std::process::id());
    let _ = std::fs::remove_file(&tmp_db);

    let config = RoboticLoopConfig {
        symbol: "BTCUSDT".into(),
        interval: "1m".into(),
        db_path: tmp_db.clone(),
        download_enabled: true,
        download_every_mins: 999,  // uzun → sadece warmup tetiği
        pipeline_enabled: false,    // backtest kapalı
        download_candle_limit: 20,
        pinned_symbols: vec![],
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));

    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    // Warmup 30 sn + scheduler tick + trigger handler işlemesi → ~35 sn pencere
    tokio::time::sleep(Duration::from_secs(35)).await;

    let logs: Vec<String> = state.lock().unwrap().guardian.log.iter().cloned().collect();
    let saw_warmup = logs.iter().any(|l| l.contains("⏰ Scheduler: warmup"));
    assert!(saw_warmup, "Warmup download log'u görülmedi. Logs: {:#?}", logs);

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    let _ = std::fs::remove_file(&tmp_db);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_respects_disabled_download() {
    let tmp_db = format!("/tmp/memos_sched_disabled_{}.db", std::process::id());
    let _ = std::fs::remove_file(&tmp_db);

    let config = RoboticLoopConfig {
        db_path: tmp_db.clone(),
        download_enabled: false,    // KAPALI
        download_every_mins: 1,
        pipeline_enabled: false,
        pinned_symbols: vec![],
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));
    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    tokio::time::sleep(Duration::from_secs(33)).await;

    let logs: Vec<String> = state.lock().unwrap().guardian.log.iter().cloned().collect();
    let saw_any_sched = logs.iter().any(|l| l.contains("⏰ Scheduler"));
    assert!(!saw_any_sched,
        "download_enabled=false olduğu halde scheduler tetik atmış. Logs: {:#?}", logs);

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    let _ = std::fs::remove_file(&tmp_db);
}
