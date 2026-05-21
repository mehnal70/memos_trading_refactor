// WebSocket userDataStream Skeleton Testleri
//
// Gerçek WS bağlantısı sandbox'ta test edilemez (API key + Binance erişim gerekir).
// Bu testler:
//   1. listenKey URL'sinin spot vs futures için doğru kurulduğunu
//   2. Paper/DryRun modunda WS task'ının pasif kaldığını
// doğrular.

use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::Ordering;
use std::time::Duration;

use memos_trading_core::core::model::{RoboticLoopConfig, TradingMode};
use memos_trading_core::robot::engines::binance_executor::BinanceFuturesExecutor;
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

// LIVE_DRY_RUN process-global; aşağıdaki 2 async test set/remove ederek
// yarışıyordu. Bu testleri dosya-içi mutex ile serileştir. URL testleri
// (3 sync) env'e dokunmuyor, lock'a ihtiyaç duymazlar.
static ENV_GUARD: Mutex<()> = Mutex::new(());
fn lock_env() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner())
}

#[test]
fn user_data_stream_url_spot() {
    let exec = BinanceFuturesExecutor::new_for_market(
        "k".into(), "s".into(), false, "spot",
    );
    let url = exec.user_data_stream_url("test_listen_key_123");
    assert_eq!(url, "wss://stream.binance.com:9443/ws/test_listen_key_123",
        "spot WS URL formatı yanlış: {}", url);
}

#[test]
fn user_data_stream_url_futures_live() {
    let exec = BinanceFuturesExecutor::new_for_market(
        "k".into(), "s".into(), false, "futures",
    );
    let url = exec.user_data_stream_url("xyz_key");
    assert_eq!(url, "wss://fstream.binance.com/ws/xyz_key",
        "futures-live WS URL formatı yanlış: {}", url);
}

#[test]
fn user_data_stream_url_futures_testnet() {
    let exec = BinanceFuturesExecutor::new_for_market(
        "k".into(), "s".into(), true, "futures",
    );
    let url = exec.user_data_stream_url("paper_key");
    assert_eq!(url, "wss://stream.binancefuture.com/ws/paper_key",
        "futures-testnet WS URL formatı yanlış: {}", url);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paper_mode_keeps_ws_task_dormant() {
    let _env = lock_env();
    let tmp_db = format!("/tmp/memos_ws_paper_{}.db", std::process::id());
    let _ = std::fs::remove_file(&tmp_db);

    std::env::remove_var("LIVE_DRY_RUN");
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

    tokio::time::sleep(Duration::from_secs(2)).await;

    let saw_pasif_msg = state.lock().unwrap().guardian.log.iter()
        .any(|l| l.contains("WS userDataStream") && l.contains("Paper/DryRun"));
    assert!(saw_pasif_msg,
        "Paper mode'da WS task'ı 'Paper/DryRun, pasif' log'u atmalı");

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    let _ = std::fs::remove_file(&tmp_db);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_dry_run_keeps_ws_task_dormant() {
    let _env = lock_env();
    let tmp_db = format!("/tmp/memos_ws_dry_{}.db", std::process::id());
    let _ = std::fs::remove_file(&tmp_db);

    std::env::set_var("LIVE_DRY_RUN", "true");
    let config = RoboticLoopConfig {
        symbol: "BTCUSDT".into(),
        db_path: tmp_db.clone(),
        trading_mode: TradingMode::Live,
        api_key: Some("dummy".into()),
        secret_key: Some("dummy".into()),
        pinned_symbols: vec![],
        download_enabled: false,
        pipeline_enabled: false,
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));
    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    tokio::time::sleep(Duration::from_secs(2)).await;

    let saw_pasif_msg = state.lock().unwrap().guardian.log.iter()
        .any(|l| l.contains("WS userDataStream") && l.contains("Paper/DryRun"));
    assert!(saw_pasif_msg,
        "DryRun mode'da WS task pasif log'u atmalı");

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    std::env::remove_var("LIVE_DRY_RUN");
    let _ = std::fs::remove_file(&tmp_db);
}
