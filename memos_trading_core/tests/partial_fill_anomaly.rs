// Partial Fill Anomali Testleri
//
// process_partial_fill içindeki detect_partial_fill_anomalies üç kriteri tespit eder
// ve push_alert ile bildirir:
//   - OVERFILL  (Critical): last_qty > local_qty_before * 1.001
//   - CUM       (Warning):  cum_qty > orig_qty * 1.001
//   - SLIPPAGE  (Warning):  adverse fiyat sapması > PARTIAL_FILL_MAX_SLIPPAGE_PCT (default 1.0%)
//
// Telegram notifier env'siz ortamda None → push_alert sadece push_log atar.
// guardian.log içinde severity prefix + etiket kontrol ediliyor.

use std::sync::{Arc, Mutex, MutexGuard};

use memos_trading_core::core::model::{PositionModel, RoboticLoopConfig};
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

// PARTIAL_FILL_MAX_SLIPPAGE_PCT process-global; aşağıdaki testler set/remove
// ederken paralel koşumda yarışıyordu (slippage_threshold_env_override_relaxes_check
// "5.0" set ediyor, başka test aynı anda remove → default 1.0 → assert patlıyor).
// Env-touch eden 5 testi mutex'le serileştir.
static ENV_GUARD: Mutex<()> = Mutex::new(());
fn lock_env() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner())
}

fn fresh_state() -> Arc<Mutex<AppState>> {
    Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())))
}

/// `current` parametresi None ise entry_price ile aynı atanır (live henüz update etmemiş).
fn open_position(
    state: &Arc<Mutex<AppState>>,
    symbol: &str,
    is_long: bool,
    entry: f64,
    qty: f64,
    current: Option<f64>,
) {
    let pos = PositionModel {
        pos_id: "pid".into(),
        symbol: symbol.into(),
        entry_price: entry,
        current_price: current.unwrap_or(entry),
        qty,
        leverage: 1.0, market: "spot".into(),
        is_long,
        trade_type: if is_long { "LONG".into() } else { "SHORT".into() },
        opened_at: "2026-05-20T00:00:00Z".into(),
        stop_loss: 0.0,
        take_profit: 0.0,
        trailing_stop: 0.0,
        max_favorable_price: entry,
        breakeven_activated: false,
        kind: None,
    };
    let st = state.lock().unwrap();
    st.finance.live_positions.write().unwrap().insert(symbol.into(), pos);
}

fn logs_contain(state: &Arc<Mutex<AppState>>, needle: &str) -> bool {
    state.lock().unwrap().guardian.log.iter().any(|l| l.contains(needle))
}

// ─── OVERFILL ────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn overfill_triggers_critical_alert() {
    let state = fresh_state();
    // Local'de 0.5 birim LONG var, ama borsa 0.7 birim dolduğunu söylüyor (SELL ile kapatma).
    open_position(&state, "BTCUSDT", true, 100.0, 0.5, None);
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"0.7","l":"0.7","L":"100.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;

    assert!(
        logs_contain(&state, "PARTIAL-ANOMALY-OVERFILL"),
        "overfill alert atmalı; mevcut log: {:?}",
        state.lock().unwrap().guardian.log,
    );
    // Critical severity prefix
    assert!(
        logs_contain(&state, "🚨"),
        "Critical severity prefix (🚨) görünmeli"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn normal_partial_within_local_qty_no_overfill_alert() {
    let state = fresh_state();
    open_position(&state, "BTCUSDT", true, 100.0, 1.0, None);
    // last_qty 0.3 ≤ local 1.0 → overfill yok
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"0.3","l":"0.3","L":"100.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    assert!(
        !logs_contain(&state, "PARTIAL-ANOMALY-OVERFILL"),
        "overfill alert atılmamalı"
    );
}

// ─── CUM > ORIG ──────────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn cum_exceeds_orig_triggers_warning_alert() {
    let state = fresh_state();
    open_position(&state, "ETHUSDT", true, 100.0, 2.0, None);
    // orig 1.0, cum 1.05 → tolerans (1.001) aşıldı
    let raw = r#"{
        "e":"executionReport","s":"ETHUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"1.05","l":"0.5","L":"100.0","i":2,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    assert!(
        logs_contain(&state, "PARTIAL-ANOMALY-CUM"),
        "cum>orig alert atmalı; mevcut log: {:?}",
        state.lock().unwrap().guardian.log,
    );
}

