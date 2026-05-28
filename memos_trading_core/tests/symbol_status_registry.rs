// exchangeInfo sembol-statü registry — cache gate + DB round-trip doğrulaması.
//
// status != "TRADING" (BREAK/HALT/delisted, örn. ALPACAUSDT) → symbol_eligible_for_live
// reddeder. Bilinmeyen sembol (registry'de yok) → izinli (registry dolmadan startup kırılmaz).

use memos_trading_core::persistence::reader::load_symbol_statuses;
use memos_trading_core::persistence::writer::save_symbol_statuses;
use memos_trading_core::robot::engines::master::{
    is_symbol_tradeable, set_symbol_statuses, symbol_status_registry_len, RuntimeTuning,
};

#[test]
fn registry_gate_tradeable_break_unknown() {
    let t = RuntimeTuning::default();
    set_symbol_statuses(&[
        ("AAAUSDT".into(), "TRADING".into()),
        ("BBBUSDT".into(), "BREAK".into()),
    ]);
    assert!(symbol_status_registry_len() >= 2);

    // is_symbol_tradeable
    assert!(is_symbol_tradeable("AAAUSDT"), "TRADING → işlem görebilir");
    assert!(!is_symbol_tradeable("BBBUSDT"), "BREAK → işlem göremez");
    assert!(is_symbol_tradeable("ZZUNKNOWNUSDT"), "registry'de yok → izinli (default)");

    // eligibility gate (tek-nokta)
    assert!(t.symbol_eligible_for_live("AAAUSDT"), "TRADING → uygun");
    assert!(!t.symbol_eligible_for_live("BBBUSDT"), "BREAK → uygun DEĞİL");
    assert!(t.symbol_eligible_for_live("ZZUNKNOWNUSDT"), "bilinmeyen → uygun");
}

#[test]
fn db_round_trip_and_upsert() {
    let db = format!("/tmp/memos_symstatus_{}.db", std::process::id());
    let _ = std::fs::remove_file(&db);
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        save_symbol_statuses(&conn, &[
            ("BTCUSDT".into(), "TRADING".into()),
            ("ALPACAUSDT".into(), "BREAK".into()),
        ]).expect("save başarısız");
        // Aynı sembolü güncelle (re-list senaryosu) → upsert.
        save_symbol_statuses(&conn, &[("ALPACAUSDT".into(), "TRADING".into())]).expect("upsert başarısız");
    }
    let loaded = load_symbol_statuses(&db).expect("load başarısız");
    let alpaca = loaded.iter().find(|(s, _)| s == "ALPACAUSDT").map(|(_, st)| st.clone());
    assert_eq!(alpaca.as_deref(), Some("TRADING"), "upsert statüyü güncellemeli");
    assert!(loaded.iter().any(|(s, st)| s == "BTCUSDT" && st == "TRADING"));
    let _ = std::fs::remove_file(&db);
}

#[test]
fn load_missing_table_returns_empty() {
    let db = format!("/tmp/memos_symstatus_empty_{}.db", std::process::id());
    let _ = std::fs::remove_file(&db);
    let _ = rusqlite::Connection::open(&db).unwrap(); // boş DB, tablo yok
    let loaded = load_symbol_statuses(&db).expect("tablo yokken hata değil boş dönmeli");
    assert!(loaded.is_empty());
    let _ = std::fs::remove_file(&db);
}
