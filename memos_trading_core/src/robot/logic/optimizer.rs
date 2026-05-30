/// Tüm strateji isimleri (make_strategy ile eşleşmeli)
use crate::prelude::*;
use crate::core::types::{Candle,Signal};
use crate::robot::strategies::base::Strategy;
pub const ALL_STRATEGY_NAMES: &[&str] = &[
    "MA", "RSI", "MACD", "BB", "DONCHIAN",
    "SUPERTREND", "EMA", "STOCH_RSI", "CCI",
    "STOCHASTIC", "WILLIAMS", "ADX", "VWAP",
    "PRICE_ACTION", "ICT_FVG", "SMC",
    "ICT_OB", "ICT_SWEEP", "ICT_KILLZONE", "ICT_OTE", "ICT_COMPOSITE",
];

// ─── Interval Grubu & Ağırlık Matrisi ────────────────────────────────────────
//
// Stratejiler üç ana gruba ayrılır; her grup belirli interval aralıklarında
// daha güvenilir sinyal üretir. `rank_strategies_for_interval` bu ağırlıkları
// composite skora çarpan olarak uygular → interval'e uygun stratejiler öne çıkar.
//
//  Grup          │ Önerilen interval  │ Mantık
//  ──────────────┼────────────────────┼──────────────────────────────────────
//  Momentum      │ 1m – 15m           │ Osilatörler hızlı gürültüyü yakalar
//  Trend         │ 15m – 4h           │ Trend-takip mumların birikmesini gerektirir
//  Yapısal (HTF) │ 4h – 1d            │ Order block / FVG büyük zaman diliminde geçerli
//  Evrensel      │ Tüm intervallar    │ Sabit ağırlık, herhangi bir interval'de tutarlı
//
// Ağırlık matrisi [0.0, 1.0]:
//
//              │ Scalp  │ Intra  │ Swing
//  ────────────┼────────┼────────┼────────
//  Momentum    │  1.00  │  0.75  │  0.35
//  Trend       │  0.50  │  1.00  │  0.80
//  Yapısal     │  0.20  │  0.55  │  1.00
//  Evrensel    │  0.80  │  0.80  │  0.80

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrategyGroup {
    Momentum,   // OSC: RSI, STOCHASTIC, STOCH_RSI, CCI, WILLIAMS, PRICE_ACTION
    Trend,      // MA, EMA, MACD, BB, SUPERTREND, DONCHIAN, ADX, VWAP
    Structural, // ICT_FVG, SMC
    Universal,  // Herhangi bir interval'de orta ağırlık
}

/// Her strateji adını bir gruba atar.
pub fn strategy_group(name: &str) -> StrategyGroup {
    match name {
        "RSI" | "STOCHASTIC" | "STOCH_RSI" | "CCI" | "WILLIAMS" | "PRICE_ACTION"
            => StrategyGroup::Momentum,
        "MA" | "EMA" | "MACD" | "BB" | "SUPERTREND" | "DONCHIAN" | "ADX" | "VWAP"
            => StrategyGroup::Trend,
        "ICT_FVG" | "SMC" | "ICT_OB" | "ICT_SWEEP" | "ICT_OTE"
            | "ICT_COMPOSITE" | "ICT_KILLZONE"
            => StrategyGroup::Structural,
        _   => StrategyGroup::Universal,
    }
}

/// İnterval string'ini sezgisel olarak üç kategoriye ayırır.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IntervalCategory {
    Scalp,   // 1m, 3m, 5m
    Intra,   // 15m, 30m, 1h, 2h
    Swing,   // 4h, 6h, 8h, 12h, 1d, 3d, 1w
}

pub fn interval_category(interval: &str) -> IntervalCategory {
    match interval {
        "1m" | "3m" | "5m"                         => IntervalCategory::Scalp,
        "15m" | "30m" | "1h" | "2h"                => IntervalCategory::Intra,
        "4h" | "6h" | "8h" | "12h" | "1d" | "3d" | "1w" => IntervalCategory::Swing,
        _                                           => IntervalCategory::Intra, // bilinmeyene orta ağırlık
    }
}

