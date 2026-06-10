// xs_factors — kesitsel FAKTÖR taraması: momentum DIŞI dik eksenler (low-vol, BAB, lottery).
//
// Amaç: per-sembol ve per-strateji-havuzlama eksenleri tükendi ([[project_edge_scan]], [[project_bb_pool]]);
// doğrulanmış tek pooled edge XS momentum ([[project_xs_momentum]]). Bu araç AYNI havuzlama makinesini
// (align_closes → kesitsel sırala → market-nötr kitap → tek portföy-serisi → Newey-West + WF binom)
// momentum DIŞI sinyallere uygular → yeni dik edge var mı? Eksenler XsSignal enum'undan (tek çekirdek):
//   • LowVol  : düşük realize-vol long / yüksek short (low-vol anomali)
//   • Beta    : düşük β long / yüksek β short (betting-against-beta; β = sepet-ortalamasına regresyon)
//   • Lottery : "piyango" (yüksek aşırı tek-bar getiri) isimleri short (MAX etkisi)
//   • Momentum: baz çizgi (kıyas + diklik korelasyonu için)
//
// DİKLİK kanıtı: her eksenin EN İYİ config getiri-serisi ile momentum-en-iyi'nin Pearson korelasyonu
// raporlanır → |ρ| düşükse eksen gerçekten yeni bilgi taşıyor (momentum'un kılığı değil).
//
// Çoklu-test dürüstlüğü: TÜM eksen×lookback×yön config'leri TEK aile → Šidák eşiği (N config).
// ANLAMLI = Newey-West p VE binom p, aile-Šidák altında.
//
// Kullanım:
//   cargo run --release --example xs_factors -- [market] [interval] [SYM1,SYM2,...]
// Env: DB_PATH, XF_LOOKBACKS (csv, default 7,14,30,60), XF_AXES (csv: momentum,lowvol,beta,lottery),
//      XF_TOP_K (default 3), XF_FEE_RATE (default 0.0005), XF_WF_WINDOW (default 30),
//      XF_REBALANCE (default 1), XF_LONG_ONLY=1, XF_FAMILY_ALPHA (default 0.10), XF_CANDLE_LIMIT (5000).

use memos_trading_core::robot::backtester::{run_xs_momentum, run_xs_returns, XsConfig, XsResult, XsSignal};

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

fn axis_of(name: &str) -> Option<(XsSignal, &'static str)> {
    match name.trim().to_lowercase().as_str() {
        "momentum" | "mom" => Some((XsSignal::Momentum, "momentum")),
        "lowvol" | "vol"   => Some((XsSignal::LowVol, "lowvol")),
        "beta" | "bab"     => Some((XsSignal::Beta, "beta")),
        "lottery" | "max"  => Some((XsSignal::MaxLottery, "lottery")),
        _ => None,
    }
}

