// Stale-feed kapısı — env-gating + freshness mantığı doğrulaması.
//
// Feed pratikte ölmüş sembol (mum günlerce eski) için YENİ açılış engellenir.
// BTCUSDC canlı audit'inde mum ~63 gün eskiydi + live_price donuk → phantom
// giriş/çıkış. Burada hem RuntimeTuning eşiğinin boot yolundan okunduğunu hem de
// candle_is_fresh_within kararının doğru olduğunu sabitleriz.

use std::sync::{Mutex, MutexGuard};

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::engines::master::candle_is_fresh_within;
use memos_trading_core::robot::robotic_loop::AppState;

static ENV_GUARD: Mutex<()> = Mutex::new(());
fn lock_env() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner())
}

#[test]
fn default_threshold_is_one_hour() {
    let _env = lock_env();
    std::env::remove_var("STALE_FEED_MAX_AGE_SECS");
    let t = &AppState::new(RoboticLoopConfig::default()).tuning;
    assert_eq!(t.stale_feed_max_age_secs, 3600, "default 1 saat (3600sn) olmalı");
}

#[test]
fn threshold_parsed_from_env_and_zero_disables() {
    let _env = lock_env();
    std::env::set_var("STALE_FEED_MAX_AGE_SECS", "7200");
    assert_eq!(AppState::new(RoboticLoopConfig::default()).tuning.stale_feed_max_age_secs, 7200);
    std::env::set_var("STALE_FEED_MAX_AGE_SECS", "0"); // 0 → kapı kapalı (cycle'da > 0 kontrolü)
    assert_eq!(AppState::new(RoboticLoopConfig::default()).tuning.stale_feed_max_age_secs, 0);
    std::env::remove_var("STALE_FEED_MAX_AGE_SECS");
}

#[test]
fn reentry_cooldown_default_zero_and_env_parse() {
    let _env = lock_env();
    std::env::remove_var("REENTRY_COOLDOWN_SECS");
    // Default 0 = kapalı (sıfır regresyon).
    assert_eq!(AppState::new(RoboticLoopConfig::default()).tuning.reentry_cooldown_secs, 0);
    std::env::set_var("REENTRY_COOLDOWN_SECS", "60");
    assert_eq!(AppState::new(RoboticLoopConfig::default()).tuning.reentry_cooldown_secs, 60);
    std::env::remove_var("REENTRY_COOLDOWN_SECS");
}

#[test]
fn fresh_candle_passes_stale_candle_blocked() {
    let now = chrono::Utc::now();
    let fresh = now - chrono::Duration::seconds(10);
    let stale_63d = now - chrono::Duration::days(63); // BTCUSDC canlı senaryosu
    // 1 saat eşik: taze mum geçer, 63 günlük mum bloklanır.
    assert!(candle_is_fresh_within(&fresh, 3600), "10sn'lik mum taze sayılmalı");
    assert!(!candle_is_fresh_within(&stale_63d, 3600), "63 günlük mum stale → açılış bloklanmalı");
}
