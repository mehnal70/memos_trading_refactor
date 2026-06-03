// edge_scan — terminalden çalıştırılabilir DB-geneli GROSS-EDGE tarayıcı.
//
// Amaç: DB'deki TARAMAYA-DEĞER tüm (exchange/market/symbol/interval) serilerinde strateji+param
// ızgarasını AYNI dürüst koşulda (veri-sağlık kapısı → holdout %70 IS / %30 OOS → strateji havuzu
// → OOS PF) backtest edip "hangi seri+strateji NET KÂRLI edge (PF≥1.0) taşıyor" sorusunu sayıyla
// yanıtlar. Çekirdek lib'de (robot::backtester::edge_scan); bu CLI yalnız arg/env → config + rapor.
//
// Kullanım:
//   cargo run --release --example edge_scan -- [market] [intervals] [symbols] [limit]
// Örnekler:
//   cargo run --release --example edge_scan -- all                  # tüm DB
//   cargo run --release --example edge_scan -- futures 1h,4h        # yalnız futures, 1h+4h
//   cargo run --release --example edge_scan -- futures 1h BTCUSDT,ETHUSDT 5000
//
// Konum argümanları: market(all|futures|spot|...), intervals(csv|all), symbols(csv|all), limit.
// Env: DB_PATH (default data/trader.db), EDGE_SCAN_OUT (rapor JSON yolu),
//      EDGE_SCAN_MAX_SERIES (güvenli üst sınır, default 300),
//      EDGE_SCAN_EDGE_MIN (giriş edge kapısı, default 0.20). CANLI giriş hunisi rejim+ml'e göre
//      0.30–0.55 YÜZEN eşik uyguladığından, seed adaylarını canlı-gerçekçi kapıda (örn. 0.45)
//      yeniden doğrulamak için bunu yükselt → rapor PF'leri canlı-temsili olur (marjinal kuyruk elenir).
//      Rapor her satırın günlük quote-volume'ünü (qvol/gün) taşır → seed'i MAJÖRLERE daraltmak için
//      EDGE_SEED_MIN_QVOL (USDT/gün) ayarla (illikit-alt edge'ler canlı feed'de purge ediliyordu).
//
// Rapor JSON'a yazılır → tekrar koşularda karşılaştır/biriktir. PF MUTLAK değil; veri-sağlık
// + holdout dürüstlüğü içinde "edge var mı" göstergesidir.

use memos_trading_core::robot::backtester::{run_edge_scan_with_progress, EdgeScanConfig};