/// Bir strateji grubunun verilen interval kategorisindeki ağırlığını döndürür.
/// Sonuç composite skora çarpan olarak uygulanır.
pub fn interval_weight(group: StrategyGroup, cat: IntervalCategory) -> f64 {
    match (group, cat) {
        (StrategyGroup::Momentum,   IntervalCategory::Scalp) => 1.00,
        (StrategyGroup::Momentum,   IntervalCategory::Intra) => 0.75,
        (StrategyGroup::Momentum,   IntervalCategory::Swing) => 0.35,

        (StrategyGroup::Trend,      IntervalCategory::Scalp) => 0.50,
        (StrategyGroup::Trend,      IntervalCategory::Intra) => 1.00,
        (StrategyGroup::Trend,      IntervalCategory::Swing) => 0.80,

        (StrategyGroup::Structural, IntervalCategory::Scalp) => 0.20,
        (StrategyGroup::Structural, IntervalCategory::Intra) => 0.55,
        (StrategyGroup::Structural, IntervalCategory::Swing) => 1.00,

        (StrategyGroup::Universal,  _)                       => 0.80,
    }
}

/// Çok metrikli strateji değerlendirme skoru.
/// Tek bir win_rate/PF yerine risk-ayarlı birden fazla boyutu birleştirir.
#[derive(Debug, Clone)]
pub struct CompositeScore {
    pub sharpe:        f64,  // ortalama_getiri / getiri_std  (risk-ayarlı getiri)
    pub sortino:       f64,  // ortalama_getiri / negatif_std (sadece kayıp volatilitesi)
    pub win_rate:      f64,  // kazanan işlem oranı 0..1
    pub profit_factor: f64,  // brüt_kâr / brüt_zarar
    pub calmar:        f64,  // toplam_getiri / max_drawdown  (kötü senaryo dayanıklılığı)
    pub composite:     f64,  // ağırlıklı bileşik skor (karşılaştırma için)
    pub trade_count:   usize,
}

impl CompositeScore {
    pub fn zero() -> Self {
        Self { sharpe: 0.0, sortino: 0.0, win_rate: 0.0, profit_factor: 0.0, calmar: 0.0, composite: 0.0, trade_count: 0 }
    }

    /// İşlem getirilerinden tüm metrikleri hesaplar.
    pub fn from_returns(returns: &[f64]) -> Self {
        let n = returns.len();
        if n < 3 { return Self::zero(); }

        let wins: Vec<f64>   = returns.iter().filter(|&&r| r > 0.0).cloned().collect();
        let losses: Vec<f64> = returns.iter().filter(|&&r| r < 0.0).cloned().collect();
        let win_rate  = wins.len() as f64 / n as f64;
        let gross_p   = wins.iter().sum::<f64>();
        let gross_l   = losses.iter().map(|r| r.abs()).sum::<f64>();
        let profit_factor = if gross_l > 0.0 { gross_p / gross_l } else { gross_p.max(0.0) };

        // Sharpe: (ortalama / std_sapma) × √n
        let mean = returns.iter().sum::<f64>() / n as f64;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n as f64;
        let std_dev  = variance.sqrt();
        let sharpe   = if std_dev > 1e-10 { (mean / std_dev) * (n as f64).sqrt() } else { 0.0 };

        // Sortino: sadece negatif sapma
        let downside_var = returns.iter()
            .filter(|&&r| r < 0.0)
            .map(|r| r.powi(2))
            .sum::<f64>() / n as f64;
        let downside_std = downside_var.sqrt();
        let sortino = if downside_std > 1e-10 { (mean / downside_std) * (n as f64).sqrt() } else { 0.0 };

        // Calmar: toplam getiri / max drawdown
        let total_ret = returns.iter().sum::<f64>();
        let max_dd = {
            let mut peak = 0.0_f64;
            let mut equity = 0.0_f64;
            let mut dd = 0.0_f64;
            for &r in returns {
                equity += r;
                if equity > peak { peak = equity; }
                let cur_dd = peak - equity;
                if cur_dd > dd { dd = cur_dd; }
            }
            dd
        };
        let calmar = if max_dd > 1e-10 { total_ret / max_dd } else { total_ret.max(0.0) };

        // Bileşik ağırlıklar: Sharpe %35, Sortino %25, WinRate %20, PF %15, Calmar %5
        let sharpe_n  = (sharpe  / 3.0).clamp(-1.0, 1.0);  // ±3 normalizasyon
        let sortino_n = (sortino / 3.0).clamp(-1.0, 1.0);
        let pf_n      = (profit_factor / 3.0).clamp(0.0, 1.0);
        let calmar_n  = (calmar  / 3.0).clamp(-1.0, 1.0);
        let composite = sharpe_n  * 0.35
                      + sortino_n * 0.25
                      + win_rate  * 0.20
                      + pf_n      * 0.15
                      + calmar_n  * 0.05;

        Self { sharpe, sortino, win_rate, profit_factor, calmar, composite, trade_count: n }
    }
}

