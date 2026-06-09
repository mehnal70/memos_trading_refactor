// download_candles — verilen sembollerin derin geçmiş mumlarını Binance'ten çekip DB'ye yazar.
//
// Amaç: kesitsel sepeti GENİŞLETMEK için (XS momentum gross OOS edge GERÇEK ama net ince → daha çok
// majör = daha kalın long-short bacak = aynı sinyal daha az gürültüyle). Public klines (API key yok);
// fetch_history_market ile `years` yıl geriye pagine eder, save_candle ile kanonik şemaya upsert eder.
// Per-sembol hata izole (delisted/yanlış sembol atlanır, döngü sürer).
//
// Kullanım:
//   cargo run --release --example download_candles -- [market] [interval] SYM1,SYM2,... [years]
// Örnek:
//   cargo run --release --example download_candles -- futures 1d LTCUSDT,LINKUSDT,ETCUSDT 8
//
// Env: DB_PATH (default data/trader.db).

use memos_trading_core::robot::data_fetcher::binance::BinanceFetcher;

fn interval_secs(iv: &str) -> i64 {
    match iv {
        "1m" => 60, "5m" => 300, "15m" => 900, "30m" => 1800,
        "1h" => 3600, "4h" => 14400, "1d" => 86400, _ => 86400,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market = args.get(1).map(|s| s.as_str()).unwrap_or("futures").to_string();
    let interval = args.get(2).map(|s| s.as_str()).unwrap_or("1d").to_string();
    let symbols: Vec<String> = args.get(3)
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    let years: i64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(8);
    if symbols.is_empty() {
        eprintln!("⚠️  Sembol listesi ver: download_candles -- futures 1d LTCUSDT,LINKUSDT 8");
        std::process::exit(2);
    }
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    // GAP-FARKINDALIK: default açık → sembolde kayıt varsa son bardan İLERİ devam (zaten
    // indirilmiş geçmişi yeniden çekmez). FORCE_FULL=1 ile kapat (tüm `years` penceresini
    // baştan çek, ör. derin-geçmiş yeniden tohumlama). save_candle zaten upsert (kopya yok).
    let force_full = matches!(std::env::var("FORCE_FULL").ok().as_deref(),
        Some("1") | Some("true") | Some("on"));

    let iv_secs = interval_secs(&interval);
    let step_ms = iv_secs * 1000;
    let now_ms = memos_trading_core::core::time::now_epoch_millis() as i64;
    let start_ms = now_ms - years * 365 * 86400 * 1000;
    // 1d × 8 yıl ≈ 2920 bar → 3 sayfa (≤1000/istek); tavanı bolca ver (kısa-TF de güvenli).
    let max_requests = ((years * 365 * 86400 / iv_secs) / 1000 + 2).max(3) as usize;

    let fetcher = BinanceFetcher::new();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    println!("🌐 download_candles · market={market} · interval={interval} · {} sembol · ~{years} yıl · db={db_path}{}",
        symbols.len(), if force_full { " · FORCE_FULL" } else { " · gap-farkında" });
    let conn = memos_trading_core::persistence::open_db(&db_path).expect("DB açılamadı");
    let mut ok = 0usize;
    let mut skipped = 0usize;
    let mut total_rows = 0usize;

    for sym in &symbols {
        // Gap-farkında başlangıç: kayıt varsa son bardan bir interval ÖTESİ (üst-üste binme yok);
        // yoksa veya FORCE_FULL → tam pencere başı. Zaten güncelse (eff_start ≥ now) → atla.
        let last_ms = if force_full { None }
            else { memos_trading_core::persistence::reader::last_candle_ts(&db_path, sym, &interval, &market) };
        let eff_start = match last_ms {
            Some(l) => l + step_ms,
            None => start_ms,
        };
        if eff_start >= now_ms {
            println!("  ⏭  {:12} güncel (son bar < 1 interval önce) → atlandı", sym);
            skipped += 1;
            continue;
        }
        let res = rt.block_on(fetcher.fetch_history_market(sym, &interval, &market, eff_start, iv_secs, max_requests));
        match res {
            Ok(candles) if !candles.is_empty() => {
                let mut saved = 0usize;
                for c in &candles {
                    if memos_trading_core::persistence::writer::save_candle(&conn, "binance", &market, c).is_ok() {
                        saved += 1;
                    }
                }
                let first = candles.first().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                let last = candles.last().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                let mode = if last_ms.is_some() { "↑artımlı" } else { "🌱tohum" };
                println!("  ✅ {:12} {} bar ({} → {}) [{}]", sym, saved, first, last, mode);
                ok += 1;
                total_rows += saved;
            }
            Ok(_) => println!("  ⚠️ {:12} veri yok (borsa bu aralığı tutmuyor)", sym),
            Err(e) => println!("  ✗ {:12} {}", sym, e),
        }
    }
    println!("\n→ {} / {} sembol indirildi · {} güncel-atlandı · toplam {} bar yazıldı.",
        ok, symbols.len(), skipped, total_rows);
}
