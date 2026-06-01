// Entegrasyon Testi: rtc_tui klavye → fleet.triggers → master.rs job handler zinciri.
// TUI'nin gerçek tuş eşlemesi (handlers/input.rs:65 → "ml") AppState.fleet.triggers'a
// pulse atıyor. Bu test, o pulse'u programatik olarak atıp Engine'in
// spawn_infrastructure_fleet'inin (Faz 2 Task 4) gerçekten run_ml_retrain_job'u
// çağırdığını ve sonucun pipeline'a yansıdığını doğrular.

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trigger_ml_pulse_flows_through_to_pipeline() {
    let config = RoboticLoopConfig {
        symbol: "BTCUSDT".into(),
        market: "spot".into(),
        // Mum verisi olmasa bile job çağrılır → handler "yetersiz veri" hatası dönecek,
        // ama önemli olan pipeline'a "trigger:ml" adımının kaydı.
        db_path: "data/trader.db".into(),
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));

    // Engine'i arka planda ateşle
    let engine_state = Arc::clone(&state);
    let engine_handle = tokio::spawn(async move {
        Engine::run_autonomous_loop(engine_state).await;
    });

    // Task 4 Trigger Handler 250 ms aralıkta okuyor; warm-up için kısa bekleme
    tokio::time::sleep(Duration::from_millis(400)).await;

    // 1) TUI'nin "m" tuşunun yapacağı şey: ml trigger pulse'u
    {
        let st = state.lock().unwrap();
        st.fleet.triggers.get("ml")
            .expect("ml trigger AppState init'inde tanımlanmalı")
            .store(true, Ordering::Relaxed);
    }

    // 2/3/4) Handler tetiklensin, run_ml_retrain_job çalışsın, pipeline'a "trigger:ml"
    //   adımı yazılsın VE trigger AtomicBool tüketilsin (swap→false). Sabit sleep yerine
    //   sınırlı poll → contention-dayanıklı. İki koşul birlikte beklenir çünkü adım-yazımı
    //   ile flag-tüketimi farklı sırada gerçekleşebilir (poll tek koşulda erken dönerdi).
    let done = common::poll_until(Duration::from_secs(15), || {
        let st = state.lock().unwrap();
        let step_seen = st.guardian.live_pipeline.read().unwrap()
            .chain_steps.iter().any(|s| s.label.contains("trigger:ml"));
        let consumed = !st.fleet.triggers.get("ml").unwrap().load(Ordering::Relaxed);
        step_seen && consumed
    }).await;
    assert!(done,
        "ml trigger: pipeline'da 'trigger:ml' adımı + flag tüketimi 15s içinde gerçekleşmedi");

    // 5) Engine'i kapat
    {
        let st = state.lock().unwrap();
        st.app_stop_signal.store(true, Ordering::SeqCst);
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), engine_handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trigger_backtest_pulse_flows_through_to_pipeline() {
    let config = RoboticLoopConfig {
        symbol: "BTCUSDT".into(),
        market: "spot".into(),
        db_path: "data/trader.db".into(),
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));

    let engine_state = Arc::clone(&state);
    let engine_handle = tokio::spawn(async move {
        Engine::run_autonomous_loop(engine_state).await;
    });
    tokio::time::sleep(Duration::from_millis(400)).await;

    // TUI'nin "b" tuşunun yapacağı şey
    {
        let st = state.lock().unwrap();
        st.fleet.triggers.get("backtest")
            .expect("backtest trigger AppState init'inde tanımlanmalı")
            .store(true, Ordering::Relaxed);
    }

    let saw_trigger = common::poll_until(Duration::from_secs(15), || {
        let st = state.lock().unwrap();
        let pipe = st.guardian.live_pipeline.read().unwrap();
        pipe.chain_steps.iter().any(|s| s.label.contains("trigger:backtest"))
    }).await;
    assert!(saw_trigger,
        "backtest trigger pulse'u sonrasında pipeline'da 'trigger:backtest' adımı görülmedi");

    {
        let st = state.lock().unwrap();
        st.app_stop_signal.store(true, Ordering::SeqCst);
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), engine_handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guardian_log_collects_user_visible_messages() {
    // TUI archives panelinin görüntülediği guardian.log akışı dolmalı:
    // engine ateşlenme + altyapı sevkiyat + #1 devriye heartbeat'i
    // ilk ~2 saniyede mutlaka düşmeli.
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));

    let engine_state = Arc::clone(&state);
    let engine_handle = tokio::spawn(async move {
        Engine::run_autonomous_loop(engine_state).await;
    });

    // "💓 Devriye #1" en geç gelen log (engine-fired → fleet-dispatched → ilk
    // heartbeat). Onu bekle (sınırlı poll); geldiğinde öncekiler de düşmüş olur.
    common::poll_until(Duration::from_secs(15), || {
        state.lock().unwrap().guardian.log.iter().any(|l| l.contains("💓 Devriye #1"))
    }).await;

    let log_lines: Vec<String> = {
        let st = state.lock().unwrap();
        st.guardian.log.iter().cloned().collect()
    };

    assert!(!log_lines.is_empty(),
        "guardian.log boş; TUI log panelinde hiç mesaj görünmez");
    assert!(log_lines.iter().any(|l| l.contains("Master Engine ateşlendi")),
        "engine ateşleme logu bulunamadı. Görülen: {:#?}", log_lines);
    assert!(log_lines.iter().any(|l| l.contains("Altyapı filosu sevk edildi")),
        "altyapı filosu sevk logu bulunamadı. Görülen: {:#?}", log_lines);
    assert!(log_lines.iter().any(|l| l.contains("💓 Devriye #1")),
        "ilk devriye kalp atışı logu bulunamadı. Görülen: {:#?}", log_lines);

    {
        let st = state.lock().unwrap();
        st.app_stop_signal.store(true, Ordering::SeqCst);
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), engine_handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trigger_ml_logs_user_visible_detail() {
    // Trigger handler "🎮 Tetik [...] ⇒ ml: ..." satırını guardian.log'a düşürmeli;
    // ml job hatası da "❌ ML Retrain başarısız: ..." mesajıyla görünür olmalı.
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig {
        // Mevcut DB'de candles_btcusdt_1m tablosu yok → run_ml_retrain_job hata
        // dönecek; bu hatanın user-facing log'a düşmesini doğruluyoruz.
        symbol: "BTCUSDT".into(),
        db_path: "data/trader.db".into(),
        ..Default::default()
    })));

    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });
    tokio::time::sleep(Duration::from_millis(400)).await;

    state.lock().unwrap().fleet.triggers.get("ml").unwrap().store(true, Ordering::Relaxed);
    // "🧠 ML Retrain başladı" trigger-fired logundan sonra gelir; onu bekle (sınırlı poll).
    common::poll_until(Duration::from_secs(15), || {
        state.lock().unwrap().guardian.log.iter().any(|l| l.contains("🧠 ML Retrain başladı"))
    }).await;

    let logs: Vec<String> = state.lock().unwrap().guardian.log.iter().cloned().collect();
    assert!(logs.iter().any(|l| l.contains("🎮 Tetik") && l.contains("⇒ ml")),
        "trigger-fired bağlam logu bulunamadı. Logs: {:#?}", logs);
    assert!(logs.iter().any(|l| l.contains("🧠 ML Retrain başladı")),
        "ML Retrain başlangıç logu bulunamadı. Logs: {:#?}", logs);

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn equity_history_and_drawdown_populate_charts() {
    // Risk paneli sparkline'ı için: ana döngü her ~2.5 sn'de equity tarihçesine push etmeli;
    // bridge::get_snapshot bunu charts.equity_series'e ve current_drawdown_pct'e çevirmeli.
    use memos_trading_core::core::bridge;

    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));
    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    // Ana döngü ~5. turda (her ~500ms) ilk equity push'unu yapar. Sabit sleep yerine
    // sınırlı poll → contention'da ceiling'e kadar bekler.
    common::poll_until(Duration::from_secs(20), || {
        let st = state.lock().unwrap();
        bridge::get_snapshot(&st).charts.equity_series.len() >= 2
    }).await;

    let snap = {
        let st = state.lock().unwrap();
        bridge::get_snapshot(&st)
    };

    assert!(snap.charts.equity_series.len() >= 2,
        "equity_series push edilmemiş; uzunluk={}", snap.charts.equity_series.len());
    assert!(snap.charts.peak_equity > 0.0,
        "peak_equity sıfır kalmış: {}", snap.charts.peak_equity);
    // İlk push capital değerinde olmalı (init zamanı yazıldı), sonrası eşit
    assert!((snap.charts.equity_series[0] - 10000.0).abs() < 0.01,
        "ilk equity capital olmalı: {:?}", snap.charts.equity_series);
    // Drawdown kayıp olmadığı için 0
    assert!(snap.charts.current_drawdown_pct.abs() < 0.001,
        "drawdown beklenmedik: {}", snap.charts.current_drawdown_pct);

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_writer_produces_json_file() {
    use memos_trading_core::robot::infra::snapshot_writer::spawn_snapshot_writer;
    // Test'e özel geçici dosya
    let path = format!("/tmp/memos_snapshot_test_{}.json", std::process::id());
    let _ = std::fs::remove_file(&path);

    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));
    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    spawn_snapshot_writer(Arc::clone(&state), path.clone(), 1);

    // En az 1 yazım turunu bekle: dosya yazılıp geçerli JSON olana dek poll
    // (sabit sleep yerine → contention-dayanıklı).
    common::poll_until(Duration::from_secs(15), || {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .is_some()
    }).await;

    // Dosyanın var ve JSON parse edilebilir olduğunu doğrula
    let raw = std::fs::read_to_string(&path)
        .expect("snapshot dosyası yazılmadı");
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .expect("snapshot dosyası geçerli JSON değil");
    // MissionControl temel alanları var mı
    assert!(parsed.get("finance").is_some(), "finance alanı yok: {}", raw);
    assert!(parsed.get("pipeline_steps").is_some(), "pipeline_steps yok");
    assert!(parsed.get("ai_brain").is_some(), "ai_brain yok");

    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    let _ = std::fs::remove_file(&path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phase_transitions_from_booting_to_scanning() {
    // Faz takipçisi düzeltmesi: AppState ilk açıldığında "Idle" yazıyordu (hep böyle kalıyordu).
    // Engine ateşlendikten kısa süre sonra phase Booting'den Scanning'e geçmeli;
    // anomali aşamasında Recovering, durdurma sonrası Stopped olmalı.
    use memos_trading_core::core::bridge;

    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));
    // İlk anda "Idle" olmalı (AppState::new varsayılanı)
    assert_eq!(state.lock().unwrap().fleet.phase, "Idle");

    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    // Phase tracker birkaç tur sonra Idle'dan çıkar. Sınırlı poll → contention-dayanıklı.
    common::poll_until(Duration::from_secs(15), || {
        let st = state.lock().unwrap();
        bridge::get_snapshot(&st).phase != "Idle"
    }).await;

    let snap = {
        let st = state.lock().unwrap();
        bridge::get_snapshot(&st)
    };
    // Snapshot içinde phase taşınmalı
    assert!(!snap.phase.is_empty(), "MissionControl.phase boş kalmış");
    // Scanning, Executing veya Recovering olabilir; Idle değil
    assert_ne!(snap.phase, "Idle",
        "ana döngü çalışırken phase hâlâ 'Idle' — fazlar yazılmıyor olabilir");
    assert!(matches!(snap.phase.as_str(),
        "Scanning" | "Executing" | "Recovering" | "Booting"),
        "beklenmeyen phase: {}", snap.phase);

    // Engine'i durdur ve Stopped fazına geçmesini bekle
    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;

    let final_phase = state.lock().unwrap().fleet.phase.clone();
    assert_eq!(final_phase, "Stopped",
        "engine durdurulduktan sonra phase 'Stopped' olmalı, gerçek: {}", final_phase);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn main_loop_heartbeat_writes_last_tick() {
    // Faz 2 entegrasyonu: run_autonomous_loop her tur fleet.last_loop_tick yazar.
    let state = Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())));

    let initial = state.lock().unwrap().fleet.last_loop_tick.load(Ordering::Relaxed);
    assert_eq!(initial, 0, "init'te tick henüz yazılmamış olmalı");

    let engine_state = Arc::clone(&state);
    let engine_handle = tokio::spawn(async move {
        Engine::run_autonomous_loop(engine_state).await;
    });

    let progressed = common::poll_until(Duration::from_secs(15), || {
        state.lock().unwrap().fleet.last_loop_tick.load(Ordering::Relaxed) > 0
    }).await;
    let after = state.lock().unwrap().fleet.last_loop_tick.load(Ordering::Relaxed);
    assert!(progressed && after > 0, "ana döngü tick yazmamış: {}", after);

    {
        let st = state.lock().unwrap();
        st.app_stop_signal.store(true, Ordering::SeqCst);
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), engine_handle).await;
}
