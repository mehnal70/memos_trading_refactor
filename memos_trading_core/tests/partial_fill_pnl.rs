// Partial Fill PnL ve Komisyon Muhasebesi Testleri
//
// process_partial_fill artık side'a bakarak entry vs close partial'i ayırır:
//   - LONG & BUY  → ENTRY partial (cum kadar tutuyoruz, sadece komisyon)
//   - LONG & SELL → CLOSE partial (qty -= last_qty, realize PnL + komisyon)
//   - SHORT & SELL → ENTRY partial
//   - SHORT & BUY  → CLOSE partial
//
// Tüm testler handle_user_data_event üzerinden gerçek WS JSON ile uçtan uca koşar.

use std::sync::{Arc, Mutex};

use memos_trading_core::core::model::{PositionModel, RoboticLoopConfig};
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

const COMMISSION_RATE: f64 = 0.001;

fn fresh_state() -> Arc<Mutex<AppState>> {
    Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())))
}

fn open_position(
    state: &Arc<Mutex<AppState>>,
    symbol: &str,
    is_long: bool,
    entry: f64,
    qty: f64,
) {
    let pos = PositionModel {
        pos_id: "pid".into(),
        symbol: symbol.into(),
        entry_price: entry, current_price: entry,
        qty,
        leverage: 1.0, market: "spot".into(), is_long,
        trade_type: if is_long { "LONG".into() } else { "SHORT".into() },
        opened_at: "2026-05-18T00:00:00Z".into(),
        stop_loss: 0.0, take_profit: 0.0, trailing_stop: 0.0,
        max_favorable_price: entry, breakeven_activated: false, kind: None,
    };
    let st = state.lock().unwrap();
    st.finance.live_positions.write().unwrap().insert(symbol.into(), pos);
}

fn position_qty(state: &Arc<Mutex<AppState>>, symbol: &str) -> Option<f64> {
    state.lock().unwrap().finance.live_positions.read().unwrap()
        .get(symbol).map(|p| p.qty)
}

fn equity(state: &Arc<Mutex<AppState>>) -> f64 {
    state.lock().unwrap().finance.equity
}

fn commission_usd(state: &Arc<Mutex<AppState>>) -> f64 {
    state.lock().unwrap().finance.live_execution_costs.read().unwrap().commission_usd
}

#[tokio::test(flavor = "current_thread")]
async fn long_close_partial_realizes_profit() {
    let state = fresh_state();
    // LONG @ 100, qty 1.0, equity başlangıç 10000
    open_position(&state, "BTCUSDT", true, 100.0, 1.0);
    let equity_before = equity(&state);

    // SELL partial: 0.3 unit @ 110 (kâr)
    // pnl = (110 - 100) * 0.3 * 1 = +3.0
    // commission = 0.3 * 110 * 0.001 = 0.033
    // equity += 3.0 - 0.033 = +2.967
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"0.3","l":"0.3","L":"110.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    assert!((position_qty(&state, "BTCUSDT").unwrap() - 0.7).abs() < 1e-9,
        "qty 1.0 - 0.3 = 0.7 olmalı");
    let delta_eq = equity(&state) - equity_before;
    assert!((delta_eq - (3.0 - 0.033)).abs() < 1e-6,
        "equity Δ = +pnl - commission = +2.967 olmalı, gerçek={}", delta_eq);
    assert!((commission_usd(&state) - 0.033).abs() < 1e-6,
        "commission 0.033 olmalı, gerçek={}", commission_usd(&state));
}

