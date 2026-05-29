// walk_forward.rs - Kayan Pencere (Walk-Forward) Analiz Motoru

use serde::{Deserialize, Serialize};
use rayon::prelude::*; // Paralel işleme desteği
use crate::core::types::Candle;
use crate::robot::backtester::{Backtester, BacktestConfig, BacktestResult, DirectionMode};

// --- 1. YAPILANDIRMA VE SONUÇ MODELLERİ ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardConfig {
    pub in_sample_bars: usize,
    pub out_of_sample_bars: usize,
    pub step_bars: usize,
    pub initial_balance: f64,
    pub strategy_name: String,
    pub symbol: String,
    pub interval: String,
    pub commission_pct: f64,
    /// Multi-TF hizalama: BacktestConfig.use_htf'e propagate edilir → WF strateji
    /// seçimi de canlıyla aynı HTF filtresini görür. Default false.
    pub use_htf: bool,
    /// Giriş kalitesi filtresi (#4): BacktestConfig.edge_min_score'a propagate edilir
    /// → WF strateji seçimi de canlının edge hunisini görür. Default None (filtre yok).
    #[serde(default)]
    pub edge_min_score: Option<f64>,
    /// Orderbook icrası (#c): BacktestConfig.orderbook_sim'e propagate. Default None.
    #[serde(default)]
    pub orderbook_sim: Option<String>,
}

