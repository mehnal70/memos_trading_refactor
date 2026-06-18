// download_yahoo — dünya-piyasası günlük OHLC'yi Yahoo Finance'ten çekip DB'ye yazar.
//
// [[project_world_markets]] Faz A: BIST/forex/emtia/ABD edge-ölçümü için yakıt. YahooFetcher
// (tek Yahoo-parse yolu) ile çeker, save_candle ile kanonik şemaya upsert eder. Her evren AYRI
// market etiketiyle saklanır (read_candles_market izole eder; kripto ile çarpışmaz):
//   exchange=market=<asset_class> (bist|forex|commodity|usequity).
// Sembol DB'de ÇIPLAK saklanır (THYAO, EURUSD, GC, AAPL); Yahoo'ya eki (.IS/=X/=F) eklenir.
//
// Kullanım:
//   cargo run --release --example download_yahoo -- <asset_class> <interval> SYM1,SYM2,... [range]
// Örnek:
//   cargo run --release --example download_yahoo -- bist 1d THYAO,GARAN,AKBNK,EREGL 5y
//   cargo run --release --example download_yahoo -- forex 1d EURUSD,GBPUSD,USDJPY 5y
//   cargo run --release --example download_yahoo -- commodity 1d GC,SI,CL 5y
//
// Env: DB_PATH (default data/trader.db). range Yahoo token: 1mo/6mo/1y/2y/5y/10y/max (default 5y).
// NOT: Yahoo datacenter-IP'yi 429 ile throttle eder → semboller arası kısa gecikme; gerekirse tekrar koş.

use std::thread::sleep;
use std::time::Duration;

use memos_trading_core::robot::data_fetcher::yahoo::YahooFetcher;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let asset_class = args.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
    let interval = args.get(2).map(|s| s.as_str()).unwrap_or("1d").to_string();
    let symbols: Vec<String> = args.get(3)
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    let range = args.get(4).map(|s| s.as_str()).unwrap_or("5y").to_string();

    let valid = ["bist", "forex", "commodity", "usequity"];
    if !valid.contains(&asset_class.as_str()) || symbols.is_empty() {
        eprintln!("⚠️  Kullanım: download_yahoo -- <bist|forex|commodity|usequity> <interval> SYM1,SYM2,... [range]");
        eprintln!("    Örn: download_yahoo -- bist 1d THYAO,GARAN,AKBNK 5y");
        std::process::exit(2);
    }
    // Her evren ayrı market etiketiyle izole (read_candles_market filtreler).
    let market = asset_class.clone();
    let exchange = asset_class.clone();
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());

    let fetcher = YahooFetcher::new();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let conn = memos_trading_core::persistence::open_db(&db_path).expect("DB açılamadı");

    println!("🌐 download_yahoo · sınıf={asset_class} · interval={interval} · range={range} · {} sembol · db={db_path}",
        symbols.len());

    let (mut ok, mut total, mut failed) = (0usize, 0usize, 0usize);
    for (i, base) in symbols.iter().enumerate() {
        let ticker = YahooFetcher::yahoo_ticker(&asset_class, base);
        // Sembol başına kısa nezaket gecikmesi (Yahoo 429 azaltma).
        if i > 0 { sleep(Duration::from_millis(400)); }
        match rt.block_on(fetcher.fetch_daily(&ticker, base, &interval, &range)) {
            Ok(candles) if !candles.is_empty() => {
                let mut saved = 0usize;
                for c in &candles {
                    if memos_trading_core::persistence::writer::save_candle(&conn, &exchange, &market, c).is_ok() {
                        saved += 1;
                    }
                }
                let first = candles.first().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                let last = candles.last().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                println!("  ✅ {:10} ({}) {} mum ({} → {})", base, ticker, saved, first, last);
                ok += 1;
                total += saved;
            }
            Ok(_) => { println!("  ⚠️ {:10} ({}) veri yok", base, ticker); failed += 1; }
            Err(e) => { println!("  ✗ {:10} ({}) {}", base, ticker, e); failed += 1; }
        }
    }
    println!("\n→ {} / {} sembol · {} başarısız · toplam {} mum yazıldı (market='{}').",
        ok, symbols.len(), failed, total, market);
    if failed > 0 {
        println!("  (429/ağ hatası geçici olabilir → başarısızları tekrar koş; save_candle upsert, kopya yok.)");
    }
}
