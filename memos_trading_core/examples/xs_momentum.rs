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
//      XS_SWAP_RATE (overnight swap/rollover — gross-exposure başına BAR-başı tutma maliyeti,
//                    turnover'dan ayrı; FX'te asıl drag; default 0=kapalı),
//      XS_LOOKBACKS (csv, default 1,3,7,14,30), XS_WF_WINDOW (binom pencere bar, default 30),
//      XS_LONG_ONLY=1 (long-short yerine long-only top), XS_FAMILY_ALPHA (Šidák aile α, default 0.10).

use memos_trading_core::robot::backtester::{run_xs_momentum, run_xs_walkforward, XsConfig, XsResult, XsWfConfig};

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
    let exit_buffer: usize = std::env::var("XS_BUFFER").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let leverage: f64 = std::env::var("XS_LEVERAGE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let long_short = std::env::var("XS_LONG_ONLY").map(|v| v != "1").unwrap_or(true);
    let family_alpha: f64 = std::env::var("XS_FAMILY_ALPHA").ok().and_then(|s| s.parse().ok()).unwrap_or(0.10);
    let lookbacks: Vec<usize> = std::env::var("XS_LOOKBACKS").ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 3, 7, 14, 30]);
    // S/R EĞİMİ (opt-in): XS_SR_TILT>0 → sıralama-öncesi skoru S/R-karşıtlığına göre kıs (OHLC yüklenir).
    let sr_tilt: f64 = std::env::var("XS_SR_TILT").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let sr_band_pct: f64 = std::env::var("XS_SR_BAND").ok().and_then(|s| s.parse().ok()).unwrap_or(3.0);
    let sr_min_strength: f64 = std::env::var("XS_SR_MIN_STRENGTH").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let sr_window: usize = std::env::var("XS_SR_WINDOW").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    // Sembol başına yüklenen en-yeni mum tavanı. Düşük TF + derin geçmişte (4h 5y ≈ 10950 bar) artır.
    let candle_limit: usize = std::env::var("XS_CANDLE_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(5000);
    // 🌙 Overnight swap/rollover — gross-exposure başına BAR-başı tutma maliyeti (turnover'dan ayrı).
    // FX'in asıl drag'i: günlük rebalance'ta swap her gün tekrar ödenir. Default 0 = kapalı.
    let swap_rate: f64 = std::env::var("XS_SWAP_RATE").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0);

    let base = XsConfig {
        db_path, market: market.clone(), interval: interval.clone(), symbols: symbols.clone(),
        candle_limit, top_k, fee_rate, swap_rate_per_bar: swap_rate, long_short, wf_window,
        rebalance_every, exit_buffer, leverage,
        bars_per_year: bars_per_year(&interval), sr_tilt, sr_band_pct, sr_min_strength, sr_window,
        ..Default::default() // lookback/momentum: aile döngüsünde override edilir
    };

    // Aile: her lookback × {momentum, reversal}. Šidák test-başı eşik = 1−(1−α)^(1/N).
    let mut configs: Vec<(usize, bool)> = Vec::new();
    for &lb in &lookbacks {
        configs.push((lb, true));  // momentum
        configs.push((lb, false)); // reversal
    }
    let n_fam = configs.len() as f64;
    let sidak = 1.0 - (1.0 - family_alpha).powf(1.0 / n_fam);

    // WALK-FORWARD MODU (XS_WF=1): tüm-veride seçim YOK — her IS penceresinde aday ızgaradan en iyiyi
    // seç, GÖRMEDİĞİ OOS'a uygula, OOS'ları birleştir → look-ahead'siz dürüst test.
    if std::env::var("XS_WF").map(|v| v == "1").unwrap_or(false) {
        run_wf_mode(&base, &configs, &interval);
        return;
    }

    println!("📐 Kesitsel relatif-güç · market={market} · interval={interval} · sepet={} sembol",
        symbols.len());
    println!("   top_k={top_k} · fee={:.4}/turnover · swap={:.4}/bar · {} · rebalance={rebalance_every} · band={exit_buffer} · lev={leverage} · aile={} · Šidák p≤{:.4}",
        fee_rate, swap_rate, if long_short { "LONG-SHORT" } else { "LONG-ONLY" }, configs.len(), sidak);
    if sr_tilt > 0.0 {
        println!("   🧱 S/R EĞİMİ AÇIK: tilt={sr_tilt} · band={sr_band_pct}% · min_güç={sr_min_strength} · pencere={sr_window} bar");
    }
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

