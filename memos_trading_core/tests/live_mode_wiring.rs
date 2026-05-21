// Live Mode Skeleton Testleri
//
// AppState'in live_executor/dry_run/max_notional alanlarının env'lerden doğru kuruldukğunu
// ve default davranışın paper-fallback olduğunu doğrular. Gerçek HTTPS testi yok
// (API key gerekir).

use std::sync::{Mutex, MutexGuard};

use memos_trading_core::core::model::{RoboticLoopConfig, TradingMode};
use memos_trading_core::robot::robotic_loop::AppState;

// LIVE_DRY_RUN ve LIVE_MAX_NOTIONAL_USD process-global; cargo paralel koşumda
// set/remove yarışı flaky panic'lere yol açıyordu. Dosya-içi mutex ile 7 testi
// serileştir. Poison'a `into_inner()` ile bağışık.
static ENV_GUARD: Mutex<()> = Mutex::new(());
fn lock_env() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner())
}

fn clear_env() {
    std::env::remove_var("LIVE_DRY_RUN");
    std::env::remove_var("LIVE_MAX_NOTIONAL_USD");
}

#[test]
fn paper_mode_does_not_create_live_executor() {
    let _env = lock_env();
    clear_env();
    let config = RoboticLoopConfig {
        trading_mode: TradingMode::Paper,
        ..Default::default()
    };
    let state = AppState::new(config);
    assert!(state.live_executor.is_none(),
        "TradingMode::Paper'da live_executor None olmalı");
    assert!(!state.live_dry_run, "default dry-run false olmalı");
    assert_eq!(state.live_max_notional_usd, 100.0, "default max notional $100 olmalı");
}

#[test]
fn live_mode_without_api_key_falls_back_to_paper() {
    let _env = lock_env();
    clear_env();
    std::env::remove_var("BINANCE_API_KEY");
    std::env::remove_var("BINANCE_API_SECRET");
    let config = RoboticLoopConfig {
        trading_mode: TradingMode::Live,
        api_key: None,
        secret_key: None,
        ..Default::default()
    };
    let state = AppState::new(config);
    assert!(state.live_executor.is_none(),
        "Live mode + API key yok ⇒ executor None (paper-fallback)");
}

#[test]
fn live_dry_run_env_is_picked_up() {
    let _env = lock_env();
    clear_env();
    std::env::set_var("LIVE_DRY_RUN", "true");
    let state = AppState::new(RoboticLoopConfig::default());
    assert!(state.live_dry_run, "LIVE_DRY_RUN=true env'i okunmalı");
    clear_env();
}

#[test]
fn live_max_notional_env_overrides_default() {
    let _env = lock_env();
    clear_env();
    std::env::set_var("LIVE_MAX_NOTIONAL_USD", "250.5");
    let state = AppState::new(RoboticLoopConfig::default());
    assert!((state.live_max_notional_usd - 250.5).abs() < 0.001,
        "LIVE_MAX_NOTIONAL_USD env'i okunmalı, gerçek: {}",
        state.live_max_notional_usd);
    clear_env();
}

#[test]
fn live_max_notional_invalid_falls_back_to_default() {
    let _env = lock_env();
    clear_env();
    std::env::set_var("LIVE_MAX_NOTIONAL_USD", "abc-invalid");
    let state = AppState::new(RoboticLoopConfig::default());
    assert_eq!(state.live_max_notional_usd, 100.0,
        "geçersiz env değeri default'a düşmeli");
    clear_env();
}

#[test]
fn live_max_notional_negative_clamps_to_zero() {
    let _env = lock_env();
    clear_env();
    std::env::set_var("LIVE_MAX_NOTIONAL_USD", "-50");
    let state = AppState::new(RoboticLoopConfig::default());
    assert_eq!(state.live_max_notional_usd, 0.0,
        "negatif env değeri 0'a clamp edilmeli");
    clear_env();
}

#[test]
fn live_mode_with_dummy_api_key_creates_executor() {
    let _env = lock_env();
    clear_env();
    let config = RoboticLoopConfig {
        trading_mode: TradingMode::Live,
        api_key: Some("dummy_key_for_test".into()),
        secret_key: Some("dummy_secret_for_test".into()),
        ..Default::default()
    };
    let state = AppState::new(config);
    assert!(state.live_executor.is_some(),
        "Live + dummy key ile executor kurulmalı (gerçek emir göndermez, sadece nesne)");
}