fn csv_or_empty(arg: Option<&String>) -> Vec<String> {
    match arg.map(|s| s.as_str()) {
        None | Some("all") | Some("") => Vec::new(),
        Some(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market = args.get(1).map(|s| s.as_str()).unwrap_or("all");
    let market_filter = if market == "all" || market.is_empty() { None } else { Some(market.to_string()) };
    let interval_filter = csv_or_empty(args.get(2));
    let symbol_filter = csv_or_empty(args.get(3));
    let limit: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(5000);
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".to_string());
    let max_series: usize = std::env::var("EDGE_SCAN_MAX_SERIES").ok()
        .and_then(|s| s.parse().ok()).unwrap_or(300);
    // Giriş edge kapısı: canlı-gerçekçi doğrulama için yükseltilebilir (default = EdgeScanConfig 0.20).
    let edge_min: f64 = std::env::var("EDGE_SCAN_EDGE_MIN").ok()
        .and_then(|s| s.parse().ok())
        .filter(|v: &f64| (0.0..=1.0).contains(v))
        .unwrap_or_else(|| EdgeScanConfig::default().edge_min);
    let out_path = std::env::var("EDGE_SCAN_OUT").unwrap_or_else(|_| {
        format!("reports/edge_scan_{}.json", chrono::Utc::now().format("%Y%m%d_%H%M%S"))
    });

    let cfg = EdgeScanConfig {
        db_path: db_path.clone(),
        market_filter: market_filter.clone(),
        symbol_filter,
        interval_filter,
        candle_limit: limit,
        max_series,
        edge_min,
        ..Default::default()
    };

    println!("\n🔬 edge_scan · db={db_path} · market={} · interval={} · symbol={} · limit={limit} · max_series={max_series}",
        market_filter.as_deref().unwrap_or("all"),
        if cfg.interval_filter.is_empty() { "all".into() } else { cfg.interval_filter.join(",") },
        if cfg.symbol_filter.is_empty() { "all".into() } else { cfg.symbol_filter.join(",") },
    );
    println!("   edge≥{} · breakeven@RR {} · holdout %{} IS / OOS · canlı-temsili trailing · min işlem {}",
        cfg.edge_min, cfg.breakeven_rr, cfg.holdout_is_pct, cfg.min_trades);
    println!("   (seri sayısına göre dakikalar sürebilir; strateji havuzu × TP/SL/PS ızgarası optimize ediliyor)\n");

    // İlerleme stderr'e (uzun toplu koşuda görünürlük; stdout tabloyu temiz tutar).
    let report = run_edge_scan_with_progress(&cfg, |i, total, s| {
        eprintln!("  [{i}/{total}] {} {} {} ({} bar)…", s.market, s.symbol, s.interval, s.rows);
    });

    // ─── Grup özeti (market × interval survey) ───────────────────────────────
    if !report.summary.is_empty() {
        println!("\n══════ ÖZET (market × interval · en iyi PF) ══════");
        println!("  {:<9} {:<5} {:>8} {:>7} {:>7}  en iyi", "market", "iv", "taranan", "kârlı", "bestPF");
        for g in &report.summary {
            println!("  {:<9} {:<5} {:>8} {:>7} {:>7.2}  {} ({})",
                g.market, g.interval, g.scanned, g.profitable, g.best_pf, g.best_symbol, g.best_strategy);
        }
    }

    // ─── Sıralı tablo ────────────────────────────────────────────────────────
    println!("\n══════ SONUÇ ({} aday seri · {} tarandı · {} atlandı · {} NET KÂRLI) ══════",
        report.series_candidates, report.series_scanned, report.series_skipped, report.profitable_count);
    if report.rows.is_empty() {
        println!("  Taranabilir seri yok — filtre çok dar ya da veri yetersiz/gappy.");
    } else {
        println!("  {:<4} {:<9} {:<10} {:<5} {:<14} {:>5} {:>6} {:>6} {:>7} {:>7} {:>5} {:>9}",
            "#", "market", "symbol", "iv", "strateji", "işl", "win%", "PF", "wfPF", "tutar%", "WF✓", "qvol/gün");
        for (i, r) in report.rows.iter().take(40).enumerate() {
            let flag = if r.profitable { "✅" } else if r.profit_factor >= 0.9 { "≈" } else { "❌" };
            let wf_flag = if r.wf_robust { "✅" } else { "—" };
            // qvol kısa-form: milyon (M) / milyar (B) → majör tabanını (EDGE_SEED_MIN_QVOL) kalibre etmek için.
            let q = r.avg_daily_quote_volume;
            let qstr = if q >= 1e9 { format!("{:.1}B", q / 1e9) }
                       else if q >= 1e6 { format!("{:.0}M", q / 1e6) }
                       else { format!("{:.0}", q) };
            println!("  {:<4} {:<9} {:<10} {:<5} {:<14} {:>5} {:>5.0}% {:>5.2}{} {:>7.2} {:>6.0}% {:>5} {:>9}",
                i + 1, r.market, r.symbol, r.interval, r.best_strategy,
                r.trades, r.win_rate, r.profit_factor, flag,
                r.wf.pooled_pf, r.wf.consistency() * 100.0, wf_flag, qstr);
        }
        if report.rows.len() > 40 { println!("  … ({} satır daha JSON'da)", report.rows.len() - 40); }
    }

    // ─── JSON mühürle ──────────────────────────────────────────────────────────
    if let Some(parent) = std::path::Path::new(&out_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
        .and_then(|j| std::fs::write(&out_path, j).map_err(|e| e.to_string()))
    {
        Ok(_) => println!("\n📄 Rapor: {out_path}"),
        Err(e) => eprintln!("\n⚠️ Rapor yazılamadı ({out_path}): {e}"),
    }
    let wf_robust_count = report.rows.iter().filter(|r| r.wf_robust).count();
    println!("\n→ NET KÂRLI (holdout PF≥1.0, işlem≥{}): {} · WF-ONAYLI (çoklu-pencere tutarlı): {}",
        cfg.min_trades, report.profitable_count, wf_robust_count);
    println!("  Seed yalnız WF-ONAYLI satırları alır (fluke eler). Başlatırken: EDGE_SEED_REPORT={out_path}\n");
}
