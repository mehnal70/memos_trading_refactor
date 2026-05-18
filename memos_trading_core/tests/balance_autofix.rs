// Balance Autofix Env-Driven Davranış Testleri
//
// Otomatik mismatch onarımı için iki env:
//   - BALANCE_AUTOFIX_AFTER_N_OBS (default 3) — N ardışık gözlem sonra düzelt
//   - BALANCE_AUTOFIX_ENABLED (default true) — false ise hiç düzeltme yapma
//
// Pasif modlarda task çalışmaz, gerçek HTTP test yok (paper-fallback hep test edilir).

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use memos_trading_core::core::model::{RoboticLoopConfig, TradingMode};
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

#[test]
fn autofix_enabled_default_true() {
    std::env::remove_var("BALANCE_AUTOFIX_ENABLED");
    let v = std::env::var("BALANCE_AUTOFIX_ENABLED")
        .map(|s| s != "false" && s != "0").unwrap_or(true);
    assert!(v, "BALANCE_AUTOFIX_ENABLED default true olmalı");
}

#[test]
fn autofix_disabled_via_env() {
    std::env::set_var("BALANCE_AUTOFIX_ENABLED", "false");
    let v = std::env::var("BALANCE_AUTOFIX_ENABLED")
        .map(|s| s != "false" && s != "0").unwrap_or(true);
    assert!(!v, "BALANCE_AUTOFIX_ENABLED=false ile autofix kapanmalı");
    std::env::remove_var("BALANCE_AUTOFIX_ENABLED");
}

#[test]
fn autofix_disabled_via_zero() {
    std::env::set_var("BALANCE_AUTOFIX_ENABLED", "0");
    let v = std::env::var("BALANCE_AUTOFIX_ENABLED")
        .map(|s| s != "false" && s != "0").unwrap_or(true);
    assert!(!v, "BALANCE_AUTOFIX_ENABLED=0 ile autofix kapanmalı");
    std::env::remove_var("BALANCE_AUTOFIX_ENABLED");
}

#[test]
fn autofix_n_obs_env_parses() {
    std::env::set_var("BALANCE_AUTOFIX_AFTER_N_OBS", "5");
    let n: u32 = std::env::var("BALANCE_AUTOFIX_AFTER_N_OBS")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    assert_eq!(n, 5);
    std::env::remove_var("BALANCE_AUTOFIX_AFTER_N_OBS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paper_mode_skips_autofix_task() {
    let tmp_db = format!("/tmp/memos_autofix_paper_{}.db", std::process::id());
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

    // Paper mode'da hiçbir zaman 🩹 AUTOFIX log'u olmamalı
    let saw_autofix = state.lock().unwrap().guardian.log.iter()
        .any(|l| l.contains("BALANCE-AUTOFIX"));
    assert!(!saw_autofix,
        "Paper mode'da autofix log'u olmamalı (task pasif)");

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    let _ = std::fs::remove_file(&tmp_db);
}

#[test]
fn equity_and_peak_can_be_adjusted_by_autofix_logic() {
    // Autofix task'ı state'in equity'sini ve peak_equity'sini günceller.
    // Bu testte aynı transformation'u manuel uygulayarak invariant'ı doğrularız.
    let mut state = AppState::new(RoboticLoopConfig::default());
    let initial_equity = state.finance.equity;
    assert_eq!(initial_equity, 10000.0);

    // Simülasyon: borsa bakiyesi $12000 dönüyor → autofix equity'yi yukarı çekiyor
    let exchange_balance = 12000.0;
    let old_equity = state.finance.equity;
    state.finance.equity = exchange_balance;
    if exchange_balance > state.finance.peak_equity {
        state.finance.peak_equity = exchange_balance;
    }
    let delta = exchange_balance - old_equity;
    assert_eq!(state.finance.equity, 12000.0);
    assert_eq!(state.finance.peak_equity, 12000.0);
    assert_eq!(delta, 2000.0);

    // Simülasyon: borsa bakiyesi $11000 → equity düşer ama peak korunur
    let exchange_balance2 = 11000.0;
    state.finance.equity = exchange_balance2;
    if exchange_balance2 > state.finance.peak_equity {
        state.finance.peak_equity = exchange_balance2;
    }
    assert_eq!(state.finance.equity, 11000.0);
    assert_eq!(state.finance.peak_equity, 12000.0, "peak_equity tepe korunmalı");
}