/// Tüm stratejileri son N mumda değerlendirir, composite skora göre sıralar.
/// `top_n`: kaç strateji döndürülsün (0 = hepsi).
/// `interval`: verilirse interval grubuna göre ağırlık çarpanı uygulanır (tümdengelem).
pub fn rank_strategies(
    candles:  &[Candle],
    params:   &StrategyParams,
    htf:      Option<&[Candle]>,
    top_n:    usize,
) -> Vec<(String, CompositeScore)> {
    rank_strategies_for_interval(candles, params, htf, top_n, None)
}

/// Interval bazlı ileriye dönük bar sayısı.
/// Kısa interval'larda piyasa daha hızlı hareket eder → daha az bar yeterli.
/// Orta/uzun interval'larda sinyal gecikmesi uzadığından daha fazla bar beklenir.
fn forward_bars_for_interval(interval: &str) -> usize {
    match interval {
        "1m" | "3m"          => 5,
        "5m"                 => 6,
        "15m" | "30m"        => 8,
        "1h" | "2h"          => 6,
        "4h" | "6h" | "8h"  => 6,
        "12h" | "1d" | "3d" => 5,
        _                    => 5,
    }
}

/// 🧠 STRATEJİ FABRİKASI (Srivastava Factory Method Kalıbı)
/// İsmi dize (string) olarak verilen stratejiyi, projenin kurallarına göre
/// otonom çalışabilecek somut nesneye dönüştürür.
///
/// Faz 4 c2: Çözüm artık `StrategyRegistry`'ye delege ediliyor. Process-genelinde
/// tek sefer kurulan `default_registry()` cache'lenir; yeni strateji eklemek
/// için artık burada `match` yazmak gerekmez, `registry::default_registry()`
/// içine satır eklemek yeterli.
pub fn make_strategy_pub(name: &str) -> Box<dyn Strategy + Send + Sync> {
    use std::sync::OnceLock;
    use crate::robot::strategies::StrategyRegistry;

    static REGISTRY: OnceLock<StrategyRegistry> = OnceLock::new();
    let registry = REGISTRY.get_or_init(crate::robot::strategies::default_registry);
    registry.make(name)
}

/// Interval tümdengeline göre ağırlıklı strateji sıralaması.
/// `interval`: "1m", "5m", "1h" vb. — None ise tüm stratejiler eşit ağırlıkta.

pub fn rank_strategies_for_interval(
    candles:   &[Candle],
    params:    &StrategyParams,
    htf:       Option<&[Candle]>,
    top_n:     usize,
    interval:  Option<&str>,
) -> Vec<(String, CompositeScore)> {
    let window       = 200.min(candles.len());
    let forward_bars = interval.map(forward_bars_for_interval).unwrap_or(5);
    if window < 10 { return vec![]; }
    let slice = &candles[candles.len() - window..];

    let int_cat = interval.map(interval_category);

    let mut results: Vec<(String, CompositeScore)> = ALL_STRATEGY_NAMES.iter().map(|&name| {
        let strat = make_strategy_pub(name);
        let returns: Vec<f64> = (0..slice.len().saturating_sub(forward_bars))
            .filter_map(|i| {
                let sub = &slice[..=i];
                if sub.len() < 5 { return None; }
                let sig = strat.generate_signal(sub, params, None, htf).ok()?;
                if matches!(sig, Signal::Hold) { return None; }
                let entry = slice[i].close;
                let exit  = slice[i + forward_bars].close;
                Some(match sig {
                    Signal::Buy  => (exit - entry) / entry,
                    Signal::Sell => (entry - exit) / entry,
                    _            => return None,
                })
            }).collect();
        let mut score = CompositeScore::from_returns(&returns);
        // Interval tümdengeli: gruba göre composite'e çarpan uygula
        if let Some(cat) = int_cat {
            let grp = strategy_group(name);
            let w   = interval_weight(grp, cat);
            score.composite *= w;
        }
        (name.to_string(), score)
    }).collect();

    results.sort_by(|a, b| b.1.composite.partial_cmp(&a.1.composite).unwrap_or(std::cmp::Ordering::Equal));
    if top_n > 0 { results.truncate(top_n); }
    results
}