#[tokio::test(flavor = "current_thread")]
async fn long_close_partial_realizes_loss() {
    let state = fresh_state();
    // LONG @ 100, qty 2.0
    open_position(&state, "ETHUSDT", true, 100.0, 2.0);
    let equity_before = equity(&state);

    // SELL partial: 0.5 unit @ 90 (zarar)
    // pnl = (90 - 100) * 0.5 * 1 = -5.0
    // commission = 0.5 * 90 * 0.001 = 0.045
    // equity += -5.0 - 0.045 = -5.045
    let raw = r#"{
        "e":"executionReport","s":"ETHUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"2.0","z":"0.5","l":"0.5","L":"90.0","i":2,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    assert!((position_qty(&state, "ETHUSDT").unwrap() - 1.5).abs() < 1e-9,
        "qty 2.0 - 0.5 = 1.5 olmalı");
    let delta = equity(&state) - equity_before;
    assert!((delta - (-5.045)).abs() < 1e-6,
        "equity Δ -5.045 olmalı, gerçek={}", delta);
}

#[tokio::test(flavor = "current_thread")]
async fn short_close_partial_with_buy_realizes_profit() {
    let state = fresh_state();
    // SHORT @ 100, qty 1.0
    open_position(&state, "BNBUSDT", false, 100.0, 1.0);
    let equity_before = equity(&state);

    // BUY partial 0.4 unit @ 90 (short kâr)
    // pnl = (100 - 90) * 0.4 * 1 = +4.0  (is_long=false → ters formül)
    // commission = 0.4 * 90 * 0.001 = 0.036
    let raw = r#"{
        "e":"ORDER_TRADE_UPDATE",
        "o":{"s":"BNBUSDT","X":"PARTIALLY_FILLED","S":"BUY","q":"1.0","z":"0.4","l":"0.4","L":"90.0","i":3}
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    assert!((position_qty(&state, "BNBUSDT").unwrap() - 0.6).abs() < 1e-9);
    let delta = equity(&state) - equity_before;
    assert!((delta - (4.0 - 0.036)).abs() < 1e-6,
        "short close partial: equity Δ +3.964 olmalı, gerçek={}", delta);
}

#[tokio::test(flavor = "current_thread")]
async fn long_entry_partial_only_commission_no_pnl() {
    let state = fresh_state();
    // LONG açılırken cum büyüyor; local qty cum'a hizalanmalı, realize PnL yok.
    open_position(&state, "BTCUSDT", true, 100.0, 1.0);
    let equity_before = equity(&state);

    // BUY partial: 0.5 unit @ 100 (entry)
    // commission = 0.5 * 100 * 0.001 = 0.05
    // equity -= 0.05 (sadece komisyon)
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"BUY",
        "q":"1.0","z":"0.5","l":"0.5","L":"100.0","i":4,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    // Entry partial: qty = cum = 0.5 (1.0'dan 0.5'e iniyor çünkü gerçekte bu kadar dolduk)
    assert!((position_qty(&state, "BTCUSDT").unwrap() - 0.5).abs() < 1e-9,
        "entry partial: qty cum'a hizalanmalı");
    let delta = equity(&state) - equity_before;
    assert!((delta - (-0.05)).abs() < 1e-6,
        "entry partial: sadece komisyon, equity Δ -0.05 olmalı, gerçek={}", delta);
}

#[tokio::test(flavor = "current_thread")]
async fn short_entry_partial_with_sell_only_commission() {
    let state = fresh_state();
    open_position(&state, "ETHUSDT", false, 200.0, 2.0);
    let equity_before = equity(&state);

    // SHORT entry: SELL partial 1.5 unit @ 200
    let raw = r#"{
        "e":"ORDER_TRADE_UPDATE",
        "o":{"s":"ETHUSDT","X":"PARTIALLY_FILLED","S":"SELL","q":"2.0","z":"1.5","l":"1.5","L":"200.0","i":5}
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    assert!((position_qty(&state, "ETHUSDT").unwrap() - 1.5).abs() < 1e-9,
        "short entry partial: qty cum (1.5) olmalı");
    let delta = equity(&state) - equity_before;
    let expected_commission = 1.5 * 200.0 * COMMISSION_RATE;
    assert!((delta - (-expected_commission)).abs() < 1e-6);
}

