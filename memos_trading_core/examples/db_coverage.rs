// db_coverage — DB'deki mum envanterini "skorlama-hazırlığı" gözüyle raporlar
//                ve sağlıksız serileri kapatacak deterministik backfill PLANI üretir.
//
// Amaç: edge_scan / xs_momentum / param_optimize / sweep gibi skorlama araçları DB'de
// O AN ne kadar derinlik varsa onun üzerinde koşar (sağlık kapısı kirli seriyi ELER ama
// eksik geçmişi DOLDURMAZ → sessiz "yetersiz örneklem" riski). Bu araç her
// (market, symbol, interval) serisi için satır/ilk-son bar/gap%/bayatlık çıkarır ki
// kapsama deliklerini SKORLAMADAN ÖNCE görelim; --plan ile bu deliklerden çalıştırılabilir
// download_candles komutları üretir.
//
// Tek-kaynak: verdikt (analyze) tek fonksiyon; tablo ve plan onu paylaşır. open_db
// (persistence), DataNormalizer::parse_interval (interval-farkında bar adımı), now_epoch_millis.
//
// Kullanım:
//   cargo run --release --example db_coverage -- [market] [interval] [min_rows] [--plan]
//   • interval: tek ('4h'), virgüllü liste ('1d,4h') veya boş/'-' (tümü) — parametrik.
// Örnekler:
//   cargo run --release --example db_coverage                          # tablo: tüm market/interval
//   cargo run --release --example db_coverage -- futures 4h            # tablo: futures 4h
//   cargo run --release --example db_coverage -- futures 1d,4h --plan  # plan: yalnız 1d ve 4h
//   cargo run --release --example db_coverage -- futures '' 300 --plan # plan: tüm interval, eşik 300
//   PLAN_YEARS=8 ... db_coverage -- futures 1d --plan > backfill.sh    # plan: yıl penceresini 8'e ez
//
// Plan modu: sağlıksız serileri (market, interval, mod) bazında gruplar; her grup için
// TEK download_candles satırı (sembol listesi alfabetik, deterministik). Mod seçimi:
//   • İÇ-GAP veya SEYREK → FORCE_FULL (tüm pencere yeniden çekilir, iç delik dolar)
//   • yalnız BAYAT-kuyruk → artımlı (gap-farkında, son bardan ileri)
// Satırlar interval'e göre seri sıralı; script baştan sona sıralı (serial) koşar.
//
// Env: DB_PATH (default data/trader.db).

use memos_trading_core::core::time::now_epoch_millis;
use memos_trading_core::robot::data_pipeline::DataNormalizer;

/// Eşikler — verdikt zemini (operatör ayarı). Tablo ve plan ortak kullanır.
struct Thresholds {
    min_rows: i64,      // altı → SEYREK
    max_gap_pct: f64,   // üstü → GAP (iç süreksizlik)
    max_age_bars: i64,  // üstü → BAYAT (kuyruk besleme durmuş)
}

/// Bir serinin ham envanteri (tek agregat sorgudan).
struct SeriesCov {
    market: String,
    symbol: String,
    interval: String,
    rows: i64,
    first_ms: i64,
    last_ms: i64,
}

/// Verdikt verilmiş seri — tablo ve plan bunu tüketir (TEK kaynak).
struct Verdict {
    market: String,
    symbol: String,
    interval: String,
    iv_secs: i64,
    rows: i64,
    first_ms: i64,
    last_ms: i64,
    expected: i64,
    gap_pct: f64,
    age_bars: i64,
    sparse: bool,
    gappy: bool,
    stale: bool,
}

impl Verdict {
    fn healthy(&self) -> bool { !self.sparse && !self.gappy && !self.stale }
    /// Plan modu: FORCE_FULL gerekir mi? İç-gap veya seyreklik tüm-pencere yeniden çekim ister;
    /// yalnız bayat-kuyruk artımlı (gap-farkında) yeterli.
    fn needs_force_full(&self) -> bool { self.gappy || self.sparse }
    fn tags(&self) -> String {
        let mut t: Vec<&str> = Vec::new();
        if self.sparse { t.push("SEYREK"); }
        if self.gappy { t.push("GAP"); }
        if self.stale { t.push("BAYAT"); }
        t.join("+")
    }
}

/// Verdikt hesabı — TEK kaynak (tablo + plan ortak). Interval-farkında bar adımı.
fn analyze(s: &SeriesCov, now_ms: i64, th: &Thresholds) -> Verdict {
    let iv_secs = DataNormalizer::parse_interval(&s.interval).max(1) as i64;
    let step_ms = iv_secs * 1000;
    let span = (s.last_ms - s.first_ms).max(0);
    let expected = span / step_ms + 1;
    let gap_pct = if expected > 0 {
        (1.0 - s.rows as f64 / expected as f64).clamp(0.0, 1.0) * 100.0
    } else { 0.0 };
    let age_bars = (now_ms - s.last_ms).max(0) / step_ms;
    Verdict {
        market: s.market.clone(),
        symbol: s.symbol.clone(),
        interval: s.interval.clone(),
        iv_secs,
        rows: s.rows,
        first_ms: s.first_ms,
        last_ms: s.last_ms,
        expected,
        gap_pct,
        age_bars,
        sparse: s.rows < th.min_rows,
        gappy: gap_pct > th.max_gap_pct,
        stale: age_bars > th.max_age_bars,
    }
}

