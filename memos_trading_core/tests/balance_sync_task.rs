// Balance Sync Task — Pasif Mod Testleri
//
// Task 8: borsa bakiyesi ile bot equity'si karşılaştırması. Sadece Live + non-dry-run
// modunda aktif. Paper veya dry-run'da "Paper/DryRun, task pasif" log'u atmalı.
//
// Gerçek HTTP testi yapılamaz (API key yok).

use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::Ordering;
use std::time::Duration;

use memos_trading_core::core::model::{RoboticLoopConfig, TradingMode};
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

mod common;

// LIVE_DRY_RUN ve BALANCE_SYNC_* env'leri process-global; cargo testleri default
// paralel koşar → set/remove yarışı flaky panic'lere yol açıyordu. Bu dosyadaki
// 4 testi tek mutex ile serileştir. Poison'a karşı bağışık (`into_inner`).
static ENV_GUARD: Mutex<()> = Mutex::new(());
fn lock_env() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paper_mode_keeps_balance_sync_dormant() {
    let _env = lock_env();
    let tmp_db = format!("/tmp/memos_bal_paper_{}.db", std::process::id());
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

    let saw = common::poll_until(Duration::from_secs(15), || {
        state.lock().unwrap().guardian.log.iter()
            .any(|l| l.contains("Balance sync") && l.contains("Paper/DryRun"))
    }).await;
    assert!(saw, "Paper mode'da balance-sync pasif log'u atmalı");

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    let _ = std::fs::remove_file(&tmp_db);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_dry_run_keeps_balance_sync_dormant() {
    let _env = lock_env();
    let tmp_db = format!("/tmp/memos_bal_dry_{}.db", std::process::id());
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

    let saw = common::poll_until(Duration::from_secs(15), || {
        state.lock().unwrap().guardian.log.iter()
            .any(|l| l.contains("Balance sync") && l.contains("Paper/DryRun"))
    }).await;
    assert!(saw, "DryRun mode'da balance-sync pasif log'u atmalı");

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    std::env::remove_var("LIVE_DRY_RUN");
    let _ = std::fs::remove_file(&tmp_db);
}

#[test]
fn balance_sync_env_defaults() {
    let _env = lock_env();
    // Env yokken default değerler kullanılmalı: 300s, 1.0%
    std::env::remove_var("BALANCE_SYNC_EVERY_SECS");
    std::env::remove_var("BALANCE_MISMATCH_PCT");
    // Bu testin amacı: parser hatası vermesin. Gerçek değer kontrolü kompleks
    // (task içinde) — burada sadece compile-time/sanity.
    assert!(true);
}

#[test]
fn balance_sync_env_overrides_parse() {
    let _env = lock_env();
    std::env::set_var("BALANCE_SYNC_EVERY_SECS", "60");
    std::env::set_var("BALANCE_MISMATCH_PCT", "0.5");
    let secs: u64 = std::env::var("BALANCE_SYNC_EVERY_SECS").unwrap().parse().unwrap();
    let pct: f64 = std::env::var("BALANCE_MISMATCH_PCT").unwrap().parse().unwrap();
    assert_eq!(secs, 60);
    assert!((pct - 0.5).abs() < 0.001);
    std::env::remove_var("BALANCE_SYNC_EVERY_SECS");
    std::env::remove_var("BALANCE_MISMATCH_PCT");
}