/// Basit hyperopt: parametre grid search ile en iyi kombinasyonu bulur
pub struct HyperOptimizer;

impl HyperOptimizer {
    pub fn optimize<S: Strategy>(strategy: &S, candles: &[Candle], param_grid: &[StrategyParams]) -> StrategyParams {
        let mut best_score = f64::MIN;
        let mut best_params = &param_grid[0];
        for params in param_grid {
            let score = Self::simulate_score(strategy, candles, params);
            if score > best_score {
                best_score = score;
                best_params = params;
            }
        }
        *best_params
    }

    pub fn simulate_score<S: Strategy>(strategy: &S, candles: &[Candle], params: &StrategyParams) -> f64 {
        Self::composite_score_impl(strategy as &dyn Strategy, candles, params, None)
    }

    /// Trait object versiyonu — robotic_loop param grid + HTF için.
    pub fn simulate_score_dyn(strategy: &dyn Strategy, candles: &[Candle], params: &StrategyParams) -> f64 {
        Self::composite_score_impl(strategy, candles, params, None)
    }

    /// HTF candle'larıyla birlikte composite skor hesabı.
    pub fn simulate_score_htf(strategy: &dyn Strategy, candles: &[Candle], params: &StrategyParams, htf: Option<&[Candle]>) -> f64 {
        Self::composite_score_impl(strategy, candles, params, htf)
    }

    fn composite_score_impl(
        strategy: &dyn Strategy,
        candles:  &[Candle],
        params:   &StrategyParams,
        htf:      Option<&[Candle]>,
    ) -> f64 {
        // Pencere: son 200 mum (120'den büyütüldü — daha temsili örnek kümesi)
        let window = 200.min(candles.len());
        // forward_bars: interval'e göre — kısa tf'de 3 bar, uzun tf'de 8 bar
        // "5 saatlik ileriye bakış" yerine "yaklaşık 2 saat" sabit tutulur
        let forward_bars: usize = if candles.len() >= 2 {
            // Mum sürelerini tahmin etmek için son iki mumu karşılaştır
            let last  = candles[candles.len() - 1].timestamp.timestamp();
            let prev  = candles[candles.len() - 2].timestamp.timestamp();
            let diff  = (last - prev).abs().max(1);
            match diff {
                d if d <= 120   => 5,   // 1m–2m  → 5 bar
                d if d <= 600   => 4,   // 5m      → 4 bar
                d if d <= 1800  => 4,   // 15m–30m → 4 bar
                d if d <= 7200  => 6,   // 1h–2h   → 6 bar
                _               => 8,   // 4h+     → 8 bar
            }
        } else { 5 };
        if window < 40 { return 0.0; }
        let slice = &candles[candles.len() - window..];

        // Walk-Forward Multi-Fold OOS (5 kayan pencere)
        // Her fold: pencere 1/5 kaydırılır → OOS sonraki 1/5 dilim
        // Fold skorları ortalaması → daha güvenilir parametre seçimi
        //
        // Örnek (200 bar, 5 fold, fold_size=40):
        //   fold 0: train=[0..120], oos=[120..160]
        //   fold 1: train=[0..140], oos=[140..180]
        //   fold 2: train=[0..160], oos=[160..200]  (son 3 aktif)
        //   … daha az bar varsa daha az fold
        const N_FOLDS: usize = 3;  // 5 yerine 3 — derleme hızı dengesi
        let fold_size  = (slice.len() / (N_FOLDS + 2)).max(forward_bars + 5);
        let min_train  = fold_size * 2;

        let mut fold_scores: Vec<f64> = Vec::with_capacity(N_FOLDS);

        for fold in 0..N_FOLDS {
            let train_end = min_train + fold * fold_size;
            let oos_end   = (train_end + fold_size).min(slice.len());
            if oos_end <= train_end + forward_bars { continue; }
            let oos_slice = &slice[train_end..oos_end];

            let returns: Vec<f64> = (0..oos_slice.len().saturating_sub(forward_bars))
                .filter_map(|i| {
                    let ctx_end = train_end + i + 1;
                    let sub = &slice[..ctx_end];
                    if sub.len() < 5 { return None; }
                    let sig = strategy.generate_signal(sub, params, None, htf).ok()?;
                    if matches!(sig, Signal::Hold) { return None; }
                    let entry = oos_slice[i].close;
                    let exit  = oos_slice[i + forward_bars].close;
                    Some(match sig {
                        Signal::Buy  => (exit - entry) / entry,
                        Signal::Sell => (entry - exit) / entry,
                        _            => return None,
                    })
                }).collect();

            if !returns.is_empty() {
                fold_scores.push(CompositeScore::from_returns(&returns).composite);
            }
        }

        if fold_scores.is_empty() { return 0.0; }
        // Ağırlıklı ortalama: daha yeni fold (büyük index) daha fazla ağırlık alır
        let total_w: f64 = (1..=fold_scores.len()).map(|i| i as f64).sum();
        fold_scores.iter().enumerate()
            .map(|(i, &s)| s * (i + 1) as f64 / total_w)
            .sum()
    }
}
// robot/optimizer.rs - İleri Seviye ML/AI Optimizasyon Modülü
// Ensemble, Reinforcement Learning, Bayesian Optimization, Meta-Learning şablonları

