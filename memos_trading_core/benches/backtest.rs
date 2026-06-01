// memos_trading_core/benches/backtest.rs — Faz 3 perf ölçümü (backtest sıcak yolu).
//
// İndikatör bench'i (benches/indicators.rs) tek-fonksiyon mikro-maliyeti ölçtü ve
// indikatörlerin darboğaz OLMADIĞINI gösterdi. Bu bench bir kademe yukarısını ölçer:
// `Backtester::run()` — parametre aramasında (hyperopt/optimizer/WF) sembol×param
// kombinasyonu başına bir kez koşan offline, CPU-bound sıcak yol. Otonom katmanın
// kaç strateji deneyebildiğini doğrudan bu hız kısıtlar.
//
// İki şey ölçülür:
//   1) mum başına maliyet (µs/bar) — mutlak yük.
//   2) ÖLÇEK: n→2n geçişinde sürenin kaç katlandığı. ~2× → O(n) (sağlıklı);
//      ~4× → O(n²) (tam-pencere yeniden-hesabı = optimizasyon hedefi).
//
// Bağımlılıksız (criterion YOK → offline, harness=false).
// Çalıştır: `cargo bench -p memos_trading_core --bench backtest`.

use std::hint::black_box;
use std::time::Instant;

use chrono::{TimeZone, Utc};
use memos_trading_core::core::types::Candle;
use memos_trading_core::robot::backtester::backtest_engine::{
    Backtester, BacktestConfig, DirectionMode, RegimeGate,
};

/// Deterministik salınımlı yükseliş serisi (rastgelelik yok → tekrarlanabilir).
/// MA_CROSSOVER/DEFAULT periyodik Buy üretir → TP/SL döngüsü gerçekçi işlem hacmi
/// yaratır (boş backtest değil). 1h bar, gerçek timestamp (HTF/rejim mantığı için).
fn make_candles(n: usize) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let f = i as f64;
            let close = 100.0 + 0.05 * f + 6.0 * (f * 0.3).sin();
            Candle {
                timestamp: Utc.timestamp_opt(1_700_000_000 + (i as i64) * 3600, 0).unwrap(),
                open: close,
                high: close * 1.004,
                low: close * 0.996,
                close,
                volume: 1_000.0,
                symbol: "BENCH".into(),
                interval: "1h".into(),
            }
        })
        .collect()
}

/// Tipik param-arama config'i. `edge_min` Some → giriş barlarında ek
/// `compute_edge_score_with` (canlı edge hunisi) maliyeti ölçüye girer.
fn cfg(edge_min: Option<f64>) -> BacktestConfig {
    BacktestConfig {
        symbol: "BENCH".into(),
        interval: "1h".into(),
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
        edge_min_score: edge_min,
        orderbook_sim: None,
        regime_gate: RegimeGate::Off,
        direction: DirectionMode::LongOnly,
        atr_sl_mult: None,
        atr_tp_mult: None,
        vol_target_pct: None,
    }
}

/// `run()`'ı `iters` kez koşar, koşu başına ortalama nanosaniyeyi döner.
fn bench_run(candles: &[Candle], edge_min: Option<f64>, iters: u64) -> f64 {
    // Isınma.
    for _ in 0..(iters / 5).max(1) {
        black_box(Backtester::new(cfg(edge_min)).run(black_box(candles)).unwrap());
    }
    let start = Instant::now();
    for _ in 0..iters {
        black_box(Backtester::new(cfg(edge_min)).run(black_box(candles)).unwrap());
    }
    start.elapsed().as_nanos() as f64 / iters as f64
}

fn main() {
    println!("\n=== backtest run() bench (DEFAULT stratejisi, 1h) ===");

    // İter sayısı n ile ters orantılı tutulur (toplam süre ~sabit).
    let sizes: &[(usize, u64)] = &[(500, 400), (1000, 200), (2000, 100), (4000, 50)];

    for &edge in &[None, Some(0.20)] {
        let tag = if edge.is_some() { "edge-filtre AÇIK " } else { "edge-filtre KAPALI" };
        println!("\n[{tag}]   n        ns/run     µs/bar    ölçek(önceki→bu)");
        let mut prev: Option<(usize, f64)> = None;
        for &(n, iters) in sizes {
            let candles = make_candles(n);
            let ns = bench_run(&candles, edge, iters);
            let us_per_bar = ns / 1000.0 / n as f64;
            let scale = match prev {
                Some((pn, pns)) => {
                    let size_ratio = n as f64 / pn as f64;
                    let time_ratio = ns / pns;
                    // norm: süre-oranı / boyut-oranı. ~1 → O(n), ~boyut-oranı → O(n²).
                    format!("{:.2}× (norm {:.2})", time_ratio, time_ratio / size_ratio)
                }
                None => "—".to_string(),
            };
            println!("  {:>6}  {:>12.0}  {:>8.3}    {}", n, ns, us_per_bar, scale);
            prev = Some((n, ns));
        }
    }
    println!("\nyorum: norm≈1 → O(n) sağlıklı; norm≫1 → süper-lineer (tam-pencere");
    println!("yeniden-hesabı). µs/bar büyük + norm>1 → optimizasyon hedefi.\n");
}
