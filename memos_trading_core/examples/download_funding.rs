// download_funding — verilen futures sembollerinin funding-rate geçmişini bir borsadan çekip DB'ye yazar.
//
// Amaç: funding-carry ekseni (fiyat-DIŞI taşıma getirisi) için yakıt; ayrıca cross-exchange funding
// spread'i (xfunding aracı) için 2. borsa (Bybit) funding'i. Funding 8 saatte bir (~3/gün).
// fetch_funding_history ile `years` yıl geriye pagine eder, save_funding ile kanonik şemaya upsert eder.
// Gap-farkında: kayıt varsa son funding_time'dan İLERİ devam (FORCE_FULL=1 ile tüm pencere yeniden).
// Per-sembol hata izole (delisted/yanlış sembol atlanır, döngü sürer). Borsa-FARKINDA gap/yazım:
// her borsanın funding'i (exchange,market,symbol,funding_time) ile ayrı saklanır.
//
// Kullanım:
//   cargo run --release --example download_funding -- [market] SYM1,SYM2,... [years]
// Örnek:
//   cargo run --release --example download_funding -- futures BTCUSDT,ETHUSDT,SOLUSDT 8
//   EXCHANGE=bybit cargo run --release --example download_funding -- futures BTCUSDT,ETHUSDT 2
//
// Env: DB_PATH (default data/trader.db), FORCE_FULL=1 (gap-farkındalığı kapat → tüm `years` baştan),
//      EXCHANGE=binance|bybit (default binance; cross-exchange spread için bybit).

use memos_trading_core::core::types::Market;
use memos_trading_core::robot::data_fetcher::binance::BinanceFetcher;
use memos_trading_core::robot::venue::bybit::BybitVenue;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market = args.get(1).map(|s| s.as_str()).unwrap_or("futures").to_string();
    let symbols: Vec<String> = args.get(2)
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    let years: i64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    if symbols.is_empty() {
        eprintln!("⚠️  Sembol listesi ver: download_funding -- futures BTCUSDT,ETHUSDT 8");
        std::process::exit(2);
    }
    if !market.eq_ignore_ascii_case("futures") {
        eprintln!("⚠️  Funding yalnız futures'ta var (market={market}).");
        std::process::exit(2);
    }
    let exchange = std::env::var("EXCHANGE").unwrap_or_else(|_| "binance".into()).to_lowercase();
    if exchange != "binance" && exchange != "bybit" {
        eprintln!("⚠️  EXCHANGE binance|bybit olmalı (verilen: {exchange}).");
        std::process::exit(2);
    }
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let force_full = matches!(std::env::var("FORCE_FULL").ok().as_deref(),
        Some("1") | Some("true") | Some("on"));

    let now_ms = memos_trading_core::core::time::now_epoch_millis() as i64;
    let start_ms = now_ms - years * 365 * 86_400 * 1000;
    // Funding ~3/gün → years×365×3 kayıt. Sayfa-boyutu BORSA-farkında: Binance ≤1000/istek,
    // Bybit ≤200/istek → istek tavanını sayfa boyutuna göre ver (yoksa Bybit erken kesilir).
    let per_page: i64 = if exchange == "bybit" { 200 } else { 1000 };
    let max_requests = ((years * 365 * 3) / per_page + 2).max(3) as usize;

    let fetcher = BinanceFetcher::new();
    let bybit = BybitVenue::new(Market::Futures);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let conn = memos_trading_core::persistence::open_db(&db_path).expect("DB açılamadı");

    println!("💰 download_funding · borsa={exchange} · market={market} · {} sembol · ~{years} yıl · db={db_path}{}",
        symbols.len(), if force_full { " · FORCE_FULL" } else { " · gap-farkında" });

    let (mut ok, mut skipped, mut total) = (0usize, 0usize, 0usize);
    for sym in &symbols {
        // Gap-farkında başlangıç (BORSA-farkında): kayıt varsa son funding_time+1ms; yoksa/FORCE_FULL → pencere başı.
        let last_ms = if force_full { None }
            else { memos_trading_core::persistence::reader::last_funding_ts_exchange(&db_path, &exchange, sym, &market) };
        let eff_start = match last_ms { Some(l) => l + 1, None => start_ms };
        if eff_start >= now_ms {
            println!("  ⏭  {:12} güncel → atlandı", sym);
            skipped += 1;
            continue;
        }
        // Borsa-doğru fetch → her ikisi de Result<Vec<(t,rate)>, String>'e normalleşir.
        let fetched: Result<Vec<(i64, f64)>, String> = match exchange.as_str() {
            "bybit" => rt.block_on(bybit.fetch_funding_history(sym, eff_start, max_requests))
                .map_err(|e| e.to_string()),
            _ => rt.block_on(fetcher.fetch_funding_history(sym, &market, eff_start, max_requests)),
        };
        match fetched {
            Ok(points) if !points.is_empty() => {
                let mut saved = 0usize;
                for (t, r) in &points {
                    if memos_trading_core::persistence::writer::save_funding(&conn, &exchange, &market, sym, *t, *r).is_ok() {
                        saved += 1;
                    }
                }
                use chrono::{TimeZone, Utc};
                let fmt = |ms: i64| Utc.timestamp_millis_opt(ms).single()
                    .map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
                let first = points.first().map(|(t, _)| fmt(*t)).unwrap_or_default();
                let last = points.last().map(|(t, _)| fmt(*t)).unwrap_or_default();
                let mode = if last_ms.is_some() { "↑artımlı" } else { "🌱tohum" };
                println!("  ✅ {:12} {} kayıt ({} → {}) [{}]", sym, saved, first, last, mode);
                ok += 1;
                total += saved;
            }
            Ok(_) => println!("  ⚠️ {:12} veri yok (borsa bu aralığı tutmuyor)", sym),
            // Artımlı fetch boş → zaten güncel (son funding'den beri yeni ödeme yok), hata değil.
            Err(e) if last_ms.is_some() && e.contains("aralıkta veri yok") => {
                println!("  ⏭  {:12} güncel (yeni funding yok)", sym);
                skipped += 1;
            }
            Err(e) => println!("  ✗ {:12} {}", sym, e),
        }
    }
    println!("\n→ {} / {} sembol · {} güncel-atlandı · toplam {} funding kaydı yazıldı.",
        ok, symbols.len(), skipped, total);
}
