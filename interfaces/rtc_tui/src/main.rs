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

    // --- Env değişkenlerinden config'i süzdür (rtc_headless ile aynı sözleşme) ---
    let symbol  = std::env::var("TRADE_SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_owned());
    let market  = std::env::var("TRADE_MARKET").unwrap_or_else(|_| "spot".to_owned());
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".to_owned());

    // Trading mode env önceliği (headless ile aynı kontrat):
    //   1. TRADING_MODE=Live|Paper|Backtest
    //   2. Legacy BINANCE_PAPER_MODE=false → Live
    //   3. Default: Paper
    let trading_mode = if let Ok(v) = std::env::var("TRADING_MODE") {
        memos_trading_core::core::model::TradingMode::from_env_str(&v)
    } else if std::env::var("BINANCE_PAPER_MODE").map(|v| v == "false").unwrap_or(false) {
        memos_trading_core::core::model::TradingMode::Live
    } else {
        memos_trading_core::core::model::TradingMode::Paper
    };
    let api_key = std::env::var("BINANCE_API_KEY").ok();
    let secret_key = std::env::var("BINANCE_API_SECRET").ok();

    // Klasörlerin varlığını garantile (logs/ ve data/ üst dizini)
    let _ = std::fs::create_dir_all("logs");
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Başlangıç sermayesi: env STARTING_CAPITAL (rtc_headless ile aynı sözleşme).
    // Geçersiz/eksik → RoboticLoopConfig default'u = $10.000.
    let capital = std::env::var("STARTING_CAPITAL").ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or_else(|| RoboticLoopConfig::default().capital);

    let config = RoboticLoopConfig {
        symbol: symbol.clone(),
        market: market.clone(),
        db_path: db_path.clone(),
        trading_mode,
        capital,
        api_key,
        secret_key,
        ..Default::default()
    };
    let state = Arc::new(Mutex::new(AppState::new(config)));

    // SQLite bağlantısının gerçekten kurulduğunu doğrula; başarısız ise kullanıcıya bildir.
    {
        let st = state.lock().unwrap();
        if st.guardian.db_conn.is_none() {
            eprintln!("⚠️  Uyarı: SQLite bağlantısı kurulamadı (path={}). Persistence devre dışı.", db_path);
        } else {
            println!("⚡ [INIT] rtc_tui | sembol={} | borsa={} | db={} | mod={}",
                symbol, market, db_path, st.config.trading_mode.as_str());
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