fn fmt_date(ms: i64) -> String {
    use chrono::{TimeZone, Utc};
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "?".into())
}

/// FORCE_FULL grubu için yıl penceresi: grubun EN ESKİ ilk-barından bugüne (tam geçmişi
/// kapsasın → iç delik dolar), [1, 10] clamp. Artımlı grupta yıl önemsiz (son bar var,
/// gap-farkında oradan devam eder) ama download_candles tohum-tabanı için yine de geçer.
fn years_for_window(min_first_ms: i64, now_ms: i64) -> i64 {
    let yr_ms = 365 * 86_400 * 1000i64;
    (((now_ms - min_first_ms).max(0) + yr_ms - 1) / yr_ms).clamp(1, 10)
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let plan_mode = raw.iter().any(|a| a == "--plan" || a == "plan");
    // Pozisyonel argümanlar (flag'leri ve boş/ayraç sembollerini ele) — sıra: market, interval, min_rows.
    let pos: Vec<&str> = raw.iter()
        .map(|s| s.as_str())
        .filter(|a| !a.starts_with("--") && *a != "plan")
        .collect();
    let market_filter = pos.first().copied().filter(|s| !s.is_empty() && *s != "-");
    // Interval parametrik: tek ('4h'), liste ('1d,4h') veya boş/'-' (tümü). Virgülle ayır.
    let intervals: Vec<String> = pos.get(1).copied()
        .filter(|s| !s.is_empty() && *s != "-")
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    let interval_label = if intervals.is_empty() { "tümü".to_string() } else { intervals.join(",") };
    let th = Thresholds {
        min_rows: pos.get(2).and_then(|s| s.parse().ok()).unwrap_or(300),
        max_gap_pct: 5.0, // %5+ iç delik → şüpheli süreklilik
        max_age_bars: 3,  // son bar 3+ bar geride → bayat besleme
    };

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let conn = memos_trading_core::persistence::open_db(&db_path).expect("DB açılamadı");

    // Tek agregat sorgu — TÜM aralık üzerinden (windowed değil) tam kapsama.
    let mut sql = String::from(
        "SELECT market, symbol, interval, COUNT(*) AS n, MIN(timestamp) AS lo, MAX(timestamp) AS hi \
         FROM candles",
    );
    let mut wheres: Vec<String> = Vec::new();
    if let Some(m) = market_filter { wheres.push(format!("market = '{}'", m.replace('\'', "''"))); }
    if !intervals.is_empty() {
        let list = intervals.iter()
            .map(|i| format!("'{}'", i.replace('\'', "''")))
            .collect::<Vec<_>>().join(",");
        wheres.push(format!("interval IN ({})", list));
    }
    if !wheres.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&wheres.join(" AND "));
    }
    sql.push_str(" GROUP BY market, symbol, interval ORDER BY market, interval, n DESC");

    let mut stmt = conn.prepare(&sql).expect("sorgu hazırlık");
    let series: Vec<SeriesCov> = stmt
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

    if series.is_empty() {
        eprintln!("⚠️  Eşleşen seri yok (db={db_path}, market={:?}, interval={}).", market_filter, interval_label);
        return;
    }

    let now_ms = now_epoch_millis() as i64;
    let verdicts: Vec<Verdict> = series.iter().map(|s| analyze(s, now_ms, &th)).collect();

    if plan_mode {
        emit_plan(&verdicts, &db_path, now_ms, &th);
    } else {
        print_table(&verdicts, &db_path, &th, market_filter, &interval_label);
    }
}

/// Tablo modu — insan-okur kapsama dökümü.
fn print_table(
    verdicts: &[Verdict],
    db_path: &str,
    th: &Thresholds,
    market_filter: Option<&str>,
    interval_label: &str,
) {
    println!(
        "🗂️  DB KAPSAMA RAPORU · db={db_path} · {} seri · sağlık eşiği={} bar{} · interval={}\n",
        verdicts.len(),
        th.min_rows,
        market_filter.map(|m| format!(" · market={m}")).unwrap_or_default(),
        interval_label,
    );
    println!(
        "{:<14} {:<8} {:>8} {:>10} {:>7} {:>7}  {:<12} {:<12}  durum",
        "SEMBOL", "TF", "bar", "beklenen", "gap%", "yaş(bar)", "ilk", "son",
    );
    println!("{}", "─".repeat(104));

    let (mut n_ok, mut n_sparse, mut n_gappy, mut n_stale, mut total_rows) = (0usize, 0usize, 0usize, 0usize, 0i64);
    let mut last_market = String::new();
    for v in verdicts {
        if v.market != last_market {
            if !last_market.is_empty() { println!(); }
            println!("▼ market = {}", v.market);
            last_market = v.market.clone();
        }
        if v.sparse { n_sparse += 1; }
        if v.gappy { n_gappy += 1; }
        if v.stale { n_stale += 1; }
        let verdict = if v.healthy() { n_ok += 1; "✓ hazır".to_string() } else { format!("⚠️ {}", v.tags()) };
        total_rows += v.rows;
        println!(
            "{:<14} {:<8} {:>8} {:>10} {:>6.1} {:>8}  {:<12} {:<12}  {}",
            v.symbol, v.interval, v.rows, v.expected, v.gap_pct, v.age_bars,
            fmt_date(v.first_ms), fmt_date(v.last_ms), verdict,
        );
    }
    println!("{}", "─".repeat(104));
    println!(
        "ÖZET · {} seri · {} bar · ✓hazır={} · ⚠️SEYREK={} · GAP={} · BAYAT={}",
        verdicts.len(), total_rows, n_ok, n_sparse, n_gappy, n_stale,
    );
    if n_sparse + n_gappy + n_stale > 0 {
        println!("→ Sağlıksız seri var. Backfill PLANI üret:  ... db_coverage <market> <interval> --plan");
    } else {
        println!("→ Tüm seriler skorlamaya hazır görünüyor.");
    }
}

