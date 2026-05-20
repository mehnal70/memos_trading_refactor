// WalkForwardTester entegrasyon testleri.
//
// run_backtest_job artık aday strateji seçimini Walk-Forward OOS metriği
// üzerinden yapıyor; bu paket WF motorunun deterministik veride doğru
// pencere üretimi ve metrik üretimi yaptığını kütüphane sınırı dışından
// doğrular. Yetersiz veri yumuşak başarısızlığa düşer (panik yok).

use memos_trading_core::core::types::Candle;
use memos_trading_core::robot::backtester::{
    WalkForwardConfig, WalkForwardTester,
};
use memos_trading_core::robot::backtester::walk_forward::aggregate_windows_by_regime;

/// Deterministik salınımlı seri: trend stratejilerinin (MA/SUPERTREND)
/// gerçekten işlem üretmesi için yukarı/aşağı dalga oluşturur.
fn synthetic_wave(n: usize) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let phase = (i as f64) * 0.10;
            let close = 100.0 + 10.0 * phase.sin() + (i as f64) * 0.05;
            Candle {
                open: close - 0.3,
                high: close + 0.6,
                low: close - 0.6,
                close,
                volume: 1_000.0,
                ..Default::default()
            }
        })
        .collect()
}

fn cfg(strategy: &str, is: usize, oos: usize, step: usize) -> WalkForwardConfig {
    WalkForwardConfig {
        in_sample_bars: is,
        out_of_sample_bars: oos,
        step_bars: step,
        initial_balance: 10_000.0,
        strategy_name: strategy.into(),
        symbol: "TEST".into(),
        interval: "1h".into(),
        commission_pct: 0.001,
    }
}

#[test]
fn insufficient_candles_returns_none_not_panic() {
    let tester = WalkForwardTester::new(cfg("RSI", 200, 50, 50));
    // 100 mum < IS+OOS = 250 → motor pencere üretmemeli, panic atmamalı.
    let res = tester.run(&synthetic_wave(100));
    assert!(res.is_none(), "yetersiz mumda None bekleniyor");
}

#[test]
fn produces_expected_window_count_on_sufficient_data() {
    let candles = synthetic_wave(500);
    // IS=200, OOS=50 → her pencere 250 mum, step 50.
    // Beklenen pencere sayısı: (500 - 250) / 50 + 1 = 6
    let res = WalkForwardTester::new(cfg("RSI", 200, 50, 50))
        .run(&candles)
        .expect("yeterli veride sonuç bekleniyor");
    assert_eq!(res.windows.len(), 6, "pencere sayısı");
    // Her pencere için in_sample + oos sınırları sıralı ve örtüşmemeli.
    for w in &res.windows {
        assert!(w.in_sample_range.1 == w.oos_range.0,
            "in_sample ile oos birleşik olmalı: {:?}", w);
        assert_eq!(w.in_sample_range.1 - w.in_sample_range.0, 200);
        assert_eq!(w.oos_range.1 - w.oos_range.0, 50);
    }
}

#[test]
fn consistency_score_is_normalized_between_zero_and_one() {
    let res = WalkForwardTester::new(cfg("MA_CROSSOVER", 200, 50, 50))
        .run(&synthetic_wave(500))
        .expect("yeterli veride sonuç bekleniyor");
    assert!(res.consistency_score >= 0.0 && res.consistency_score <= 1.0,
        "consistency_score 0..1 aralığında olmalı: {}", res.consistency_score);
}

#[test]
fn avg_oos_metrics_match_window_sample_mean() {
    let res = WalkForwardTester::new(cfg("RSI", 200, 50, 50))
        .run(&synthetic_wave(500))
        .expect("yeterli veride sonuç bekleniyor");
    let n = res.windows.len() as f64;
    let manual_pnl = res.windows.iter().map(|w| w.oos_metrics.pnl_pct).sum::<f64>() / n;
    let manual_sharpe = res.windows.iter().map(|w| w.oos_metrics.sharpe).sum::<f64>() / n;
    assert!((res.avg_oos_pnl_pct - manual_pnl).abs() < 1e-9,
        "avg_oos_pnl_pct pencere ortalamasıyla aynı olmalı");
    assert!((res.avg_oos_sharpe - manual_sharpe).abs() < 1e-9);
}

// ─────────────────────────────────────────────────────────────────────────────
// Rejim-bazlı agregasyon — run_backtest_job buradan beslenir
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn aggregate_groups_real_wf_windows_by_classify_closure() {
    // Gerçek WF çalıştır, sonra pencereleri "kapanış > giriş mi" testiyle iki
    // sentetik rejime böl. Agregasyon median TP/SL üretmeli ve sample_count'u
    // pencere dağılımına oransal olmalı.
    let candles = synthetic_wave(500);
    let wf = WalkForwardTester::new(cfg("RSI", 200, 50, 50))
        .run(&candles)
        .expect("yeterli veride sonuç");

    let classify = |s: &[Candle]| {
        let first = s.first().map(|c| c.close).unwrap_or(0.0);
        let last  = s.last().map(|c| c.close).unwrap_or(0.0);
        if last >= first { "Up".to_string() } else { "Down".to_string() }
    };
    let agg = aggregate_windows_by_regime(&candles, &wf.windows, classify, 1);
    let total_samples: usize = agg.values().map(|a| a.sample_count).sum();
    assert_eq!(total_samples, wf.windows.len(),
        "agregasyon tüm pencereleri içermeli");
    for (regime, a) in &agg {
        assert!(a.median_tp_pct > 0.0, "{regime} medyan TP > 0 olmalı: {a:?}");
        assert!(a.median_sl_pct > 0.0, "{regime} medyan SL > 0 olmalı: {a:?}");
    }
}

#[test]
fn aggregate_min_samples_filters_noisy_regimes() {
    // 6 pencere üreten WF; eşiği yüksek tut → çıktı ya boş ya tek rejim olur.
    let candles = synthetic_wave(500);
    let wf = WalkForwardTester::new(cfg("RSI", 200, 50, 50))
        .run(&candles)
        .expect("yeterli veride sonuç");
    let agg_high = aggregate_windows_by_regime(
        &candles, &wf.windows, |_| "Single".into(), 100,
    );
    assert!(agg_high.is_empty(),
        "min_samples=100 → hiçbir rejim yazılmamalı, ama: {:?}", agg_high);
    let agg_low = aggregate_windows_by_regime(
        &candles, &wf.windows, |_| "Single".into(), 1,
    );
    let single = agg_low.get("Single").expect("Single rejimi yazılmalı");
    assert_eq!(single.sample_count, wf.windows.len());
}

#[test]
fn config_is_carried_through_into_result() {
    let cfg_in = cfg("MACD", 200, 50, 50);
    let res = WalkForwardTester::new(cfg_in.clone())
        .run(&synthetic_wave(500))
        .expect("sonuç");
    assert_eq!(res.config.strategy_name, "MACD");
    assert_eq!(res.config.in_sample_bars, cfg_in.in_sample_bars);
    assert_eq!(res.config.out_of_sample_bars, cfg_in.out_of_sample_bars);
    assert_eq!(res.config.step_bars, cfg_in.step_bars);
}