use crate::robot::ml_engine::{MLModel, FeatureVector};
use crate::robot::ml_engine::hyperopt::HyperOptResult;
use crate::core::types::StrategyParams;

pub struct AdvancedOptimizer;

impl AdvancedOptimizer {
    /// Ensemble model örneği (birden fazla MLModel ile ortalama tahmin)
    pub fn ensemble_predict(models: &[MLModel], features: &FeatureVector) -> f64 {
        if models.is_empty() { return 0.0; }
        models.iter().map(|m| m.predict(features).score).sum::<f64>() / models.len() as f64
    }

    /// Basit RL (Reinforcement Learning) şablonu (dummy)
    pub fn rl_update(_model: &mut MLModel, _reward: f64, _features: &FeatureVector) {
        // Gerçek uygulamada: Q-learning, policy gradient, actor-critic, vs.
        // Dummy: reward pozitifse ağırlıkları hafifçe artır
        if _reward > 0.0 {
            for w in &mut _model.weights {
                *w *= 1.01;
            }
        } else {
            for w in &mut _model.weights {
                *w *= 0.99;
            }
        }
    }

    /// Bayesian Optimization şablonu (dummy)
    pub fn bayesian_optimize(_model: &MLModel, _data: &[FeatureVector], _param_space: &[StrategyParams]) -> HyperOptResult {
        // Gerçek uygulamada: Gaussian Process, acquisition function, vs.
        // Dummy: param_space'in ilkini döndür
        HyperOptResult {
            best_params:   _param_space[0],
            best_score:    0.0,
            best_win_rate: 0.0,
            best_pnl_pct:  0.0,
            best_sharpe:   0.0,
            best_pf:       0.0,
            best_max_dd:   0.0,
            combinations_tested: 0,
            top_results:   vec![],
        }
    }

    /// Meta-Learning şablonu (dummy)
    pub fn meta_learn(_models: &[MLModel], _tasks: &[Vec<FeatureVector>]) -> MLModel {
        // Gerçek uygulamada: MAML, Reptile, few-shot learning, vs.
        // Dummy: İlk modelin kopyası
        if let Some(m) = _models.first() {
            m.clone()
        } else {
            MLModel::new()
        }
    }
}