/// Tail-hizalı Pearson korelasyonu (iki seriyi sondan eşitle → warmup farkını ele; ikisi de son bara biter).
fn pearson_tail(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 3 { return 0.0; }
    let (a, b) = (&a[a.len() - n..], &b[b.len() - n..]);
    let (ma, mb) = (a.iter().sum::<f64>() / n as f64, b.iter().sum::<f64>() / n as f64);
    let mut cov = 0.0; let mut va = 0.0; let mut vb = 0.0;
    for i in 0..n {
        let (da, db) = (a[i] - ma, b[i] - mb);
        cov += da * db; va += da * da; vb += db * db;
    }
    if va <= 0.0 || vb <= 0.0 { 0.0 } else { cov / (va.sqrt() * vb.sqrt()) }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market = args.get(1).map(|s| s.as_str()).unwrap_or("futures").to_string();
    let interval = args.get(2).map(|s| s.as_str()).unwrap_or("1d").to_string();
    let symbols = csv(args.get(3));
    if symbols.len() < 4 {
        eprintln!("⚠️  En az 4 sembollük bir sepet ver (kesitsel sıralama anlamlı olsun).");
        eprintln!("    cargo run --release --example xs_factors -- futures 1d BTCUSDT,ETHUSDT,BCHUSDT,XRPUSDT,...");
        std::process::exit(2);
    }

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let top_k: usize = std::env::var("XF_TOP_K").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let fee_rate: f64 = std::env::var("XF_FEE_RATE").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0005);
    let wf_window: usize = std::env::var("XF_WF_WINDOW").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
    let rebalance_every: usize = std::env::var("XF_REBALANCE").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let long_short = std::env::var("XF_LONG_ONLY").map(|v| v != "1").unwrap_or(true);
    let family_alpha: f64 = std::env::var("XF_FAMILY_ALPHA").ok().and_then(|s| s.parse().ok()).unwrap_or(0.10);
    let candle_limit: usize = std::env::var("XF_CANDLE_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(5000);
    let lookbacks: Vec<usize> = std::env::var("XF_LOOKBACKS").ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![7, 14, 30, 60]);
    let axes: Vec<(XsSignal, &str)> = std::env::var("XF_AXES").ok()
        .map(|s| s.split(',').filter_map(|x| axis_of(x)).collect::<Vec<_>>())
        .filter(|v: &Vec<_>| !v.is_empty())
        .unwrap_or_else(|| vec![
            (XsSignal::Momentum, "momentum"), (XsSignal::LowVol, "lowvol"),
            (XsSignal::Beta, "beta"), (XsSignal::MaxLottery, "lottery"),
        ]);

    let base = XsConfig {
        db_path, market: market.clone(), interval: interval.clone(), symbols: symbols.clone(),
        candle_limit, top_k, fee_rate, long_short, wf_window, rebalance_every,
        bars_per_year: bars_per_year(&interval), ..Default::default()
    };

    // AİLE = tüm eksen × lookback × {yön doğal, reversal}. Šidák test-başı eşik.
    let n_fam = (axes.len() * lookbacks.len() * 2) as f64;
    let sidak = 1.0 - (1.0 - family_alpha).powf(1.0 / n_fam);

    println!("🧭 Kesitsel FAKTÖR taraması · market={market} · interval={interval} · sepet={} sembol",
        symbols.len());
    println!("   eksenler=[{}] · lookback=[{}] · top_k={top_k} · fee={:.4} · {} · aile={} · Šidák p≤{:.4}",
        axes.iter().map(|(_, n)| *n).collect::<Vec<_>>().join(","),
        lookbacks.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(","),
        fee_rate, if long_short { "L/S" } else { "LONG-ONLY" }, n_fam as usize, sidak);
    println!();
    println!("   {:<9} {:>4} {:>4} | {:>5} {:>8} {:>7} {:>6} | {:>7} {:>7} | {:>7} | verdikt",
        "eksen", "lb", "yön", "bar", "annRet%", "Sharpe", "win%", "NW-t", "NW-p", "binom-p");
    println!("   {}", "-".repeat(104));

    // En iyi config'in (eksen başına) getiri serisini diklik korelasyonu için sakla.
    let mut best_rets: std::collections::HashMap<&str, (f64, Vec<f64>, usize)> = std::collections::HashMap::new();
    let mut survivors: Vec<(String, usize, bool, XsResult)> = Vec::new();

    for (sig, aname) in &axes {
        for &lb in &lookbacks {
            for &mom in &[true, false] {
                let cfg = XsConfig { signal: *sig, lookback: lb, momentum: mom, ..base.clone() };
                let r = run_xs_momentum(&cfg);
                let np = r.nw_t_pvalue();
                let bp = r.wf.window_significance();
                let is_sig = r.nw_t_stat > 0.0 && np <= sidak && bp <= sidak;
                if is_sig { survivors.push((aname.to_string(), lb, mom, r.clone())); }
                // Eksen-en-iyi (en yüksek NW-t) getiri serisini sakla (diklik için).
                if r.bars > 0 {
                    let better = best_rets.get(*aname).map(|(t, _, _)| r.nw_t_stat > *t).unwrap_or(true);
                    if better {
                        let (rets, _) = run_xs_returns(&cfg);
                        best_rets.insert(aname, (r.nw_t_stat, rets, lb));
                    }
                }
                let verdict = if r.bars == 0 { "—veri yok".into() }
                    else if is_sig { "✅ ANLAMLI".into() }
                    else if r.nw_t_stat > 0.0 && np <= family_alpha { "~marjinal".to_string() }
                    else { "·".into() };
                println!("   {:<9} {:>4} {:>4} | {:>5} {:>8.1} {:>7.2} {:>5.0}% | {:>7.2} {:>7.3} | {:>7.3} | {}",
                    aname, lb, if mom { "nat" } else { "rev" }, r.bars, 100.0 * r.ann_return,
                    r.ann_sharpe, 100.0 * r.win_rate, r.nw_t_stat, np, bp, verdict);
            }
        }
    }

    // ───────── DİKLİK: her eksen-en-iyi'nin momentum-en-iyi ile getiri korelasyonu ─────────
    println!();
    if let Some((_, mom_rets, mom_lb)) = best_rets.get("momentum").cloned() {
        println!("🧭 DİKLİK (eksen-en-iyi getiri serisi ↔ momentum-en-iyi, lb={mom_lb}):");
        for (sig, aname) in &axes {
            if *aname == "momentum" { continue; }
            if let Some((_, rets, lb)) = best_rets.get(*aname) {
                let rho = pearson_tail(rets, &mom_rets);
                let tag = if rho.abs() < 0.3 { "✓ dik" } else if rho.abs() < 0.6 { "~ kısmi örtüşme" } else { "✗ momentum kılığı" };
                let _ = sig;
                println!("   {:<9} (lb={:>3}) · ρ={:+.2}  {}", aname, lb, rho, tag);
            }
        }
    }

    println!();
    if survivors.is_empty() {
        println!("→ SONUÇ: aile-düzeyi (Šidák p≤{:.4}) anlamlı dik faktör edge YOK bu sepette/maliyette.", sidak);
        println!("  (Tek-test geçenler çoklu-karşılaştırmada yıkanıyor. Sonraki: en güçlü ekseni BB_WF tarzı");
        println!("   look-ahead'siz OOS'ta teyit, ya da funding-carry ekseni — fiyat-dışı taşıma getirisi.)");
    } else {
        println!("→ SONUÇ: {} config aile-düzeyi anlamlı (Šidák p≤{:.4}):", survivors.len(), sidak);
        for (ax, lb, mom, r) in &survivors {
            println!("  ✅ {ax} lb={lb} {} · annRet {:.1}% · Sharpe {:.2} · NW-t={:.2} (p={:.4}) · binom-p={:.4} · {} bar",
                if *mom { "nat" } else { "rev" }, 100.0 * r.ann_return, r.ann_sharpe,
                r.nw_t_stat, r.nw_t_pvalue(), r.wf.window_significance(), r.bars);
        }
        println!("  Tek portföy-serisinde yüzlerce rebalance → fluke DEĞİL. Diklik ρ düşükse momentum'a EK edge.");
    }
}
