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
    /// Canlı çıkış modeli: ATR-trail çarpanı. TP/SL araması canlının uyguladığı
    /// trailing'le BİRLİKTE yapılsın diye (eskiden None = trailing'siz → seçilen TP
    /// canlıda trailing erken çıkınca nadiren ateşleniyordu). Default None (geriye-uyum:
    /// eski testler/screener trailing'siz kalır). Backtest job canlı-temsili değerle doldurur.
    #[serde(default)]
    pub atr_trail_mult: Option<f64>,
    /// Canlı çıkış modeli: breakeven RR eşiği (canlı default 1.0). Default None.
    #[serde(default)]
    pub breakeven_at_rr: Option<f64>,
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
            atr_trail_mult: None,
            breakeven_at_rr: None,
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
            // Canlı çıkış modeli (varsa) → strateji seçimi de trailing'i görür.
            atr_trail_mult: self.config.atr_trail_mult,
            breakeven_at_rr: self.config.breakeven_at_rr,
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
            if xs.len().is_multiple_of(2) { (xs[m - 1] + xs[m]) / 2.0 } else { xs[m] }
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

/// Bir cfg'i dilimde koşar; `(brüt_kâr, brüt_zarar≥0, işlem_sayısı)` döner. Pooled
/// Profit Factor agregasyonu için: pencereler boyunca gp/gl AYRI toplanır, PF sonda
/// `Σgp/Σgl` ile hesaplanır → per-pencere PF ortalamasından (az-işlemli pencerede
/// sentinel'lerle bozulan) çok daha stabil. Hata/boş → (0,0,0).
pub(crate) fn backtest_gross(cfg: &BacktestConfig, slice: &[Candle]) -> (f64, f64, usize) {
    match Backtester::new(cfg.clone()).run(slice) {
        Ok(r) => {
            let gp: f64 = r.trades.iter().filter(|t| t.pnl > 0.0).map(|t| t.pnl).sum();
            let gl: f64 = r.trades.iter().filter(|t| t.pnl < 0.0).map(|t| -t.pnl).sum();
            (gp, gl, r.total_trades)
        }
        Err(_) => (0.0, 0.0, 0),
    }
}

/// WF çoklu-pencere çapraz-kontrol istatistiği — tek-holdout fluke'una karşı tutarlılık ölçer.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WfCrossCheck {
    /// İşlem üreten OOS pencere sayısı (denominatör).
    pub windows: usize,
    /// PF≥1.0 olan pencere sayısı (tutarlılık payı).
    pub profitable_windows: usize,
    /// Pencereler boyunca pooled PF (Σgp/Σgl) — şanslı tek pencereye dayanmaz.
    pub pooled_pf: f64,
    /// Toplam işlem (tüm pencereler).
    pub trades: usize,
}

impl Default for WfCrossCheck {
    fn default() -> Self { Self { windows: 0, profitable_windows: 0, pooled_pf: 0.0, trades: 0 } }
}

impl WfCrossCheck {
    /// Kâr-eden pencere oranı (0..1). İşlemli pencere yoksa 0.
    pub fn consistency(&self) -> f64 {
        if self.windows == 0 { 0.0 } else { self.profitable_windows as f64 / self.windows as f64 }
    }
}

/// Bir cfg'i OOS pencerelerinde TEK TEK koşar → pooled PF + kâr-eden pencere oranı (tutarlılık).
/// `score_config_over_windows` yalnız pooled PF döndürür; bu, EK OLARAK pencere-bazlı tutarlılığı
/// (edge bir şanslı pencereden mi yoksa süregelen mi) ölçer. İşlemsiz pencere `windows` sayımına
/// girmez (boş pencere tutarlılığı bozmasın). Edge-tarama çapraz-kontrolünün tek-kaynağı.
pub fn wf_cross_check(cfg: &BacktestConfig, candles: &[Candle], windows: &[WindowResult]) -> WfCrossCheck {
    let (mut gp_sum, mut gl_sum, mut trades, mut with_trades, mut profitable) = (0.0_f64, 0.0_f64, 0usize, 0usize, 0usize);
    for w in windows {
        let (s, e) = w.oos_range;
        if e > candles.len() || s >= e || (e - s) < MIN_EVAL_WINDOW_LEN { continue; }
        let (gp, gl, t) = backtest_gross(cfg, &candles[s..e]);
        if t == 0 { continue; }
        gp_sum += gp; gl_sum += gl; trades += t; with_trades += 1;
        let win_pf = if gl > f64::EPSILON { gp / gl } else if gp > 0.0 { f64::INFINITY } else { 0.0 };
        if win_pf >= 1.0 { profitable += 1; }
    }
    let pooled_pf = if gl_sum > f64::EPSILON { gp_sum / gl_sum }
                    else if gp_sum > 0.0 { 100.0 } else { 0.0 };
    WfCrossCheck { windows: with_trades, profitable_windows: profitable, pooled_pf, trades }
}

