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

    // --- AppState (4 Bakanlık) inşa ---
    let config = RoboticLoopConfig::default();
    let state = Arc::new(Mutex::new(AppState::new(config)));

    // --- Master Engine'i arka planda ateşle (TUI'den bağımsız) ---
    let engine_state = Arc::clone(&state);
    tokio::spawn(async move {
        Engine::run_autonomous_loop(engine_state).await;
    });

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
