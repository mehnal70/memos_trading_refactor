// xs_momentum — kesitsel (cross-sectional) relatif-güç sinyali ÖLÇÜM aracı.
//
// Amaç: per-sembol gross-edge taraması istatistiksel duvara tosladıktan sonra ([[project_edge_scan]]:
// 15 majörden çoklu-test düzeltmesi altında 0 robust edge) DİK bir sinyal ekseni dener — edge'i tek
// sembolde değil majör SEPETİ üzerinde KESİTSEL sorar. Her bar sepeti relatif momentuma göre sıralar,
// en güçlüyü long / en zayıfı short (market-nötr spread) → getiri TEK portföy-zaman-serisi (yüzlerce
// rebalance), 20-pencere/sembol değil → t-istatistiği gerçek güce kavuşur.
//
// Lookback × {momentum, reversal} ızgarasını tarar; her satır için annRet/Sharpe/win% + t-stat,
// t-p-değeri ve projenin binom pencere-kapısı p-değeri. Çoklu-test dürüstlüğü: aile-düzeyi Šidák
// eşiği (N config) → "şans eseri geçen" yıkanır; ANLAMLI satır = HER İKİ p de Šidák altında.
//
// Kullanım:
//   cargo run --release --example xs_momentum -- [market] [interval] [SYM1,SYM2,...]
// Örnek:
//   cargo run --release --example xs_momentum -- futures 1d BTCUSDT,ETHUSDT,BCHUSDT,XRPUSDT,...
//
// Env: DB_PATH (default data/trader.db), XS_TOP_K (sepet kenarı, default 3),
//      XS_FEE_RATE (turnover birimi başına tek-yön maliyet, default 0.0005=5bps),
//      XS_LOOKBACKS (csv, default 1,3,7,14,30), XS_WF_WINDOW (binom pencere bar, default 30),
//      XS_LONG_ONLY=1 (long-short yerine long-only top), XS_FAMILY_ALPHA (Šidák aile α, default 0.10).

use memos_trading_core::robot::backtester::{run_xs_momentum, XsConfig, XsResult};

fn csv(arg: Option<&String>) -> Vec<String> {
    match arg.map(|s| s.as_str()) {
        None | Some("all") | Some("") => Vec::new(),
        Some(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
    }
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
        eprintln!("⚠️  En az 4 sembollük bir sepet ver (kesitsel sıralama anlamlı olsun).");
        eprintln!("    cargo run --release --example xs_momentum -- futures 1d BTCUSDT,ETHUSDT,BCHUSDT,XRPUSDT,...");
        std::process::exit(2);
    }

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let top_k: usize = std::env::var("XS_TOP_K").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let fee_rate: f64 = std::env::var("XS_FEE_RATE").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0005);
    let wf_window: usize = std::env::var("XS_WF_WINDOW").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
    let rebalance_every: usize = std::env::var("XS_REBALANCE").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let long_short = std::env::var("XS_LONG_ONLY").map(|v| v != "1").unwrap_or(true);
    let family_alpha: f64 = std::env::var("XS_FAMILY_ALPHA").ok().and_then(|s| s.parse().ok()).unwrap_or(0.10);
    let lookbacks: Vec<usize> = std::env::var("XS_LOOKBACKS").ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 3, 7, 14, 30]);

    let base = XsConfig {
        db_path, market: market.clone(), interval: interval.clone(), symbols: symbols.clone(),
        candle_limit: 5000, top_k, fee_rate, long_short, wf_window, rebalance_every,
        bars_per_year: bars_per_year(&interval), ..Default::default()
    };

    // Aile: her lookback × {momentum, reversal}. Šidák test-başı eşik = 1−(1−α)^(1/N).
    let mut configs: Vec<(usize, bool)> = Vec::new();
    for &lb in &lookbacks {
        configs.push((lb, true));  // momentum
        configs.push((lb, false)); // reversal
    }
    let n_fam = configs.len() as f64;
    let sidak = 1.0 - (1.0 - family_alpha).powf(1.0 / n_fam);

    println!("📐 Kesitsel relatif-güç · market={market} · interval={interval} · sepet={} sembol",
        symbols.len());
    println!("   top_k={top_k} · fee={:.4}/turnover · {} · rebalance={rebalance_every} bar · wf_window={wf_window} · aile={} config · Šidák p≤{:.4}",
        fee_rate, if long_short { "LONG-SHORT (nötr)" } else { "LONG-ONLY" }, configs.len(), sidak);
    println!();
    println!("   {:>4} {:>9} | {:>5} {:>8} {:>7} {:>6} | {:>7} {:>7} | {:>4}/{:<4} {:>7} | verdikt",
        "lb", "yön", "bar", "annRet%", "Sharpe", "win%", "t-stat", "t-p", "wfW", "wfN", "binom-p");
    println!("   {}", "-".repeat(104));

    let mut survivors: Vec<(usize, bool, XsResult)> = Vec::new();
    for (lb, mom) in &configs {
        let cfg = XsConfig { lookback: *lb, momentum: *mom, ..base.clone() };
        let r = run_xs_momentum(&cfg);
        let tp = r.t_pvalue();
        let bp = r.wf.window_significance();
        // ANLAMLI = pozitif edge VE her iki p-değeri de Šidák altında (aile-düzeyi).
        let sig = r.t_stat > 0.0 && tp <= sidak && bp <= sidak;
        if sig { survivors.push((*lb, *mom, r.clone())); }
        let verdict = if r.bars == 0 { "—veri yok".into() }
            else if sig { "✅ ANLAMLI".into() }
            else if r.t_stat > 0.0 && tp <= family_alpha { "~marjinal (Šidák değil)".to_string() }
            else { "·".into() };
        println!("   {:>4} {:>9} | {:>5} {:>8.1} {:>7.2} {:>5.0}% | {:>7.2} {:>7.3} | {:>4}/{:<4} {:>7.3} | {}",
            lb, if *mom { "momentum" } else { "reversal" },
            r.bars, 100.0 * r.ann_return, r.ann_sharpe, 100.0 * r.win_rate,
            r.t_stat, tp, r.wf.profitable_windows, r.wf.windows, bp, verdict);
    }

    println!();
    if survivors.is_empty() {
        println!("→ SONUÇ: aile-düzeyi (Šidák p≤{:.4}) anlamlı kesitsel edge YOK bu sepette/maliyette.", sidak);
        println!("  (Pozitif t-stat'lı satırlar tek-test geçse de çoklu-karşılaştırmada yıkanıyor.)");
    } else {
        println!("→ SONUÇ: {} config aile-düzeyi anlamlı (Šidák p≤{:.4}):", survivors.len(), sidak);
        for (lb, mom, r) in &survivors {
            println!("  ✅ lookback={lb} {} · annRet {:.1}% · Sharpe {:.2} · t={:.2} (p={:.4}) · binom-p={:.4} · {} bar",
                if *mom { "momentum" } else { "reversal" },
                100.0 * r.ann_return, r.ann_sharpe, r.t_stat, r.t_pvalue(),
                r.wf.window_significance(), r.bars);
        }
        println!("  Bunlar tek portföy-serisinde yüzlerce rebalance üzerinde ölçüldü → küçük-örneklem fluke DEĞİL.");
    }
}
