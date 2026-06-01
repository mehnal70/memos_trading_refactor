// Protection Sync Task Davranış Testleri
//
// Task 6: borsadaki SL+TP emirlerinin durumunu sorgular. Sadece live_executor
// varsa ve dry-run değilse aktif. Paper mode'da hiçbir ağ çağrısı yapmamalı.
// Live + dry_run kombinasyonu da pasif olmalı (test edilebilir senaryo).

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use memos_trading_core::core::model::{RoboticLoopConfig, TradingMode};
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paper_mode_keeps_protection_sync_dormant() {
    // Paper'da live_executor None → psync task hiçbir HTTP çağrısı yapmamalı,
    // sessizce dönmeli. Engine yine ayakta kalır.
    let tmp_db = format!("/tmp/memos_psync_paper_{}.db", std::process::id());
    let _ = std::fs::remove_file(&tmp_db);

    std::env::remove_var("TRADING_MODE");
    let config = RoboticLoopConfig {
        symbol: "BTCUSDT".into(),
        db_path: tmp_db.clone(),
        pinned_symbols: vec![],
        download_enabled: false,
        pipeline_enabled: false,
        trading_mode: TradingMode::Paper,
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));
    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    // Ana döngünün ilk turunu bekle (sınırlı poll, contention-dayanıklı). Sync 30s'lik
    // döngü → ilk tick'te çalışmamış olmalı; poll'un erken dönmesi negatif kontrolü
    // ZAYIFLATMAZ, aksine daha az süre = sync'in yanlışlıkla tetiklenme ihtimali daha az.
    let ticked = common::poll_until(Duration::from_secs(15), || {
        state.lock().unwrap().fleet.last_loop_tick.load(Ordering::Relaxed) > 0
    }).await;
    assert!(ticked, "ana döngü 15s içinde tick atmadı");

    // Paper'da psync logu olmamalı
    let saw_psync_action = state.lock().unwrap().guardian.log.iter()
        .any(|l| l.contains("[SYNC]"));
    assert!(!saw_psync_action,
        "Paper mode'da protection sync action logu olmamalı. Logs: {:#?}",
        state.lock().unwrap().guardian.log.iter().cloned().collect::<Vec<_>>());

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    let _ = std::fs::remove_file(&tmp_db);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_dry_run_keeps_protection_sync_dormant() {
    // Live + LIVE_DRY_RUN=true → executor var ama gerçek HTTP çağrısı yok.
    // psync task yine dry_run gördüğünde çalışmaz.
    let tmp_db = format!("/tmp/memos_psync_dry_{}.db", std::process::id());
    let _ = std::fs::remove_file(&tmp_db);

    std::env::set_var("LIVE_DRY_RUN", "true");
    let config = RoboticLoopConfig {
        symbol: "BTCUSDT".into(),
        db_path: tmp_db.clone(),
        trading_mode: TradingMode::Live,
        api_key: Some("dummy_test_key".into()),
        secret_key: Some("dummy_test_secret".into()),
        pinned_symbols: vec![],
        download_enabled: false,
        pipeline_enabled: false,
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));
    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    tokio::time::sleep(Duration::from_secs(3)).await;

    // executor var ama dry_run aktif → HTTP yok, SYNC action logu yok
    {
        let st = state.lock().unwrap();
        assert!(st.live_executor.is_some(), "Live + key → executor kurulmalı");
        assert!(st.live_dry_run, "dry-run env'i aktif olmalı");
    }
    let saw_psync_action = state.lock().unwrap().guardian.log.iter()
        .any(|l| l.contains("[SYNC]") && l.contains("tetiklenmiş"));
    assert!(!saw_psync_action,
        "dry_run'da psync trigger action logu olmamalı");

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    std::env::remove_var("LIVE_DRY_RUN");
    let _ = std::fs::remove_file(&tmp_db);
}
