// interval_scan — terminalden çalıştırılabilir interval-tarama aracı.
//
// Amaç: bir sembol için DB'deki her interval'i AYNI gerçekçi koşullarda (market-saf
// veri + edge filtresi 0.20 + canlı-temsili trailing/breakeven) backtest edip
// "hangi interval(ler) işlem yapılabilir edge taşıyor" sorusunu sayıyla yanıtlamak.
// Her interval için önce VERİ SAĞLIĞI (satır, gap%, bayatlık) sonra strateji havuzu
// taranıp EN İYİ stratejinin PF/beklenti/işlem/sharpe'ı raporlanır.
//
// Kullanım:
//   cargo run --release --example interval_scan -- BTCUSDT [market] [intervals] [limit]
// Örnek:
//   cargo run --release --example interval_scan -- BTCUSDT spot 5m,15m,30m,1h,4h,1d 5000
//   cargo run --release --example interval_scan -- ETHUSDT futures
//
// DB yolu: DB_PATH env (default data/trader.db).
//
// Notlar:
//  • read_candles_market ile market-SAF okur (candles tablosunun spot+futures
//    karışımından kaçınır — read_candles market'i yok sayar).
//  • Tarama tek-TF'tir (use_htf=false) → her interval'in KENDİ edge'ini izole eder.
//  • HOLDOUT: ilk %70'te TP/SL/PS optimize edilir, son %30'da (OOS) ÖLÇÜLÜR →
//    in-sample overfit'i (aynı veride en iyi param) eler, dürüst PF verir.
//  • PF mutlak değil, interval'ler arası karşılaştırma + veri-sağlık filtresi içindir.

use memos_trading_core::core::types::Candle;
use memos_trading_core::persistence::reader::read_candles_market;
use memos_trading_core::robot::backtester::{Backtester, BacktestConfig, ParameterOptimizer};
use memos_trading_core::robot::data_pipeline::CandleHealth;
use memos_trading_core::robot::parameters::window_noise_floor_pct;
use memos_trading_core::robot::strategies::default_registry;

fn interval_secs(iv: &str) -> Option<i64> {
    Some(match iv {
        "1m" => 60, "3m" => 180, "5m" => 300, "15m" => 900, "30m" => 1800,
        "1h" => 3600, "2h" => 7200, "4h" => 14400, "6h" => 21600, "12h" => 43200,
        "1d" => 86400, _ => return None,
    })
}

