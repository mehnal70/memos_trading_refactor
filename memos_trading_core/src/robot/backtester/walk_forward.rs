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

// ─────────────────────────────────────────────────────────────────────────────
// Otonom değerlendirme/seçim çekirdeği (DRY) — rejim-yön ve sembol-interval
// değerlendiricilerinin ORTAK atomu. Kopyala-yapıştır yerine tek kaynak.
// ─────────────────────────────────────────────────────────────────────────────

/// Anlamlı backtest için bir OOS penceresinin minimum mum derinliği.
pub(crate) const MIN_EVAL_WINDOW_LEN: usize = 30;

/// Bir `BacktestConfig` varyantını tek bir mum dilimi üzerinde koşar, toplam PnL
/// döndürür (hata/boş → 0.0). Otonom değerlendiricilerin paylaşılan skorlama atomu.
pub(crate) fn backtest_pnl(cfg: &BacktestConfig, slice: &[Candle]) -> f64 {
    Backtester::new(cfg.clone()).run(slice).map(|r| r.total_pnl).unwrap_or(0.0)
}

/// `cfg`'i tüm OOS pencere dilimlerinde koşup toplam PnL döndürür — her pencere
/// bağımsız (look-ahead'siz). `MIN_EVAL_WINDOW_LEN` altı pencereler atlanır.
/// Sembol-interval değerlendirmesi bunu aday TF başına kullanır.
pub(crate) fn score_config_over_windows(
    cfg: &BacktestConfig, candles: &[Candle], windows: &[WindowResult],
) -> f64 {
    let mut total = 0.0;
    for w in windows {
        let (s, e) = w.oos_range;
        if e > candles.len() || s >= e || (e - s) < MIN_EVAL_WINDOW_LEN { continue; }
        total += backtest_pnl(cfg, &candles[s..e]);
    }
    total
}

/// Aday `(varyant, skor)` listesinden en iyiyi seç; ancak `current` (mevcut seçim,
/// varsa) skorunu `margin` ile AŞIYORSA değiştir — aksi halde mevcut korunur
/// (flip-flop/instabilite koruması). `current` yoksa salt en iyi. Boş → None.
pub(crate) fn pick_best_with_margin<T: Clone + PartialEq>(
    scored: &[(T, f64)], current: Option<&T>, margin: f64,
) -> Option<T> {
    let best = scored.iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    match current {
        Some(cur) => {
            let cur_score = scored.iter().find(|(t, _)| t == cur)
                .map(|(_, s)| *s).unwrap_or(f64::NEG_INFINITY);
            if best.1 > cur_score + margin { Some(best.0.clone()) } else { Some(cur.clone()) }
        }
        None => Some(best.0.clone()),
    }
}

