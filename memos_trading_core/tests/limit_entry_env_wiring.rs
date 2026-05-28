// Maker LIMIT giriş — env-gating doğrulaması.
//
// `USE_LIMIT_ENTRY=1` ve ilgili LIMIT_ENTRY_* / MAKER_COMMISSION_RATE env'lerinin
// gerçek boot yolundan (AppState::new → RuntimeTuning::from_env) doğru okunduğunu
// ve default'ların korunduğunu doğrular.
//
// NOT: Canlı maker HTTP yolu (place_smart_limit_entry → Binance POST_ONLY) gerçek
// API key + TradingMode::Live gerektirir ve gerçek emir gönderir → burada
// çalıştırılmaz. Bu test, runtime'ın bayrağı doğru kapıdan geçirdiğini kanıtlar;
// fiyatlandırma saf-mantığı binance_executor birim testlerinde (maker_limit_price /
// spread_bps) ayrıca kapsanır.

use std::sync::{Mutex, MutexGuard};

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::robotic_loop::AppState;

// LIMIT_ENTRY_* ve COMMISSION_RATE process-global; cargo paralel koşumda set/remove
// yarışını önlemek için dosya-içi mutex ile serileştir. Poison'a into_inner() bağışık.
static ENV_GUARD: Mutex<()> = Mutex::new(());
fn lock_env() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner())
}

fn clear_env() {
    for k in [
        "USE_LIMIT_ENTRY",
        "LIMIT_ENTRY_TIMEOUT_MS",
        "LIMIT_ENTRY_MAX_ATTEMPTS",
        "LIMIT_ENTRY_MAX_SPREAD_BPS",
        "LIMIT_ENTRY_FALLBACK_MARKET",
        "MAKER_COMMISSION_RATE",
        "COMMISSION_RATE",
    ] {
        std::env::remove_var(k);
    }
}

#[test]
fn defaults_when_env_absent() {
    let _env = lock_env();
    clear_env();
    let st = AppState::new(RoboticLoopConfig::default());
    let t = &st.tuning;
    assert!(!t.use_limit_entry, "default: maker giriş kapalı olmalı (opt-in)");
    assert_eq!(t.limit_entry_timeout_ms, 2000, "default timeout 2000ms");
    assert_eq!(t.limit_entry_max_attempts, 3, "default 3 deneme");
    assert!((t.limit_entry_max_spread_bps - 50.0).abs() < 1e-9, "default spread guard 50bps");
    assert!(t.limit_entry_fallback_market, "default: maker dolmazsa market'e düş");
    // MAKER_COMMISSION_RATE yoksa taker commission_rate'e eşitlenir (default 0.001).
    assert!((t.maker_commission_rate - t.commission_rate).abs() < 1e-12,
        "maker komisyon default = taker commission_rate");
    clear_env();
}

#[test]
fn use_limit_entry_enabled_via_env() {
    let _env = lock_env();
    clear_env();
    std::env::set_var("USE_LIMIT_ENTRY", "1");
    let st = AppState::new(RoboticLoopConfig::default());
    assert!(st.tuning.use_limit_entry, "USE_LIMIT_ENTRY=1 → maker giriş açık olmalı");
    clear_env();
}

#[test]
fn use_limit_entry_accepts_true_keyword() {
    let _env = lock_env();
    clear_env();
    std::env::set_var("USE_LIMIT_ENTRY", "true");
    let st = AppState::new(RoboticLoopConfig::default());
    assert!(st.tuning.use_limit_entry, "USE_LIMIT_ENTRY=true → açık (env_truthy)");
    clear_env();
}

#[test]
fn limit_entry_params_parsed_from_env() {
    let _env = lock_env();
    clear_env();
    std::env::set_var("USE_LIMIT_ENTRY", "1");
    std::env::set_var("LIMIT_ENTRY_TIMEOUT_MS", "4500");
    std::env::set_var("LIMIT_ENTRY_MAX_ATTEMPTS", "5");
    std::env::set_var("LIMIT_ENTRY_MAX_SPREAD_BPS", "12.5");
    let st = AppState::new(RoboticLoopConfig::default());
    let t = &st.tuning;
    assert!(t.use_limit_entry);
    assert_eq!(t.limit_entry_timeout_ms, 4500);
    assert_eq!(t.limit_entry_max_attempts, 5);
    assert!((t.limit_entry_max_spread_bps - 12.5).abs() < 1e-9);
    clear_env();
}

#[test]
fn fallback_market_can_be_disabled() {
    let _env = lock_env();
    clear_env();
    std::env::set_var("LIMIT_ENTRY_FALLBACK_MARKET", "0");
    let st = AppState::new(RoboticLoopConfig::default());
    assert!(!st.tuning.limit_entry_fallback_market,
        "LIMIT_ENTRY_FALLBACK_MARKET=0 → maker dolmazsa trade atlanır");
    clear_env();
}

#[test]
fn maker_commission_defaults_to_taker_rate_then_overrides() {
    let _env = lock_env();
    clear_env();
    // MAKER_COMMISSION_RATE set edilmezse taker COMMISSION_RATE'e eşitlenir.
    std::env::set_var("COMMISSION_RATE", "0.0004");
    let st = AppState::new(RoboticLoopConfig::default());
    assert!((st.tuning.maker_commission_rate - 0.0004).abs() < 1e-12,
        "maker komisyon default = taker (0.0004), gerçek: {}", st.tuning.maker_commission_rate);

    // Ayrıca set edilirse onun değeri kullanılır (taker'dan bağımsız).
    std::env::set_var("MAKER_COMMISSION_RATE", "0.0002");
    let st2 = AppState::new(RoboticLoopConfig::default());
    assert!((st2.tuning.maker_commission_rate - 0.0002).abs() < 1e-12,
        "MAKER_COMMISSION_RATE override edilmeli, gerçek: {}", st2.tuning.maker_commission_rate);
    clear_env();
}
