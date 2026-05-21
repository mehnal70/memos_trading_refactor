// Download Job Integration Testi
//
// Trigger 'download' geldiğinde aktif sembollerin mumlarını çekip SQLite'a yazmalı.
// Test sembol filosunu test scope'da kuruyoruz (BTCUSDT 1m), geçici DB kullanıyoruz.
// Internet erişimi yoksa test başarısız olur — bu durumda IGNORE_NETWORK env'i set'lenebilir.

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use memos_trading_core::core::model::RoboticLoopConfig;
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

// Gerçek HTTP çağrısı (Binance API'den candle çeker); offline/CI'da sahte fail
// verir. Manuel: `cargo test --test download_job -- --ignored`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "external network: Binance API"]
async fn download_trigger_creates_candle_table_for_symbol() {

    let tmp_db = format!("/tmp/memos_download_test_{}.db", std::process::id());
    let _ = std::fs::remove_file(&tmp_db);

    let config = RoboticLoopConfig {
        symbol: "BTCUSDT".into(),
        market: "spot".into(),
        interval: "1m".into(),
        db_path: tmp_db.clone(),
        // küçük kanepe, hızlı test
        download_candle_limit: 50,
        pinned_symbols: vec![], // sadece config.symbol
        ..Default::default()
    };

    let state = Arc::new(Mutex::new(AppState::new(config)));
    let engine_state = Arc::clone(&state);
    let h = tokio::spawn(async move { Engine::run_autonomous_loop(engine_state).await; });

    // Trigger handler 250ms aralıkta okuyor, warm-up
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Download trigger pulse
    state.lock().unwrap().fleet.triggers.get("download").unwrap()
        .store(true, Ordering::Relaxed);

    // Job ağ çağrısı + DB yazımı + tek-thread cycle paylaşımı → cömert pencere
    tokio::time::sleep(Duration::from_secs(12)).await;

    // Logs içeriği — "Download" mesajları görülmeli
    let logs: Vec<String> = state.lock().unwrap().guardian.log.iter().cloned().collect();
    let saw_start = logs.iter().any(|l| l.contains("🌐 Download başladı"));
    assert!(saw_start, "Download başlangıç logu bulunamadı. Logs: {:#?}", logs);

    // İndirme sonuç logu (✓ veya ❌)
    let saw_done = logs.iter().any(|l| l.contains("🌐 Download ✓"))
                || logs.iter().any(|l| l.contains("Download başarısız"));
    assert!(saw_done, "Download sonuç logu bulunamadı. Logs: {:#?}", logs);

    // Eğer başarıyla bittiyse → ana `candles` tablosunda BTCUSDT/1m satırları olmalı.
    // (Eski şema: per-symbol `candles_btcusdt_1m`. Yeni şema: tek tablo, symbol+interval kolonu.)
    if logs.iter().any(|l| l.contains("🌐 Download ✓")) {
        let conn = rusqlite::Connection::open(&tmp_db).expect("db açılmadı");
        let table_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='candles')",
            [],
            |row| row.get(0),
        ).unwrap_or(false);
        assert!(table_exists, "candles tablosu DB'de oluşturulmamış");

        let count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM candles WHERE symbol = ?1 AND interval = ?2",
            rusqlite::params!["BTCUSDT", "1m"],
            |row| row.get(0),
        ).unwrap_or(0);
        assert!(count > 0, "candles tablosunda BTCUSDT/1m mumu yok (count=0)");
    }

    // Engine kapat
    state.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    let _ = std::fs::remove_file(&tmp_db);
}