impl Default for WalkForwardConfig {
    fn default() -> Self {
        Self {
            in_sample_bars: 200,
            out_of_sample_bars: 50,
            step_bars: 50,
            initial_balance: 10_000.0,
            strategy_name: "RSI".to_owned(),
            symbol: "BTCUSDT".to_owned(),
            interval: "1h".to_owned(),
            commission_pct: 0.001,
            use_htf: false,
            edge_min_score: None,
            orderbook_sim: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowResult {
    pub window_idx: usize,
    pub in_sample_range: (usize, usize),
    pub oos_range: (usize, usize),
    pub best_tp_pct: f64,
    pub best_sl_pct: f64,
    pub oos_metrics: BacktestMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BacktestMetrics {
    pub trades: usize,
    pub win_rate: f64,
    pub pnl_pct: f64,
    pub profit_factor: f64,
    pub max_dd_pct: f64,
    pub sharpe: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardResult {
    pub config: WalkForwardConfig,
    pub windows: Vec<WindowResult>,
    pub avg_oos_pnl_pct: f64,
    pub avg_oos_sharpe: f64,
    pub consistency_score: f64, // Kârlı pencere oranı
}

// --- 2. ANALİZ MOTORU ---

pub struct WalkForwardTester {
    pub config: WalkForwardConfig,
}

impl WalkForwardTester {
    pub fn new(config: WalkForwardConfig) -> Self {
        Self { config }
    }

    /// Ana Walk-Forward döngüsü
    pub fn run(&self, candles: &[Candle]) -> Option<WalkForwardResult> {
        let total = candles.len();
        let window_size = self.config.in_sample_bars + self.config.out_of_sample_bars;
        if total < window_size { return None; }

        // Pencereleri önceden tanımla (Allocation-optimized)
        let mut window_definitions = Vec::new();
        let mut start = 0;
        while start + window_size <= total {
            window_definitions.push(start);
            start += self.config.step_bars;
        }

        // PARALEL İŞLEME: Her pencereyi farklı CPU çekirdeğinde analiz et
        let windows: Vec<WindowResult> = window_definitions.par_iter().enumerate().map(|(idx, &start)| {
            let is_end = start + self.config.in_sample_bars;
            let oos_end = is_end + self.config.out_of_sample_bars;

            let in_sample = &candles[start..is_end];
            let oos = &candles[is_end..oos_end];

            // 1. In-Sample: En iyi parametreleri bul (Eğitim)
            let (best_tp, best_sl) = self.quick_optimize(in_sample);

            // 2. Out-of-Sample: Parametreleri test et (Validasyon)
            let metrics = self.run_backtest(oos, best_tp, best_sl);

            WindowResult {
                window_idx: idx,
                in_sample_range: (start, is_end),
                oos_range: (is_end, oos_end),
                best_tp_pct: best_tp,
                best_sl_pct: best_sl,
                oos_metrics: metrics,
            }
        }).collect();

        if windows.is_empty() { return None; }

        self.finalize_report(windows)
    }

    /// In-Sample optimizasyonu: Grid Search (Hafifletilmiş)
    fn quick_optimize(&self, candles: &[Candle]) -> (f64, f64) {
        let tp_grid = [2.5, 5.0, 7.5, 10.0, 15.0];
        let sl_grid = [1.0, 2.0, 3.0, 4.0, 5.0];

        let mut best_params = (5.0, 2.0);
        let mut best_score = f64::NEG_INFINITY;

        for &tp in &tp_grid {
            for &sl in &sl_grid {
                if tp <= sl { continue; }
                let res = self.run_backtest(candles, tp, sl);
                
                // Kompozit Skor: Sharpe %40 + PnL %35 + WinRate %25
                let score = (res.sharpe * 0.40) + (res.pnl_pct * 0.35) + (res.win_rate * 0.0025);
                if score > best_score {
                    best_score = score;
                    best_params = (tp, sl);
                }
            }
        }
        best_params
    }

    /// Alt-Backtest çalıştırıcı (Zero-Panic)
    fn run_backtest(&self, candles: &[Candle], tp: f64, sl: f64) -> BacktestMetrics {
        let cfg = BacktestConfig {
            symbol: self.config.symbol.clone(),
            interval: self.config.interval.clone(),
            initial_balance: self.config.initial_balance,
            max_position_size: 1.0,
            take_profit_pct: tp,
            stop_loss_pct: sl,
            strategy_name: self.config.strategy_name.clone(),
            commission_pct: self.config.commission_pct,
            use_htf: self.config.use_htf,
            edge_min_score: self.config.edge_min_score,
            orderbook_sim: self.config.orderbook_sim.clone(),
            ..Default::default()
        };

        match Backtester::new(cfg).run(candles) {
            Ok(r) => BacktestMetrics {
                trades: r.total_trades,
                win_rate: r.win_rate,
                pnl_pct: r.total_pnl_pct,
                profit_factor: r.profit_factor,
                max_dd_pct: r.max_drawdown_pct,
                sharpe: r.sharpe_ratio,
            },
            Err(_) => BacktestMetrics::default(),
        }
    }

    fn finalize_report(&self, windows: Vec<WindowResult>) -> Option<WalkForwardResult> {
        let n = windows.len() as f64;
        let profitable_count = windows.iter().filter(|w| w.oos_metrics.pnl_pct > 0.0).count();

        Some(WalkForwardResult {
            avg_oos_pnl_pct: windows.iter().map(|w| w.oos_metrics.pnl_pct).sum::<f64>() / n,
            avg_oos_sharpe: windows.iter().map(|w| w.oos_metrics.sharpe).sum::<f64>() / n,
            consistency_score: profitable_count as f64 / n,
            config: self.config.clone(),
            windows,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rejim-bazlı parametre agregasyonu
// ─────────────────────────────────────────────────────────────────────────────
//
// Walk-Forward her pencere için (best_tp, best_sl) bulur. Bu pencereyi rejime
// göre sınıflandırıp her rejim için ortanca TP/SL'i çıkartabiliriz —
// `run_backtest_job` bu agregasyonu kullanıp ParameterStore.regime_overrides'a
// yazar, böylece engine cycle rejime özgü parametrelerle çalışır.

use std::collections::HashMap;

/// Bir rejim için Walk-Forward pencerelerinden çıkartılan agreged parametreler.
/// `sample_count` agregasyona katılan pencere sayısı (azlık halinde yazma
/// kararı çağırana bırakılır).
#[derive(Debug, Clone, PartialEq)]
pub struct RegimeAggregate {
    pub median_tp_pct: f64,
    pub median_sl_pct: f64,
    pub sample_count: usize,
}

/// Pencereleri rejime göre grupla; her rejim için (median TP, median SL) hesapla.
/// `classify` fonksiyonu pencerenin OOS dilimini alır ve rejim adını döndürür
/// (motor `Engine::classify_regime` → `MarketRegime::as_str()` chain'iyle).
/// `min_samples` altındaki rejimler atlanır (gürültü → yanlış patch yazımı önlenir).
pub fn aggregate_windows_by_regime<F>(
    candles: &[Candle],
    windows: &[WindowResult],
    classify: F,
    min_samples: usize,
) -> HashMap<String, RegimeAggregate>
where
    F: Fn(&[Candle]) -> String,
{
    let mut buckets: HashMap<String, Vec<(f64, f64)>> = HashMap::new();
    for w in windows {
        let (start, end) = w.oos_range;
        if end > candles.len() || start >= end {
            continue;
        }
        let regime = classify(&candles[start..end]);
        buckets.entry(regime).or_default()
            .push((w.best_tp_pct, w.best_sl_pct));
    }

    let mut out: HashMap<String, RegimeAggregate> = HashMap::new();
    for (regime, samples) in buckets {
        if samples.len() < min_samples {
            continue;
        }
        let mut tps: Vec<f64> = samples.iter().map(|(t, _)| *t).collect();
        let mut sls: Vec<f64> = samples.iter().map(|(_, s)| *s).collect();
        tps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sls.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = |xs: &[f64]| -> f64 {
            let m = xs.len() / 2;
            if xs.len() % 2 == 0 { (xs[m - 1] + xs[m]) / 2.0 } else { xs[m] }
        };
        out.insert(regime, RegimeAggregate {
            median_tp_pct: median(&tps),
            median_sl_pct: median(&sls),
            sample_count: samples.len(),
        });
    }
    out
}

/// Per-rejim YÖN DİSİPLİNİ A/B'si (otonom `RegimePolicy.regime_directional` girdisi).
/// Her rejimin OOS pencerelerinde aynı strateji/param ile LongOnly vs RegimeDirectional
/// backtest koşar, rejim başına toplam PnL'i kıyaslar. Dönen map: regime → disiplin
/// uygulansın mı (`RegimeDirectional PnL >= LongOnly PnL`). `min_samples` altı rejimler
/// atlanır. `base`'in YALNIZ `direction`'ı override edilir (gate/strateji/param sabit →
/// izole yön etkisi). `run_backtest_job` bunu `regime_overrides[regime].policy`'ye yazar;
/// canlı cycle o rejimde `regime_directional_for` ile okur. [[project_adaptive_regime]].
pub fn evaluate_regime_direction<F>(
    candles: &[Candle],
    windows: &[WindowResult],
    classify: F,
    base: &BacktestConfig,
    min_samples: usize,
) -> HashMap<String, bool>
where
    F: Fn(&[Candle]) -> String,
{
    // regime → (long_pnl_toplam, regimedir_pnl_toplam, pencere_sayısı)
    let mut acc: HashMap<String, (f64, f64, usize)> = HashMap::new();
    for w in windows {
        let (start, end) = w.oos_range;
        if end > candles.len() || start >= end { continue; }
        let slice = &candles[start..end];
        if slice.len() < 30 { continue; } // anlamlı backtest için min derinlik
        let regime = classify(slice);
        let run = |dir: DirectionMode| -> f64 {
            let mut c = base.clone();
            c.direction = dir;
            Backtester::new(c).run(slice).map(|r| r.total_pnl).unwrap_or(0.0)
        };
        let lp = run(DirectionMode::LongOnly);
        let rp = run(DirectionMode::RegimeDirectional);
        let e = acc.entry(regime).or_insert((0.0, 0.0, 0));
        e.0 += lp; e.1 += rp; e.2 += 1;
    }
    // RD >= Long → disiplin uygula (eşitlikte uygula: RD ayrıca tail-risk azaltır).
    acc.into_iter()
        .filter(|(_, (_, _, n))| *n >= min_samples)
        .map(|(r, (lp, rp, _))| (r, rp >= lp))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wnd(start: usize, end: usize, tp: f64, sl: f64) -> WindowResult {
        WindowResult {
            window_idx: 0,
            in_sample_range: (0, start),
            oos_range: (start, end),
            best_tp_pct: tp,
            best_sl_pct: sl,
            oos_metrics: BacktestMetrics::default(),
        }
    }

    #[test]
    fn aggregate_groups_by_regime_and_computes_median() {
        // 6 pencere: 3'ü "Ranging", 3'ü "Trending"
        let candles: Vec<Candle> = (0..100).map(|i| Candle {
            close: 100.0 + i as f64,
            ..Default::default()
        }).collect();
        let windows = vec![
            wnd(0,  10, 2.0, 1.0),
            wnd(10, 20, 3.0, 1.5),
            wnd(20, 30, 4.0, 2.0),
            wnd(30, 40, 5.0, 2.5),
            wnd(40, 50, 6.0, 3.0),
            wnd(50, 60, 7.0, 3.5),
        ];
        // İlk 3 pencere Ranging, kalan 3 Trending
        let classify = |s: &[Candle]| {
            if s.first().map(|c| c.close).unwrap_or(0.0) < 130.0 { "Ranging".into() }
            else { "Trending".into() }
        };
        let agg = aggregate_windows_by_regime(&candles, &windows, classify, 1);
        assert_eq!(agg.len(), 2);
        let r = agg.get("Ranging").unwrap();
        assert_eq!(r.sample_count, 3);
        assert!((r.median_tp_pct - 3.0).abs() < 1e-9);
        assert!((r.median_sl_pct - 1.5).abs() < 1e-9);
        let t = agg.get("Trending").unwrap();
        assert_eq!(t.sample_count, 3);
        assert!((t.median_tp_pct - 6.0).abs() < 1e-9);
        assert!((t.median_sl_pct - 3.0).abs() < 1e-9);
    }

    fn dir_base_cfg() -> BacktestConfig {
        BacktestConfig {
            symbol: "T".into(), interval: "1h".into(),
            initial_balance: 10_000.0, max_position_size: 1.0,
            take_profit_pct: 4.0, stop_loss_pct: 2.0,
            strategy_name: "EMA_CROSSOVER".into(),
            commission_pct: 0.0004, breakeven_at_rr: Some(1.0), atr_trail_mult: Some(2.0),
            ..Default::default()
        }
    }

    #[test]
    fn evaluate_regime_direction_prefers_directional_in_downtrend() {
        use chrono::{TimeZone, Utc};
        // İstikrarlı düşüş: LongOnly ya işlem açmaz ya zarar; RegimeDirectional short ile
        // yakalar → rp >= lp → "Down" rejimi için true.
        let candles: Vec<Candle> = (0..200).map(|i| {
            let f = i as f64;
            let c = 200.0 - 0.10 * f + 4.0 * (f * 0.3).sin();
            Candle {
                timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64 * 3600, 0).unwrap(),
                open: c, high: c * 1.004, low: c * 0.996, close: c,
                volume: 1000.0, symbol: "T".into(), interval: "1h".into(),
            }
        }).collect();
        let windows = vec![wnd(0, 200, 4.0, 2.0)];
        let map = evaluate_regime_direction(
            &candles, &windows, |_| "Down".to_string(), &dir_base_cfg(), 1,
        );
        assert_eq!(map.get("Down"), Some(&true),
            "düşüşte RegimeDirectional LongOnly'yi en az eşitlemeli (short kazancı)");
    }

    #[test]
    fn evaluate_regime_direction_respects_min_samples() {
        let candles: Vec<Candle> = (0..60).map(|i| Candle {
            close: 100.0 - i as f64, open: 100.0 - i as f64,
            high: 100.0 - i as f64, low: 100.0 - i as f64,
            ..Default::default()
        }).collect();
        // Tek pencere → n=1; min_samples=2 ile elenir → boş map.
        let windows = vec![wnd(0, 60, 4.0, 2.0)];
        let map = evaluate_regime_direction(
            &candles, &windows, |_| "Down".to_string(), &dir_base_cfg(), 2,
        );
        assert!(map.is_empty(), "min_samples=2 altında rejim yazılmamalı");
    }

    #[test]
    fn aggregate_skips_regimes_below_min_samples() {
        let candles: Vec<Candle> = (0..40).map(|i| Candle {
            close: 100.0 + i as f64,
            ..Default::default()
        }).collect();
        let windows = vec![
            wnd(0,  10, 2.0, 1.0),
            wnd(10, 20, 3.0, 1.5),
            // Aşağıdaki tek pencere Trending — min_samples=2 ile elenir.
            wnd(20, 30, 5.0, 2.5),
        ];
        let classify = |s: &[Candle]| {
            if s.first().map(|c| c.close).unwrap_or(0.0) < 120.0 { "Ranging".into() }
            else { "Trending".into() }
        };
        let agg = aggregate_windows_by_regime(&candles, &windows, classify, 2);
        assert!(agg.contains_key("Ranging"));
        assert!(!agg.contains_key("Trending"),
            "tek örnekli rejim yazılmamalı, min_samples=2");
    }

    #[test]
    fn aggregate_handles_empty_windows() {
        let candles: Vec<Candle> = (0..10).map(|_| Candle::default()).collect();
        let agg = aggregate_windows_by_regime(&candles, &[], |_| "Any".into(), 1);
        assert!(agg.is_empty());
    }

    #[test]
    fn aggregate_skips_out_of_range_windows() {
        let candles: Vec<Candle> = (0..10).map(|_| Candle::default()).collect();
        let bad = vec![
            wnd(0, 5, 2.0, 1.0),
            wnd(8, 100, 3.0, 1.5), // end > len
        ];
        let agg = aggregate_windows_by_regime(&candles, &bad, |_| "Test".into(), 1);
        let t = agg.get("Test").unwrap();
        assert_eq!(t.sample_count, 1, "sınır dışı pencere atlanmalı");
    }
}
