// param_optimize — strateji/indikatör/osilatör paramlarının (RSI period/overbought,
// MACD fast/slow, BB period/std_dev…) sembol × TF sepeti üzerinde optimum DEĞERLERİNİ
// tespit eden DB-tabanlı tarayıcı.
//
// Fark (edge_scan'den): edge_scan SADECE çıkış/boyut ızgarasını (TP/SL/PS) + havuz
// seçimini optimize eder, indikatör paramlarını default'ta bırakır. Bu araç tersini
// yapar: HyperOpt::spec_search ile her stratejinin KENDİ param_spec uzayından örnekleyip
// "bu seride bu strateji için en iyi periyot/eşik nedir" sorusunu yanıtlar.
//
// DÜRÜSTLÜK: in-sample tek-örneklem optimizasyon overfit/fluke üretir ([[project_edge_scan]]
// dersi). Bu yüzden holdout: ilk %IS_PCT'te optimize, kalan %(100-IS_PCT)'te ölç. Rapor
// hem IS hem OOS skorunu gösterir → IS-iyi/OOS-kötü satır overfit'tir. `robust` bayrağı
// yalnız OOS'ta da (skor>0, pnl>0, yeterli işlem) tutan paramları işaretler.
//
// REPORT-ONLY: ParameterStore'a hiçbir şey YAZMAZ — kararı operatör verir.
//
// Kullanım:
//   cargo run --release --example param_optimize -- [market] [intervals] [symbols] [n] [limit]
// Örnekler:
//   cargo run --release --example param_optimize -- futures 4h BTCUSDT,ETHUSDT
//   cargo run --release --example param_optimize -- futures 15m,1h,4h,1d BTCUSDT,ETHUSDT,... 300 6000
//   cargo run --release --example param_optimize -- futures all BTCUSDT 200   # all → 15m,1h,4h,1d
//
// Konum argümanları: market, intervals(csv|all), symbols(csv·zorunlu), n_örnek, candle_limit.
// Env: DB_PATH (default data/trader.db), PARAM_OPT_OUT (rapor JSON yolu),
//      IS_PCT (holdout IS yüzdesi, default 70), MIN_ROWS (default 400), MAX_GAP_PCT (default 50),
//      MIN_OOS_TRADES (robust için, default 5), SEED (spec_search determinizmi, default 12345).

use memos_trading_core::robot::backtester::{
    BacktestConfig, BacktestResult, Backtester, wf_cross_check, wf_oos_windows, WfCrossCheck,
};
use memos_trading_core::robot::data_pipeline::health::CandleHealth;
use memos_trading_core::robot::ml_engine::hyperopt::HyperOpt;
use memos_trading_core::robot::strategies::default_registry;
use memos_trading_core::robot::strategies::param_spec::{ParamKind, ParamSpec};
use memos_trading_core::core::types::{Candle, StrategyParams};
use serde::Serialize;

const DEFAULT_INTERVALS: &[&str] = &["15m", "1h", "4h", "1d"];

fn csv(arg: Option<&String>) -> Vec<String> {
    arg.map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}
fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// HyperOpt'un (private) composite skoruyla AYNI formül → IS↔OOS karşılaştırılabilir.
fn composite(r: &BacktestResult) -> f64 {
    if r.total_trades < 3 { return f64::NEG_INFINITY; }
    let pf_norm = if r.profit_factor > 1.0 { r.profit_factor.ln() + 1.0 } else { r.profit_factor.max(0.0) };
    r.sharpe_ratio * 0.35 + pf_norm * 0.25 + (r.win_rate / 100.0) * 0.25
        - (r.max_drawdown_pct / 100.0).min(1.0) * 0.15
}

