// Çoklu Sembol Paralel İnfaz Testi
//
// `execute_trade_cycle` artık her sembolü ayrı `tokio::spawn` ile paralelleştiriyor.
// Bu testte:
//   1. 10 farklı sembol pinned olarak verilir
//   2. Engine ateşlenir, ana döngü dönerken birden fazla sembol için DB okuma yapılır
//   3. Race condition veya panic olmadığını + log akışını doğrularız
//
// Hız ölçümü değil (CI yumuşaması) — sadece "panic'siz N paralel iş" kanıtı.

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn parallel_cycle_handles_many_symbols_without_panic() {
    let tmp_db = format!("/tmp/memos_parallel_test_{}.db", std::process::id());
    let _ = std::fs::remove_file(&tmp_db);

    // 10 farklı sembol — orchestrator'a register olacak (yoksa hiç tablo da yok demektir,
    // bu durumda read_candles dönüyor → process_symbol_cycle erken return ediyor; panic yok).
    let config = RoboticLoopConfig {
        symbol: "BTCUSDT".into(),
        interval: "1m".into(),
        db_path: tmp_db.clone(),
        pinned_symbols: vec![
            "BTCUSDT".into(), "ETHUSDT".into(), "BNBUSDT".into(),
            "XRPUSDT".into(), "ADAUSDT".into(), "SOLUSDT".into(),
            "DOTUSDT".into(), "DOGEUSDT".into(), "AVAXUSDT".into(),
            "LINKUSDT".into(),
        ],
        download_enabled: false, // Trafik yok, sadece döngü
        pipeline_enabled: false,
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));

    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    // Ana döngünün dönüp tick attığını + ilk Devriye logunu düşürdüğünü bekle.
    // Sabit sleep yerine sınırlı poll → ağır paralel test yükünde (CPU contention)
    // ana döngü aç kalsa bile ceiling'e kadar bekler, kapsama kaybı yok.
    let progressed = common::poll_until(Duration::from_secs(15), || {
        let st = state.lock().unwrap();
        st.fleet.last_loop_tick.load(Ordering::Relaxed) > 0
            && st.guardian.log.iter().any(|l| l.contains("Devriye"))
    }).await;
    assert!(progressed, "ana döngü 15s içinde tick atmadı / Devriye logu düşmedi (paralel iş thread'leri kilitlemiş olabilir)");

    // Hâlâ yaşıyor mu (panik atıp kendini durdurmadı mı)
    let alive = !state.lock().unwrap().app_stop_signal.load(Ordering::Relaxed);
    assert!(alive, "engine bir noktada kendini durdurdu");

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    let _ = std::fs::remove_file(&tmp_db);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn process_symbol_cycle_isolates_failures_across_symbols() {
    // İçinde mum verisi olmayan semboller arasında process_symbol_cycle erken return etse de,
    // diğer iyi sembollerin başarısı etkilenmemeli. Bu testte tüm semboller eşit boş veri ile
    // başlar; hiçbiri pozisyon açmaz ama panik de atmaz.
    let tmp_db = format!("/tmp/memos_parallel_iso_{}.db", std::process::id());
    let _ = std::fs::remove_file(&tmp_db);

    let config = RoboticLoopConfig {
        symbol: "BTCUSDT".into(),
        db_path: tmp_db.clone(),
        pinned_symbols: vec!["BTCUSDT".into(), "ETHUSDT".into(), "XYZUSDT".into()],
        download_enabled: false,
        pipeline_enabled: false,
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));
    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    // Ana döngünün en az bir tur dönüp sembolleri işlediğini bekle (sınırlı poll) →
    // sonra "veri yokken pozisyon açılmadı"yı kontrol et. Sabit sleep'ten daha GÜÇLÜ:
    // turun fiilen koştuğunu garanti eder, contention'da da düşmez.
    let ticked = common::poll_until(Duration::from_secs(15), || {
        state.lock().unwrap().fleet.last_loop_tick.load(Ordering::Relaxed) > 0
    }).await;
    assert!(ticked, "ana döngü 15s içinde tick atmadı");

    // Döngü döndü ama veri yok → hâlâ pozisyon yok (sinyal yok → açılış yok)
    let n_pos = state.lock().unwrap().finance.live_positions.read()
        .map(|p| p.len()).unwrap_or(99);
    assert_eq!(n_pos, 0, "veri olmadığı halde pozisyon açılmış: {}", n_pos);

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
    let _ = std::fs::remove_file(&tmp_db);
}
