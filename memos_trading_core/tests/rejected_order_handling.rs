// REJECTED / EXPIRED WS Event Testleri
//
// WS userDataStream'de bir emir REJECTED veya EXPIRED status'uyla geldiğinde
// handle_user_data_event hem push_log'a hem repair_log'a uyarı satırı düşürür.
// NEW ve CANCELED ise sessiz (normal yaşam döngüsü).
//
// Gerçek WS bağlantısı kurulmaz; handler'a direkt JSON string verilir.

use std::sync::{Arc, Mutex};

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

fn fresh_state() -> Arc<Mutex<AppState>> {
    Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())))
}

#[tokio::test(flavor = "current_thread")]
async fn rejected_spot_event_logs_warning() {
    let state = fresh_state();
    // Spot executionReport — REJECTED, sebep INSUFFICIENT_BALANCE
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"REJECTED","S":"BUY",
        "q":"0.001","z":"0","l":"0","i":12345,"r":"INSUFFICIENT_BALANCE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    let st = state.lock().unwrap();
    let saw_log = st.guardian.log.iter().any(|l|
        l.contains("[WS-REJECTED]") && l.contains("BTCUSDT")
        && l.contains("BUY") && l.contains("INSUFFICIENT_BALANCE")
    );
    assert!(saw_log, "push_log'da REJECTED satırı görünmeli; mevcut: {:?}", st.guardian.log);

    let saw_repair = st.guardian.repair_log.iter().any(|l|
        l.contains("REJECTED") && l.contains("BTCUSDT") && l.contains("INSUFFICIENT_BALANCE")
    );
    assert!(saw_repair, "repair_log'a da yazılmalı; mevcut: {:?}", st.guardian.repair_log);
}

#[tokio::test(flavor = "current_thread")]
async fn rejected_futures_event_logs_warning() {
    let state = fresh_state();
    // Futures ORDER_TRADE_UPDATE — REJECTED
    let raw = r#"{
        "e":"ORDER_TRADE_UPDATE",
        "o":{"s":"ETHUSDT","X":"REJECTED","S":"SELL","q":"0.5","z":"0","l":"0","i":98765,"r":"MARGIN_INSUFFICIENT"}
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    let st = state.lock().unwrap();
    let saw = st.guardian.log.iter().any(|l|
        l.contains("[WS-REJECTED]") && l.contains("ETHUSDT") && l.contains("SELL")
    );
    assert!(saw, "futures REJECTED log atmalı; mevcut: {:?}", st.guardian.log);
}

#[tokio::test(flavor = "current_thread")]
async fn expired_event_logs_with_warning_prefix() {
    // EXPIRED → push_alert(Severity::Warning) → log mesajı ⚠️ prefix'i taşır.
    let state = fresh_state();
    let raw = r#"{
        "e":"executionReport","s":"BNBUSDT","X":"EXPIRED","S":"BUY",
        "q":"1.0","z":"0","l":"0","i":555,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    let st = state.lock().unwrap();
    let saw = st.guardian.log.iter().any(|l|
        l.contains("[WS-EXPIRED]") && l.contains("BNBUSDT") && l.contains("⚠️")
    );
    assert!(saw, "EXPIRED log ⚠️ Warning prefix'i taşımalı; mevcut: {:?}", st.guardian.log);
}

#[tokio::test(flavor = "current_thread")]
async fn none_reject_reason_hidden_from_log() {
    let state = fresh_state();
    // r="NONE" → sebep stringi log'a yazılmamalı (çift "sebep=NONE" çirkin görünür)
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"REJECTED","S":"BUY",
        "q":"0.001","z":"0","l":"0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    let st = state.lock().unwrap();
    let any_log_has_none = st.guardian.log.iter().any(|l|
        l.contains("[WS-REJECTED]") && l.contains("sebep=NONE")
    );
    assert!(!any_log_has_none, "sebep=NONE log'da görünmemeli; mevcut: {:?}", st.guardian.log);
}

#[tokio::test(flavor = "current_thread")]
async fn new_status_is_silent() {
    let state = fresh_state();
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"NEW","S":"BUY",
        "q":"0.001","z":"0","l":"0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    let st = state.lock().unwrap();
    // NEW sessiz — log'a [WS-*] satırı düşmemeli
    let has_ws_log = st.guardian.log.iter().any(|l| l.contains("[WS-"));
    assert!(!has_ws_log, "NEW status'u sessiz olmalı; mevcut: {:?}", st.guardian.log);
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_status_is_silent() {
    let state = fresh_state();
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"CANCELED","S":"SELL",
        "q":"0.001","z":"0","l":"0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    let st = state.lock().unwrap();
    let has_ws_log = st.guardian.log.iter().any(|l| l.contains("[WS-"));
    assert!(!has_ws_log, "CANCELED status'u sessiz olmalı");
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_event_type_is_ignored() {
    let state = fresh_state();
    // outboundAccountPosition gibi pozisyon dışı event'ler ignored
    let raw = r#"{"e":"outboundAccountPosition","u":1620000000000}"#;
    Engine::handle_user_data_event(&state, raw).await;

    let st = state.lock().unwrap();
    let has_ws_log = st.guardian.log.iter().any(|l| l.contains("[WS-"));
    assert!(!has_ws_log, "bilinmeyen event tipi sessiz işlenmeli");
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_json_is_ignored() {
    let state = fresh_state();
    Engine::handle_user_data_event(&state, "{not valid json").await;
    let st = state.lock().unwrap();
    assert!(st.guardian.log.iter().all(|l| !l.contains("[WS-")),
        "bozuk JSON sessiz işlenmeli, panik yok");
}

#[tokio::test(flavor = "current_thread")]
async fn empty_symbol_short_circuits() {
    let state = fresh_state();
    // s="" → process_order_anomaly içinde early return
    let raw = r#"{
        "e":"executionReport","s":"","X":"REJECTED","S":"BUY",
        "q":"1.0","z":"0","l":"0","i":1,"r":"FOO"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    let st = state.lock().unwrap();
    let has_ws_log = st.guardian.log.iter().any(|l| l.contains("[WS-"));
    assert!(!has_ws_log, "boş sembol log atmamalı");
}

#[tokio::test(flavor = "current_thread")]
async fn rejected_repair_log_caps_at_100() {
    let state = fresh_state();
    // 105 REJECTED at — repair_log 100 ile sınırlandırılmalı (FIFO)
    for i in 0..105 {
        let raw = format!(r#"{{
            "e":"executionReport","s":"BTCUSDT","X":"REJECTED","S":"BUY",
            "q":"0.001","z":"0","l":"0","i":{},"r":"SOMETHING"
        }}"#, i);
        Engine::handle_user_data_event(&state, &raw).await;
    }
    let st = state.lock().unwrap();
    assert!(st.guardian.repair_log.len() <= 100,
        "repair_log 100 ile sınırlandırılmalı, mevcut={}", st.guardian.repair_log.len());
}