/// param_spec'in tüm fields'ı default TP/SL/trail ile (spec_search bunu klonlayıp
/// strategy_params'ı doldurur; OOS yeniden-koşumda da aynı çıkış modeli kullanılır).
fn base_cfg(symbol: &str, interval: &str, strategy: &str) -> BacktestConfig {
    BacktestConfig {
        symbol: symbol.into(),
        interval: interval.into(),
        initial_balance: 10_000.0,
        max_position_size: 0.3,
        take_profit_pct: 4.0,
        stop_loss_pct: 2.0,
        strategy_name: strategy.into(),
        strategy_params: None,
        commission_pct: 0.001,
        breakeven_at_rr: Some(1.0),
        atr_trail_mult: Some(2.0),
        partial_tp_ratio: None,
        position_profile: None,
        security_profile: None,
        use_htf: false,
        edge_min_score: None,
        orderbook_sim: None,
        regime_gate: Default::default(),
        direction: Default::default(),
        regime_style_fit: false,
        atr_sl_mult: None,
        atr_tp_mult: None,
        vol_target_pct: None,
    }
}

fn fmt_param(spec: &ParamSpec, v: f64) -> String {
    match spec.kind {
        ParamKind::Int => format!("{}={}", spec.name, v.round() as i64),
        ParamKind::Pct => format!("{}={:.1}%", spec.name, v),
        ParamKind::Float => format!("{}={:.2}", spec.name, v),
    }
}

#[derive(Serialize, Clone)]
struct ParamVal { name: String, value: f64, kind: String }

/// WF çapraz-kontrol ayarları (edge_scan disipliniyle aynı; env-ayarlanabilir).
/// Pencere boyutları BAR sayısıdır; eşikler `wf_robust` kapısını belirler.
#[derive(Clone, Copy)]
struct WfParams {
    is_bars: usize,
    oos_bars: usize,
    step: usize,
    min_windows: usize,
    min_consistency: f64,
    max_pvalue: f64,
}

#[derive(Serialize)]
struct OptRow {
    symbol: String,
    interval: String,
    strategy: String,
    params: Vec<ParamVal>,
    params_fmt: String,
    is_score: f64,
    is_pnl_pct: f64,
    is_win_rate: f64,
    oos_score: f64,
    oos_pnl_pct: f64,
    oos_win_rate: f64,
    oos_sharpe: f64,
    oos_pf: f64,
    oos_trades: usize,
    robust: bool,
    // ── WF çoklu-pencere çapraz-kontrol (IS-en-iyi paramlar TÜM seride rolling OOS) ──
    wf_windows: usize,
    wf_profitable_windows: usize,
    wf_pooled_pf: f64,
    wf_consistency: f64,
    wf_pvalue: f64,
    /// WF-ROBUST: pooled PF≥1 + pencere≥min + tutarlılık≥eşik + p-değeri≤eşik (fluke eler).
    /// `robust` (tek-holdout) güçlü versiyonu; az-işlemli yüksek-OOS flukelerini düşürür.
    wf_robust: bool,
}

/// Sembol başına "kötünün iyisi" — OOS-skoru en yüksek strateji+param+TF.
/// robust=false olsa bile çıkar (amaç en-az-kötüyü belirlemek); etiket güveni gösterir.
#[derive(Serialize)]
struct ChampionRow {
    symbol: String,
    interval: String,
    strategy: String,
    /// Yapısal paramlar (name→value→kind) — uygulamaya entegre için makine-okunur.
    params: Vec<ParamVal>,
    params_fmt: String,
    oos_score: f64,
    oos_pnl_pct: f64,
    oos_win_rate: f64,
    oos_trades: usize,
    robust: bool,
    wf_pooled_pf: f64,
    wf_windows: usize,
    wf_consistency: f64,
    wf_pvalue: f64,
    /// WF-ROBUST: şampiyonun çoklu-pencerede de tuttuğunu söyler — entegrasyon için ASIL güven kapısı.
    wf_robust: bool,
}

#[derive(Serialize)]
struct Report {
    generated_at: String,
    db_path: String,
    market: String,
    n_samples: usize,
    is_pct: usize,
    series_scanned: usize,
    series_skipped: usize,
    /// Sembol başına şampiyon (kötünün iyisi) — operasyonel "bu sembolde bunu koş" listesi.
    champions: Vec<ChampionRow>,
    rows: Vec<OptRow>,
}