/// Bir cfg'i dilimde koşar; tek tek işlem PnL'lerini döner (boş/hata → []).
/// Çıkış-modeli taraması pencereler boyunca tüm PnL'leri havuzlayıp beklenti/PF/
/// sharpe hesaplamak için kullanır (tek-kaynak istatistik).
pub(crate) fn backtest_trade_pnls(cfg: &BacktestConfig, slice: &[Candle]) -> Vec<f64> {
    Backtester::new(cfg.clone()).run(slice)
        .map(|r| r.trades.iter().map(|t| t.pnl).collect())
        .unwrap_or_default()
}

/// Bir çıkış-modeli varyantının havuzlanmış (tüm OOS pencereleri birleştirilmiş)
/// performans istatistikleri. R/R teşhisi için: trailing'in edge'i mi yediği
/// (gevşet/kapat → düzelir) yoksa girişlerde edge'in mi olmadığı (hiçbir çıkış
/// kurtarmaz) sorusunu sayıyla ayırır.
#[derive(Debug, Clone)]
pub struct ExitModelStats {
    pub label: String,
    pub total_trades: usize,
    pub win_rate: f64,      // 0..1
    pub avg_win: f64,
    pub avg_loss: f64,      // pozitif
    pub expectancy: f64,    // işlem başına ortalama PnL
    pub profit_factor: f64,
    pub sharpe: f64,        // işlem-başı PnL ortalaması / std (yıllıklaştırılmamış proxy)
    pub total_pnl: f64,
}

/// Bir (hazırlanmış) config varyantını OOS pencerelerinde koşar, tüm işlem PnL'lerini
/// HAVUZLAR ve beklenti/PF/sharpe/win-rate istatistiğini tek seferde üretir. Config-varyant
/// A/B'lerinin (çıkış-modeli, edge-filtre, …) paylaşılan çekirdeği — tek-kaynak.
fn pooled_variant_stats(
    label: String, candles: &[Candle], windows: &[WindowResult], cfg: &BacktestConfig,
) -> ExitModelStats {
    let mut pnls: Vec<f64> = Vec::new();
    for w in windows {
        let (s, e) = w.oos_range;
        if e > candles.len() || s >= e || (e - s) < MIN_EVAL_WINDOW_LEN { continue; }
        pnls.extend(backtest_trade_pnls(cfg, &candles[s..e]));
    }
    let n = pnls.len();
    let wins: Vec<f64> = pnls.iter().copied().filter(|&p| p > 0.0).collect();
    let losses: Vec<f64> = pnls.iter().copied().filter(|&p| p < 0.0).collect();
    let gp: f64 = wins.iter().sum();
    let gl: f64 = losses.iter().map(|p| -p).sum();
    let total_pnl: f64 = pnls.iter().sum();
    let win_rate = if n > 0 { wins.len() as f64 / n as f64 } else { 0.0 };
    let avg_win = if !wins.is_empty() { gp / wins.len() as f64 } else { 0.0 };
    let avg_loss = if !losses.is_empty() { gl / losses.len() as f64 } else { 0.0 };
    let expectancy = if n > 0 { total_pnl / n as f64 } else { 0.0 };
    let profit_factor = if gl > f64::EPSILON { gp / gl }
                        else if gp > 0.0 { f64::INFINITY } else { 0.0 };
    let sharpe = if n >= 2 {
        let mean = total_pnl / n as f64;
        let var = pnls.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
        let sd = var.sqrt();
        if sd > f64::EPSILON { mean / sd } else { 0.0 }
    } else { 0.0 };
    ExitModelStats {
        label, total_trades: n, win_rate, avg_win, avg_loss,
        expectancy, profit_factor, sharpe, total_pnl,
    }
}

/// Çıkış-modeli A/B taraması — yalnız ÇIKIŞ ekseni değişir, giriş/strateji/param sabit.
/// `exits`: `(etiket, atr_trail_mult, breakeven_at_rr)` üçlüleri (None trail = trailing yok).
/// Döner: girişteki sırayla havuzlanmış `ExitModelStats` listesi.
pub fn evaluate_exit_models(
    candles: &[Candle],
    windows: &[WindowResult],
    base: &BacktestConfig,
    exits: &[(String, Option<f64>, Option<f64>)],
) -> Vec<ExitModelStats> {
    exits.iter().map(|(label, trail, be)| {
        let mut c = base.clone();
        c.atr_trail_mult = *trail;
        c.breakeven_at_rr = *be;
        pooled_variant_stats(label.clone(), candles, windows, &c)
    }).collect()
}

/// Edge-filtre A/B taraması — yalnız GİRİŞ HUNİSİ (edge_min_score) değişir, çıkış/strateji/
/// param sabit (base'in canlı çıkış modeli korunur). Teşhis: huni mi çok sıkı (işlem
/// kıtlığı → gevşetince PF düzelir) yoksa sinyalde edge mi yok (gevşetince işlem artar
/// ama PF<1 kalır). `thresholds`: `None` = filtre yok (her Buy açılır), `Some(t)` = edge≥t.
/// Döner: girişteki sırayla havuzlanmış `ExitModelStats` (label = eşik).
pub fn evaluate_edge_filters(
    candles: &[Candle],
    windows: &[WindowResult],
    base: &BacktestConfig,
    thresholds: &[Option<f64>],
) -> Vec<ExitModelStats> {
    thresholds.iter().map(|t| {
        let mut c = base.clone();
        c.edge_min_score = *t;
        let label = match t {
            Some(v) => format!("edge≥{:.2}", v),
            None => "filtre-yok".to_string(),
        };
        pooled_variant_stats(label, candles, windows, &c)
    }).collect()
}