/// Per-sembol otonom INTERVAL seçimi (otonom `symbol_interval` girdisi). Her aday TF için
/// `load(tf) -> candles` ile o TF'in mumlarını yükle, `score(tf, &candles) -> Option<f64>`
/// ile WF skoru hesapla (yeterli mumu olmayan/skorlanamayan aday → None ile atlanır),
/// sonra `pick_best_with_margin` ile mevcut `current`'i `margin` ile geçen en iyiyi seç.
/// Döner: (seçim, tüm aday skorları) — skorlar log/snapshot için. Hiç aday yoksa (None, []).
/// `score`/`load` closure'ları persistence + WalkForwardTester'ı çağırana bırakır (decoupled,
/// test edilebilir). Faz 0 `pick_best_with_margin`'i yeniden kullanır. [[project_adaptive_regime]].
pub fn evaluate_symbol_interval<L, S>(
    candidates: &[String], load: L, score: S, current: Option<&str>, margin: f64,
) -> (Option<String>, Vec<(String, f64)>)
where
    L: Fn(&str) -> Vec<Candle>,
    S: Fn(&str, &[Candle]) -> Option<f64>,
{
    let mut scored: Vec<(String, f64)> = Vec::new();
    for c in candidates {
        let candles = load(c);
        if let Some(s) = score(c, &candles) { scored.push((c.clone(), s)); }
    }
    if scored.is_empty() { return (None, scored); }
    let cur = current.map(|s| s.to_string());
    let choice = pick_best_with_margin(&scored, cur.as_ref(), margin);
    (choice, scored)
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
        if slice.len() < MIN_EVAL_WINDOW_LEN { continue; }
        let regime = classify(slice);
        // Ortak skorlama atomu (backtest_pnl) — yalnız direction override edilir.
        let score = |dir: DirectionMode| -> f64 {
            let mut c = base.clone();
            c.direction = dir;
            backtest_pnl(&c, slice)
        };
        let lp = score(DirectionMode::LongOnly);
        let rp = score(DirectionMode::RegimeDirectional);
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
    fn score_config_over_windows_sums_and_skips_short() {
        // 2 geçerli pencere (≥MIN_EVAL_WINDOW_LEN) + 1 kısa (atlanır). Düşüş serisinde
        // long-only ~zarar; toplam sonlu ve kısa pencere toplama girmez (idempotent).
        let candles: Vec<Candle> = (0..120).map(|i| {
            let c = 200.0 - 0.1 * i as f64;
            Candle { open: c, high: c * 1.004, low: c * 0.996, close: c,
                     volume: 1000.0, symbol: "T".into(), interval: "1h".into(),
                     ..Default::default() }
        }).collect();
        let windows = vec![wnd(0, 50, 4.0, 2.0), wnd(50, 100, 4.0, 2.0), wnd(100, 110, 4.0, 2.0)];
        let s_all = score_config_over_windows(&dir_base_cfg(), &candles, &windows);
        // Kısa pencereyi tek başına vermek toplama 0 katkı verir (skip guard).
        let only_short = score_config_over_windows(&dir_base_cfg(), &candles, &[wnd(100, 110, 4.0, 2.0)]);
        assert_eq!(only_short, 0.0, "kısa pencere (<30) atlanmalı");
        assert!(s_all.is_finite());
    }

    #[test]
    fn evaluate_symbol_interval_selects_and_skips_and_holds() {
        let cands = vec!["5m".to_string(), "15m".to_string(), "1h".to_string()];
        // load: hepsi dolu (boş döndürmüyoruz); score: 5m skorlanamaz (None → atlanır),
        // 15m=1.0, 1h=1.5. current yok → en iyi (1h) + skorlar 5m hariç.
        let load = |_tf: &str| -> Vec<Candle> { vec![Candle::default(); 100] };
        let score = |tf: &str, _c: &[Candle]| -> Option<f64> {
            match tf { "5m" => None, "15m" => Some(1.0), "1h" => Some(1.5), _ => None }
        };
        let (choice, scored) = evaluate_symbol_interval(&cands, load, score, None, 0.0);
        assert_eq!(choice, Some("1h".to_string()));
        assert_eq!(scored.len(), 2, "5m skorlanamadı → atlanmalı");

        // current=15m, margin 1.0: 1h (1.5) > 15m (1.0)+1.0=2.0 DEĞİL → 15m korunur.
        let (hold, _) = evaluate_symbol_interval(&cands, load, score, Some("15m"), 1.0);
        assert_eq!(hold, Some("15m".to_string()), "marj altında interval değişmemeli");

        // Hiçbir aday skorlanamazsa (None,[]) — flip-flop/yanlış yazım önleme.
        let none_score = |_tf: &str, _c: &[Candle]| -> Option<f64> { None };
        let (empty_choice, empty_scored) = evaluate_symbol_interval(&cands, load, none_score, Some("1h"), 0.0);
        assert_eq!(empty_choice, None);
        assert!(empty_scored.is_empty());
    }

    #[test]
    fn pick_best_with_margin_respects_current_and_margin() {
        let scored = vec![("a", 10.0), ("b", 12.0), ("c", 8.0)];
        // current yok → salt en iyi (b).
        assert_eq!(pick_best_with_margin(&scored, None, 0.0), Some("b"));
        // current=a, b (12) > a (10) + margin 1.0 → değiş (b).
        assert_eq!(pick_best_with_margin(&scored, Some(&"a"), 1.0), Some("b"));
        // current=a, margin 3.0 → b (12) > a (10)+3=13 DEĞİL → a korunur (flip-flop yok).
        assert_eq!(pick_best_with_margin(&scored, Some(&"a"), 3.0), Some("a"));
        // current listede yok → en iyiye geç (b).
        assert_eq!(pick_best_with_margin(&scored, Some(&"z"), 1.0), Some("b"));
        // boş → None.
        let empty: Vec<(&str, f64)> = vec![];
        assert_eq!(pick_best_with_margin(&empty, None, 0.0), None);
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
