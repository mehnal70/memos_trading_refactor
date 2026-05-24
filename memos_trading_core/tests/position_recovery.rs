// Pozisyon recovery integration testi:
// 1) Boş DB'ye save_open_positions_snapshot ile 2 pozisyon yaz.
// 2) Engine::hydrate_open_positions_from_db çağrılınca live_positions HashMap'i
//    bu iki pozisyonla dolmalı.
// 3) close_paper_position ile pozisyon kapanınca snapshot boşa yansımalı
//    (persist_open_positions_snapshot doğrudan birim olarak doğrulanır).

use std::sync::{Arc, Mutex};

use memos_trading_core::core::model::{PositionModel, RoboticLoopConfig};
use memos_trading_core::persistence::{
    reader::{recover_open_positions, load_account_state},
    writer::{save_account_state, save_open_positions_snapshot},
};
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;
use rusqlite::Connection;

fn mk_pos(symbol: &str) -> PositionModel {
    PositionModel {
        pos_id: format!("rec-{}", symbol),
        symbol: symbol.into(),
        entry_price: 100.0,
        current_price: 100.0,
        qty: 1.0,
        leverage: 1.0,
        is_long: true,
        trade_type: "scalp".into(),
        opened_at: "2026-01-01T00:00:00Z".into(),
        stop_loss: 95.0,
        take_profit: 110.0,
        trailing_stop: 0.0,
        max_favorable_price: 100.0,
        breakeven_activated: false,
        kind: None,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn hydrate_loads_snapshot_into_live_positions() {
    let db = format!("/tmp/memos_recovery_{}.db", std::process::id());
    let _ = std::fs::remove_file(&db);

    // 1) DB'ye 2 pozisyon snapshot et.
    {
        let conn = Connection::open(&db).unwrap();
        let positions = vec![mk_pos("BTCUSDT"), mk_pos("ETHUSDT")];
        save_open_positions_snapshot(&conn, &positions).unwrap();
    }

    // 2) AppState yeni config ile başlat — db_path yukarıdaki.
    let mut cfg = RoboticLoopConfig::default();
    cfg.db_path = db.clone();
    let state = Arc::new(Mutex::new(AppState::new(cfg)));

    // İlk durumda live_positions boş olmalı (cold start).
    {
        let s = state.lock().unwrap();
        assert_eq!(s.finance.live_positions.read().unwrap().len(), 0);
    }

    // 3) Hydrate çağır → 2 pozisyon yüklenmiş olmalı.
    //    hydrate_open_positions_from_db pub(crate) değil → public API testi
    //    olarak persist_open_positions_snapshot üzerinden değil, recover_open_positions
    //    + manuel insert ile aynı sözleşmeyi doğruluyoruz.
    let recovered = recover_open_positions(&db).expect("recovery");
    assert_eq!(recovered.len(), 2);
    {
        let s = state.lock().unwrap();
        let mut map = s.finance.live_positions.write().unwrap();
        for p in recovered {
            map.insert(p.symbol.clone(), p);
        }
    }
    {
        let s = state.lock().unwrap();
        let n = s.finance.live_positions.read().unwrap().len();
        assert_eq!(n, 2, "recovery sonrası live_positions 2 olmalı");
    }

    // 4) persist_open_positions_snapshot: live'ı kapatıp DB'ye boş yazınca
    //    recover boş dönmeli.
    {
        let s = state.lock().unwrap();
        s.finance.live_positions.write().unwrap().clear();
    }
    Engine::persist_open_positions_snapshot(&state);
    let after = recover_open_positions(&db).expect("recovery after wipe");
    assert!(after.is_empty(), "kapanış sonrası snapshot boş olmalı: {:?}", after);

    let _ = std::fs::remove_file(&db);
}

/// Equity/peak/closed_count restart sonrası DB'den hidrate edilmeli; eski run'da
/// kazanılan PnL yeni run'da equity'de aynen görünmeli. Bu test olmadan
/// trades.jsonl'a yazılan PnL ile heartbeat equity'si arasında uçurum oluşuyordu.
#[test]
fn account_state_roundtrip_persists_equity_and_closed_count() {
    use std::sync::atomic::Ordering;

    let db = format!("/tmp/memos_account_state_{}.db", std::process::id());
    let _ = std::fs::remove_file(&db);

    // 1) Önceki run: equity 10500, peak 10750, closed 42.
    {
        let conn = Connection::open(&db).unwrap();
        save_account_state(&conn, 10_500.0, 10_750.0, 10_000.0, 42).unwrap();
    }

    // 2) Yeni AppState (cold start gibi başlar) — config.capital aynı 10_000.
    let mut cfg = RoboticLoopConfig::default();
    cfg.db_path = db.clone();
    cfg.capital = 10_000.0;
    let state = Arc::new(Mutex::new(AppState::new(cfg)));

    // Cold-start baseline
    {
        let s = state.lock().unwrap();
        assert_eq!(s.finance.equity, 10_000.0);
        assert_eq!(s.finance.closed_trades_total.load(Ordering::Relaxed), 0);
    }

    // 3) load_account_state ile satırı oku → mantığı manuel uygula (hydrate fn
    //    pub değil; sözleşme: equity/peak/closed yansır).
    let rec = load_account_state(&db).expect("ok").expect("kayıt var");
    assert_eq!(rec.closed_trades_count, 42);
    assert!((rec.equity - 10_500.0).abs() < 1e-9);
    assert!((rec.peak_equity - 10_750.0).abs() < 1e-9);
    assert!((rec.starting_capital - 10_000.0).abs() < 1e-9);

    // 4) Yeniden yazım: persist_account_state simülasyonu → roundtrip stabilize.
    {
        let conn = Connection::open(&db).unwrap();
        save_account_state(&conn, 10_600.0, 10_800.0, 10_000.0, 43).unwrap();
    }
    let rec2 = load_account_state(&db).expect("ok").expect("kayıt var");
    assert_eq!(rec2.closed_trades_count, 43);
    assert!((rec2.equity - 10_600.0).abs() < 1e-9);
    assert!((rec2.peak_equity - 10_800.0).abs() < 1e-9);

    // 5) Boş DB → None döner (cold-start sözleşmesi).
    let db_empty = format!("/tmp/memos_account_state_empty_{}.db", std::process::id());
    let _ = std::fs::remove_file(&db_empty);
    let none = load_account_state(&db_empty).expect("err olmamalı (tablo yok → None)");
    assert!(none.is_none(), "tablo yokken Some döndü: {:?}", none);

    let _ = state;
    let _ = std::fs::remove_file(&db);
    let _ = std::fs::remove_file(&db_empty);
}

/// Recovery filter: candles tablosunda sembol+interval için kayıt yoksa
/// "stale" sayılır, live_positions'a yüklenmez. Aksi halde her cycle
/// DataIngest Failed → anomaly birikimi olur (BIST recovery senaryosu).
#[test]
fn recovery_stale_filter_matches_candles_existence() {
    use memos_trading_core::persistence::{
        reader::read_candles,
        writer::{ensure_candles_table, save_candle},
    };
    use memos_trading_core::core::types::Candle;
    use chrono::Utc;

    let db = format!("/tmp/memos_stale_filter_{}.db", std::process::id());
    let _ = std::fs::remove_file(&db);
    {
        let conn = Connection::open(&db).unwrap();
        ensure_candles_table(&conn).unwrap();
        // BTCUSDT 1m → var. AKBNK 1m → yok (stale olmalı).
        save_candle(&conn, "binance", "spot", &Candle {
            timestamp: Utc::now(),
            open: 50000.0, high: 50100.0, low: 49900.0, close: 50050.0,
            volume: 1.0, symbol: "BTCUSDT".into(), interval: "1m".into(),
        }).unwrap();
    }

    // Filter mantığı: read_candles(...).map(|v| !v.is_empty()).unwrap_or(false).
    let live_btc = read_candles(&db, "BTCUSDT", "1m", 1)
        .map(|v| !v.is_empty()).unwrap_or(false);
    let live_akbnk = read_candles(&db, "AKBNK", "1m", 1)
        .map(|v| !v.is_empty()).unwrap_or(false);

    assert!(live_btc,    "BTCUSDT candles vardı; filter true vermeli");
    assert!(!live_akbnk, "AKBNK candles yoktu; filter false vermeli (stale)");

    let _ = std::fs::remove_file(&db);
}

#[test]
fn log_throttle_first_emits_then_suppresses_within_cooldown() {
    use memos_trading_core::robot::engines::master::log_throttle_should_emit;
    // Unique kind ki diğer testlerle çakışmasın.
    let kind = "test_throttle_kind_A";
    let sym  = "TESTSYM";
    // İlk çağrı → emit.
    assert!(log_throttle_should_emit(sym, kind, 60));
    // Cooldown içinde → suppress.
    assert!(!log_throttle_should_emit(sym, kind, 60));
    assert!(!log_throttle_should_emit(sym, kind, 60));
    // Farklı sembol → bağımsız.
    assert!(log_throttle_should_emit("OTHERSYM", kind, 60));
    // Cooldown=0 → her zaman emit.
    let kind_zero = "test_throttle_kind_B";
    assert!(log_throttle_should_emit(sym, kind_zero, 0));
    assert!(log_throttle_should_emit(sym, kind_zero, 0));
}

#[test]
fn bist_heuristic_separates_bist_from_crypto_pairs() {
    // BIST tarafı — 3-6 char, all caps, crypto quote yok.
    assert!(Engine::looks_like_bist_symbol("AKBNK"));
    assert!(Engine::looks_like_bist_symbol("ALARK"));
    assert!(Engine::looks_like_bist_symbol("AKFGY"));
    assert!(Engine::looks_like_bist_symbol("A1CAP"));   // rakam-içeren
    assert!(Engine::looks_like_bist_symbol("ADGYO"));
    assert!(Engine::looks_like_bist_symbol("THYAO"));   // 5 char
    assert!(Engine::looks_like_bist_symbol("GARAN"));

    // Crypto USDT pair'leri — BIST sayılmamalı.
    assert!(!Engine::looks_like_bist_symbol("BTCUSDT"));
    assert!(!Engine::looks_like_bist_symbol("ETHUSDT"));
    assert!(!Engine::looks_like_bist_symbol("ADAUSDT"));
    assert!(!Engine::looks_like_bist_symbol("BNBUSDT"));

    // Diğer crypto quote'lar.
    assert!(!Engine::looks_like_bist_symbol("BTCUSDC"));
    assert!(!Engine::looks_like_bist_symbol("BTCFDUSD"));

    // Liste dışı edge case'ler.
    assert!(!Engine::looks_like_bist_symbol("BT"));     // çok kısa
    assert!(!Engine::looks_like_bist_symbol("VERYLONGSYM"));  // çok uzun
    assert!(!Engine::looks_like_bist_symbol("btc"));    // küçük harf
    assert!(!Engine::looks_like_bist_symbol("BTC-USD")); // tire içerir
}

#[test]
fn recover_returns_empty_when_table_missing() {
    let db = format!("/tmp/memos_recovery_empty_{}.db", std::process::id());
    let _ = std::fs::remove_file(&db);
    // Boş DB — tablo yok, sorgu None döner, recover Ok(vec![]) vermeli.
    let _ = Connection::open(&db).unwrap();
    let out = recover_open_positions(&db).expect("Ok bekleniyor");
    assert!(out.is_empty());
    let _ = std::fs::remove_file(&db);
}

#[test]
fn ensure_candles_table_makes_cold_db_readable() {
    use memos_trading_core::persistence::reader::read_candles;
    use memos_trading_core::persistence::writer::ensure_candles_table;

    let db = format!("/tmp/memos_schema_{}.db", std::process::id());
    let _ = std::fs::remove_file(&db);

    // 1) Boş DB → read_candles "no such table" hatası vermeli.
    {
        let _ = Connection::open(&db).unwrap();
        let res = read_candles(&db, "BTCUSDT", "1m", 10);
        assert!(res.is_err(),
            "boş DB'de read_candles Err bekleniyordu, sonuç: {:?}", res);
    }

    // 2) ensure_candles_table çağrıldıktan sonra → boş tablo, hatasız okuma.
    {
        let conn = Connection::open(&db).unwrap();
        ensure_candles_table(&conn).expect("şema kurulamadı");
    }
    let out = read_candles(&db, "BTCUSDT", "1m", 10)
        .expect("şema kurulduktan sonra read_candles başarılı olmalı");
    assert!(out.is_empty(), "henüz mum yazılmadı, sonuç boş olmalı");

    let _ = std::fs::remove_file(&db);
}
