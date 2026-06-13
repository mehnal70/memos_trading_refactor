// A/B backtest: giriş kalitesi (#4) — edge filtresi KAPALI vs AÇIK (canlı edge hunisi).
// AYNI strateji/param/TP/SL; sadece BacktestConfig.edge_min_score değişir →
// kontrollü karşılaştırma. Amaç: edge filtresi 1m'deki aşırı-işlem + komisyon erozyonunu
// kesiyor mu? (işlem sayısı n ↓, Σ% / PF ↑ beklenir). Gerçek DB gerektirir → #[ignore]:
//   BT_DB=/abs/yol/trader.db cargo test --test bt_ab_entry_quality -- --ignored --nocapture
//
// NOT: Backtester long-only; mutlak sayılar canlıyla birebir eşleşmez ama edge_min_score
// dışında her şey sabit olduğundan baseline↔variant GÖRELİ kıyas geçerlidir.

use memos_trading_core::robot::backtester::{Backtester, BacktestConfig, BacktestResult};
use memos_trading_core::persistence::reader::read_candles;

fn db_path() -> String {
    std::env::var("BT_DB")
        .unwrap_or_else(|_| "/home/ulas/PyCharmMiscProject/memos_trading_refactor/data/trader.db".into())
}

struct Summary {
    sum_pnl_pct: f64,
    win_rate: f64,
    profit_factor: f64,
    max_dd: f64,
    n: usize,
}

fn summarize(r: &BacktestResult) -> Summary {
    let sum_pnl_pct: f64 = r.trades.iter().map(|t| t.pnl_pct).sum();
    Summary {
        sum_pnl_pct, win_rate: r.win_rate, profit_factor: r.profit_factor,
        max_dd: r.max_drawdown_pct, n: r.total_trades,
    }
}

/// edge_min_score dışında her şey sabit cfg.
fn cfg(symbol: &str, interval: &str, edge_min: Option<f64>) -> BacktestConfig {
    BacktestConfig {
        symbol: symbol.into(),
        interval: interval.into(),
        initial_balance: 10_000.0,
        max_position_size: 1.0,        // pnl_pct metriği qty-bağımsız
        take_profit_pct: 3.0,
        stop_loss_pct: 1.5,
        strategy_name: "DEFAULT".into(), // close>SMA20 — bol sinyal üretir, filtreye malzeme
        strategy_params: None,
        commission_pct: 0.0004,        // her konfigte AYNI → adil
        breakeven_at_rr: Some(1.0),
        atr_trail_mult: Some(2.0),
        partial_tp_ratio: None,
        position_profile: None,
        security_profile: None,
        use_htf: false,
        edge_min_score: edge_min,      // ← TEK değişken
        orderbook_sim: None,
        regime_gate: Default::default(),
        direction: Default::default(),
        regime_style_fit: false,
        atr_sl_mult: None, atr_tp_mult: None, vol_target_pct: None,
    }
}

fn run_one(symbol: &str, interval: &str, edge_min: Option<f64>, candles: &[memos_trading_core::core::types::Candle]) -> Option<Summary> {
    let mut bt = Backtester::new(cfg(symbol, interval, edge_min));
    bt.run(candles).ok().map(|r| summarize(&r))
}

#[test]
#[ignore]
fn ab_edge_filter_off_vs_on() {
    let symbols = ["ADAUSDT", "BNBUSDT", "BTCUSDT", "ETHUSDT", "DOGEUSDT"];
    let intervals = ["1h", "1m"];
    let limit = 20_000;
    // baseline (filtre yok) vs iki varyant: canlı cold-start 0.20, daha katı 0.35.
    let variants: [(&str, Option<f64>); 3] = [
        ("OFF",      None),
        ("ON 0.20",  Some(0.20)),
        ("ON 0.35",  Some(0.35)),
    ];

    for interval in intervals {
        println!("\n══════════════════ interval={} ══════════════════", interval);
        println!("{:<10} │ {:>3} │ {:>8} {:>7} {:>6} {:>6} {:>6}",
            "symbol", "var", "Σ%", "win%", "PF", "n", "dd");
        // agregat: her varyant için Σ(Σ%), Σ(win), Σ(PF), Σ(n), sembol sayısı
        let mut agg: Vec<(f64, f64, f64, usize, usize)> = vec![(0.0, 0.0, 0.0, 0, 0); variants.len()];

        for sym in symbols {
            let candles = match read_candles(&db_path(), sym, interval, limit) {
                Ok(c) if c.len() > 200 => c,
                _ => { println!("{:<10} │ veri yok/yetersiz", sym); continue; }
            };
            let base_n = run_one(sym, interval, None, &candles).map(|s| s.n).unwrap_or(0);
            for (vi, (label, edge)) in variants.iter().enumerate() {
                let Some(s) = run_one(sym, interval, *edge, &candles) else { continue };
                let trim = if base_n > 0 && vi > 0 {
                    format!(" ({:+.0}% işlem)", (s.n as f64 - base_n as f64) / base_n as f64 * 100.0)
                } else { String::new() };
                println!("{:<10} │ {:<7} │ {:>8.2} {:>7.1} {:>6.2} {:>6} {:>6.1}{}",
                    if vi == 0 { sym } else { "" }, label,
                    s.sum_pnl_pct, s.win_rate, s.profit_factor, s.n, s.max_dd, trim);
                let a = &mut agg[vi];
                a.0 += s.sum_pnl_pct; a.1 += s.win_rate; a.2 += s.profit_factor; a.3 += s.n; a.4 += 1;
            }
        }

        println!("─────────── ORTALAMA ───────────");
        for (vi, (label, _)) in variants.iter().enumerate() {
            let a = agg[vi];
            if a.4 == 0 { continue; }
            let nb = a.4 as f64;
            println!("{:<8} │ ΣΣ%={:>9.2}  win%={:>5.1}  PF={:>5.2}  toplam_n={:>6}",
                label, a.0, a.1 / nb, a.2 / nb, a.3);
        }
    }
}
