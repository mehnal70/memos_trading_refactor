// db_coverage — DB'deki mum envanterini "skorlama-hazırlığı" gözüyle raporlar.
//
// Amaç: edge_scan / xs_momentum / param_optimize / sweep gibi skorlama araçları DB'de
// O AN ne kadar derinlik varsa onun üzerinde koşar (sağlık kapısı kirli seriyi ELER ama
// eksik geçmişi DOLDURMAZ → sessiz "yetersiz örneklem" riski). Bu rapor her
// (exchange, market, symbol, interval) serisi için satır/ilk-son bar/gap%/bayatlık çıkarır
// ki kapsama deliklerini SKORLAMADAN ÖNCE görelim ve hedefli download_candles atayalım.
//
// Tek-kaynak yeniden kullanım: open_db (persistence), DataNormalizer::parse_interval
// (interval-farkında bar adımı), now_epoch_millis (time). Ek tablo/IO yok — tek agregat sorgu.
//
// Kullanım:
//   cargo run --release --example db_coverage -- [market] [interval] [min_rows]
// Örnekler:
//   cargo run --release --example db_coverage                       # tüm market/interval
//   cargo run --release --example db_coverage -- futures            # yalnız futures
//   cargo run --release --example db_coverage -- futures 1d 300     # futures 1d, sağlık eşiği 300 bar
//
// Env: DB_PATH (default data/trader.db).

use memos_trading_core::core::time::now_epoch_millis;
use memos_trading_core::robot::data_pipeline::DataNormalizer;

struct SeriesCov {
    market: String,
    symbol: String,
    interval: String,
    rows: i64,
    first_ms: i64,
    last_ms: i64,
}

fn fmt_date(ms: i64) -> String {
    use chrono::{TimeZone, Utc};
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "?".into())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market_filter = args.get(1).map(|s| s.as_str()).filter(|s| !s.is_empty() && *s != "-");
    let interval_filter = args.get(2).map(|s| s.as_str()).filter(|s| !s.is_empty() && *s != "-");
    // Sağlık eşiği: "skorlamaya yeter mi" zemini (operatör ayarı, default 300 bar).
    let min_rows: i64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(300);

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let conn = memos_trading_core::persistence::open_db(&db_path).expect("DB açılamadı");

    // Tek agregat sorgu — TÜM aralık üzerinden (windowed değil) tam kapsama.
    let mut sql = String::from(
        "SELECT market, symbol, interval, COUNT(*) AS n, MIN(timestamp) AS lo, MAX(timestamp) AS hi \
         FROM candles",
    );
    let mut wheres: Vec<String> = Vec::new();
    if let Some(m) = market_filter { wheres.push(format!("market = '{}'", m.replace('\'', "''"))); }
    if let Some(i) = interval_filter { wheres.push(format!("interval = '{}'", i.replace('\'', "''"))); }
    if !wheres.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&wheres.join(" AND "));
    }
    sql.push_str(" GROUP BY market, symbol, interval ORDER BY market, interval, n DESC");

    let mut stmt = conn.prepare(&sql).expect("sorgu hazırlık");
    let rows: Vec<SeriesCov> = stmt
        .query_map([], |r| {
            Ok(SeriesCov {
                market: r.get(0)?,
                symbol: r.get(1)?,
                interval: r.get(2)?,
                rows: r.get::<_, i64>(3)?,
                first_ms: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                last_ms: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
            })
        })
        .expect("sorgu")
        .filter_map(|x| x.ok())
        .collect();

    if rows.is_empty() {
        println!("⚠️  Eşleşen seri yok (db={db_path}, market={:?}, interval={:?}).", market_filter, interval_filter);
        return;
    }

    let now_ms = now_epoch_millis() as i64;

    println!(
        "🗂️  DB KAPSAMA RAPORU · db={db_path} · {} seri · sağlık eşiği={} bar{}{}\n",
        rows.len(),
        min_rows,
        market_filter.map(|m| format!(" · market={m}")).unwrap_or_default(),
        interval_filter.map(|i| format!(" · interval={i}")).unwrap_or_default(),
    );
    println!(
        "{:<14} {:<8} {:>8} {:>10} {:>7} {:>7}  {:<12} {:<12}  durum",
        "SEMBOL", "TF", "bar", "beklenen", "gap%", "yaş(bar)", "ilk", "son",
    );
    println!("{}", "─".repeat(104));

    let mut n_ok = 0usize;
    let mut n_sparse = 0usize;
    let mut n_gappy = 0usize;
    let mut n_stale = 0usize;
    let mut total_rows = 0i64;
    let mut last_market = String::new();

    for s in &rows {
        if s.market != last_market {
            if !last_market.is_empty() { println!(); }
            println!("▼ market = {}", s.market);
            last_market = s.market.clone();
        }
        let iv_secs = DataNormalizer::parse_interval(&s.interval).max(1) as i64;
        let step_ms = iv_secs * 1000;
        let span = (s.last_ms - s.first_ms).max(0);
        let expected = span / step_ms + 1;
        let gap_pct = if expected > 0 {
            (1.0 - s.rows as f64 / expected as f64).clamp(0.0, 1.0) * 100.0
        } else { 0.0 };
        let age_bars = (now_ms - s.last_ms).max(0) / step_ms;

        // Verdikt: skorlamaya GİRER mi? (eşikler salt-gösterim, karar değil)
        let sparse = s.rows < min_rows;
        let gappy = gap_pct > 5.0;          // %5+ delik → şüpheli süreklilik
        let stale = age_bars > 3;           // son bar 3+ bar geride → bayat besleme

        let mut tags: Vec<&str> = Vec::new();
        if sparse { tags.push("SEYREK"); n_sparse += 1; }
        if gappy { tags.push("GAP"); n_gappy += 1; }
        if stale { tags.push("BAYAT"); n_stale += 1; }
        let verdict = if tags.is_empty() { n_ok += 1; "✓ hazır".to_string() } else { format!("⚠️ {}", tags.join("+")) };

        total_rows += s.rows;
        println!(
            "{:<14} {:<8} {:>8} {:>10} {:>6.1} {:>8}  {:<12} {:<12}  {}",
            s.symbol, s.interval, s.rows, expected, gap_pct, age_bars,
            fmt_date(s.first_ms), fmt_date(s.last_ms), verdict,
        );
    }

    println!("{}", "─".repeat(104));
    println!(
        "ÖZET · {} seri · {} bar · ✓hazır={} · ⚠️SEYREK={} · GAP={} · BAYAT={}",
        rows.len(), total_rows, n_ok, n_sparse, n_gappy, n_stale,
    );
    if n_sparse + n_gappy > 0 {
        println!(
            "→ Skorlama evreninde delik var. Hedefli backfill örneği:\n   \
             cargo run --release --example download_candles -- <market> <interval> SEMBOL1,SEMBOL2,... <yıl>",
        );
    } else {
        println!("→ Tüm seriler skorlamaya hazır görünüyor.");
    }
}
