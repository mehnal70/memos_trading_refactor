// download_mt5 — MetaTrader 5 köprüsünden OHLC çekip kanonik candles şemasına yazar.
//
// [[project_venue_multimarket]] MT5 venue Faz 1 (veri). Mt5Venue::fetch_candles yerel MT5 EA
// köprüsüne gider (Rust = tokio TCP server, EA = native client; bkz. scripts/mt5/README.md),
// save_candle ile DB'ye upsert eder. Edge-ölçümü yakıtı (forex/emtia).
//
// İZOLASYON: MT5 verisi KENDİ namespace'inde saklanır — exchange="mt5", market=<market_tag>.
// read_candles_market YALNIZ market ile filtreler; "spot" Binance kriptoyla, "forex" Yahoo
// ile çarpışırdı → MT5'e ayrı etiket (varsayılan "mt5") ver. Sembol DB'de ÇIPLAK (EURUSD).
//
// ÖN KOŞUL: MT5 terminali açık + MemosBridge.mq5 EA bir grafiğe ekli + köprüye bağlı olmalı.
// EA bağlı değilse her sembol accept zaman aşımına (≈15s) uğrar ve açık hata basar (sahte yok).
//
// Kullanım:
//   cargo run --release --example download_mt5 -- <market_tag> <interval> SYM1,SYM2,... [count]
// Örnek:
//   cargo run --release --example download_mt5 -- mt5 1h EURUSD,GBPUSD,XAUUSD 2000
//   cargo run --release --example download_mt5 -- mt5 1d EURUSD,USDJPY 1000
//
// Env: DB_PATH (default data/trader.db), MT5_BRIDGE_ADDR (default 127.0.0.1:9001).

use std::sync::Arc;

use memos_trading_core::core::types::Market;
use memos_trading_core::robot::venue::adapter::MarketData;
use memos_trading_core::robot::venue::mt5::{Mt5Bridge, Mt5Venue};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market_tag = args.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
    let interval = args.get(2).map(|s| s.as_str()).unwrap_or("1h").to_string();
    let symbols: Vec<String> = args
        .get(3)
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    let count: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1000);

    if market_tag.is_empty() || symbols.is_empty() {
        eprintln!("⚠️  Kullanım: download_mt5 -- <market_tag> <interval> SYM1,SYM2,... [count]");
        eprintln!("    Örn: download_mt5 -- mt5 1h EURUSD,GBPUSD,XAUUSD 2000");
        eprintln!("    NOT: MT5 verisi exchange='mt5', market=<market_tag> ile izole saklanır");
        eprintln!("         (kripto 'spot'/Yahoo 'forex' ile karışmasın → ayrı etiket ver).");
        std::process::exit(2);
    }

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let bridge = Arc::new(Mt5Bridge::with_defaults(std::env::var("MT5_BRIDGE_ADDR").ok()));
    // Venue market'i yalnız kimlik/routing içindir; veri çekimi EA'da sembol+TF ile yapılır.
    let venue = Mt5Venue::new(Market::Spot, bridge);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let conn = memos_trading_core::persistence::open_db(&db_path).expect("DB açılamadı");

    println!(
        "🌉 download_mt5 · market='{market_tag}' · interval={interval} · count={count} · {} sembol · db={db_path}",
        symbols.len()
    );

    let (mut ok, mut total, mut failed) = (0usize, 0usize, 0usize);
    for base in &symbols {
        match rt.block_on(venue.fetch_candles(base, &interval, count)) {
            Ok(candles) if !candles.is_empty() => {
                let mut saved = 0usize;
                for c in &candles {
                    if memos_trading_core::persistence::writer::save_candle(&conn, "mt5", &market_tag, c).is_ok() {
                        saved += 1;
                    }
                }
                let first = candles.first().map(|c| c.timestamp.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                let last = candles.last().map(|c| c.timestamp.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                println!("  ✅ {:10} {} mum ({} → {})", base, saved, first, last);
                ok += 1;
                total += saved;
            }
            Ok(_) => {
                println!("  ⚠️ {:10} veri yok", base);
                failed += 1;
            }
            Err(e) => {
                println!("  ✗ {:10} {}", base, e);
                failed += 1;
            }
        }
    }
    println!(
        "\n→ {} / {} sembol · {} başarısız · toplam {} mum yazıldı (exchange='mt5', market='{}').",
        ok, symbols.len(), failed, total, market_tag
    );
    if failed > 0 {
        println!("  (Hepsi başarısızsa: MT5 terminali açık + MemosBridge EA bağlı mı? MT5_BRIDGE_ADDR doğru mu?)");
    }
}
