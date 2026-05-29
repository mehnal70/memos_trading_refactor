// interfaces/rtc_tui/src/main.rs - Srivastava ATP TUI Başlatıcı
//
// Yeni AppState (4 bakanlık) üzerinden Master Engine'i arka planda ateşler,
// ön planda ratatui TUI ile snapshot'ları çizer. Ctrl+C ile graceful shutdown.

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;

mod ui;
mod handlers;

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::robotic_loop::AppState;
use memos_trading_core::robot::engines::master::Engine;
use crate::handlers::input::TuiManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // --- Env → config TEK KAYNAK: RoboticLoopConfig::from_env (rtc_headless ile ortak).
    // Eskiden her main env okumasını kopyalıyordu; TUI `TRADE_INTERVAL`'i düşürmüştü →
    // hep 1m koşuyordu. Artık tek nokta → tekrar yok, divergence imkânsız.
    let config = RoboticLoopConfig::from_env();

    // Klasörlerin varlığını garantile (logs/ ve data/ üst dizini)
    let _ = std::fs::create_dir_all("logs");
    if let Some(parent) = std::path::Path::new(&config.db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let state = Arc::new(Mutex::new(AppState::new(config)));

    // SQLite bağlantısının gerçekten kurulduğunu doğrula; başarısız ise kullanıcıya bildir.
    {
        let st = state.lock().unwrap();
        if st.guardian.db_conn.is_none() {
            eprintln!("⚠️  Uyarı: SQLite bağlantısı kurulamadı (path={}). Persistence devre dışı.", st.config.db_path);
        } else {
            println!("⚡ [INIT] rtc_tui | sembol={} | borsa={} | interval={} | db={} | mod={}",
                st.config.symbol, st.config.market, st.config.interval, st.config.db_path,
                st.config.trading_mode.as_str());
        }
    }

    // --- Master Engine'i arka planda ateşle (TUI'den bağımsız) ---
    let engine_state = Arc::clone(&state);
    tokio::spawn(async move {
        Engine::run_autonomous_loop(engine_state).await;
    });

    // --- Ortak Snapshot Writer (Android/Web istemciler bu JSON'u poll eder) ---
    let snap_path = std::env::var("SNAPSHOT_PATH")
        .unwrap_or_else(|_| "data/snapshot.json".to_owned());
    if let Some(parent) = std::path::Path::new(&snap_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    memos_trading_core::robot::infra::snapshot_writer::spawn_snapshot_writer(
        Arc::clone(&state),
        snap_path,
        1,
    );

    // --- Mod seçimi: TUI mu, headless mi? ---
    let args: Vec<String> = std::env::args().collect();
    let is_headless = args.contains(&"--headless".to_string());

    if is_headless {
        println!("🚀 Srivastava ATP Headless Mod: Robot devriyede...");
        // Ctrl+C beklerken state'in stop sinyalini de izle
        tokio::signal::ctrl_c().await?;
        let st = state.lock().unwrap();
        st.app_stop_signal.store(true, Ordering::SeqCst);
    } else {
        // TUI MODU
        let mut tui_manager = TuiManager::new();
        if let Err(e) = tui_manager.spawn_tui_loop(Arc::clone(&state)).await {
            eprintln!("🚨 Srivastava TUI Kritik Hata: {}", e);
        }
        // TUI sona erdiyse engine'i de durdur
        let st = state.lock().unwrap();
        st.app_stop_signal.store(true, Ordering::SeqCst);
    }

    println!("✅ Harekât sonlandırıldı.");
    Ok(())
}
