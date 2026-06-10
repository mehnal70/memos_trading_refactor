// bb_pool — "1d-BB havuzlanmış" hipotez ÖLÇÜM aracı.
//
// Amaç: edge_scan'in per-sembol ekseni çoklu-test (Šidák) altında tükendi; tek yapısal ipucu
// BB(Bollinger) stratejisinin 1d'de birden çok majörde robust çıkmasıydı. Tek tek hiçbiri
// aile-düzeyini geçmez ve aynı strateji+TF oldukları için bağımsız kanıt değiller. Bu araç doğru
// testi yapar: BB-ortalama-dönüşünü per-sembol değil, majör sepetinde TEK PORTFÖY-SERİSİ olarak
// HAVUZLAR (alt-bandın altı long, üst-bandın üstü short; her bacak eşit-ağırlık market-nötr) →
// küçük örneklem dik kesilir, gerçek edge varsa N büyür ve p-değeri anlamlıya gider. XS momentum'la
// AYNI istatistik makinesi (Newey-West HAC + WF binom + Šidák) — [[project_xs_momentum]].
//
// Kullanım:
//   cargo run --release --example bb_pool -- [market] [interval] [SYM1,SYM2,...]
// Örnek:
//   cargo run --release --example bb_pool -- futures 1d BTCUSDT,ETHUSDT,BCHUSDT,XRPUSDT,...
//
// Env: DB_PATH (default data/trader.db), BB_FEE_RATE (turnover tek-yön, default 0.0005=5bps),
//      BB_PERIODS (csv, default 14,20,30), BB_KS (csv, default 1.5,2.0,2.5),
//      BB_WF_WINDOW (binom pencere bar, default 30), BB_REBALANCE (kadans, default 1),
//      BB_LONG_ONLY=1 (long-short yerine yalnız-long), BB_LEVERAGE (default 1.0),
//      BB_FAMILY_ALPHA (Šidák aile α, default 0.10), BB_CANDLE_LIMIT (default 5000),
//      BB_WF=1 (walk-forward OOS modu: IS-seç → kör OOS → Newey-West dürüst verdikt).

use memos_trading_core::robot::backtester::{
    run_bb_pool, run_bb_pool_walkforward, BbPoolConfig, BbWfConfig, BbWfResult,
};

fn csv(arg: Option<&String>) -> Vec<String> {
    match arg.map(|s| s.as_str()) {
        None | Some("all") | Some("") => Vec::new(),
        Some(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
    }
}

fn parse_csv_f64(key: &str, default: &[f64]) -> Vec<f64> {
    std::env::var(key).ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect::<Vec<f64>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_vec())
}

fn parse_csv_usize(key: &str, default: &[usize]) -> Vec<usize> {
    std::env::var(key).ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect::<Vec<usize>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_vec())
}

