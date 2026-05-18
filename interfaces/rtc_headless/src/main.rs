// rtc_headless/src/main.rs - Headless Robotik Trade Başlatıcı (Yeni Bakanlık Mimarisi)
//
// Yeni AppState (4 bakanlık: finance / brain / fleet / guardian) üzerinden
// Master Engine'in otonom döngüsünü ateşler. Ctrl+C ile graceful shutdown.

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::{env, fs};
use anyhow::Result;
use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::robotic_loop::AppState;
use memos_trading_core::robot::engines::master::Engine;

// --- Profil Yapılandırması (config/robotic_profiles.json'dan okunur) ---

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct ProfileConfig {
    position_profile: String,
    security_profile: String,
    #[serde(default)] sl_cooldown_secs:  Option<u64>,
    #[serde(default)] breakeven_at_rr:   Option<f64>,
    #[serde(default)] atr_trail_mult:    Option<f64>,
    #[serde(default)] partial_tp_ratio:  Option<f64>,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            position_profile:  "Balanced".to_owned(),
            security_profile:  "Production".to_owned(),
            sl_cooldown_secs:  Some(300),
            breakeven_at_rr:   Some(1.0),
            atr_trail_mult:    Some(2.0),
            partial_tp_ratio:  Some(0.5),
        }
    }
}

fn load_profiles() -> ProfileConfig {
    fs::read_to_string("config/robotic_profiles.json")
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

// --- Ana çalışma zamanı ---

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Ortam Değişkenleri + global logger
    dotenvy::dotenv().ok();
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .try_init();

    let symbol      = env::var("TRADE_SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_owned());
    let market      = env::var("TRADE_MARKET").unwrap_or_else(|_| "spot".to_owned());
    let is_paper    = env::var("BINANCE_PAPER_MODE").map(|v| v == "true").unwrap_or(true);
    let db_path     = env::var("DB_PATH").unwrap_or_else(|_| "data/memos_trading.db".to_owned());

    println!("⚡ [INIT] rtc_headless | sembol={} | borsa={} | mod={}",
        symbol, market, if is_paper { "PAPER" } else { "LIVE" });

    // 2. Klasörlerin varlığı
    fs::create_dir_all("logs").ok();
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        fs::create_dir_all(parent).ok();
    }

    // 3. Profil yüklemesi (best_params'a yansıtılır)
    let profile = load_profiles();
    let config = RoboticLoopConfig {
        symbol: symbol.clone(),
        market: market.clone(),
        db_path: db_path.clone(),
        ..Default::default()
    };

    // Pozisyon yönetimi parametreleri config'den geliyor ama yeni AppState onları
    // brain.best_params üzerinden tutuyor — burada init'ten sonra dolduracağız.
    let _ = profile;

    // 4. AppState (4 Bakanlık) inşa
    let state = Arc::new(Mutex::new(AppState::new(config)));

    // 5. Profil parametrelerini brain.best_params'a sızdır
    {
        let mut st = state.lock().unwrap();
        if let Some(v) = profile_value(&load_profiles().sl_cooldown_secs.map(|x| x as f64)) {
            st.brain.best_params.insert("pos_sl_cooldown".into(), v);
        }
        if let Some(v) = load_profiles().breakeven_at_rr {
            st.brain.best_params.insert("pos_breakeven_at_rr".into(), v);
        }
        if let Some(v) = load_profiles().atr_trail_mult {
            st.brain.best_params.insert("pos_atr_trail_mult".into(), v);
        }
        if let Some(v) = load_profiles().partial_tp_ratio {
            st.brain.best_params.insert("pos_partial_tp_ratio".into(), v);
        }
        st.push_log(format!("Profil yüklendi: {:?}", load_profiles().position_profile));
    }

    // 6. Graceful shutdown (Ctrl+C)
    let stop_handle = {
        let st = state.lock().unwrap();
        Arc::clone(&st.app_stop_signal)
    };
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            println!("\n🛑 [SHUTDOWN] Sinyal alındı. Engine durduruluyor...");
            stop_handle.store(true, Ordering::SeqCst);
        }
    });

    // 6b. Ortak snapshot writer (TUI/Android/Web istemciler bu dosyayı okur)
    let snap_path = std::env::var("SNAPSHOT_PATH")
        .unwrap_or_else(|_| "data/snapshot.json".to_owned());
    if let Some(parent) = std::path::Path::new(&snap_path).parent() {
        fs::create_dir_all(parent).ok();
    }
    memos_trading_core::robot::infra::snapshot_writer::spawn_snapshot_writer(
        Arc::clone(&state),
        snap_path.clone(),
        1, // her saniye
    );
    println!("📤 [SNAPSHOT] {} (her 1s) Android/web istemciler için yazılıyor", snap_path);

    // 7. Master Engine'i ateşle
    println!("🚀 [START] Master Engine devriye giriyor...");
    Engine::run_autonomous_loop(Arc::clone(&state)).await;

    println!("🏁 [EXIT] Pipeline güvenli bir şekilde durduruldu.");
    Ok(())
}

/// Option<f64>'i değer olarak döndürür (yardımcı: yukarıdaki insert akışını okunabilir tutar).
fn profile_value(v: &Option<f64>) -> Option<f64> { *v }
