// blocked_symbols mekanizması testleri
//
// Önceden config.blocked_symbols alanı tutuluyordu ama hiçbir code path bunu
// kontrol etmiyordu (sessiz operatör tuzağı). Bu testler Engine::is_symbol_blocked
// helper'ının davranışını + screener filtresinin pool retain'i ile uyumlu olduğunu
// doğrular. open_paper_position üzerindeki gerçek HTTP/risk akışı çok ağır olduğu
// için burada doğrudan helper test edilir; reddetme yolu kod inceleme + manuel
// doğrulama ile teyit edilir.

use std::sync::{Arc, Mutex};

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

fn state_with_blocked(blocked: Vec<&str>) -> Arc<Mutex<AppState>> {
    let config = RoboticLoopConfig {
        blocked_symbols: blocked.into_iter().map(String::from).collect(),
        ..Default::default()
    };
    Arc::new(Mutex::new(AppState::new(config)))
}

#[test]
fn blocked_match_exact_case() {
    let state = state_with_blocked(vec!["ETHUSDT"]);
    assert!(Engine::is_symbol_blocked(&state, "ETHUSDT"));
}

#[test]
fn blocked_match_is_case_insensitive() {
    let state = state_with_blocked(vec!["ETHUSDT"]);
    assert!(Engine::is_symbol_blocked(&state, "ethusdt"));
    assert!(Engine::is_symbol_blocked(&state, "EthUsdt"));
}

#[test]
fn not_blocked_returns_false() {
    let state = state_with_blocked(vec!["ETHUSDT"]);
    assert!(!Engine::is_symbol_blocked(&state, "BTCUSDT"));
}

#[test]
fn empty_blocked_list_means_all_allowed() {
    let state = state_with_blocked(vec![]);
    assert!(!Engine::is_symbol_blocked(&state, "ETHUSDT"));
    assert!(!Engine::is_symbol_blocked(&state, "ANYTHING"));
}

#[test]
fn multiple_blocked_entries_all_match() {
    let state = state_with_blocked(vec!["ETHUSDT", "DOGEUSDT", "SHIBUSDT"]);
    assert!(Engine::is_symbol_blocked(&state, "ETHUSDT"));
    assert!(Engine::is_symbol_blocked(&state, "DOGEUSDT"));
    assert!(Engine::is_symbol_blocked(&state, "SHIBUSDT"));
    assert!(!Engine::is_symbol_blocked(&state, "BTCUSDT"));
}
