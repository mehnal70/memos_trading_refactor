// download_twelvedata — dünya-piyasası günlük OHLC'yi Twelve Data'dan çekip DB'ye yazar.
//
// [[project_world_markets]] Faz A: Yahoo/Stooq bot-kapısına takılınca sağlam keyed kaynak.
// download_yahoo ile AYNI DB etiketleri (market=<asset_class>) → ölçüm (xs_momentum) kaynak-agnostik.
// Sembol DB'de ÇIPLAK saklanır (THYAO, EURUSD, XAUUSD, AAPL); TD'ye doğal biçim (EUR/USD, exchange=BIST).
//
// Kullanım:
//   TWELVEDATA_API_KEY=xxx cargo run --release --example download_twelvedata -- <asset_class> <interval> SYM1,... [outputsize]
// Örnek:
//   TWELVEDATA_API_KEY=xxx cargo run --release --example download_twelvedata -- bist 1d THYAO,GARAN,AKBNK 5000
//   TWELVEDATA_API_KEY=xxx cargo run --release --example download_twelvedata -- forex 1d EURUSD,GBPUSD,USDJPY 5000
//   TWELVEDATA_API_KEY=xxx cargo run --release --example download_twelvedata -- commodity 1d XAUUSD,XAGUSD 5000
//
// Env: TWELVEDATA_API_KEY (zorunlu), DB_PATH (default data/trader.db),
//      TD_SLEEP_MS (semboller arası gecikme, default 8000 → ücretsiz 8 istek/dk limiti).

use std::thread::sleep;
use std::time::Duration;

use memos_trading_core::robot::data_fetcher::twelvedata::TwelveDataFetcher;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let asset_class = args.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
    let interval = args.get(2).map(|s| s.as_str()).unwrap_or("1d").to_string();
    let symbols: Vec<String> = args.get(3)
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    let outputsize: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(5000);

    let valid = ["bist", "forex", "commodity", "usequity"];
    if !valid.contains(&asset_class.as_str()) || symbols.is_empty() {
        eprintln!("⚠️  Kullanım: download_twelvedata -- <bist|forex|commodity|usequity> <interval> SYM1,... [outputsize]");
        std::process::exit(2);
    }
    let api_key = match std::env::var("TWELVEDATA_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => { eprintln!("⚠️  TWELVEDATA_API_KEY gerekli (ücretsiz key: twelvedata.com)."); std::process::exit(2); }
    };
    // download_yahoo ile aynı izolasyon: her evren ayrı market etiketi (read_candles_market filtreler).
    let market = asset_class.clone();
    let exchange = asset_class.clone();
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let sleep_ms: u64 = std::env::var("TD_SLEEP_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(8000);

    let fetcher = TwelveDataFetcher::new(api_key);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let conn = memos_trading_core::persistence::open_db(&db_path).expect("DB açılamadı");

    println!("🌐 download_twelvedata · sınıf={asset_class} · interval={interval} · {} sembol · gecikme={sleep_ms}ms · db={db_path}",
        symbols.len());

    let (mut ok, mut total, mut failed) = (0usize, 0usize, 0usize);
    for (i, base) in symbols.iter().enumerate() {
        let (td_sym, ex) = TwelveDataFetcher::td_symbol(&asset_class, base);
        // Rate-limit (ücretsiz 8/dk): ilk hariç semboller arası bekle.
        if i > 0 { sleep(Duration::from_millis(sleep_ms)); }
        match rt.block_on(fetcher.fetch_daily(&td_sym, ex, base, &interval, outputsize)) {
            Ok(candles) if !candles.is_empty() => {
                let mut saved = 0usize;
                for c in &candles {
                    if memos_trading_core::persistence::writer::save_candle(&conn, &exchange, &market, c).is_ok() {
                        saved += 1;
                    }
                }
                let first = candles.first().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                let last = candles.last().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                println!("  ✅ {:10} ({}) {} mum ({} → {})", base, td_sym, saved, first, last);
                ok += 1;
                total += saved;
            }
            Ok(_) => { println!("  ⚠️ {:10} ({}) veri yok", base, td_sym); failed += 1; }
            Err(e) => { println!("  ✗ {:10} ({}) {}", base, td_sym, e); failed += 1; }
        }
    }
    println!("\n→ {} / {} sembol · {} başarısız · toplam {} mum yazıldı (market='{}').",
        ok, symbols.len(), failed, total, market);
    if failed > 0 {
        println!("  (rate-limit/sembol-yok olabilir → TD_SLEEP_MS artır ya da başarısızları tekrar koş; upsert.)");
    }
}