/// `cfg`'i tüm OOS pencere dilimlerinde koşup toplam PnL döndürür — her pencere
/// bağımsız (look-ahead'siz). `MIN_EVAL_WINDOW_LEN` altı pencereler atlanır.
/// Sembol-interval değerlendirmesi bunu aday TF başına kullanır.
pub(crate) fn score_config_over_windows(
    cfg: &BacktestConfig, candles: &[Candle], windows: &[WindowResult],
) -> f64 {
    // OBJEKTİF: pooled Profit Factor (Σgp/Σgl, pencereler boyunca) — ham PnL DEĞİL.
    // Gerekçe ([[project_rr_trail_ab]] lever A): gürültü TF'i (örn. 1m, fee/gürültü baskın,
    // ölçülen PF≈0.01) trend'li bir OOS penceresinde yüksek ham PnL gösterip R/R'si berbat
    // olabilir; PnL objektifi interval seçicisini bu "kumarbaz" TF'i sağlam 1h'a TERCİH
    // etmeye iter (kazanan-çok/kaybeden-büyük = net negatif). PF R/R dengesini ölçtüğü için
    // düşük-PF gürültü TF'lerini otonom eler (hard-coded TF yasağı yok [[feedback_autonomy_first]]).
    // Trail A/B'de PnL→PF dönüşümüyle (0fe2c33) aynı ders; backtest_gross tek-kaynak (DRY).
    const MIN_IV_TRADES: usize = 5;   // pooled işlem altı → seçim yapma (küçük-örneklem gürültüsü)
    const PF_CAP: f64 = 100.0;        // kayıpsız varyant → INF yerine finite tavan (seçim stabil)
    let (mut gp, mut gl, mut n) = (0.0_f64, 0.0_f64, 0usize);
    for w in windows {
        let (s, e) = w.oos_range;
        if e > candles.len() || s >= e || (e - s) < MIN_EVAL_WINDOW_LEN { continue; }
        let (p, l, t) = backtest_gross(cfg, &candles[s..e]);
        gp += p; gl += l; n += t;
    }
    if n < MIN_IV_TRADES { return 0.0; }
    if gl > f64::EPSILON { gp / gl }
    else if gp > 0.0 { PF_CAP }
    else { 0.0 }
}