// ─── SLIPPAGE ────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn long_close_adverse_slippage_triggers_alert() {
    let _env = lock_env();
    std::env::remove_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT");
    let state = fresh_state();
    // LONG current=100, kapanışta SELL 90 → (100-90)/100 = +10% adverse > 1%
    open_position(&state, "BTCUSDT", true, 100.0, 1.0, Some(100.0));
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"0.3","l":"0.3","L":"90.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    assert!(
        logs_contain(&state, "PARTIAL-ANOMALY-SLIPPAGE"),
        "long close adverse slipaj alert atmalı; mevcut log: {:?}",
        state.lock().unwrap().guardian.log,
    );
    assert!(logs_contain(&state, "CLOSE"), "kind=CLOSE etiketi olmalı");
}

#[tokio::test(flavor = "current_thread")]
async fn long_entry_adverse_slippage_triggers_alert() {
    let _env = lock_env();
    std::env::remove_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT");
    let state = fresh_state();
    // LONG entry=100, BUY 110 → (110-100)/100 = +10% adverse
    open_position(&state, "BTCUSDT", true, 100.0, 1.0, None);
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"BUY",
        "q":"1.0","z":"0.5","l":"0.5","L":"110.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    assert!(
        logs_contain(&state, "PARTIAL-ANOMALY-SLIPPAGE"),
        "long entry adverse slipaj alert atmalı; mevcut log: {:?}",
        state.lock().unwrap().guardian.log,
    );
    assert!(logs_contain(&state, "ENTRY"), "kind=ENTRY etiketi olmalı");
}

#[tokio::test(flavor = "current_thread")]
async fn favorable_price_does_not_trigger_slippage_alert() {
    let _env = lock_env();
    std::env::remove_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT");
    let state = fresh_state();
    // LONG current=100, SELL 110 → favorable (+10% bot lehine), adverse negatif
    open_position(&state, "BTCUSDT", true, 100.0, 1.0, Some(100.0));
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"0.3","l":"0.3","L":"110.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    assert!(
        !logs_contain(&state, "PARTIAL-ANOMALY-SLIPPAGE"),
        "favorable fill için slipaj alert atılmamalı; log: {:?}",
        state.lock().unwrap().guardian.log,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn slippage_threshold_env_override_relaxes_check() {
    let _env = lock_env();
    // Eşiği %5'e çıkar → %3 adverse pas geçmeli (default %1'de tetiklenirdi).
    std::env::set_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT", "5.0");
    let state = fresh_state();
    open_position(&state, "BTCUSDT", true, 100.0, 1.0, Some(100.0));
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"0.3","l":"0.3","L":"97.0","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    let saw = logs_contain(&state, "PARTIAL-ANOMALY-SLIPPAGE");
    std::env::remove_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT");
    assert!(!saw, "%3 adverse, %5 eşik altında — alert atılmamalı");
}

// ─── HEALTHY PATH ────────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn healthy_partial_no_anomaly_alerts() {
    let _env = lock_env();
    std::env::remove_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT");
    let state = fresh_state();
    open_position(&state, "BTCUSDT", true, 100.0, 1.0, Some(100.0));
    // Normal partial: cum<orig, last_qty<local, fiyat current'a yakın (0.2% sapma → eşik altı)
    let raw = r#"{
        "e":"executionReport","s":"BTCUSDT","X":"PARTIALLY_FILLED","S":"SELL",
        "q":"1.0","z":"0.3","l":"0.3","L":"99.8","i":1,"r":"NONE"
    }"#;
    Engine::handle_user_data_event(&state, raw).await;
    assert!(!logs_contain(&state, "PARTIAL-ANOMALY-OVERFILL"));
    assert!(!logs_contain(&state, "PARTIAL-ANOMALY-CUM"));
    assert!(!logs_contain(&state, "PARTIAL-ANOMALY-SLIPPAGE"));
    // Normal WS-PARTIAL log gene de var olmalı
    assert!(
        logs_contain(&state, "[WS-PARTIAL-CLOSE]"),
        "normal partial log'u gene de atılmalı"
    );
}