/// Sembol başına şampiyon: ÖNCE WF-robust (entegrasyona güvenli), eşitse OOS-skoru en yüksek.
/// Böylece bir sembolde WF-stabil bir strateji varsa, yüksek-OOS ama WF-kırılgan olana yeğlenir
/// ("kötünün iyisi" → "kötünün STABİL iyisi"). Semboller alfabetik. Boş → boş.
fn pick_champions(rows: &[OptRow]) -> Vec<ChampionRow> {
    use std::collections::BTreeMap;
    let better = |r: &OptRow, c: &OptRow| {
        (r.wf_robust && !c.wf_robust) || (r.wf_robust == c.wf_robust && r.oos_score > c.oos_score)
    };
    let mut best: BTreeMap<&str, &OptRow> = BTreeMap::new();
    for r in rows {
        best.entry(r.symbol.as_str())
            .and_modify(|c| { if better(r, c) { *c = r; } })
            .or_insert(r);
    }
    best.values().map(|r| ChampionRow {
        symbol: r.symbol.clone(),
        interval: r.interval.clone(),
        strategy: r.strategy.clone(),
        params: r.params.clone(),
        params_fmt: r.params_fmt.clone(),
        oos_score: r.oos_score,
        oos_pnl_pct: r.oos_pnl_pct,
        oos_win_rate: r.oos_win_rate,
        oos_trades: r.oos_trades,
        robust: r.robust,
        wf_pooled_pf: r.wf_pooled_pf,
        wf_windows: r.wf_windows,
        wf_consistency: r.wf_consistency,
        wf_pvalue: r.wf_pvalue,
        wf_robust: r.wf_robust,
    }).collect()
}

