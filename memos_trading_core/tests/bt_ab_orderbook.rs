// A/B backtest: orderbook icrası (#c) — KAPALI vs liquid vs illiquid.
// AYNI strateji/param/TP/SL; sadece BacktestConfig.orderbook_sim değişir → giriş/çıkış
// fill'lerine slippage eklenir. Slippage getiriyi aşındırmalı (illiquid en çok). Opt-in
// fidelity aracının canlı paper motoruyla (OrderBookSimulator) aynı slippage'i ürettiğini
// ve param aramasına etkisini gösterir. Gerçek DB → #[ignore]:
//   BT_DB=/abs/yol/trader.db cargo test --test bt_ab_orderbook -- --ignored --nocapture

use memos_trading_core::robot::backtester::{Backtester, BacktestConfig, BacktestResult};
use memos_trading_core::persistence::reader::read_candles;

fn db_path() -> String {
    std::env::var("BT_DB")
        .unwrap_or_else(|_| "/home/ulas/PyCharmMiscProject/memos_trading_refactor/data/trader.db".into())
}

struct Summary { sum_pnl_pct: f64, win_rate: f64, profit_factor: f64, n: usize }

fn summarize(r: &BacktestResult) -> Summary {
    Summary {
        sum_pnl_pct: r.trades.iter().map(|t| t.pnl_pct).sum(),
        win_rate: r.win_rate, profit_factor: r.profit_factor, n: r.total_trades,
    }
}

/// orderbook_sim dışında her şey sabit. qty (max_position_size) slippage'i etkiler.
fn cfg(symbol: &str, interval: &str, ob: Option<&str>) -> BacktestConfig {
    BacktestConfig {
        symbol: symbol.into(),
        interval: interval.into(),
        initial_balance: 10_000.0,
        max_position_size: 1.0,
        take_profit_pct: 3.0,
        stop_loss_pct: 1.5,
        strategy_name: "DEFAULT".into(),
        strategy_params: None,
        commission_pct: 0.0004,
        breakeven_at_rr: Some(1.0),
        atr_trail_mult: Some(2.0),
        partial_tp_ratio: None,
        position_profile: None,
        security_profile: None,
        use_htf: false,
        edge_min_score: None,
        orderbook_sim: ob.map(|s| s.to_string()), // ← TEK değişken
    }
}

fn run_one(symbol: &str, interval: &str, ob: Option<&str>,
           candles: &[memos_trading_core::core::types::Candle]) -> Option<Summary> {
    Backtester::new(cfg(symbol, interval, ob)).run(candles).ok().map(|r| summarize(&r))
}

#[test]
#[ignore]
fn ab_orderbook_off_vs_liquid_vs_illiquid() {
    let symbols = ["ADAUSDT", "BNBUSDT", "BTCUSDT", "ETHUSDT", "DOGEUSDT"];
    let intervals = ["1h", "1m"];
    let limit = 20_000;
    let variants: [(&str, Option<&str>); 3] = [
        ("OFF",      None),
        ("liquid",   Some("liquid")),
        ("illiquid", Some("illiquid")),
    ];

    for interval in intervals {
        println!("\n══════════════════ interval={} ══════════════════", interval);
        println!("{:<10} │ {:>9} │ {:>8} {:>7} {:>6} {:>6}",
            "symbol", "variant", "Σ%", "win%", "PF", "n");
        let mut agg: Vec<(f64, f64, f64, usize)> = vec![(0.0, 0.0, 0.0, 0); variants.len()];

        for sym in symbols {
            let candles = match read_candles(&db_path(), sym, interval, limit) {
                Ok(c) if c.len() > 200 => c,
                _ => { println!("{:<10} │ veri yok/yetersiz", sym); continue; }
            };
            let base = run_one(sym, interval, None, &candles).map(|s| s.sum_pnl_pct).unwrap_or(0.0);
            for (vi, (label, ob)) in variants.iter().enumerate() {
                let Some(s) = run_one(sym, interval, *ob, &candles) else { continue };
                let delta = if vi > 0 { format!("  (Δ{:+.2} slippage)", s.sum_pnl_pct - base) } else { String::new() };
                println!("{:<10} │ {:>9} │ {:>8.2} {:>7.1} {:>6.2} {:>6}{}",
                    if vi == 0 { sym } else { "" }, label, s.sum_pnl_pct, s.win_rate, s.profit_factor, s.n, delta);
                let a = &mut agg[vi];
                a.0 += s.sum_pnl_pct; a.1 += s.win_rate; a.2 += s.profit_factor; a.3 += s.n;
            }
        }
        println!("─────────── ORTALAMA ───────────");
        let nsym = symbols.len() as f64;
        for (vi, (label, _)) in variants.iter().enumerate() {
            let a = agg[vi];
            println!("{:<9} │ ΣΣ%={:>9.2}  win%={:>5.1}  PF={:>5.2}  toplam_n={:>6}",
                label, a.0, a.1 / nsym, a.2 / nsym, a.3);
        }
    }
}
