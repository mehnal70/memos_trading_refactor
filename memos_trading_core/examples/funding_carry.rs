// funding_carry — kesitsel FUNDING-CARRY ÖLÇÜM aracı (fiyat-dışı taşıma getirisi; son dik eksen).
//
// Amaç: mum-türevi dik eksenler (low-vol/BAB/lottery) edge'siz çıktı ([[project_xs_factors]]). Funding-
// carry fiyat sinyali bile değil — perp TAŞIMA getirisi. Yüksek-funding'i short (funding alır) /
// düşük-funding'i long ederek market-nötr book funding SPREAD'ini hasat eder. Getiri = fiyat − funding
// (carry nakit-akışı dahil). XS makinesiyle AYNI istatistik (Newey-West HAC + WF binom + Šidák).
//
// ÖN-KOŞUL: funding verisi DB'de olmalı → önce:
//   cargo run --release --example download_funding -- futures BTCUSDT,ETHUSDT,... 8
//
// Kullanım:
//   cargo run --release --example funding_carry -- [market] [interval] [SYM1,SYM2,...]
// Env: DB_PATH, FC_LOOKBACKS (csv, default 3,7,14,30), FC_TOP_K (3), FC_FEE_RATE (0.0005),
//      FC_WF_WINDOW (30), FC_REBALANCE (1), FC_LONG_ONLY=1, FC_FAMILY_ALPHA (0.10),
//      FC_CANDLE_LIMIT (5000), FC_FUNDING_LIMIT (20000).

use memos_trading_core::robot::backtester::{run_funding_carry, FundingCarryConfig, XsResult};

fn csv(arg: Option<&String>) -> Vec<String> {
    match arg.map(|s| s.as_str()) {
        None | Some("all") | Some("") => Vec::new(),
        Some(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
    }
}

fn bars_per_year(interval: &str) -> f64 {
    match interval {
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
        eprintln!("    cargo run --release --example funding_carry -- futures 1d BTCUSDT,ETHUSDT,SOLUSDT,XRPUSDT,...");
        std::process::exit(2);
    }

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let top_k: usize = std::env::var("FC_TOP_K").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let fee_rate: f64 = std::env::var("FC_FEE_RATE").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0005);
    let wf_window: usize = std::env::var("FC_WF_WINDOW").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
    let rebalance_every: usize = std::env::var("FC_REBALANCE").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let long_short = std::env::var("FC_LONG_ONLY").map(|v| v != "1").unwrap_or(true);
    let family_alpha: f64 = std::env::var("FC_FAMILY_ALPHA").ok().and_then(|s| s.parse().ok()).unwrap_or(0.10);
    let candle_limit: usize = std::env::var("FC_CANDLE_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(5000);
    let funding_limit: usize = std::env::var("FC_FUNDING_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let lookbacks: Vec<usize> = std::env::var("FC_LOOKBACKS").ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![3, 7, 14, 30]);

    let base = FundingCarryConfig {
        db_path, market: market.clone(), interval: interval.clone(), symbols: symbols.clone(),
        candle_limit, funding_limit, top_k, fee_rate, long_short, wf_window, rebalance_every,
        bars_per_year: bars_per_year(&interval), ..Default::default()
    };

    // AİLE = lookback'ler (carry yönü tek: yüksek-funding short). Šidák test-başı eşik.
    // (Reversal'ı negatifleyerek simüle etmek turnover/fee asimetrisi yüzünden YANLIŞ olurdu → yalnız doğal yön.)
    let n_fam = lookbacks.len() as f64;
    let sidak = 1.0 - (1.0 - family_alpha).powf(1.0 / n_fam);

    println!("💰 Funding-carry (havuzlanmış taşıma) · market={market} · interval={interval} · sepet={} sembol",
        symbols.len());
    println!("   top_k={top_k} · fee={:.4}/turnover · {} · rebalance={rebalance_every} · aile={} · Šidák p≤{:.4}",
        fee_rate, if long_short { "L/S" } else { "LONG-ONLY" }, n_fam as usize, sidak);
    println!("   (getiri = fiyat_getirisi − funding; yüksek-funding short / düşük-funding long)");
    println!();
    println!("   {:>4} {:>4} | {:>5} {:>8} {:>7} {:>6} | {:>7} {:>7} | {:>7} | verdikt",
        "lb", "yön", "bar", "annRet%", "Sharpe", "win%", "NW-t", "NW-p", "binom-p");
    println!("   {}", "-".repeat(96));

    let mut survivors: Vec<(usize, XsResult)> = Vec::new();
    let mut any_bars = false;
    for &lb in &lookbacks {
        let cfg = FundingCarryConfig { lookback: lb, ..base.clone() };
        let r = run_funding_carry(&cfg);
        if r.bars > 0 { any_bars = true; }
        let np = r.nw_t_pvalue();
        let bp = r.wf.window_significance();
        let is_sig = r.nw_t_stat > 0.0 && np <= sidak && bp <= sidak;
        if is_sig { survivors.push((lb, r.clone())); }
        let verdict = if r.bars == 0 { "—veri yok".into() }
            else if is_sig { "✅ ANLAMLI".into() }
            else if r.nw_t_stat > 0.0 && np <= family_alpha { "~marjinal".to_string() }
            else { "·".into() };
        println!("   {:>4} {:>4} | {:>5} {:>8.1} {:>7.2} {:>5.0}% | {:>7.2} {:>7.3} | {:>7.3} | {}",
            lb, "nat", r.bars, 100.0 * r.ann_return, r.ann_sharpe, 100.0 * r.win_rate,
            r.nw_t_stat, np, bp, verdict);
    }

    println!();
    if !any_bars {
        println!("⚠️  Hiç bar üretilmedi → DB'de bu sepet için funding/mum yok ya da hizalama boş.");
        println!("    Önce: cargo run --release --example download_funding -- futures {} 8",
            symbols.iter().take(4).cloned().collect::<Vec<_>>().join(","));
        return;
    }
    if survivors.is_empty() {
        println!("→ SONUÇ: aile-düzeyi (Šidák p≤{:.4}) anlamlı funding-carry edge YOK bu sepette/maliyette.", sidak);
        println!("  (Pozitif satırlar çoklu-karşılaştırmada yıkanıyor.) Aile-Šidák eşiği ile NW p her ikisi gerek.");
    } else {
        println!("→ SONUÇ: {} config aile-düzeyi anlamlı (Šidák p≤{:.4}):", survivors.len(), sidak);
        for (lb, r) in &survivors {
            println!("  ✅ lookback={lb} · annRet {:.1}% · Sharpe {:.2} · NW-t={:.2} (p={:.4}) · binom-p={:.4} · {} bar",
                100.0 * r.ann_return, r.ann_sharpe, r.nw_t_stat, r.nw_t_pvalue(),
                r.wf.window_significance(), r.bars);
        }
        println!("  Tek portföy-serisinde yüzlerce rebalance → fluke DEĞİL. Diğer eksenlerle korelasyonu düşükse EK edge.");
    }
}