#[allow(clippy::too_many_arguments)]
fn optimize_series(
    symbol: &str,
    interval: &str,
    candles: &[Candle],
    n: usize,
    is_pct: usize,
    seed: u64,
    min_oos_trades: usize,
    wf: &WfParams,
    out: &mut Vec<OptRow>,
) {
    // Holdout: ilk %is_pct optimize (IS), kalan OOS.
    let is_len = (candles.len() * is_pct / 100).max(1);
    if is_len >= candles.len() { return; }
    let (is_slice, oos_slice) = candles.split_at(is_len);

    for strat in default_registry().canonical_pool() {
        let specs = default_registry().make(&strat).param_spec();
        if specs.is_empty() { continue; } // yapısal paramı yok → optimize edilecek bir şey yok

        let cfg = base_cfg(symbol, interval, &strat);
        let Some(res) = HyperOpt::spec_search(is_slice, &specs, n, &cfg, Some(seed)) else { continue; };
        let best = res.best_params;

        // OOS: IS-en-iyi paramları kalan veride dürüstçe ölç.
        let mut oos_cfg = base_cfg(symbol, interval, &strat);
        oos_cfg.strategy_params = Some(best);
        let Ok(oos) = Backtester::new(oos_cfg).run(oos_slice) else { continue; };
        let oos_score = composite(&oos);

        let params: Vec<ParamVal> = specs.iter().filter_map(|s| {
            StrategyParams::get(&best, s.name).map(|v| ParamVal {
                name: s.name.to_string(),
                value: v,
                kind: format!("{:?}", s.kind),
            })
        }).collect();
        let params_fmt = specs.iter()
            .filter_map(|s| StrategyParams::get(&best, s.name).map(|v| fmt_param(s, v)))
            .collect::<Vec<_>>()
            .join(" ");

        let robust = oos_score.is_finite() && oos_score > 0.0
            && oos.total_pnl_pct > 0.0 && oos.total_trades >= min_oos_trades;

        // WF çapraz-kontrol: IS-en-iyi paramları TÜM seride rolling OOS pencerelerinde dene
        // (edge_scan ile AYNI disiplin: wf_oos_windows + wf_cross_check). Tek-holdout flukesini
        // pooled PF + pencere-tutarlılığı + binom p-değeri ile eler.
        let mut wf_cfg = base_cfg(symbol, interval, &strat);
        wf_cfg.strategy_params = Some(best);
        let windows = wf_oos_windows(candles.len(), wf.is_bars, wf.oos_bars, wf.step);
        let cc: WfCrossCheck = wf_cross_check(&wf_cfg, candles, &windows);
        let wf_robust = cc.pooled_pf >= 1.0
            && cc.windows >= wf.min_windows
            && cc.consistency() >= wf.min_consistency
            && cc.window_significance() <= wf.max_pvalue;

        out.push(OptRow {
            symbol: symbol.into(),
            interval: interval.into(),
            strategy: strat.clone(),
            params,
            params_fmt,
            is_score: res.best_score,
            is_pnl_pct: res.best_pnl_pct,
            is_win_rate: res.best_win_rate,
            oos_score,
            oos_pnl_pct: oos.total_pnl_pct,
            oos_win_rate: oos.win_rate,
            oos_sharpe: oos.sharpe_ratio,
            oos_pf: oos.profit_factor,
            oos_trades: oos.total_trades,
            robust,
            wf_windows: cc.windows,
            wf_profitable_windows: cc.profitable_windows,
            wf_pooled_pf: cc.pooled_pf,
            wf_consistency: cc.consistency(),
            wf_pvalue: cc.window_significance(),
            wf_robust,
        });
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market = args.get(1).map(|s| s.as_str()).unwrap_or("futures").to_string();
    let intervals: Vec<String> = match args.get(2).map(|s| s.as_str()) {
        None | Some("all") | Some("") => DEFAULT_INTERVALS.iter().map(|s| s.to_string()).collect(),
        Some(_) => csv(args.get(2)),
    };
    let symbols = csv(args.get(3));
    let n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(200);
    let limit: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(5000);

    if symbols.is_empty() {
        eprintln!("⚠️  Sembol listesi ver: param_optimize -- futures 4h BTCUSDT,ETHUSDT [n] [limit]");
        std::process::exit(2);
    }

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let is_pct = env_usize("IS_PCT", 70).clamp(10, 95);
    let min_rows = env_usize("MIN_ROWS", 400);
    let max_gap_pct = env_f64("MAX_GAP_PCT", 50.0);
    let min_oos_trades = env_usize("MIN_OOS_TRADES", 5);
    let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(12345u64);
    // WF çapraz-kontrol (edge_scan defaultları; env-ayarlanabilir). Pencere boyutları BAR.
    let wf = WfParams {
        is_bars: env_usize("WF_IS", 300),
        oos_bars: env_usize("WF_OOS", 100),
        step: env_usize("WF_STEP", 100),
        min_windows: env_usize("WF_MIN_WINDOWS", 3),
        min_consistency: env_f64("WF_MIN_CONSISTENCY", 0.5),
        max_pvalue: env_f64("WF_MAX_PVALUE", 0.10),
    };
    let out_path = std::env::var("PARAM_OPT_OUT").unwrap_or_else(|_| {
        format!("reports/param_optimize_{}.json", chrono::Utc::now().format("%Y%m%d_%H%M%S"))
    });

    println!("\n🎯 param_optimize · db={db_path} · market={market} · iv={} · {} sembol · n={n} · limit={limit}",
        intervals.join(","), symbols.len());
    println!("   holdout %{is_pct} IS / OOS · spec_search (param_spec uzayı) · min_rows={min_rows} · max_gap={max_gap_pct}% · seed={seed}");
    println!("   WF: is={}/oos={}/step={} bar · min_pencere={} · min_tutarlılık={} · max_p={}",
        wf.is_bars, wf.oos_bars, wf.step, wf.min_windows, wf.min_consistency, wf.max_pvalue);
    println!("   (REPORT-ONLY. robust=tek-holdout OOS; WF✓=çoklu-pencere stabil — entegrasyon için ASIL kapı)\n");

    let mut rows: Vec<OptRow> = Vec::new();
    let mut scanned = 0usize;
    let mut skipped = 0usize;

    for sym in &symbols {
        for iv in &intervals {
            let candles = match memos_trading_core::persistence::reader::read_candles_market(&db_path, sym, iv, &market, limit) {
                Ok(c) => c,
                Err(_) => { skipped += 1; continue; }
            };
            let health = CandleHealth::from_candles(&candles, iv);
            if health.rows < min_rows || health.gap_pct > max_gap_pct {
                eprintln!("  ⏭  {sym:12} {iv:4} · {} bar · gap {:.0}% (sağlık kapısı)", health.rows, health.gap_pct);
                skipped += 1;
                continue;
            }
            eprintln!("  🔎 {sym:12} {iv:4} · {} bar · gap {:.0}% optimize ediliyor…", health.rows, health.gap_pct);
            optimize_series(sym, iv, &candles, n, is_pct, seed, min_oos_trades, &wf, &mut rows);
            scanned += 1;
        }
    }

    // OOS skoruna göre sırala (en sağlam edge üstte).
    rows.sort_by(|a, b| b.oos_score.partial_cmp(&a.oos_score).unwrap_or(std::cmp::Ordering::Equal));

    let robust_count = rows.iter().filter(|r| r.robust).count();
    let wf_robust_count = rows.iter().filter(|r| r.wf_robust).count();
    let champions = pick_champions(&rows);

    println!("\n══════ SONUÇ ({} seri · {} atlandı · {} satır · {} OOS-robust · {} WF-ROBUST) ══════",
        scanned, skipped, rows.len(), robust_count, wf_robust_count);
    if rows.is_empty() {
        println!("  Optimize edilebilir seri yok — filtre çok dar, veri yetersiz/gappy ya da sinyal üretmedi.");
    } else {
        println!("  {:<10} {:<5} {:<14} {:>7} {:>7} {:>6} {:>6} {:>5} {:>4} {:>4}  en iyi paramlar",
            "symbol", "iv", "strateji", "OOS_skr", "wfPF", "wfP", "wf#", "işl", "R", "WF");
        for r in rows.iter().take(50) {
            let flag = if r.robust { "✅" } else { "—" };
            let wff = if r.wf_robust { "✅" } else { "—" };
            println!("  {:<10} {:<5} {:<14} {:>7.2} {:>7.2} {:>5.2} {:>6} {:>5} {:>4} {:>4}  {}",
                r.symbol, r.interval, r.strategy, r.oos_score,
                r.wf_pooled_pf, r.wf_pvalue, r.wf_windows, r.oos_trades, flag, wff, r.params_fmt);
        }
        if rows.len() > 50 { println!("  … ({} satır daha JSON'da)", rows.len() - 50); }
    }

    // ─── Şampiyonlar: sembol başına kötünün iyisi (operasyonel seçim) ─────────
    if !champions.is_empty() {
        let champ_wf = champions.iter().filter(|c| c.wf_robust).count();
        println!("\n══════ ŞAMPİYONLAR (sembol başına kötünün iyisi · {}/{} WF-ROBUST) ══════",
            champ_wf, champions.len());
        println!("  {:<10} {:<5} {:<14} {:>7} {:>7} {:>6} {:>6} {:>5} {:>4}  en iyi paramlar",
            "symbol", "iv", "strateji", "OOS_skr", "wfPF", "wfP", "wf#", "işl", "WF");
        for c in &champions {
            let wff = if c.wf_robust { "✅" } else { "—" };
            println!("  {:<10} {:<5} {:<14} {:>7.2} {:>7.2} {:>5.2} {:>6} {:>5} {:>4}  {}",
                c.symbol, c.interval, c.strategy, c.oos_score,
                c.wf_pooled_pf, c.wf_pvalue, c.wf_windows, c.oos_trades, wff, c.params_fmt);
        }
        println!("  (WF✓ = çoklu-pencere stabil → entegrasyona güvenli aday. WF— = sembolün en-az-kötüsü ama stabil değil.)");
    }

    let report = Report {
        generated_at: chrono::Utc::now().to_rfc3339(),
        db_path: db_path.clone(),
        market,
        n_samples: n,
        is_pct,
        series_scanned: scanned,
        series_skipped: skipped,
        champions,
        rows,
    };
    if let Some(parent) = std::path::Path::new(&out_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
        .and_then(|j| std::fs::write(&out_path, j).map_err(|e| e.to_string()))
    {
        Ok(_) => println!("\n📄 Rapor: {out_path}"),
        Err(e) => eprintln!("\n⚠️ Rapor yazılamadı ({out_path}): {e}"),
    }
    println!("\n→ OOS-robust (tek-holdout): {robust_count} / {}  ·  WF-ROBUST (çoklu-pencere stabil): {wf_robust_count} / {}",
        report.rows.len(), report.rows.len());
    println!("  WF-ROBUST = entegrasyona güvenli aday (fluke elendi). Yine de canlı öncesi slippage ile teyit et ([[project_edge_scan]]).\n");
}
