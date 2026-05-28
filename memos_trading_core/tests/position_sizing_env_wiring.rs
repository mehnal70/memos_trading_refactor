// Pozisyon boyutlama sabitleri — env-gating doğrulaması.
//
// BASE_ALLOC_FRACTION (0.10) ve ALLOC_FLOOR_FRACTION (0.25) eskiden positions.rs'de
// gömülüydü; artık RuntimeTuning üzerinden env'den okunuyor. Gerçek boot yolundan
// (AppState::new → RuntimeTuning::from_env) doğru parse + geçersizde default'a düşme.

use std::sync::{Mutex, MutexGuard};

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::robotic_loop::AppState;

static ENV_GUARD: Mutex<()> = Mutex::new(());
fn lock_env() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner())
}

fn clear_env() {
    std::env::remove_var("BASE_ALLOC_FRACTION");
    std::env::remove_var("ALLOC_FLOOR_FRACTION");
}

#[test]
fn defaults_match_previous_hardcoded_values() {
    let _env = lock_env();
    clear_env();
    let t = &AppState::new(RoboticLoopConfig::default()).tuning;
    assert!((t.base_alloc_fraction - 0.10).abs() < 1e-12, "default base alloc %10 olmalı");
    assert!((t.alloc_floor_fraction - 0.25).abs() < 1e-12, "default floor 0.25 olmalı");
    clear_env();
}

#[test]
fn fractions_parsed_from_env() {
    let _env = lock_env();
    clear_env();
    std::env::set_var("BASE_ALLOC_FRACTION", "0.05");
    std::env::set_var("ALLOC_FLOOR_FRACTION", "0.50");
    let t = &AppState::new(RoboticLoopConfig::default()).tuning;
    assert!((t.base_alloc_fraction - 0.05).abs() < 1e-12);
    assert!((t.alloc_floor_fraction - 0.50).abs() < 1e-12);
    clear_env();
}

#[test]
fn invalid_or_nonpositive_base_falls_back_to_default() {
    let _env = lock_env();
    clear_env();
    // base_alloc 0/negatif/geçersiz → default 0.10 (sıfır pozisyon boyutu riskini önler)
    std::env::set_var("BASE_ALLOC_FRACTION", "0");
    std::env::set_var("ALLOC_FLOOR_FRACTION", "abc");
    let t = &AppState::new(RoboticLoopConfig::default()).tuning;
    assert!((t.base_alloc_fraction - 0.10).abs() < 1e-12, "0 → default'a düşmeli");
    assert!((t.alloc_floor_fraction - 0.25).abs() < 1e-12, "geçersiz → default'a düşmeli");
    clear_env();
}

#[test]
fn starting_capital_default_is_ten_thousand() {
    // STARTING_CAPITAL binary main'lerde config.capital'e bağlanır; default kaynağı
    // RoboticLoopConfig::default().capital. Burada default'un beklenen değerde
    // kaldığını sabitleriz (binary parse'ı bu default'a düşer).
    assert!((RoboticLoopConfig::default().capital - 10_000.0).abs() < 1e-9);
}