/// `n` mumdan WF OOS pencere aralıklarını üretir — param-opt YOK, yalnız index aralıkları
/// (run()'ın stepping aritmetiğiyle aynı). `score_config_over_windows` için HAFİF pencere
/// kaynağı: pool-wide interval eval'de full WalkForwardTester (per-pencere param re-opt)
/// yerine kullanılır → ucuz + ekseni (interval) izole eder. tp/sl alanları kullanılmaz.
pub(crate) fn wf_oos_windows(n: usize, is_bars: usize, oos_bars: usize, step: usize) -> Vec<WindowResult> {
    let window_size = is_bars + oos_bars;
    let step = step.max(1);
    let mut out = Vec::new();
    let (mut start, mut idx) = (0usize, 0usize);
    while window_size > 0 && start + window_size <= n {
        let is_end = start + is_bars;
        out.push(WindowResult {
            window_idx: idx,
            in_sample_range: (start, is_end),
            oos_range: (is_end, is_end + oos_bars),
            best_tp_pct: 0.0, best_sl_pct: 0.0,
            oos_metrics: BacktestMetrics::default(),
        });
        start += step; idx += 1;
    }
    out
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

/// Per-sembol otonom strateji seçiminin verdikti. Eski `Option<String>` sözleşmesi "mevcut yok +
/// edge yok" (no-op) ile "mevcut vardı + edge ÇÜRÜDÜ" (kaldırılmalı) durumlarını `None`'da
/// birleştiriyordu → çürüyen seed/keşif `symbol_strategy`'de kalıcı kalıyor, ölü edge canlı trade'i
/// ve screener bonusunu sürdürüyordu. 3-durumlu karar bu boşluğu kapatır. [[project_edge_scan]].
#[derive(Debug, Clone, PartialEq)]
pub enum StrategyChoice {
    /// Değişiklik yok: hiç aday yok, ya da mevcut atama hâlâ en iyi + edge'li → çağıran ne
    /// yapıyorsa sürsün (mevcut yoksa global/auto, varsa korunur). Yazma gerektirmez.
    Keep,
    /// Gerçek edge'li (PF ≥ min_score) stratejiyi ata (mevcut yoktu, ya da margin'le geçildi,
    /// ya da çürüyen mevcudun yerine geçti).
    Assign(String),
    /// Mevcut per-symbol atama artık min_score'u geçmiyor VE yerini alacak edge'li aday da yok →
    /// KALDIR (sembol global/auto'ya döner; ölü edge canlıyı sürüklemeyi bırakır).
    Demote,
}

/// Per-sembol otonom STRATEJİ seçimi (otonom `symbol_strategy` girdisi). `evaluate_symbol_interval`'in
/// strateji-ekseni kardeşi: aday strateji adlarını `score(name) -> Option<f64>` (pooled PF) ile
/// skorlar. Mumlar SABİT (tek seri); yalnız strateji değişir → `load` adımı YOK.
///
/// Karar mantığı:
///   - Mevcut atama YOK ya da hâlâ edge'li (≥min_score): `pick_best_with_margin` (flip-flop
///     koruması) → kazanan ≥min_score ise (ve mevcuttan farklıysa) `Assign`, değilse `Keep`.
///   - Mevcut atama ÇÜRÜDÜ (skoru min_score altına düştü/skorlanamadı): flip-flop margin'i ölü
///     seed'i KORUMAZ → edge'li (≥min_score) en iyi aday varsa ona geç (`Assign`), yoksa `Demote`.
///
/// Döner: (karar, tüm skorlar — log/snapshot için). [[project_edge_scan]].
pub fn evaluate_symbol_strategy<S>(
    candidates: &[String], score: S, current: Option<&str>, margin: f64, min_score: f64,
) -> (StrategyChoice, Vec<(String, f64)>)
where
    S: Fn(&str) -> Option<f64>,
{
    let mut scored: Vec<(String, f64)> = Vec::new();
    for c in candidates {
        if let Some(s) = score(c) { scored.push((c.clone(), s)); }
    }
    // Skorlanacak aday yok → veriden hüküm çıkmaz; ÇÜRÜME kararı verme (mevcut neyse kalsın).
    if scored.is_empty() { return (StrategyChoice::Keep, scored); }

    // Mevcut atamanın güncel skoru (listede yoksa → çürümüş/skorlanamaz sayılır).
    let cur_score = current.and_then(|cur| scored.iter().find(|(n, _)| n == cur).map(|(_, s)| *s));
    let decayed = current.is_some() && cur_score.is_none_or(|s| s < min_score);

    if decayed {
        // Ölü seed'i margin korumaz: edge'li en iyi aday varsa ona geç, yoksa KALDIR.
        let best = scored.iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let decision = match best {
            Some((name, s)) if *s >= min_score => StrategyChoice::Assign(name.clone()),
            _ => StrategyChoice::Demote,
        };
        return (decision, scored);
    }

    // Mevcut yok ya da hâlâ edge'li → margin disiplini (flip-flop koruması).
    let cur = current.map(|s| s.to_string());
    let decision = match pick_best_with_margin(&scored, cur.as_ref(), margin) {
        Some(name) => {
            let s = scored.iter().find(|(n, _)| *n == name).map(|(_, s)| *s).unwrap_or(f64::NEG_INFINITY);
            if s < min_score || current == Some(name.as_str()) {
                StrategyChoice::Keep // edge yok (mevcut da yok), ya da mevcut zaten en iyi → yazma yok
            } else {
                StrategyChoice::Assign(name)
            }
        }
        None => StrategyChoice::Keep,
    };
    (decision, scored)
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

/// Per-rejim trailing-stop hedef (`target_trail_pct`) A/B — R/R asimetrisi lever'ı.
///
/// Her rejimin OOS pencerelerinde aday `target_trail_pct` kümesini, CANLI
/// `resolve_atr_mult` formülüyle birebir (`mult = target / pencere_noise_floor%`,
/// clamp [1.5, 30]) backtest motorunun `atr_trail_mult`'una çevirip skorlar.
/// **Objektif: pooled Profit Factor** (`Σgp/Σgl`, pencereler boyunca toplanır) —
/// trail bir R/R lever'ı olduğundan PF doğru hedef; ham PnL trend'li veride gevşek
/// trail'e doğru MONOTON sapma yapar (daha az çıkış = daha çok PnL), PF ise
/// kazanç/kayıp dengesini ölçtüğü için iç optimumu yakalar. Pencere noise floor'u
/// canlı `symbol_stats` ile aynı çekirdekten (`window_noise_floor_pct`) gelir.
///
/// `base` canlı çıkışı modellemeli (breakeven_at_rr/atr_trail_mult set; A/B yalnız
/// trail eksenini değiştirir). Döner: regime → kazanan `target_trail_pct`. Noise
/// floor üretilemeyen pencereler atlanır; `min_samples` altı rejim ve toplam işlem
/// `MIN_TRADES` altı aday dışlanır (az-işlemli gürültü). Hiç eligible aday yoksa rejim yazılmaz.
pub fn evaluate_regime_trail<F>(
    candles: &[Candle],
    windows: &[WindowResult],
    classify: F,
    base: &BacktestConfig,
    candidates: &[f64],
    min_samples: usize,
) -> HashMap<String, f64>
where
    F: Fn(&[Candle]) -> String,
{
    use crate::robot::parameters::window_noise_floor_pct;
    const MIN_MULT: f64 = 1.5;
    const MAX_MULT: f64 = 30.0;
    // Bir adayın eligible sayılması için rejim genelinde gereken min toplam işlem —
    // tek-iki işlemli "gl=0 → PF=∞" flukelerini eler.
    const MIN_TRADES: usize = 5;
    if candidates.is_empty() { return HashMap::new(); }

    // regime → (aday başına (Σgp, Σgl, Σtrades), geçerli pencere sayısı)
    type Acc = (Vec<(f64, f64, usize)>, usize);
    let mut acc: HashMap<String, Acc> = HashMap::new();
    for w in windows {
        let (start, end) = w.oos_range;
        if end > candles.len() || start >= end { continue; }
        let slice = &candles[start..end];
        if slice.len() < MIN_EVAL_WINDOW_LEN { continue; }
        // Pencere mikro-yapısı (canlı noise floor ile aynı hesap). Üretilemezse atla.
        let Some(noise) = window_noise_floor_pct(slice).filter(|&n| n > 0.0) else { continue };
        let regime = classify(slice);
        let entry = acc.entry(regime)
            .or_insert_with(|| (vec![(0.0, 0.0, 0); candidates.len()], 0));
        for (i, &target) in candidates.iter().enumerate() {
            let mult = (target / noise).clamp(MIN_MULT, MAX_MULT);
            let mut c = base.clone();
            c.atr_trail_mult = Some(mult);
            let (gp, gl, n) = backtest_gross(&c, slice);
            entry.0[i].0 += gp;
            entry.0[i].1 += gl;
            entry.0[i].2 += n;
        }
        entry.1 += 1;
    }

    acc.into_iter()
        .filter(|(_, (_, n))| *n >= min_samples)
        .filter_map(|(r, (stats, _))| {
            // Pooled PF; yalnız MIN_TRADES eşiğini geçen adaylar yarışır.
            let best = candidates.iter().zip(stats.iter())
                .filter(|(_, (_, _, trades))| *trades >= MIN_TRADES)
                .map(|(t, (gp, gl, _))| {
                    let pf = if *gl > f64::EPSILON { gp / gl }
                             else if *gp > 0.0 { f64::INFINITY } else { 0.0 };
                    (*t, pf)
                })
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(t, _)| t)?;
            Some((r, best))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔬 TEŞHİS (gerçek DB, #[ignore]): interval seçim objektifi ham PnL → pooled PF
    /// dönüşümünün lever A iddiasını kanıtlar — gürültü TF'i (1m) PnL'de yüksek/PF'de düşük,
    /// 1h ise PF'de üstün. Elle:
    /// `cargo test -p memos_trading_core lever_a_interval_pnl_vs_pf -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lever_a_interval_pnl_vs_pf() {
        // Gerçek DB repo-kökünde (crate CWD'sinden bir üst). İkisini de dene.
        let db = ["../data/trader.db", "data/trader.db"].into_iter()
            .find(|p| std::path::Path::new(p).exists())
            .expect("trader.db bulunamadı");
        let (sym, market) = ("BTCUSDT", "futures");
        // Üretim iv_base'iyle aynı ruhta temsili config (çıkış modeli set: canlıyı modeller).
        let base = BacktestConfig {
            symbol: sym.into(), initial_balance: 10_000.0, max_position_size: 1.0,
            take_profit_pct: 4.0, stop_loss_pct: 2.0, strategy_name: "EMA_CROSSOVER".into(),
            commission_pct: 0.0004, edge_min_score: Some(0.20),
            breakeven_at_rr: Some(1.0), atr_trail_mult: Some(2.0),
            ..Default::default()
        };
        let (wf_is, wf_oos, wf_step) = (300usize, 100usize, 100usize);
        println!("\n=== Lever A teşhis: {sym} {market} — ham PnL vs pooled PF ===");
        for tf in ["1m", "15m", "1h"] {
            let candles = crate::persistence::reader::read_candles_market(db, sym, tf, market, 5000)
                .unwrap_or_default();
            if candles.len() < wf_is + wf_oos { println!("{tf:>4}: yetersiz mum ({})", candles.len()); continue; }
            let windows = wf_oos_windows(candles.len(), wf_is, wf_oos, wf_step);
            let mut cfg = base.clone(); cfg.interval = tf.to_string();
            // Ham PnL (eski objektif) — manuel topla.
            let pnl: f64 = windows.iter().filter_map(|w| {
                let (s, e) = w.oos_range;
                if e > candles.len() || s >= e || (e - s) < MIN_EVAL_WINDOW_LEN { None }
                else { Some(backtest_pnl(&cfg, &candles[s..e])) }
            }).sum();
            // Pooled PF (yeni objektif).
            let pf = score_config_over_windows(&cfg, &candles, &windows);
            println!("{tf:>4}: bar={:<5} pencere={:<3} hamPnL={:>10.2}  pooledPF={:>6.3}",
                     candles.len(), windows.len(), pnl, pf);
        }
        println!("Beklenti: 1m hamPnL yanıltıcı olabilir; pooledPF 1h'ı üstün göstermeli.\n");
    }

    /// 🔬 KOL B TEŞHİS (gerçek DB, #[ignore]): fee düşürmek (maker-limit) 1h PF'ini >1.0'a
    /// taşıyabilir mi? SIFIR-fee tavanı PF<1.0 ise hiçbir fee optimizasyonu kurtaramaz =
    /// sorun gross edge, Kol B beyhude. Elle:
    /// `cargo test -p memos_trading_core kol_b_fee_sensitivity -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn kol_b_fee_sensitivity() {
        let db = ["../data/trader.db", "data/trader.db"].into_iter()
            .find(|p| std::path::Path::new(p).exists()).expect("trader.db yok");
        let (sym, market, tf) = ("BTCUSDT", "futures", "1h");
        let base = BacktestConfig {
            symbol: sym.into(), interval: tf.into(), initial_balance: 10_000.0,
            max_position_size: 1.0, take_profit_pct: 4.0, stop_loss_pct: 2.0,
            strategy_name: "EMA_CROSSOVER".into(), edge_min_score: Some(0.20),
            breakeven_at_rr: Some(1.0), atr_trail_mult: Some(2.0),
            ..Default::default()
        };
        let candles = crate::persistence::reader::read_candles_market(db, sym, tf, market, 5000)
            .unwrap_or_default();
        let windows = wf_oos_windows(candles.len(), 300, 100, 100);
        println!("\n=== Kol B teşhis: {sym} {market} {tf} — fee duyarlılığı (pooled PF) ===");
        // (etiket, simetrik commission_pct). 0.0003 ≈ maker-giriş(0.0002)+taker-çıkış(0.0004) ort.
        for (label, c) in [("taker 0.0004", 0.0004), ("maker-ort 0.0003", 0.0003),
                           ("maker 0.0002", 0.0002), ("SIFIR (tavan)", 0.0)] {
            let mut cfg = base.clone(); cfg.commission_pct = c;
            let pf = score_config_over_windows(&cfg, &candles, &windows);
            println!("  {label:<18}: pooledPF={pf:>6.3}");
        }
        println!("Karar: SIFIR-fee PF<1.0 ise Kol B fee'yle PF>1.0 yapamaz (gross edge sorunu).\n");
    }

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
    fn score_config_over_windows_is_profit_factor_finite_and_skips_short() {
        // Düşüş serisinde long-only ~hep zarar → pooled PF = 0 (gp=0). Kısa pencere
        // (<MIN_EVAL_WINDOW_LEN) atlanır → işlem yok → MIN_IV_TRADES guard'ı 0 döndürür.
        let candles: Vec<Candle> = (0..120).map(|i| {
            let c = 200.0 - 0.1 * i as f64;
            Candle { open: c, high: c * 1.004, low: c * 0.996, close: c,
                     volume: 1000.0, symbol: "T".into(), interval: "1h".into(),
                     ..Default::default() }
        }).collect();
        let windows = vec![wnd(0, 50, 4.0, 2.0), wnd(50, 100, 4.0, 2.0), wnd(100, 110, 4.0, 2.0)];
        let s_all = score_config_over_windows(&dir_base_cfg(), &candles, &windows);
        let only_short = score_config_over_windows(&dir_base_cfg(), &candles, &[wnd(100, 110, 4.0, 2.0)]);
        assert_eq!(only_short, 0.0, "kısa pencere (<30) → işlem yok → guard 0 döndürür");
        // PF her zaman SONLU (INF değil — PF_CAP) ve negatif değil (ham PnL'in aksine).
        assert!(s_all.is_finite() && s_all >= 0.0, "skor finite, non-negatif PF olmalı (PnL değil)");
    }

    #[test]
    fn evaluate_regime_trail_selects_candidate_and_respects_min_samples() {
        // Belirgin zigzag (~%4 tepe-dip) → EMA crossover sık tetiklenir, hem kazanç
        // hem kayıp üretir (pooled PF anlamlı + MIN_TRADES eşiği aşılır). 4 OOS penceresi.
        let candles: Vec<Candle> = (0..240).map(|i| {
            let phase = (i / 8) % 2; // 8 barlık yukarı/aşağı dalgalar
            let dir = if phase == 0 { 1.0 } else { -1.0 };
            let base = 100.0 + dir * (i % 8) as f64 * 0.5;
            let c = base + dir * 0.5;
            Candle { open: base, high: c.max(base) + 0.6, low: c.min(base) - 0.6, close: c,
                     volume: 1000.0, symbol: "T".into(), interval: "1h".into(),
                     ..Default::default() }
        }).collect();
        let windows = vec![wnd(0, 60, 4.0, 2.0), wnd(60, 120, 4.0, 2.0),
                           wnd(120, 180, 4.0, 2.0), wnd(180, 240, 4.0, 2.0)];
        let candidates = [0.5, 1.0, 2.0, 3.0];
        let classify = |_s: &[Candle]| "Trending".to_string();

        let map = evaluate_regime_trail(&candles, &windows, classify, &dir_base_cfg(),
            &candidates, 2);
        // Eligible aday varsa seçilen aday kümesinde olmalı (PF tabanlı seçim).
        if let Some(chosen) = map.get("Trending").copied() {
            assert!(candidates.contains(&chosen), "seçilen {chosen} aday kümesinde olmalı");
        }

        // min_samples pencere sayısından büyük → rejim dışlanır (boş map).
        let strict = evaluate_regime_trail(&candles, &windows, classify, &dir_base_cfg(),
            &candidates, 99);
        assert!(strict.is_empty(), "min_samples üstünde rejim yazılmamalı");

        // Boş aday kümesi → boş map (panik yok).
        let empty = evaluate_regime_trail(&candles, &windows, classify, &dir_base_cfg(),
            &[], 1);
        assert!(empty.is_empty());
    }

    #[test]
    fn evaluate_exit_models_returns_stats_per_model_in_order() {
        let candles: Vec<Candle> = (0..240).map(|i| {
            let phase = (i / 8) % 2;
            let dir = if phase == 0 { 1.0 } else { -1.0 };
            let base = 100.0 + dir * (i % 8) as f64 * 0.5;
            let c = base + dir * 0.5;
            Candle { open: base, high: c.max(base) + 0.6, low: c.min(base) - 0.6, close: c,
                     volume: 1000.0, symbol: "T".into(), interval: "1h".into(),
                     ..Default::default() }
        }).collect();
        let windows = vec![wnd(0, 60, 4.0, 2.0), wnd(60, 120, 4.0, 2.0),
                           wnd(120, 180, 4.0, 2.0), wnd(180, 240, 4.0, 2.0)];
        let exits = vec![
            ("trailing-yok".to_string(), None,      Some(1.0)),
            ("baseline".to_string(),     Some(2.0), Some(1.0)),
            ("gevsek".to_string(),       Some(8.0), Some(1.0)),
        ];
        let stats = evaluate_exit_models(&candles, &windows, &dir_base_cfg(), &exits);
        assert_eq!(stats.len(), 3, "her model için bir kayıt");
        assert_eq!(stats[0].label, "trailing-yok");
        assert_eq!(stats[2].label, "gevsek");
        for s in &stats {
            assert!(s.expectancy.is_finite() && s.total_pnl.is_finite());
            assert!((0.0..=1.0).contains(&s.win_rate));
            // PF inf olabilir (gl=0) ama NaN olmamalı
            assert!(!s.profit_factor.is_nan());
        }
    }

    #[test]
    fn evaluate_edge_filters_returns_stats_and_tighter_filter_trades_no_more() {
        let candles: Vec<Candle> = (0..240).map(|i| {
            let phase = (i / 8) % 2;
            let dir = if phase == 0 { 1.0 } else { -1.0 };
            let base = 100.0 + dir * (i % 8) as f64 * 0.5;
            let c = base + dir * 0.5;
            Candle { open: base, high: c.max(base) + 0.6, low: c.min(base) - 0.6, close: c,
                     volume: 1000.0, symbol: "T".into(), interval: "1h".into(),
                     ..Default::default() }
        }).collect();
        let windows = vec![wnd(0, 60, 4.0, 2.0), wnd(60, 120, 4.0, 2.0),
                           wnd(120, 180, 4.0, 2.0), wnd(180, 240, 4.0, 2.0)];
        let thresholds = vec![None, Some(0.10), Some(0.40)];
        let stats = evaluate_edge_filters(&candles, &windows, &dir_base_cfg(), &thresholds);
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0].label, "filtre-yok");
        assert_eq!(stats[2].label, "edge≥0.40");
        // Daha sıkı eşik daha çok işlem AÇMAMALI (monotonluk: filtre yalnız eler).
        assert!(stats[0].total_trades >= stats[1].total_trades,
            "filtre-yok ({}) ≥ edge≥0.10 ({})", stats[0].total_trades, stats[1].total_trades);
        assert!(stats[1].total_trades >= stats[2].total_trades,
            "edge≥0.10 ({}) ≥ edge≥0.40 ({})", stats[1].total_trades, stats[2].total_trades);
        for s in &stats { assert!(!s.profit_factor.is_nan() && s.expectancy.is_finite()); }
    }

    #[test]
    fn wf_oos_windows_steps_and_bounds() {
        // n=100, is=20, oos=10 (window=30), step=10 → start 0,10,...,70 = 8 pencere.
        let w = wf_oos_windows(100, 20, 10, 10);
        assert_eq!(w.len(), 8);
        assert_eq!(w[0].oos_range, (20, 30));
        assert_eq!(w[1].oos_range, (30, 40));
        assert_eq!(w.last().unwrap().oos_range, (90, 100));
        // n < window → boş.
        assert!(wf_oos_windows(20, 20, 10, 10).is_empty());
        // step=0 → 1'e clamp (sonsuz döngü yok).
        assert!(!wf_oos_windows(100, 20, 10, 0).is_empty());
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
    fn wf_cross_check_counts_windows_and_consistency() {
        // Düşüş serisi + long-only → her pencere zarar → profitable_windows=0, pooled_pf=0.
        let candles: Vec<Candle> = (0..120).map(|i| {
            let c = 200.0 - 0.1 * i as f64;
            Candle { open: c, high: c * 1.004, low: c * 0.996, close: c,
                     volume: 1000.0, symbol: "T".into(), interval: "1h".into(), ..Default::default() }
        }).collect();
        let windows = vec![wnd(0, 50, 4.0, 2.0), wnd(50, 100, 4.0, 2.0)];
        let cc = wf_cross_check(&dir_base_cfg(), &candles, &windows);
        assert_eq!(cc.profitable_windows, 0, "düşüşte hiçbir pencere kârlı değil");
        assert!(cc.pooled_pf >= 0.0 && cc.pooled_pf < 1.0);
        // consistency math.
        let synth = WfCrossCheck { windows: 4, profitable_windows: 3, pooled_pf: 1.2, trades: 40 };
        assert!((synth.consistency() - 0.75).abs() < 1e-9);
        assert_eq!(WfCrossCheck::default().consistency(), 0.0, "işlemsiz → 0");
    }

    #[test]
    fn evaluate_symbol_strategy_gates_on_min_score_and_margin() {
        let pool = vec!["EMA".to_string(), "ICT".to_string(), "RSI".to_string()];
        // PF: EMA=0.68 (edge yok), ICT=1.53 (edge), RSI skorlanamaz (None).
        let score = |name: &str| -> Option<f64> {
            match name { "EMA" => Some(0.68), "ICT" => Some(1.53), _ => None }
        };
        // current yok, min_score=1.0 → en iyi ICT (1.53 ≥ 1.0) atanır; RSI atlanır.
        let (choice, scored) = evaluate_symbol_strategy(&pool, score, None, 0.10, 1.0);
        assert_eq!(choice, StrategyChoice::Assign("ICT".to_string()));
        assert_eq!(scored.len(), 2, "RSI skorlanamadı → atlanmalı");

        // Tüm adaylar PF<1.0 + mevcut YOK → Keep (gürültüde per-symbol override yok).
        let weak = |name: &str| -> Option<f64> { match name { "EMA" => Some(0.7), "ICT" => Some(0.9), _ => None } };
        let (none_choice, _) = evaluate_symbol_strategy(&pool, weak, None, 0.10, 1.0);
        assert_eq!(none_choice, StrategyChoice::Keep, "edge yokken (PF<1.0) atama yapılmamalı");

        // current=ICT(1.53), aday EMA daha düşük → margin'le ICT korunur, yazma yok → Keep.
        let (hold, _) = evaluate_symbol_strategy(&pool, score, Some("ICT"), 0.10, 1.0);
        assert_eq!(hold, StrategyChoice::Keep, "mevcut zaten en iyi + edge'li → Keep (flip-flop yok)");
    }

    #[test]
    fn evaluate_symbol_strategy_demotes_decayed_current() {
        let pool = vec!["EMA".to_string(), "ICT".to_string()];
        // Çürüme: seed=EMA artık 0.7 (<1.0), tek alternatif ICT=0.9 de edge'siz → DEMOTE (kaldır).
        let decayed = |name: &str| -> Option<f64> { match name { "EMA" => Some(0.7), "ICT" => Some(0.9), _ => None } };
        let (d, _) = evaluate_symbol_strategy(&pool, decayed, Some("EMA"), 0.10, 1.0);
        assert_eq!(d, StrategyChoice::Demote,
            "çürüyen mevcut + edge'li aday yok → KALDIR (ölü seed canlıyı sürüklemesin)");

        // Çürüyen seed ama edge'li alternatif var → margin'i beklemeden o stratejiye GEÇ.
        // EMA(seed)=0.7<1.0 çürüdü; ICT=1.4≥1.0 → flip-flop margin ölü seed'i korumaz → Assign(ICT).
        let migrate = |name: &str| -> Option<f64> { match name { "EMA" => Some(0.7), "ICT" => Some(1.4), _ => None } };
        let (m, _) = evaluate_symbol_strategy(&pool, migrate, Some("EMA"), 0.10, 1.0);
        assert_eq!(m, StrategyChoice::Assign("ICT".to_string()),
            "çürüyen seed yerine edge'li adaya margin'siz geçilir");

        // Mevcut listede hiç skorlanamadı (None) → çürümüş sayılır; edge'li aday yoksa Demote.
        let cur_unscorable = |name: &str| -> Option<f64> { match name { "ICT" => Some(0.8), _ => None } };
        let (u, _) = evaluate_symbol_strategy(&pool, cur_unscorable, Some("EMA"), 0.10, 1.0);
        assert_eq!(u, StrategyChoice::Demote, "skorlanamayan mevcut + edge yok → Demote");

        // Aday hiç skorlanamadı (boş scored) → veriden hüküm çıkmaz → Keep (yanlışlıkla demote etme).
        let empty = |_: &str| -> Option<f64> { None };
        let (k, scored) = evaluate_symbol_strategy(&pool, empty, Some("EMA"), 0.10, 1.0);
        assert_eq!(k, StrategyChoice::Keep, "skor yokken çürüme kararı verilmez");
        assert!(scored.is_empty());
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
