// Telegram notifier saf yardımcı + throttle testleri.
//
// Network çağrısı yok — sadece saf fonksiyonlar (format_message, build_payload,
// should_send, parse_cooldown_from_env) ve in-memory cooldown davranışı.

use std::time::{Duration, Instant};

use memos_trading_core::robot::infra::telegram_notifier::{
    build_payload, format_message, parse_cooldown_from_env, should_send,
    Severity, TelegramNotifier,
};

#[test]
fn severity_prefix_matches_spec() {
    assert_eq!(Severity::Info.prefix(),     "ℹ️");
    assert_eq!(Severity::Warning.prefix(),  "⚠️");
    assert_eq!(Severity::Critical.prefix(), "🚨");
}

#[test]
fn format_message_prepends_prefix() {
    let m = format_message(Severity::Critical, "[LIVE-EMERGENCY] SL verilemedi");
    assert!(m.starts_with("🚨 "), "Critical prefix: {}", m);
    assert!(m.ends_with("[LIVE-EMERGENCY] SL verilemedi"));
}

#[test]
fn build_payload_has_required_fields() {
    let v = build_payload("12345", "merhaba");
    assert_eq!(v["chat_id"], "12345");
    assert_eq!(v["text"], "merhaba");
    assert_eq!(v["parse_mode"], "HTML", "HTML parse-mode kullanılmalı");
}

#[test]
fn should_send_first_time_returns_true() {
    let now = Instant::now();
    assert!(should_send(now, None, Duration::from_secs(60)));
}

#[test]
fn should_send_within_cooldown_returns_false() {
    let earlier = Instant::now();
    // cooldown 60 sn, last şimdi → henüz 60 geçmedi
    let now = earlier + Duration::from_secs(10);
    assert!(!should_send(now, Some(earlier), Duration::from_secs(60)));
}

#[test]
fn should_send_after_cooldown_returns_true() {
    let earlier = Instant::now();
    let now = earlier + Duration::from_secs(61);
    assert!(should_send(now, Some(earlier), Duration::from_secs(60)));
}

#[test]
fn parse_cooldown_invalid_env_falls_back_to_default() {
    // SAFETY: env'i sıfırla → default 60 sn beklenir
    std::env::remove_var("TELEGRAM_COOLDOWN_SECS");
    assert_eq!(parse_cooldown_from_env(), Duration::from_secs(60));
}

#[test]
fn parse_cooldown_valid_env_overrides_default() {
    std::env::set_var("TELEGRAM_COOLDOWN_SECS", "5");
    assert_eq!(parse_cooldown_from_env(), Duration::from_secs(5));
    std::env::remove_var("TELEGRAM_COOLDOWN_SECS");
}

#[test]
fn parse_cooldown_garbage_env_falls_back() {
    std::env::set_var("TELEGRAM_COOLDOWN_SECS", "abc-def");
    assert_eq!(parse_cooldown_from_env(), Duration::from_secs(60));
    std::env::remove_var("TELEGRAM_COOLDOWN_SECS");
}

#[test]
fn from_env_returns_none_when_token_missing() {
    std::env::remove_var("TELEGRAM_BOT_TOKEN");
    std::env::set_var("TELEGRAM_CHAT_ID", "12345");
    assert!(TelegramNotifier::from_env().is_none());
    std::env::remove_var("TELEGRAM_CHAT_ID");
}

#[test]
fn from_env_returns_none_when_chat_id_missing() {
    std::env::set_var("TELEGRAM_BOT_TOKEN", "abc");
    std::env::remove_var("TELEGRAM_CHAT_ID");
    assert!(TelegramNotifier::from_env().is_none());
    std::env::remove_var("TELEGRAM_BOT_TOKEN");
}

#[test]
fn from_env_returns_none_when_values_whitespace() {
    std::env::set_var("TELEGRAM_BOT_TOKEN", "   ");
    std::env::set_var("TELEGRAM_CHAT_ID",  "\t\n");
    assert!(TelegramNotifier::from_env().is_none(), "boş/whitespace değerler reddedilmeli");
    std::env::remove_var("TELEGRAM_BOT_TOKEN");
    std::env::remove_var("TELEGRAM_CHAT_ID");
}

#[tokio::test(flavor = "current_thread")]
async fn notify_throttles_same_key_within_cooldown() {
    // 60 sn cooldown'lu notifier — dummy token (gerçek HTTP atılır ama spawn
    // olan task test bitince düşer; testin senkron sonucunu etkilemez).
    let n = TelegramNotifier::new("dummy", "1", Duration::from_secs(60));
    assert!(n.notify("KEY-A", Severity::Warning, "msg1"));
    assert!(!n.notify("KEY-A", Severity::Warning, "msg2"));
    assert!(n.notify("KEY-B", Severity::Critical, "msg3"));
}

#[tokio::test(flavor = "current_thread")]
async fn notify_allows_after_zero_cooldown() {
    let n = TelegramNotifier::new("dummy", "1", Duration::from_secs(0));
    assert!(n.notify("X", Severity::Info, "1"));
    assert!(n.notify("X", Severity::Info, "2"));
    assert!(n.notify("X", Severity::Info, "3"));
}