struct IntervalResult {
    interval: String,
    gap_pct: f64,
    stale_days: f64,
    best_strategy: String,
    trades: usize,
    win_rate: f64,
    profit_factor: f64,
    expectancy: f64,
    sharpe: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Kullanım: interval_scan <SYMBOL> [market=spot] [intervals=5m,15m,30m,1h,4h,1d] [limit=5000]");
        std::process::exit(1);
    }
    let symbol = args[1].to_uppercase();
    let market = args.get(2).map(|s| s.as_str()).unwrap_or("spot");
    let intervals: Vec<String> = args.get(3).map(|s| s.as_str())
        .unwrap_or("5m,15m,30m,1h,4h,1d")
        .split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let limit: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(5000);
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".to_string());

    const EDGE_MIN: f64 = 0.20;
    const BREAKEVEN_RR: f64 = 1.0;
    const MIN_CANDLES: usize = 120;   // altında backtest anlamsız
    const MIN_TRADES: usize = 10;     // PF güvenilirliği için minimum işlem
    let capital = 10_000.0;

    println!("\n🔭 interval_scan · sembol={symbol} · market={market} · db={db_path} · limit={limit}");
    println!("   edge≥{EDGE_MIN} · breakeven@RR {BREAKEVEN_RR} · tek-TF · holdout %70 IS / %30 OOS · canlı-temsili trailing\n");

    let pool = default_registry().canonical_pool();
    let mut results: Vec<IntervalResult> = Vec::new();

    for iv in &intervals {
        if interval_secs(iv).is_none() {
            println!("• {iv:<4} → bilinmeyen interval, atlandı");
            continue;
        }
        let candles: Vec<Candle> = match read_candles_market(&db_path, &symbol, iv, market, limit) {
            Ok(c) => c,
            Err(e) => { println!("• {iv:<4} → okuma hatası: {e}"); continue; }
        };
        let n = candles.len();
        if n < 2 {
            println!("• {iv:<4} → veri yok ({n} mum)");
            continue;
        }
        // Veri sağlığı: runtime kapısıyla TEK KAYNAK (CandleHealth::from_candles).
        let health = CandleHealth::from_candles(&candles, iv);
        let gap_pct = health.gap_pct;
        let stale_days = health.stale_secs as f64 / 86_400.0;

        if n < MIN_CANDLES {
            println!("• {iv:<4} → n={n} gap={gap_pct:.0}% bayat={stale_days:.1}g · yetersiz veri (<{MIN_CANDLES}), tarama atlandı");
            continue;
        }

        // Canlı-temsili trailing mult: target(0.7) / pencere_noise_floor, clamp[1.5,30].
        let trail_mult = match window_noise_floor_pct(&candles) {
            Some(nf) if nf > 0.0 => (0.7 / nf).clamp(1.5, 30.0),
            _ => 2.0,
        };

        // HOLDOUT: ilk %70 IS (TP/SL/PS optimizasyonu), son %30 OOS (dürüst ölçüm).
        // In-sample tek başına overfit (aynı veride en iyi param) → iyimser PF.
        let split = (n * 70) / 100;
        let (is_slice, oos_slice) = candles.split_at(split);
        if oos_slice.len() < 40 {
            println!("• {iv:<4} → n={n} gap={gap_pct:.0}% bayat={stale_days:.1}g · OOS dilimi çok kısa ({}), atlandı", oos_slice.len());
            continue;
        }

        // Strateji havuzu: IS'te optimize et, OOS'ta ölç. En iyi = OOS PF.
        let mut best: Option<IntervalResult> = None;
        for strat in &pool {
            let opt = ParameterOptimizer::new(symbol.clone(), iv.clone(), capital, strat.clone())
                .with_edge_min_score(Some(EDGE_MIN))
                .with_exit_model(Some(trail_mult), Some(BREAKEVEN_RR));
            let Ok(res) = opt.optimize_parallel(is_slice, (2.0, 6.0, 2.0), (1.0, 3.0, 1.0), (0.2, 0.4, 0.1)) else {
                continue;
            };
            // OOS değerlendirme: IS'te bulunan en iyi param ile son %30'u koş.
            let p = &res.best_parameters;
            let oos_cfg = BacktestConfig {
                symbol: symbol.clone(),
                interval: iv.clone(),
                initial_balance: capital,
                max_position_size: p.max_position_size,
                take_profit_pct: p.take_profit_pct,
                stop_loss_pct: p.stop_loss_pct,
                strategy_name: strat.clone(),
                commission_pct: 0.001,
                edge_min_score: Some(EDGE_MIN),
                atr_trail_mult: Some(trail_mult),
                breakeven_at_rr: Some(BREAKEVEN_RR),
                ..Default::default()
            };
            let Ok(r) = Backtester::new(oos_cfg).run(oos_slice) else { continue; };
            let expectancy = if r.total_trades > 0 { r.total_pnl / r.total_trades as f64 } else { 0.0 };
            let cand = IntervalResult {
                interval: iv.clone(), gap_pct, stale_days,
                best_strategy: strat.clone(),
                trades: r.total_trades,
                win_rate: r.win_rate,
                profit_factor: r.profit_factor,
                expectancy,
                sharpe: r.sharpe_ratio,
            };
            // En iyi = yeterli işlemli + en yüksek PF. (Az-işlemli yüksek-PF flukelerini ele.)
            let better = match &best {
                None => true,
                Some(b) => {
                    let cand_ok = cand.trades >= MIN_TRADES;
                    let b_ok = b.trades >= MIN_TRADES;
                    match (cand_ok, b_ok) {
                        (true, false) => true,
                        (false, true) => false,
                        _ => cand.profit_factor > b.profit_factor,
                    }
                }
            };
            if better { best = Some(cand); }
        }

        match best {
            Some(b) => {
                println!("• {iv:<4} → n={n} gap={gap_pct:.0}% bayat={stale_days:.1}g | en iyi: {:<14} işlem={} win={:.0}% PF={:.2} beklenti={:+.2} Sh={:+.2}",
                    b.best_strategy, b.trades, b.win_rate, b.profit_factor, b.expectancy, b.sharpe);
                results.push(b);
            }
            None => println!("• {iv:<4} → n={n} gap={gap_pct:.0}% · hiçbir strateji sonuç vermedi"),
        }
    }

    // ─── Sıralama + öneri ────────────────────────────────────────────────────
    println!("\n══════ SIRALAMA (yeterli-işlemli, PF'e göre) ══════");
    let mut ranked: Vec<&IntervalResult> = results.iter().filter(|r| r.trades >= MIN_TRADES).collect();
    ranked.sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap_or(std::cmp::Ordering::Equal));
    if ranked.is_empty() {
        println!("  Yeterli işlemli (≥{MIN_TRADES}) interval yok — veri kıt/gappy ya da sinyal üretmiyor.");
    } else {
        for (i, r) in ranked.iter().enumerate() {
            let verdict = if r.profit_factor >= 1.0 { "✅ kârlı" } else if r.profit_factor >= 0.9 { "≈ breakeven" } else { "❌ kaybeden" };
            let health = if r.gap_pct > 50.0 || r.stale_days > 7.0 { "⚠️ veri-şüpheli" } else { "veri-sağlam" };
            println!("  {}. {:<4} PF={:.2} {verdict} | {} (gap={:.0}% bayat={:.1}g) | {} işlem, win {:.0}%",
                i + 1, r.interval, r.profit_factor, health, r.gap_pct, r.stale_days, r.trades, r.win_rate);
        }
        let top = ranked[0];
        println!("\n→ ÖNERİ: en yüksek PF '{}' ({:.2}). PF<1.0 ise hiçbiri net kârlı değil; veri-şüpheli (gap>50% / bayat>7g) interval'lere güvenme.", top.interval, top.profit_factor);
    }
    println!();
}