/// Plan modu — sağlıksız serilerden çalıştırılabilir download_candles komutları (stdout → script).
fn emit_plan(verdicts: &[Verdict], db_path: &str, now_ms: i64, th: &Thresholds) {
    use std::collections::BTreeMap;
    // (market, interval, force_full) → (semboller, en-eski ilk-bar). BTreeMap → deterministik sıra.
    let mut groups: BTreeMap<(String, String, bool), (Vec<String>, i64)> = BTreeMap::new();
    let mut iv_order: BTreeMap<(String, String), i64> = BTreeMap::new(); // interval'i bar-süresine göre sırala
    for v in verdicts.iter().filter(|v| !v.healthy()) {
        let key = (v.market.clone(), v.interval.clone(), v.needs_force_full());
        let e = groups.entry(key).or_insert((Vec::new(), i64::MAX));
        e.0.push(v.symbol.clone());
        e.1 = e.1.min(v.first_ms);
        iv_order.insert((v.market.clone(), v.interval.clone()), v.iv_secs);
    }

    // Yıl penceresi: default grubun en-eski ilk-barından türetilir; PLAN_YEARS env'i ile
    // tüm gruplara sabit ezme (ör. PLAN_YEARS=8 → derin 1d geçmişi).
    let years_override: Option<i64> = std::env::var("PLAN_YEARS").ok().and_then(|s| s.parse().ok());

    let unhealthy: usize = verdicts.iter().filter(|v| !v.healthy()).count();
    println!("#!/usr/bin/env bash");
    println!("# db_coverage backfill planı · {} sağlıksız seri · eşik: min_rows={} max_gap={}% max_age={}bar{}",
        unhealthy, th.min_rows, th.max_gap_pct, th.max_age_bars,
        years_override.map(|y| format!(" · PLAN_YEARS={y}")).unwrap_or_default());
    println!("# FORCE_FULL = iç-gap/seyrek (tüm pencere yeniden) · artımlı = yalnız bayat-kuyruk");
    println!("# Komutlar SIRALI (serial) koşar; save_candle upsert → tekrar çalıştırmak güvenli.");
    println!("set -euo pipefail");
    println!("DB_PATH=\"${{DB_PATH:-{}}}\"", db_path);
    println!("export DB_PATH");
    println!();

    if groups.is_empty() {
        println!("echo 'Sağlıksız seri yok — plan boş.'");
        return;
    }

    // interval'i bar-süresine göre sıralı gez (1m → 1d): deterministik + kısa-TF önce.
    let mut keys: Vec<&(String, String, bool)> = groups.keys().collect();
    keys.sort_by(|a, b| {
        let sa = iv_order.get(&(a.0.clone(), a.1.clone())).copied().unwrap_or(0);
        let sb = iv_order.get(&(b.0.clone(), b.1.clone())).copied().unwrap_or(0);
        a.0.cmp(&b.0).then(sa.cmp(&sb)).then(b.2.cmp(&a.2)) // force_full grubu önce
    });

    for key in keys {
        let (market, interval, force_full) = key;
        let (syms, min_first) = groups.get(key).unwrap();
        let mut syms = syms.clone();
        syms.sort();          // alfabetik → deterministik
        syms.dedup();
        let years = years_override.unwrap_or_else(|| years_for_window(*min_first, now_ms));
        let mode = if *force_full { "FORCE_FULL (iç-gap/seyrek)" } else { "artımlı (bayat-kuyruk)" };
        println!("echo '▶ {} {} · {} · {} sembol'", market, interval, mode, syms.len());
        let prefix = if *force_full { "FORCE_FULL=1 " } else { "" };
        println!(
            "{}cargo run --release -p memos_trading_core --example download_candles -- {} {} {} {}",
            prefix, market, interval, syms.join(","), years,
        );
        println!();
    }
    println!("echo '✅ backfill tamam — doğrula: cargo run --release -p memos_trading_core --example db_coverage'");
}
