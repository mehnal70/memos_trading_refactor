// Canlı-yol backtest A/B: gerçek strateji+edge+exit motoruyla ölçüm.
//   #3  → compute_edge_score ters-momentum cezası (0.4 baseline · 0.2 sıkı · 1.0 ceza yok)
//   sizing → notional %10 vs risk-fraksiyon %1
// Gerçek DB gerektirir → #[ignore]. Çalıştır:
//   BT_DB=/abs/yol/trader.db cargo test --test bt_live_path -- --ignored --nocapture
//
// NOT: edge cezası SADECE ters-momentum (mean-reversion) sinyallerini etkiler; trend
// stratejilerinde nadiren devreye girer → her iki strateji ailesinde de ölçülür.

use memos_trading_core::robot::backtester::live_path::{run, LivePathConfig, Sizing};
use memos_trading_core::persistence::reader::read_candles;

fn db_path() -> String {
    std::env::var("BT_DB")
        .unwrap_or_else(|_| "/home/ulas/PyCharmMiscProject/memos_trading_refactor/data/trader.db".into())
}

const SYMBOLS: &[&str] = &["ADAUSDT", "BNBUSDT", "ETHUSDT", "DOGEUSDT"];
const INTERVAL: &str = "1m";
const LIMIT: usize = 6000;

#[derive(Default)]
struct Agg { sum_pct: f64, win: f64, pf: f64, sharpe: f64, trades: usize, k: f64 }
impl Agg {
    fn add(&mut self, r: &memos_trading_core::robot::backtester::live_path::LivePathResult) {
        self.sum_pct += r.sum_trade_pnl_pct; self.win += r.win_rate; self.pf += r.profit_factor;
        self.sharpe += r.sharpe; self.trades += r.total_trades; self.k += 1.0;
    }
    fn line(&self, tag: &str) -> String {
        let k = self.k.max(1.0);
        format!("{:<28} Σ%={:>9.2}  win={:>5.1}  PF={:>5.2}  sharpe={:>6.3}  n={}",
            tag, self.sum_pct, self.win / k, self.pf / k, self.sharpe / k, self.trades)
    }
}

fn load_all() -> Vec<(String, Vec<memos_trading_core::core::types::Candle>)> {
    SYMBOLS.iter().filter_map(|s| {
        match read_candles(&db_path(), s, INTERVAL, LIMIT) {
            Ok(c) if c.len() > 500 => Some((s.to_string(), c)),
            _ => { eprintln!("⚠️ {} veri yok/yetersiz", s); None }
        }
    }).collect()
}

#[test]
#[ignore]
fn ab_edge_reverse_penalty() {
    let data = load_all();
    assert!(!data.is_empty(), "DB'de veri yok (BT_DB?)");
    println!("\n#### #3 EDGE TERS-MOMENTUM CEZASI (1m, {} sembol) ####", data.len());
    for strat in ["RSI", "BB", "MA_CROSSOVER", "SUPERTREND", "AUTO"] {
        println!("── strateji: {} ──", strat);
        for penalty in [0.4_f64, 0.2, 1.0] {
            let mut agg = Agg::default();
            for (_sym, candles) in &data {
                let cfg = LivePathConfig {
                    strategy_name: strat.into(),
                    edge_reverse_penalty: penalty,
                    sizing: Sizing::NotionalPct(0.10),
                    ..Default::default()
                };
                agg.add(&run(candles, &cfg));
            }
            let tag = match penalty {
                0.4 => "ceza=0.4 (BASELINE)",
                0.2 => "ceza=0.2 (daha sıkı)",
                _   => "ceza=1.0 (ceza YOK)",
            };
            println!("   {}", agg.line(tag));
        }
    }
}

#[test]
#[ignore]
fn ab_risk_sizing() {
    let data = load_all();
    assert!(!data.is_empty(), "DB'de veri yok (BT_DB?)");
    println!("\n#### RISK-BAZLI vs NOTIONAL BOYUTLANDIRMA (1m, {} sembol) ####", data.len());
    for strat in ["AUTO", "SUPERTREND", "RSI"] {
        println!("── strateji: {} ──", strat);
        let variants: [(&str, Sizing); 3] = [
            ("notional %10 (BASELINE)", Sizing::NotionalPct(0.10)),
            ("risk-frac %1", Sizing::RiskFraction(0.01)),
            ("risk-frac %0.5", Sizing::RiskFraction(0.005)),
        ];
        for (tag, sizing) in variants {
            let mut agg = Agg::default();
            let mut final_eq = 0.0;
            for (_sym, candles) in &data {
                let cfg = LivePathConfig { strategy_name: strat.into(), sizing, ..Default::default() };
                let r = run(candles, &cfg);
                agg.add(&r);
                final_eq += r.total_pnl_pct;
            }
            println!("   {}  | Σtotal_pnl%={:.2}", agg.line(tag), final_eq);
        }
    }
}
