// persistence::reader::list_symbols + list_symbols_for_market testleri.
//
// Önceki bug: paper_trading_results tablosundan okuyordu → işlem yapılmadıkça
// boş → screener havuzu hep boş. Şimdi candles tablosundan okuyor (indirilmiş
// ham mum havuzu). Bu test temp DB ile davranışı sabitler.
//
// Market segmentasyonu: list_symbols_for_market candles şemasındaki
// (exchange, market, interval) kolonlarına göre filtre uygular →
// crypto vs BIST karışıklığı önlenir.

use rusqlite::Connection;

use memos_trading_core::persistence::reader::{list_symbols, list_symbols_for_market};

struct Row<'a> {
    exchange: &'a str,
    market:   &'a str,
    symbol:   &'a str,
    interval: &'a str,
}

fn make_temp_db_with_rows(tag: &str, rows: &[Row<'_>]) -> String {
    let path = format!("/tmp/memos_list_symbols_{}_{}.db", std::process::id(), tag);
    let _ = std::fs::remove_file(&path);
    let conn = Connection::open(&path).expect("temp DB aç");
    conn.execute_batch(
        "CREATE TABLE candles (\
            exchange TEXT NOT NULL, market TEXT NOT NULL, \
            symbol TEXT NOT NULL, interval TEXT NOT NULL, \
            timestamp INTEGER NOT NULL, \
            open REAL, high REAL, low REAL, close REAL, volume REAL \
        );",
    )
    .expect("candles tablosu");
    for r in rows {
        conn.execute(
            "INSERT INTO candles (exchange, market, symbol, interval, timestamp, open, high, low, close, volume) \
             VALUES (?1, ?2, ?3, ?4, 1700000000000, 1.0, 1.0, 1.0, 1.0, 1.0)",
            [r.exchange, r.market, r.symbol, r.interval],
        )
        .expect("candle insert");
    }
    path
}

fn make_temp_db_with_candles(tag: &str, symbols: &[&str]) -> String {
    let rows: Vec<Row> = symbols.iter().map(|s| Row {
        exchange: "binance", market: "futures", symbol: s, interval: "1m",
    }).collect();
    make_temp_db_with_rows(tag, &rows)
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

#[test]
fn segmented_market_filter_excludes_other_markets() {
    // futures + BIST karışık DB → market="futures" yalnız crypto döner.
    let path = make_temp_db_with_rows("seg_market", &[
        Row { exchange: "binance", market: "futures", symbol: "BTCUSDT", interval: "1m" },
        Row { exchange: "binance", market: "futures", symbol: "ETHUSDT", interval: "1m" },
        Row { exchange: "bist",    market: "bist",    symbol: "AKBNK",   interval: "1m" },
        Row { exchange: "bist",    market: "bist",    symbol: "AGHOL",   interval: "1m" },
    ]);
    let got = list_symbols_for_market(&path, Some("futures"), None).expect("filtered");
    assert_eq!(got, vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn segmented_interval_filter_excludes_other_intervals() {
    // Aynı sembol farklı interval'lerde → interval="1m" yalnız 1m döner.
    let path = make_temp_db_with_rows("seg_interval", &[
        Row { exchange: "binance", market: "futures", symbol: "BTCUSDT", interval: "1m" },
        Row { exchange: "binance", market: "futures", symbol: "BTCUSDT", interval: "1h" },
        Row { exchange: "binance", market: "futures", symbol: "ETHUSDT", interval: "1h" },
    ]);
    let got = list_symbols_for_market(&path, None, Some("1m")).expect("filtered");
    assert_eq!(got, vec!["BTCUSDT".to_string()]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn segmented_market_and_interval_combined() {
    // market=futures AND interval=1m → her iki filtreyi karşılayan tek sembol.
    let path = make_temp_db_with_rows("seg_combo", &[
        Row { exchange: "binance", market: "futures", symbol: "BTCUSDT", interval: "1m" },
        Row { exchange: "binance", market: "futures", symbol: "BTCUSDT", interval: "1h" },
        Row { exchange: "binance", market: "spot",    symbol: "ETHUSDT", interval: "1m" },
        Row { exchange: "bist",    market: "bist",    symbol: "AKBNK",   interval: "1m" },
    ]);
    let got = list_symbols_for_market(&path, Some("futures"), Some("1m")).expect("filtered");
    assert_eq!(got, vec!["BTCUSDT".to_string()]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn segmented_no_filters_equals_list_symbols() {
    // None/None → list_symbols ile aynı havuz.
    let path = make_temp_db_with_rows("seg_nofilter", &[
        Row { exchange: "binance", market: "futures", symbol: "BTCUSDT", interval: "1m" },
        Row { exchange: "bist",    market: "bist",    symbol: "AKBNK",   interval: "1m" },
    ]);
    let plain = list_symbols(&path).expect("plain");
    let none  = list_symbols_for_market(&path, None, None).expect("segmented");
    assert_eq!(plain, none);
    let _ = std::fs::remove_file(&path);
}
