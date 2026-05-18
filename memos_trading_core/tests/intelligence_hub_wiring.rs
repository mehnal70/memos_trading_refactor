// IntelligenceHub Entegrasyon Testleri
//
// AppState ↔ IntelligenceHub köprüsünün uçtan uca çalıştığını doğrular:
//   1. AppState::new IntelligenceHub'ı varsayılan controller ile başlatır
//   2. track_trade çağrısı hub.pending_trades'e eklenir
//   3. learn_from_exit çağrısı kayıt kaybı durumunda consecutive_failures artırır

use std::sync::{Arc, Mutex};

use memos_trading_core::core::model::{PositionModel, RoboticLoopConfig};
use memos_trading_core::core::types::PositionId;
use memos_trading_core::evolution::{AutonomousState, MarketRegime};
use memos_trading_core::robot::robotic_loop::AppState;

#[test]
fn brainbox_initializes_intelligence_hub_with_default_controller() {
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));
    let st = state.lock().unwrap();
    let hub = st.brain.intelligence_hub.read().unwrap();

    // Controller default state Observe olmalı
    assert_eq!(hub.controller.state, AutonomousState::Observe);
    // Cycle henüz 0
    assert_eq!(hub.controller.cycle_id, 0);
    // pending_trades boş
    assert!(hub.pending_trades.is_empty());
    // drift_history boş
    assert!(hub.drift_history.is_empty());
}

#[test]
fn track_trade_registers_position_in_pending() {
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));
    let pid = PositionId::new();

    {
        let st = state.lock().unwrap();
        let mut hub = st.brain.intelligence_hub.write().unwrap();
        hub.track_trade(pid, MarketRegime::StrongUptrend, "MA_CROSSOVER".into());
    }

    let st = state.lock().unwrap();
    let hub = st.brain.intelligence_hub.read().unwrap();
    assert_eq!(hub.pending_trades.len(), 1);
    let entry = hub.pending_trades.get(&pid).expect("pos_id eklenmemiş");
    assert!(matches!(entry.0, MarketRegime::StrongUptrend));
    assert_eq!(entry.1, "MA_CROSSOVER");
}

#[test]
fn learn_from_exit_consumes_pending_and_updates_controller() {
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));
    let pid = PositionId::new();

    // Önce track
    {
        let st = state.lock().unwrap();
        let mut hub = st.brain.intelligence_hub.write().unwrap();
        hub.track_trade(pid, MarketRegime::Ranging, "RSI".into());
    }

    // Kayıplı çıkış
    {
        let st = state.lock().unwrap();
        let mut hub = st.brain.intelligence_hub.write().unwrap();
        hub.learn_from_exit(pid, -3.5); // %3.5 kayıp
    }

    let st = state.lock().unwrap();
    let hub = st.brain.intelligence_hub.read().unwrap();
    // pending_trades temizlenmiş
    assert!(hub.pending_trades.is_empty(), "pending_trades temizlenmemiş");
    // consecutive_failures arttı
    assert_eq!(hub.controller.consecutive_failures, 1);
}

#[test]
fn five_consecutive_losses_trigger_safe_mode() {
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));

    // 5 kayıp ardışık → SafeMode
    for _ in 0..5 {
        let pid = PositionId::new();
        let st = state.lock().unwrap();
        let mut hub = st.brain.intelligence_hub.write().unwrap();
        hub.track_trade(pid, MarketRegime::HighVolatility, "MACD".into());
        hub.learn_from_exit(pid, -1.0);
    }

    let st = state.lock().unwrap();
    let hub = st.brain.intelligence_hub.read().unwrap();
    assert_eq!(hub.controller.state, AutonomousState::SafeMode,
        "5 ardışık kayıptan sonra SafeMode bekleniyordu, gerçek: {:?}", hub.controller.state);
}

#[test]
fn position_pos_id_round_trip_via_position_id() {
    let pid = PositionId::new();
    let pid_str = pid.to_string();

    let pos = PositionModel {
        pos_id: pid_str.clone(),
        symbol: "BTCUSDT".into(),
        entry_price: 100.0, current_price: 100.0, qty: 1.0,
        leverage: 1.0, is_long: true, trade_type: "LONG".into(),
        opened_at: "2026-05-18T00:00:00Z".into(),
        stop_loss: 95.0, take_profit: 105.0, trailing_stop: 0.0,
        max_favorable_price: 100.0, breakeven_activated: false,
    };

    // Round-trip: string -> PositionId -> aynı hash anahtarı
    let recovered = PositionId::from_str_or_new(&pos.pos_id);
    assert_eq!(pid, recovered, "pos_id round-trip kırık");
}