/// Walk-forward OOS: aday ızgaradan IS'te seç, kör OOS'a uygula, birleştirilmiş OOS'ta anlamlılık.
fn run_wf_mode(base: &XsConfig, candidates: &[(usize, bool)], interval: &str) {
    // IS/OOS pencere uzunlukları (bar). 1d için ~2 yıl IS / ~yarım yıl OOS makul.
    let is_bars: usize = std::env::var("XS_WF_IS").ok().and_then(|s| s.parse().ok())
        .unwrap_or(if interval == "1d" { 730 } else { 2000 });
    let oos_bars: usize = std::env::var("XS_WF_OOS").ok().and_then(|s| s.parse().ok())
        .unwrap_or(if interval == "1d" { 180 } else { 500 });

    let wf = XsWfConfig { is_bars, oos_bars, candidates: candidates.to_vec() };
    let r = run_xs_walkforward(base, &wf);

    println!("🔁 WALK-FORWARD OOS · IS={is_bars} bar / OOS={oos_bars} bar (örtüşmesiz) · aday={} config",
        candidates.len());
    println!("   Her pencerede IS-Sharpe en iyisi seçilir → GÖRMEDİĞİ OOS'a uygulanır → OOS'lar birleşir.");
    println!();
    if r.windows == 0 {
        println!("⚠️  Yeterli veri yok (IS+OOS toplamı seriden uzun). XS_WF_IS / XS_WF_OOS küçült.");
        return;
    }
    // Pencere bazında seçim + IS→OOS tutarlılık (overfit teşhisi: IS iyi ama OOS kötü mü?).
    println!("   {:>4} | {:>10} | {:>9} | {:>10}", "pen", "seçim", "IS-Sharpe", "OOS-ort%");
    println!("   {}", "-".repeat(44));
    for (i, ((lb, mom), (is_sh, oos_m))) in r.selections.iter().zip(&r.is_oos_pairs).enumerate() {
        println!("   {:>4} | {:>3} {:<6} | {:>9.2} | {:>10.3}",
            i + 1, lb, if *mom { "mom" } else { "rev" }, is_sh, 100.0 * oos_m);
    }
    // Seçim kararlılığı: momentum ne sıklıkta seçildi?
    let mom_share = r.selections.iter().filter(|(_, m)| *m).count() as f64 / r.windows as f64;

    println!();
    let o = &r.oos;
    let tp = o.t_pvalue();
    let np = o.nw_t_pvalue();
    let bp = o.wf.window_significance();
    println!("══════ BİRLEŞTİRİLMİŞ OOS (look-ahead'siz) ══════");
    println!("   pencere={} · OOS bar={} · momentum-seçim oranı={:.0}%", r.windows, o.bars, 100.0 * mom_share);
    println!("   annRet={:.1}% · Sharpe={:.2} · win%={:.0} · turnover_ort={:.2}",
        100.0 * o.ann_return, o.ann_sharpe, 100.0 * o.win_rate, o.avg_turnover);
    println!("   naif t={:.2} (p={:.4}) · NEWEY-WEST t={:.2} (p={:.4}, lag={}) · binom {}/{} (p={:.4})",
        o.t_stat, tp, o.nw_t_stat, np, o.nw_lag, o.wf.profitable_windows, o.wf.windows, bp);
    println!();
    // DÜRÜST verdikt: otokorelasyona-dayanıklı Newey-West p-değerine dayanır (naif değil).
    if o.nw_t_stat > 0.0 && np <= 0.05 {
        println!("✅ OOS ANLAMLI (Newey-West p={:.4}≤0.05): edge otokorelasyon düzeltmesinden SONRA da tutuyor.", np);
    } else if o.nw_t_stat > 0.0 {
        println!("~ OOS POZİTİF ama NW p={:.4}>0.05: naif t (p={:.4}) otokorelasyonla şişmiş; dürüst güç sınırda.", np, tp);
    } else {
        println!("✗ OOS NEGATİF/sıfır: edge kör veride tutmadı → overfit/regime-bağımlı. Canlıya BAĞLAMA.");
    }
}