#[tokio::test(flavor = "current_thread")]
async fn three_sequential_close_partials_accumulate_pnl() {
    let state = fresh_state();
    open_position(&state, "BTCUSDT", true, 100.0, 1.0);
    let equity_before = equity(&state);

    // 3 ardışık close partial: 0.3 @ 110, 0.3 @ 115, 0.4 @ 120
    // Toplam pnl = (10*0.3) + (15*0.3) + (20*0.4) = 3 + 4.5 + 8 = 15.5
    // Toplam komisyon = (0.3*110 + 0.3*115 + 0.4*120) * 0.001 = (33 + 34.5 + 48) * 0.001 = 0.1155
    for (cum, last_qty, price, oid) in [
        (0.3_f64, 0.3_f64, 110.0_f64, 1),
        (0.6, 0.3, 115.0, 2),
        (1.0, 0.4, 120.0, 3),
    ] {
        let raw = format!(r#"{{
            "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
            "q":"1.0","z":"{}","l":"{}","L":"{}","i":{},"r":"NONE"
        }}"#, cum, last_qty, price, oid);
        Engine::handle_user_data_event(&state, &raw).await;
    }

    let qty = position_qty(&state, "BTCUSDT").unwrap();
    assert!(qty.abs() < 1e-9, "3 partial sonra qty 0'a inmeli, gerçek={}", qty);

    let delta = equity(&state) - equity_before;
    assert!((delta - (15.5 - 0.1155)).abs() < 1e-6,
        "kümülatif equity Δ = pnl(15.5) - fee(0.1155); gerçek={}", delta);
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_symbol_skips_partial() {
    let state = fresh_state();
    let equity_before = equity(&state);
    let raw = r#"{
        "e":"executionReport","s":"DOGEUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"100","z":"10","l":"10","L":"0.1","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    // Pozisyon yok → mutation yok, equity ve commission değişmez
    assert!((equity(&state) - equity_before).abs() < 1e-9);
    assert!(commission_usd(&state).abs() < 1e-9);
}

#[tokio::test(flavor = "current_thread")]
async fn last_qty_zero_skips_event() {
    let state = fresh_state();
    open_position(&state, "BTCUSDT", true, 100.0, 1.0);
    let equity_before = equity(&state);
    // l=0 → bu event'te yeni fill yok (örn. cum aynı kalmış, sadece status update)
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"0.3","l":"0","L":"110.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    assert!((equity(&state) - equity_before).abs() < 1e-9,
        "l=0 olduğunda equity değişmemeli");
    assert!((position_qty(&state, "BTCUSDT").unwrap() - 1.0).abs() < 1e-9,
        "l=0 olduğunda qty de değişmemeli");
}

#[tokio::test(flavor = "current_thread")]
async fn close_partial_log_includes_pnl() {
    let state = fresh_state();
    open_position(&state, "BTCUSDT", true, 100.0, 1.0);
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"0.3","l":"0.3","L":"110.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    let st = state.lock().unwrap();
    let saw = st.guardian.log.iter().any(|l|
        l.contains("[WS-PARTIAL-CLOSE]") && l.contains("pnl=") && l.contains("fee=")
    );
    assert!(saw, "close partial log'unda pnl ve fee görünmeli; mevcut: {:?}", st.guardian.log);
}

#[tokio::test(flavor = "current_thread")]
async fn entry_partial_log_has_no_pnl_field() {
    let state = fresh_state();
    open_position(&state, "BTCUSDT", true, 100.0, 1.0);
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"BUY",
        "q":"1.0","z":"0.5","l":"0.5","L":"100.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    let st = state.lock().unwrap();
    let saw_entry = st.guardian.log.iter().any(|l| l.contains("[WS-PARTIAL-ENTRY]"));
    let saw_pnl = st.guardian.log.iter().any(|l|
        l.contains("[WS-PARTIAL-ENTRY]") && l.contains("pnl=")
    );
    assert!(saw_entry, "ENTRY tag'i görünmeli");
    assert!(!saw_pnl, "ENTRY partial log'unda pnl= görünmemeli (henüz realize yok)");
}
