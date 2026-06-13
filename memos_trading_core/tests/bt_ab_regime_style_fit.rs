// A/B backtest: rejim-STİL uyum kapısı (opt-in D) — KAPALI vs AÇIK.
// AYNI strateji/param/TP/SL; sadece BacktestConfig.regime_style_fit değişir → kontrollü
// karşılaştırma. Hipotez: trend-stratejisini (SUPERTREND/MACD) yatay rejimde, MR'ı
// (RSI/BOLLINGER_BANDS) trend rejiminde açmamak → işlem sayısı n ↓, Σ%/PF ↑ (churn kesilir).
// Gerçek DB gerektirir → #[ignore]:
//   BT_DB=/abs/yol/trader.db cargo test --test bt_ab_regime_style_fit -- --ignored --nocapture
//
// NOT: Backtester long-only; mutlak sayılar canlıyla birebir eşleşmez ama regime_style_fit
// dışında her şey sabit olduğundan baseline↔variant GÖRELİ kıyas geçerlidir. Kapı yalnız
// SINIFLANDIRILMIŞ stratejileri (trend/MR) etkiler → nötr stratejilerde fark beklenmez.

use memos_trading_core::robot::backtester::{Backtester, BacktestConfig, BacktestResult};
use memos_trading_core::persistence::reader::read_candles;

fn db_path() -> String {
    std::env::var("BT_DB")
        .unwrap_or_else(|_| "/home/ulas/PyCharmMiscProject/memos_trading_refactor/data/trader.db".into())
}

struct Summary { sum_pnl_pct: f64, win_rate: f64, profit_factor: f64, max_dd: f64, n: usize }

fn summarize(r: &BacktestResult) -> Summary {
    Summary {
        sum_pnl_pct: r.trades.iter().map(|t| t.pnl_pct).sum(),
        win_rate: r.win_rate, profit_factor: r.profit_factor,
        max_dd: r.max_drawdown_pct, n: r.total_trades,
    }
}

/// regime_style_fit dışında her şey sabit cfg. direction=RegimeDirectional (canlı varsayılanına
/// yakın: short bacağı + ters-trend eleme zaten açık → stil kapısının MARJİNAL katkısı ölçülür).
fn cfg(symbol: &str, interval: &str, strategy: &str, style_fit: bool) -> BacktestConfig {
    use memos_trading_core::robot::backtester::DirectionMode;
    BacktestConfig {
        symbol: symbol.into(),
        interval: interval.into(),
        initial_balance: 10_000.0,
        max_position_size: 1.0,
        take_profit_pct: 3.0,
        stop_loss_pct: 1.5,
        strategy_name: strategy.into(),
        strategy_params: None,
        commission_pct: 0.0004,
        breakeven_at_rr: Some(1.0),
        atr_trail_mult: Some(2.0),
        partial_tp_ratio: None,
        position_profile: None,
        security_profile: None,
        use_htf: false,
        edge_min_score: None,
        orderbook_sim: None,
        regime_gate: Default::default(),
        direction: DirectionMode::RegimeDirectional,
        regime_style_fit: style_fit,   // ← TEK değişken
        atr_sl_mult: None, atr_tp_mult: None, vol_target_pct: None,
    }
}

fn run_one(symbol: &str, interval: &str, strategy: &str, style_fit: bool,
           candles: &[memos_trading_core::core::types::Candle]) -> Option<Summary> {
    let mut bt = Backtester::new(cfg(symbol, interval, strategy, style_fit));
    bt.run(candles).ok().map(|r| summarize(&r))
}

#[test]
#[ignore]
fn ab_regime_style_fit_off_vs_on() {
    let symbols = ["ADAUSDT", "BNBUSDT", "BTCUSDT", "ETHUSDT", "DOGEUSDT", "SOLUSDT", "AVAXUSDT", "LINKUSDT"];
    let intervals = ["15m", "1h"];
    // Trend stratejileri (yatayda bloklanmalı) + MR stratejileri (trendde bloklanmalı).
    let strategies = ["SUPERTREND", "MACD", "RSI", "BOLLINGER_BANDS"];
    let limit = 20_000;

    for interval in intervals {
        for strategy in strategies {
            println!("\n══════════════ interval={} · strateji={} ══════════════", interval, strategy);
            println!("{:<10} │ {:<8} │ {:>8} {:>6} {:>6} {:>6} {:>6}",
                "symbol", "stil-fit", "Σ%", "win%", "PF", "n", "dd");
            // agg: (ΣΣ%, Σwin, ΣPF, Σn, sembol_sayısı) — [OFF, ON]
            let mut agg: [(f64, f64, f64, usize, usize); 2] = [(0.0, 0.0, 0.0, 0, 0); 2];

            for sym in symbols {
                let candles = match read_candles(&db_path(), sym, interval, limit) {
                    Ok(c) if c.len() > 200 => c,
                    _ => continue,
                };
                let base_n = run_one(sym, interval, strategy, false, &candles).map(|s| s.n).unwrap_or(0);
                for (vi, style_fit) in [false, true].into_iter().enumerate() {
                    let Some(s) = run_one(sym, interval, strategy, style_fit, &candles) else { continue };
                    let trim = if base_n > 0 && vi == 1 {
                        format!(" ({:+.0}% işlem)", (s.n as f64 - base_n as f64) / base_n as f64 * 100.0)
                    } else { String::new() };
                    println!("{:<10} │ {:<8} │ {:>8.2} {:>6.1} {:>6.2} {:>6} {:>6.1}{}",
                        if vi == 0 { sym } else { "" }, if style_fit { "ON" } else { "OFF" },
                        s.sum_pnl_pct, s.win_rate, s.profit_factor, s.n, s.max_dd, trim);
                    let a = &mut agg[vi];
                    a.0 += s.sum_pnl_pct; a.1 += s.win_rate; a.2 += s.profit_factor; a.3 += s.n; a.4 += 1;
                }
            }

            println!("─────────── ORTALAMA ({} · {}) ───────────", interval, strategy);
            for (vi, label) in ["OFF", "ON "].into_iter().enumerate() {
                let a = agg[vi];
                if a.4 == 0 { continue; }
                let nb = a.4 as f64;
                println!("stil-fit {} │ ΣΣ%={:>9.2}  win%={:>5.1}  PF={:>5.2}  toplam_n={:>6}",
                    label, a.0, a.1 / nb, a.2 / nb, a.3);
            }
        }
    }
}
