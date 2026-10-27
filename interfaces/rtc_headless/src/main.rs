// rtc_headless/src/main.rs - Headless Robotik Trade Başlatıcı (Yeni Bakanlık Mimarisi)
//
// Yeni AppState (4 bakanlık: finance / brain / fleet / guardian) üzerinden
// Master Engine'in otonom döngüsünü ateşler. Ctrl+C ile graceful shutdown.

use anyhow::Result;
use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;
use std::fs;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .try_init();

    // ── 🧠 MERKEZİ OTONOM KONFİGÜRASYON MOTORU ENTEGRASYONU ────────────────────
    // Statik JSON dosyalarından profil yükleme kısıtlamasını tamamen kaldırıyoruz.
    // rtc_headless bacağı da artık doğrudan merkezi otonom config_engine standartlarını konuşuyor.
    let central_cfg =
        memos_trading_core::robot::config_engine::OtonomConfigEngine::load_central_config();

    let mut config = RoboticLoopConfig::from_env();
    config.db_path = central_cfg.db_path;
    config.interval = central_cfg.interval;
    config.pinned_symbols = central_cfg.pinned_symbols;
    config.download_candle_limit = central_cfg.download_candle_limit;

    println!(
        "⚡ [INIT] rtc_headless | sembol={} | borsa={} | interval={} | mod={}",
        config.symbol,
        config.market,
        config.interval,
        config.trading_mode.as_str()
    );

    if matches!(
        config.trading_mode,
        memos_trading_core::core::model::TradingMode::Live
    ) && (config.api_key.is_none() || config.secret_key.is_none())
    {
        eprintln!(
            "⚠️ TRADING_MODE=Live ama API anahtarları eksik → Fallback senaryoları denetleniyor"
        );
    }

    fs::create_dir_all("logs").ok();
    if let Some(parent) = std::path::Path::new(&config.db_path).parent() {
        fs::create_dir_all(parent).ok();
    }

    let state = Arc::new(Mutex::new(AppState::new(config)));
    memos_trading_core::robot::infra::app_state_lock_watchdog::spawn(Arc::clone(&state));

    let metrics_bind = std::env::var("METRICS_BIND").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let metrics_port = std::env::var("METRICS_PORT").unwrap_or_else(|_| "9100".to_owned());
    let metrics_addr = format!("{}:{}", metrics_bind, metrics_port);
    memos_trading_core::robot::infra::telemetry::start_metrics_server(
        Arc::clone(&state),
        &metrics_addr,
    )
    .await?;

    // Otonom parametre tohumlaması: Boş kalan risk parametrelerini varsayılan akıllı değerlerle dolduruyoruz
    {
        let mut st = state.lock().unwrap();
        st.brain.best_params.insert("pos_sl_cooldown".into(), 300.0);
        st.brain
            .best_params
            .insert("pos_breakeven_at_rr".into(), 1.0);
        st.brain
            .best_params
            .insert("pos_atr_trail_mult".into(), 2.0);
        st.brain
            .best_params
            .insert("pos_partial_tp_ratio".into(), 0.5);
        st.push_log(
            "🤖 Headless mod: Otonom risk ve marjin parametreleri başarıyla enjekte edildi."
                .to_string(),
        );
    }

    let stop_handle = {
        let st = state.lock().unwrap();
        Arc::clone(&st.app_stop_signal)
    };
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            println!(
                "\n🛑 [SHUTDOWN] Sinyal alındı. Headless motor güvenli durdurma fazına geçiyor..."
            );
            stop_handle.store(true, Ordering::SeqCst);
        }
    });

    let snap_path =
        std::env::var("SNAPSHOT_PATH").unwrap_or_else(|_| "data/snapshot.json".to_owned());
    if let Some(parent) = std::path::Path::new(&snap_path).parent() {
        fs::create_dir_all(parent).ok();
    }
    memos_trading_core::robot::infra::snapshot_writer::spawn_snapshot_writer(
        Arc::clone(&state),
        snap_path.clone(),
        1,
    );
    println!(
        "📤 [SNAPSHOT] {} (her 1s) Android/web istemciler için yazılıyor",
        snap_path
    );

    println!("🚀 [START] Headless Master Engine devriye giriyor...");
    Engine::run_autonomous_loop(Arc::clone(&state)).await;

    println!("🏁 [EXIT] Headless pipeline güvenli bir şekilde durduruldu.");
    return Ok(());
}
