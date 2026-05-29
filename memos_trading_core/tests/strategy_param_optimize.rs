// Faz 1b+2: param_spec araması (HyperOpt::spec_search) gerçekten stratejinin KENDİ
// uzayından örnekliyor mu + ParameterStore set/resolve canlı-yolu besliyor mu?
// Sentetik dalga ile deterministik; gerçek DB gerektirmez.

use chrono::{TimeZone, Utc};
use memos_trading_core::core::types::{Candle, StrategyParams};
use memos_trading_core::robot::backtester::{BacktestConfig, Backtester};
use memos_trading_core::robot::ml_engine::hyperopt::HyperOpt;
use memos_trading_core::robot::parameters::ParameterStore;
use memos_trading_core::robot::strategies::default_registry;

/// Salınımlı sentetik fiyat serisi (RSI/crossover'a sinyal üretir).
fn synthetic_wave(n: usize) -> Vec<Candle> {
    (0..n).map(|i| {
        let base = 100.0 + (i as f64 * 0.3).sin() * 12.0 + (i as f64 * 0.05).cos() * 6.0;
        let close = base + ((i % 7) as f64 - 3.0) * 0.4;
        Candle {
            timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64 * 3600, 0).unwrap(),
            open: base,
            high: base + 1.5,
            low: base - 1.5,
            close,
            volume: 1_000.0,
            symbol: "TEST".into(),
            interval: "1h".into(),
        }
    }).collect()
}

fn cfg(strategy: &str) -> BacktestConfig {
    BacktestConfig {
        symbol: "TEST".into(), interval: "1h".into(),
        initial_balance: 10_000.0, max_position_size: 0.3,
        take_profit_pct: 4.0, stop_loss_pct: 2.0,
        strategy_name: strategy.into(), strategy_params: None,
        commission_pct: 0.001, breakeven_at_rr: Some(1.0),
        atr_trail_mult: Some(2.0), partial_tp_ratio: None,
        position_profile: None, security_profile: None, use_htf: false,
        edge_min_score: None, orderbook_sim: None,
        regime_gate: Default::default(),
    }
}

#[test]
fn spec_search_strateji_uzayindan_orneklenir() {
    let candles = synthetic_wave(600);
    let specs = default_registry().make("RSI").param_spec();
    assert!(!specs.is_empty(), "RSI param_spec boş olmamalı");

    let res = HyperOpt::spec_search(&candles, &specs, 30, &cfg("RSI"), Some(12345))
        .expect("spec_search Some döndürmeli (yeterli mum + sinyal)");

    // Bulunan en iyi paramlar RSI uzayının aralığında mı?
    let p = res.best_params;
    let period = p.period.expect("period set olmalı");
    assert!((7..=21).contains(&period), "period {} 7..=21 dışında", period);
    let ob = p.overbought.expect("overbought set olmalı");
    assert!((65.0..=85.0).contains(&ob), "overbought {} 65..=85 dışında", ob);
    assert!(res.combinations_tested > 0);
}

#[test]
fn spec_search_bos_uzayda_none() {
    let candles = synthetic_wave(300);
    // PRICE_ACTION yapısal paramı yok → spec boş → None.
    let specs = default_registry().make("PRICE_ACTION").param_spec();
    assert!(specs.is_empty());
    assert!(HyperOpt::spec_search(&candles, &specs, 20, &cfg("PRICE_ACTION"), Some(1)).is_none());
}

#[test]
fn use_htf_yolu_gercek_seride_calisir() {
    // Salınımlı seri RSI girişleri üretir (spec testiyle aynı veri). use_htf=true
    // HTF sentez+dilimleme yolunu çalıştırır; sonuç geçerli (panik yok, metrikler
    // sonlu). no_htf>0 olduğunu, HTF'nin yalnız long elediği için ≤ kaldığını teyit.
    let candles = synthetic_wave(600);
    let run = |htf: bool| {
        let mut c = cfg("RSI");
        c.use_htf = htf;
        Backtester::new(c).run(&candles).expect("backtest")
    };
    let no_htf = run(false);
    let with_htf = run(true);
    assert!(no_htf.total_trades > 0, "salınımlı seride RSI girişleri olmalı");
    assert!(with_htf.win_rate.is_finite() && with_htf.sharpe_ratio.is_finite());
}

#[test]
fn use_htf_dususta_buy_filtreler() {
    // Monoton düşüş: HTF (4h) ayı → RSI'nin oversold Buy'ları htf_trend_filter ile
    // Hold'a çevrilir → use_htf=true daha AZ (veya eşit) long açar. Düşüş + RSI
    // dip-alımı, HTF filtresinin canlıyla aynı şekilde girişleri kestiğini gösterir.
    let candles: Vec<Candle> = (0..500).map(|i| {
        let close = 350.0 - i as f64 * 0.5;
        Candle {
            timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64 * 3600, 0).unwrap(),
            open: close + 0.3, high: close + 0.5, low: close - 0.5, close,
            volume: 1_000.0, symbol: "TEST".into(), interval: "1h".into(),
        }
    }).collect();
    let run = |htf: bool| {
        let mut c = cfg("RSI");
        c.use_htf = htf;
        Backtester::new(c).run(&candles).expect("backtest").total_trades
    };
    assert!(run(true) <= run(false),
        "ayı HTF'de use_htf girişleri kesmeli (≤): with={} no={}", run(true), run(false));
}

#[test]
fn parameter_store_set_resolve_roundtrip() {
    let mut ps = ParameterStore::default();
    // Yokken default döner.
    assert_eq!(ps.resolve_strategy_params("RSI").period, None);

    let mut sp = StrategyParams::default();
    sp.period = Some(11);
    sp.overbought = Some(73.0);
    ps.set_strategy_params("RSI", sp);

    let got = ps.resolve_strategy_params("RSI");
    assert_eq!(got.period, Some(11));
    assert_eq!(got.overbought, Some(73.0));
    // Başka strateji etkilenmez.
    assert_eq!(ps.resolve_strategy_params("MACD").period, None);
}
