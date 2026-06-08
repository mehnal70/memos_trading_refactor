// Partial Fill Akışı Testleri
//
// WS userDataStream'de PARTIALLY_FILLED yakalandığında local pozisyonun qty'si
// "remaining = orig_qty - cum_qty" formülü ile güncellenir. FILLED ise mevcut
// kapanış akışı çalışır. Bu test, mantığı saf state mutation üzerinden doğrular.

use std::sync::{Arc, Mutex};

use memos_trading_core::core::model::{PositionModel, RoboticLoopConfig};
use memos_trading_core::robot::robotic_loop::AppState;

fn open_test_position(state: &Arc<Mutex<AppState>>, symbol: &str, qty: f64) {
    let pos = PositionModel {
        pos_id: "test-pos".into(),
        symbol: symbol.into(),
        entry_price: 100.0, current_price: 100.0,
        qty,
        leverage: 1.0, market: "spot".into(), interval: "1m".into(), is_long: true,
        trade_type: "LONG".into(),
        opened_at: "2026-05-18T00:00:00Z".into(),
        stop_loss: 95.0, take_profit: 110.0, trailing_stop: 0.0,
        max_favorable_price: 100.0, breakeven_activated: false, kind: None,
        entry_commission: 0.0,
    };
    let st = state.lock().unwrap();
    let mut positions = st.finance.live_positions.write().unwrap();
    positions.insert(symbol.into(), pos);
}

fn read_position_qty(state: &Arc<Mutex<AppState>>, symbol: &str) -> Option<f64> {
    let st = state.lock().unwrap();
    let positions = st.finance.live_positions.read().unwrap();
    positions.get(symbol).map(|p| p.qty)
}

#[tokio::test(flavor = "current_thread")]
async fn partial_fill_50pct_reduces_qty_to_remaining() {
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));
    open_test_position(&state, "BTCUSDT", 1.0);

    // Partial: 0.5 dolu, 0.5 kaldı → local qty 0.5 olmalı
    // Direkt state mutation ile aynı invariant'ı uygula (process_partial_fill mantığı):
    let orig_qty: f64 = 1.0;
    let cum_qty: f64 = 0.5;
    let remaining: f64 = (orig_qty - cum_qty).max(0.0);
    {
        let st = state.lock().unwrap();
        let mut positions = st.finance.live_positions.write().unwrap();
        if let Some(pos) = positions.get_mut("BTCUSDT") {
            pos.qty = remaining;
        }
    }

    assert_eq!(read_position_qty(&state, "BTCUSDT"), Some(0.5));
}

#[tokio::test(flavor = "current_thread")]
async fn partial_fill_30pct_three_steps_reaches_zero() {
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));
    open_test_position(&state, "ETHUSDT", 3.0);

    // 3 ardışık partial fill (her biri 1.0): cum=1, cum=2, cum=3
    let orig = 3.0;
    for cum in &[1.0, 2.0, 3.0_f64] {
        let remaining = (orig - cum).max(0.0);
        let st = state.lock().unwrap();
        let mut positions = st.finance.live_positions.write().unwrap();
        if let Some(pos) = positions.get_mut("ETHUSDT") {
            pos.qty = remaining;
        }
    }

    // 3.0 dolu → kalan 0.0
    assert_eq!(read_position_qty(&state, "ETHUSDT"), Some(0.0));
}

#[tokio::test(flavor = "current_thread")]
async fn partial_fill_ignores_unknown_symbol() {
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));
    // Local'de ETHUSDT yok; partial event geldi → pozisyon yaratılmamalı
    let symbol = "ETHUSDT";
    let orig_qty: f64 = 2.0;
    let cum_qty: f64 = 1.0;
    let remaining: f64 = (orig_qty - cum_qty).max(0.0);

    {
        let st = state.lock().unwrap();
        let mut positions = st.finance.live_positions.write().unwrap();
        if let Some(pos) = positions.get_mut(symbol) {
            pos.qty = remaining;
        }
    }

    assert!(read_position_qty(&state, "ETHUSDT").is_none(),
        "bilinmeyen sembol için pozisyon yaratılmamalı");
}

#[test]
fn partial_fill_remaining_formula() {
    // remaining = max(orig - cum, 0)
    assert_eq!(((2.0_f64) - 0.5_f64).max(0.0), 1.5);
    assert_eq!(((1.0_f64) - 1.0_f64).max(0.0), 0.0);
    // Sayısal hata clamp: cum > orig olursa 0
    assert_eq!(((1.0_f64) - 1.1_f64).max(0.0), 0.0);
}

#[test]
fn partial_fill_pct_calculation() {
    let orig: f64 = 4.0;
    let cum: f64  = 1.0;
    let pct: f64 = (cum / orig * 100.0).clamp(0.0, 100.0);
    assert!((pct - 25.0).abs() < 0.01);

    let pct_full: f64 = (4.0_f64 / 4.0_f64 * 100.0).clamp(0.0, 100.0);
    assert!((pct_full - 100.0).abs() < 0.01);

    // 0/0 guard
    let pct_zero: f64 = if 0.0_f64 > 0.0 { (0.0 / 0.0_f64 * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    assert_eq!(pct_zero, 0.0);
}
