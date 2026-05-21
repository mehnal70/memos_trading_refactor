// persistence::reader::list_symbols testleri.
//
// Önceki bug: paper_trading_results tablosundan okuyordu → işlem yapılmadıkça
// boş → screener havuzu hep boş. Şimdi candles tablosundan okuyor (indirilmiş
// ham mum havuzu). Bu test temp DB ile davranışı sabitler.

use rusqlite::Connection;

use memos_trading_core::persistence::reader::list_symbols;

fn make_temp_db_with_candles(tag: &str, symbols: &[&str]) -> String {
    let path = format!("/tmp/memos_list_symbols_{}_{}.db", std::process::id(), tag);
    let _ = std::fs::remove_file(&path);
    let conn = Connection::open(&path).expect("temp DB aç");
    conn.execute_batch(
        "CREATE TABLE candles (\
            symbol TEXT NOT NULL, interval TEXT NOT NULL, \
            timestamp INTEGER NOT NULL, \
            open REAL, high REAL, low REAL, close REAL, volume REAL \
        );",
    )
    .expect("candles tablosu");
    for sym in symbols {
        conn.execute(
            "INSERT INTO candles (symbol, interval, timestamp, open, high, low, close, volume) \
             VALUES (?1, '1m', 1700000000000, 1.0, 1.0, 1.0, 1.0, 1.0)",
            [sym],
        )
        .expect("candle insert");
    }
    path
}

#[test]
fn list_symbols_returns_distinct_from_candles() {
    let path = make_temp_db_with_candles("distinct", &["BTCUSDT", "ETHUSDT", "BTCUSDT", "ADAUSDT"]);
    let mut got = list_symbols(&path).expect("list_symbols");
    got.sort();
    assert_eq!(got, vec!["ADAUSDT".to_string(), "BTCUSDT".to_string(), "ETHUSDT".to_string()]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn list_symbols_empty_when_no_candles() {
    let path = make_temp_db_with_candles("empty", &[]);
    let got = list_symbols(&path).expect("list_symbols");
    assert!(got.is_empty(), "boş candles → boş liste; gelen: {:?}", got);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn list_symbols_orders_alphabetically() {
    // SQL ORDER BY symbol → deterministik sıra.
    let path = make_temp_db_with_candles("order", &["ZRXUSDT", "AAVEUSDT", "MKRUSDT"]);
    let got = list_symbols(&path).expect("list_symbols");
    assert_eq!(got, vec!["AAVEUSDT".to_string(), "MKRUSDT".to_string(), "ZRXUSDT".to_string()]);
    let _ = std::fs::remove_file(&path);
}
