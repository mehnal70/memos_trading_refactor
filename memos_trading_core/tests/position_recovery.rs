// Pozisyon recovery integration testi:
// 1) Boş DB'ye save_open_positions_snapshot ile 2 pozisyon yaz.
// 2) Engine::hydrate_open_positions_from_db çağrılınca live_positions HashMap'i
//    bu iki pozisyonla dolmalı.
// 3) close_paper_position ile pozisyon kapanınca snapshot boşa yansımalı
//    (persist_open_positions_snapshot doğrudan birim olarak doğrulanır).

use std::sync::{Arc, Mutex};

use memos_trading_core::core::model::{PositionModel, RoboticLoopConfig};
use memos_trading_core::persistence::{
    reader::recover_open_positions,
    writer::save_open_positions_snapshot,
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
