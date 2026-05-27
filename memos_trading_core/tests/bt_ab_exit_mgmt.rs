// A/B backtest: exit yönetimi — sabit TP vs "let winners run" (TP devre dışı, trailing).
// AYNI girişler (aynı strateji/param) iki konfigde de; sadece exit parametreleri değişir
// → saf, kontrollü karşılaştırma. Gerçek DB gerektirir → #[ignore]; elle çalıştır:
//   BT_DB=/abs/yol/trader.db cargo test --test bt_ab_exit_mgmt -- --ignored --nocapture
//
// NOT: Backtester long-only + basitleştirilmiş giriş matrisi kullanır; mutlak sayılar
// canlı motorla birebir eşleşmez. Ama baseline↔variant GÖRELİ kıyas geçerlidir (entries sabit).

use memos_trading_core::robot::backtester::{Backtester, BacktestConfig, BacktestResult};
use memos_trading_core::persistence::reader::read_candles;

fn db_path() -> String {
    std::env::var("BT_DB")
        .unwrap_or_else(|_| "/home/ulas/PyCharmMiscProject/memos_trading_refactor/data/trader.db".into())
}

struct Summary {
    sum_pnl_pct: f64, // Σ trade pnl_pct (qty-bağımsız toplam getiri proxy'si)
    win_rate: f64,
    profit_factor: f64,
    sharpe: f64,
    max_dd: f64,
    n: usize,
    avg_dur_min: f64,
}

fn summarize(r: &BacktestResult) -> Summary {
    let sum_pnl_pct: f64 = r.trades.iter().map(|t| t.pnl_pct).sum();
    let avg_dur_min = if r.trades.is_empty() { 0.0 }
        else { r.trades.iter().map(|t| t.duration_minutes as f64).sum::<f64>() / r.trades.len() as f64 };
    Summary {
        sum_pnl_pct, win_rate: r.win_rate, profit_factor: r.profit_factor,
        sharpe: r.sharpe_ratio, max_dd: r.max_drawdown_pct, n: r.total_trades, avg_dur_min,
    }
}

fn cfg(symbol: &str, interval: &str, tp: f64) -> BacktestConfig {
    BacktestConfig {
        symbol: symbol.into(),
        interval: interval.into(),
        initial_balance: 10_000.0,
        max_position_size: 1.0,       // pnl_pct metriği qty-bağımsız → 1.0 yeterli
        take_profit_pct: tp,          // baseline: 3.0 · variant: 1000.0 (etkin devre dışı)
        stop_loss_pct: 1.5,
        strategy_name: "DEFAULT".into(), // close>SMA20 trend girişi (let-winners-run ile uyumlu)
        strategy_params: None,
        commission_pct: 0.0004,       // her iki konfigte AYNI → kıyas adil
        breakeven_at_rr: Some(1.0),
        atr_trail_mult: Some(2.0),
        partial_tp_ratio: None,
        position_profile: None,
        security_profile: None,
        use_htf: false,
        edge_min_score: None,
    }
}

#[test]
#[ignore]
fn ab_fixed_tp_vs_let_winners_run() {
    let symbols = ["ADAUSDT", "BNBUSDT", "BTCUSDT", "ETHUSDT", "DOGEUSDT"];
    let intervals = ["1h", "1m"];
    let limit = 20_000;

    for interval in intervals {
        println!("\n══════════ interval={} ══════════", interval);
        println!("{:<10} │ {:>22} │ {:>22}", "symbol", "BASELINE(TP3%)", "VARIANT(let-run)");
        println!("{:<10} │ {:>7}{:>8}{:>7} │ {:>7}{:>8}{:>7}", "", "Σ%", "win%", "PF", "Σ%", "win%", "PF");
        let mut agg_b = (0.0, 0.0, 0.0, 0.0, 0usize); // sum_pnl_pct, win, pf, sharpe(avg), n_sym
        let mut agg_v = (0.0, 0.0, 0.0, 0.0, 0usize);
        let mut variant_wins = 0;
        let mut sym_count = 0;

        for sym in symbols {
            let candles = match read_candles(&db_path(), sym, interval, limit) {
                Ok(c) if c.len() > 200 => c,
                _ => { println!("{:<10} │ veri yok/yetersiz", sym); continue; }
            };
            let mut bt_b = Backtester::new(cfg(sym, interval, 3.0));
            let mut bt_v = Backtester::new(cfg(sym, interval, 1000.0));
            let rb = match bt_b.run(&candles) { Ok(r) => r, Err(_) => continue };
            let rv = match bt_v.run(&candles) { Ok(r) => r, Err(_) => continue };
            let b = summarize(&rb);
            let v = summarize(&rv);
            println!(
                "{:<10} │ {:>7.2}{:>8.1}{:>7.2} │ {:>7.2}{:>8.1}{:>7.2}  [n {}→{} · dur {:.0}→{:.0}dk · dd {:.1}→{:.1}]",
                sym, b.sum_pnl_pct, b.win_rate, b.profit_factor,
                v.sum_pnl_pct, v.win_rate, v.profit_factor,
                b.n, v.n, b.avg_dur_min, v.avg_dur_min, b.max_dd, v.max_dd,
            );
            agg_b.0 += b.sum_pnl_pct; agg_b.1 += b.win_rate; agg_b.2 += b.profit_factor; agg_b.3 += b.sharpe; agg_b.4 += 1;
            agg_v.0 += v.sum_pnl_pct; agg_v.1 += v.win_rate; agg_v.2 += v.profit_factor; agg_v.3 += v.sharpe; agg_v.4 += 1;
            if v.sum_pnl_pct > b.sum_pnl_pct { variant_wins += 1; }
            sym_count += 1;
        }
        if sym_count == 0 { println!("(veri yok)"); continue; }
        let nb = agg_b.4 as f64; let nv = agg_v.4 as f64;
        println!("─────────── ORTALAMA ({} sembol) ───────────", sym_count);
        println!("BASELINE: Σ%={:.2}  win%={:.1}  PF={:.2}  sharpe={:.3}",
            agg_b.0, agg_b.1/nb, agg_b.2/nb, agg_b.3/nb);
        println!("VARIANT : Σ%={:.2}  win%={:.1}  PF={:.2}  sharpe={:.3}",
            agg_v.0, agg_v.1/nv, agg_v.2/nv, agg_v.3/nv);
        println!("→ variant Σ% üstün olduğu sembol: {}/{}", variant_wins, sym_count);
    }
}
