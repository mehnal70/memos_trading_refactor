// download_isyatirim — BIST günlük OHLC'yi İş Yatırım'dan (ücretsiz, TR-yerli) çekip DB'ye yazar.
//
// [[project_world_markets]] Faz A: TD-ücretsiz BIST'i kilitliyor → İş Yatırım HisseTekno.
// download_twelvedata/download_yahoo ile AYNI DB etiketi (market='bist') → ölçüm kaynak-agnostik;
// THYAO gibi TD'den gelmiş semboller upsert edilir (çarpışma yok). Sembol ÇIPLAK saklanır.
//
// Kullanım:
//   cargo run --release --example download_isyatirim -- THYAO,GARAN,AKBNK,... [years]
// Örnek:
//   cargo run --release --example download_isyatirim -- THYAO,GARAN,AKBNK,ISCTR,EREGL 10
//
// Env: DB_PATH (default data/trader.db).
// NOT: HisseTekno endpoint TR residential IP + Referer/X-Requested-With ister (datacenter/yurt-dışı
// IP → 401). Bu yüzden kullanıcı kendi makinesinde koşar. years yıl-yıl pagine edilir.

use std::thread::sleep;
use std::time::Duration;

use memos_trading_core::robot::data_fetcher::isyatirim::IsYatirimFetcher;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let symbols: Vec<String> = args.get(1)
        .map(|s| s.split(',').map(|x| x.trim().to_uppercase()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    let years: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    if symbols.is_empty() {
        eprintln!("⚠️  Kullanım: download_isyatirim -- THYAO,GARAN,AKBNK,... [years]");
        std::process::exit(2);
    }
    // download_twelvedata/yahoo ile aynı izolasyon: market='bist'.
    let (exchange, market) = ("bist", "bist");
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());

    let fetcher = IsYatirimFetcher::new();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let conn = memos_trading_core::persistence::open_db(&db_path).expect("DB açılamadı");

    println!("🇹🇷 download_isyatirim · {} sembol · ~{years} yıl · market='bist' · db={db_path}", symbols.len());

    let (mut ok, mut total, mut failed) = (0usize, 0usize, 0usize);
    for (i, sym) in symbols.iter().enumerate() {
        if i > 0 { sleep(Duration::from_millis(500)); }
        match rt.block_on(fetcher.fetch_daily(sym, sym, years)) {
            Ok(candles) if !candles.is_empty() => {
                let mut saved = 0usize;
                for c in &candles {
                    if memos_trading_core::persistence::writer::save_candle(&conn, exchange, market, c).is_ok() {
                        saved += 1;
                    }
                }
                let first = candles.first().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                let last = candles.last().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                println!("  ✅ {:10} {} mum ({} → {})", sym, saved, first, last);
                ok += 1;
                total += saved;
            }
            Ok(_) => { println!("  ⚠️ {:10} veri yok", sym); failed += 1; }
            Err(e) => { println!("  ✗ {:10} {}", sym, e); failed += 1; }
        }
    }
    println!("\n→ {} / {} sembol · {} başarısız · toplam {} mum yazıldı (market='bist').",
        ok, symbols.len(), failed, total);
    if failed > 0 {
        println!("  (401 → TR residential IP gerekir; sembol-yok → ticker'ı kontrol et. save_candle upsert.)");
    }
}