fn bars_per_year(interval: &str) -> f64 {
    match interval {
        "1m" => 525_600.0, "5m" => 105_120.0, "15m" => 35_040.0, "30m" => 17_520.0,
        "1h" => 8_760.0, "4h" => 2_190.0, "1d" => 365.0, _ => 365.0,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market = args.get(1).map(|s| s.as_str()).unwrap_or("futures").to_string();
    let interval = args.get(2).map(|s| s.as_str()).unwrap_or("1d").to_string();
    let symbols = csv(args.get(3));
    if symbols.len() < 4 {
        eprintln!("⚠️  En az 4 sembollük bir sepet ver (havuzlama anlamlı olsun).");
        eprintln!("    cargo run --release --example bb_pool -- futures 1d BTCUSDT,ETHUSDT,BCHUSDT,XRPUSDT,...");
        std::process::exit(2);
    }

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let fee_rate: f64 = std::env::var("BB_FEE_RATE").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0005);
    let wf_window: usize = std::env::var("BB_WF_WINDOW").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
    let rebalance_every: usize = std::env::var("BB_REBALANCE").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let leverage: f64 = std::env::var("BB_LEVERAGE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let long_short = std::env::var("BB_LONG_ONLY").map(|v| v != "1").unwrap_or(true);
    let family_alpha: f64 = std::env::var("BB_FAMILY_ALPHA").ok().and_then(|s| s.parse().ok()).unwrap_or(0.10);
    let candle_limit: usize = std::env::var("BB_CANDLE_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(5000);
    let periods = parse_csv_usize("BB_PERIODS", &[14, 20, 30]);
    let ks = parse_csv_f64("BB_KS", &[1.5, 2.0, 2.5]);

    let base = BbPoolConfig {
        db_path, market: market.clone(), interval: interval.clone(), symbols: symbols.clone(),
        candle_limit, fee_rate, long_short, rebalance_every, leverage, wf_window,
        bars_per_year: bars_per_year(&interval),
        ..Default::default() // bb_period/bb_k aile döngüsünde override
    };

    // Aile: her (period × k). Šidák test-başı eşik = 1−(1−α)^(1/N).
    let mut configs: Vec<(usize, f64)> = Vec::new();
    for &p in &periods { for &k in &ks { configs.push((p, k)); } }
    let n_fam = configs.len() as f64;
    let sidak = 1.0 - (1.0 - family_alpha).powf(1.0 / n_fam);

    // WALK-FORWARD MODU (BB_WF=1): tüm-veride seçim YOK — IS'te aday ızgaradan en iyiyi seç,
    // GÖRMEDİĞİ OOS'a uygula, birleştir → Newey-West dürüst verdikt.
    if std::env::var("BB_WF").map(|v| v == "1").unwrap_or(false) {
        run_wf_mode(&base, &configs, &interval);
        return;
    }

    println!("📐 BB-pool (havuzlanmış ortalama-dönüş) · market={market} · interval={interval} · sepet={} sembol",
        symbols.len());
    println!("   fee={:.4}/turnover · {} · rebalance={rebalance_every} · lev={leverage} · aile={} · Šidák p≤{:.4}",
        fee_rate, if long_short { "LONG-SHORT" } else { "LONG-ONLY" }, configs.len(), sidak);
    println!();
    println!("   {:>4} {:>5} | {:>5} {:>8} {:>7} {:>6} | {:>7} {:>7} | {:>7} {:>7} | {:>4}/{:<4} {:>7} | verdikt",
        "per", "k", "bar", "annRet%", "Sharpe", "win%", "naif-t", "t-p", "NW-t", "NW-p", "wfW", "wfN", "binom-p");
    println!("   {}", "-".repeat(116));

    let mut survivors: Vec<(usize, f64, memos_trading_core::robot::backtester::XsResult)> = Vec::new();
    for (p, k) in &configs {
        let cfg = BbPoolConfig { bb_period: *p, bb_k: *k, ..base.clone() };
        let r = run_bb_pool(&cfg);
        let tp = r.t_pvalue();
        let np = r.nw_t_pvalue();
        let bp = r.wf.window_significance();
        // ANLAMLI = pozitif NW-t VE Newey-West p VE binom p Šidák altında (otokorelasyona dayanıklı).
        let sig = r.nw_t_stat > 0.0 && np <= sidak && bp <= sidak;
        if sig { survivors.push((*p, *k, r.clone())); }
        let verdict = if r.bars == 0 { "—veri yok".into() }
            else if sig { "✅ ANLAMLI".into() }
            else if r.nw_t_stat > 0.0 && np <= family_alpha { "~marjinal (Šidák değil)".to_string() }
            else { "·".into() };
        println!("   {:>4} {:>5.1} | {:>5} {:>8.1} {:>7.2} {:>5.0}% | {:>7.2} {:>7.3} | {:>7.2} {:>7.3} | {:>4}/{:<4} {:>7.3} | {}",
            p, k, r.bars, 100.0 * r.ann_return, r.ann_sharpe, 100.0 * r.win_rate,
            r.t_stat, tp, r.nw_t_stat, np, r.wf.profitable_windows, r.wf.windows, bp, verdict);
    }

    println!();
    if survivors.is_empty() {
        println!("→ SONUÇ: aile-düzeyi (Šidák p≤{:.4}) anlamlı havuzlanmış BB edge YOK bu sepette/maliyette.", sidak);
        println!("  (Tek-test geçen pozitif satırlar çoklu-karşılaştırmada yıkanıyor — per-sembol robust'lar fluke idi.)");
        println!("  Sonraki adım: BB_WF=1 ile look-ahead'siz OOS teyidi (tek-örneklem optimizmini de keser).");
    } else {
        println!("→ SONUÇ: {} config aile-düzeyi anlamlı (Šidák p≤{:.4}):", survivors.len(), sidak);
        for (p, k, r) in &survivors {
            println!("  ✅ period={p} k={k} · annRet {:.1}% · Sharpe {:.2} · NW-t={:.2} (p={:.4}) · binom-p={:.4} · {} bar",
                100.0 * r.ann_return, r.ann_sharpe, r.nw_t_stat, r.nw_t_pvalue(),
                r.wf.window_significance(), r.bars);
        }
        println!("  Tek portföy-serisinde yüzlerce rebalance → küçük-örneklem fluke DEĞİL. BB_WF=1 ile OOS teyidi şart.");
    }
}

/// Walk-forward OOS: aday ızgaradan IS'te seç, kör OOS'a uygula, birleştirilmiş OOS'ta Newey-West.
fn run_wf_mode(base: &BbPoolConfig, candidates: &[(usize, f64)], interval: &str) {
    let is_bars: usize = std::env::var("BB_WF_IS").ok().and_then(|s| s.parse().ok())
        .unwrap_or(if interval == "1d" { 730 } else { 2000 });
    let oos_bars: usize = std::env::var("BB_WF_OOS").ok().and_then(|s| s.parse().ok())
        .unwrap_or(if interval == "1d" { 180 } else { 500 });

    let wf = BbWfConfig { is_bars, oos_bars, candidates: candidates.to_vec() };
    let r: BbWfResult = run_bb_pool_walkforward(base, &wf);

    println!("🔁 BB-pool WALK-FORWARD OOS · IS={is_bars} bar / OOS={oos_bars} bar (örtüşmesiz) · aday={} config",
        candidates.len());
    println!("   Her pencerede IS-Sharpe en iyisi seçilir → GÖRMEDİĞİ OOS'a uygulanır → OOS'lar birleşir.");
    println!();
    if r.windows == 0 {
        println!("⚠️  Yeterli veri yok (IS+OOS toplamı seriden uzun). BB_WF_IS / BB_WF_OOS küçült.");
        return;
    }
    println!("   {:>4} | {:>12} | {:>9} | {:>10}", "pen", "seçim(per,k)", "IS-Sharpe", "OOS-ort%");
    println!("   {}", "-".repeat(46));
    for (i, ((p, k), (is_sh, oos_m))) in r.selections.iter().zip(&r.is_oos_pairs).enumerate() {
        println!("   {:>4} | {:>8},{:<3.1} | {:>9.2} | {:>10.3}", i + 1, p, k, is_sh, 100.0 * oos_m);
    }

    println!();
    let o = &r.oos;
    let tp = o.t_pvalue();
    let np = o.nw_t_pvalue();
    let bp = o.wf.window_significance();
    println!("══════ BİRLEŞTİRİLMİŞ OOS (look-ahead'siz) ══════");
    println!("   pencere={} · OOS bar={} · turnover_ort={:.2}", r.windows, o.bars, o.avg_turnover);
    println!("   annRet={:.1}% · Sharpe={:.2} · win%={:.0}", 100.0 * o.ann_return, o.ann_sharpe, 100.0 * o.win_rate);
    println!("   naif t={:.2} (p={:.4}) · NEWEY-WEST t={:.2} (p={:.4}, lag={}) · binom {}/{} (p={:.4})",
        o.t_stat, tp, o.nw_t_stat, np, o.nw_lag, o.wf.profitable_windows, o.wf.windows, bp);
    println!();
    if o.nw_t_stat > 0.0 && np <= 0.05 {
        println!("✅ OOS ANLAMLI (Newey-West p={:.4}≤0.05): havuzlanmış BB edge otokorelasyon düzeltmesinden SONRA da tutuyor.", np);
    } else if o.nw_t_stat > 0.0 {
        println!("~ OOS POZİTİF ama NW p={:.4}>0.05: naif t (p={:.4}) otokorelasyonla şişmiş; dürüst güç sınırda.", np, tp);
    } else {
        println!("✗ OOS NEGATİF/sıfır: havuzlanmış BB edge kör veride tutmadı → per-sembol robust'lar fluke idi. Canlıya BAĞLAMA.");
    }
}
