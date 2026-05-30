/// Pure-Rust CART Decision Tree + Gradient Boosted Trees (GBT)
///
/// Kullanım:
///   - DecisionTree: tek ağaç, max_depth ile kontrol edilir
///   - GradientBoostedTrees: n_estimators sığ ağaç ensemble'ı
///
/// Girdi: &[[f64; N]] feature matrisi, &[f64] hedef (regression)
/// Çıktı: f64 tahmin ([-1, +1] aralığında score)

// robot/ml_engine/decision_tree.rs - Otonom GBRT Tahmin ve Optimizasyon Motoru

use itertools::Itertools as _;
use crate::core::types::Candle;
use super::feature_extractor::{FeatureExtractor, FeatureVector};
use super::linear_regressor::N_FEATURES;

// --- 1. DÜĞÜM YAPISI ---

#[derive(Debug, Clone)]
enum Node {
    Leaf { value: f64 },
    Split {
        feature_idx: usize,
        threshold:   f64,
        left:        Box<Node>,
        right:       Box<Node>,
    },
}

impl Node {
    fn predict(&self, x: &[f64; N_FEATURES]) -> f64 {
        match self {
            Node::Leaf { value } => *value,
            Node::Split { feature_idx, threshold, left, right } => {
                if x[*feature_idx] <= *threshold { left.predict(x) }
                else { right.predict(x) }
            }
        }
    }
}

// --- 2. YARDIMCI MATEMATİK ---

fn mean(vals: &[f64]) -> f64 {
    if vals.is_empty() { return 0.0; }
    vals.iter().sum::<f64>() / vals.len() as f64
}

fn mse(vals: &[f64]) -> f64 {
    let m = mean(vals);
    vals.iter().map(|v| (v - m).powi(2)).sum::<f64>() / vals.len().max(1) as f64
}

// --- 3. BÖLÜNME VE İNŞA MANTIĞI ---

fn best_split(data: &[([f64; N_FEATURES], f64)], n_features_sample: usize) -> Option<(usize, f64)> {
    if data.len() < 4 { return None; }
    let n = data.len();
    let targets: Vec<f64> = data.iter().map(|(_, y)| *y).collect();
    let base_mse = mse(&targets);

    let mut best_gain = 1e-9;
    let mut best_feat = 0;
    let mut best_thr  = 0.0;

    let step = (N_FEATURES / n_features_sample).max(1);
    for feat in (0..N_FEATURES).step_by(step) {
        let mut vals: Vec<f64> = data.iter().map(|(x, _)| x[feat]).collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        vals.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

        let step_t = (vals.len() / 20).max(1);
        for wi in (0..vals.len().saturating_sub(1)).step_by(step_t) {
            let thr = (vals[wi] + vals[wi + 1]) / 2.0;
            let (left_y, right_y): (Vec<f64>, Vec<f64>) = data.iter()
                .partition_map(|(x, y)| {
                    if x[feat] <= thr { itertools::Either::Left(*y) }
                    else { itertools::Either::Right(*y) }
                });

            if left_y.is_empty() || right_y.is_empty() { continue; }
            let gain = base_mse 
                - (left_y.len() as f64 / n as f64) * mse(&left_y)
                - (right_y.len() as f64 / n as f64) * mse(&right_y);

            if gain > best_gain {
                best_gain = gain; best_feat = feat; best_thr = thr;
            }
        }
    }
    if best_gain > 1e-9 { Some((best_feat, best_thr)) } else { None }
}

fn build_tree(data: &[([f64; N_FEATURES], f64)], depth: usize, max_depth: usize, min_samples: usize) -> Node {
    let targets: Vec<f64> = data.iter().map(|(_, y)| *y).collect();
    if depth >= max_depth || data.len() < min_samples {
        return Node::Leaf { value: mean(&targets) };
    }

    let n_feats = ((N_FEATURES as f64).sqrt() as usize).max(1);
    match best_split(data, n_feats) {
        None => Node::Leaf { value: mean(&targets) },
        Some((feat, thr)) => {
            let (left_d, right_d): (Vec<_>, Vec<_>) = data.iter().partition(|(x, _)| x[feat] <= thr);
            if left_d.is_empty() || right_d.is_empty() { return Node::Leaf { value: mean(&targets) }; }
            Node::Split {
                feature_idx: feat, threshold: thr,
                left: Box::new(build_tree(&left_d, depth + 1, max_depth, min_samples)),
                right: Box::new(build_tree(&right_d, depth + 1, max_depth, min_samples)),
            }
        }
    }
}

// --- 4. PUBLIC API: DECISION TREE & GBRT ---

#[derive(Debug, Clone)]
pub struct DecisionTree {
    root: Option<Node>,
    max_depth: usize,
    min_samples: usize,
    pub is_trained: bool,
}

impl DecisionTree {
    pub fn new(max_depth: usize) -> Self { Self { root: None, max_depth, min_samples: 4, is_trained: false } }
    pub fn train(&mut self, data: &[([f64; N_FEATURES], f64)]) {
        if data.len() < self.min_samples { return; }
        self.root = Some(build_tree(data, 0, self.max_depth, self.min_samples));
        self.is_trained = true;
    }
    pub fn predict_raw(&self, x: &[f64; N_FEATURES]) -> f64 { self.root.as_ref().map_or(0.0, |n| n.predict(x)) }
}

#[derive(Debug, Clone)]
pub struct GradientBoostedTrees {
    trees: Vec<DecisionTree>,
    learning_rate: f64,
    n_estimators: usize,
    max_depth: usize,
    pub is_trained: bool,
    base_score: f64,
}

impl GradientBoostedTrees {
    pub fn new(n_estimators: usize, learning_rate: f64, max_depth: usize) -> Self {
        Self { trees: Vec::with_capacity(n_estimators), learning_rate, n_estimators, max_depth, is_trained: false, base_score: 0.0 }
    }
    pub fn with_defaults() -> Self { Self::new(5, 0.1, 3) }

    pub fn train(&mut self, data: &[([f64; N_FEATURES], f64)]) {
        if data.len() < 8 { return; }
        let targets: Vec<f64> = data.iter().map(|(_, y)| *y).collect();
        self.base_score = mean(&targets);
        let mut residuals: Vec<f64> = targets.iter().map(|y| y - self.base_score).collect();
        self.trees.clear();
        for _ in 0..self.n_estimators {
            let train_data: Vec<_> = data.iter().zip(residuals.iter()).map(|((x, _), r)| (*x, *r)).collect();
            let mut tree = DecisionTree::new(self.max_depth);
            tree.train(&train_data);
            for (j, (x, _)) in data.iter().enumerate() { residuals[j] -= self.learning_rate * tree.predict_raw(x); }
            self.trees.push(tree);
        }
        self.is_trained = true;
    }
    pub fn predict_raw(&self, x: &[f64; N_FEATURES]) -> f64 {
        let boosted: f64 = self.trees.iter().map(|t| self.learning_rate * t.predict_raw(x)).sum();
        (self.base_score + boosted).clamp(-1.0, 1.0)
    }
    pub fn predict(&self, fv: &FeatureVector) -> f64 { self.predict_raw(&fv.normalize().to_array()) }
    pub fn is_ready(&self) -> bool { self.is_trained && !self.trees.is_empty() }
}

// --- 5. GRID SEARCH ---

#[derive(Debug, Clone)]
pub struct GbtTuneResult {
    pub n_estimators:  usize,
    pub learning_rate: f64,
    pub max_depth:     usize,
    pub oos_accuracy:  f64,
}

/// Mum dizisinden GBT eğitim seti üretir.
///
/// Akış: i = window_bars..(len - forward_bars), her i için:
///   - FeatureVector = FeatureExtractor::extract(&candles[i-window_bars..i]) → normalize → to_array
///   - target = sign(forward_return), forward_return = (close[i+forward_bars] - close[i]) / close[i]
///     (sign: yukarı +1, aşağı -1, sıfırsa 0; GBT regresyonu için sürekli bir yön sinyali)
///
/// `window_bars` < 5 veya `forward_bars` < 1 veya `candles.len() < window_bars + forward_bars + 1`
/// olursa boş Vec döner — eğitim için çağıran erken bail edebilsin.
pub fn build_training_set(
    candles: &[Candle],
    window_bars: usize,
    forward_bars: usize,
) -> Vec<([f64; N_FEATURES], f64)> {
    if window_bars < 5 || forward_bars < 1 { return Vec::new(); }
    let need = window_bars + forward_bars + 1;
    if candles.len() < need { return Vec::new(); }

    let mut out = Vec::with_capacity(candles.len().saturating_sub(need));
    let last = candles.len().saturating_sub(forward_bars);
    for i in window_bars..last {
        let entry = candles[i].close;
        if entry <= 0.0 { continue; }
        let exit = candles[i + forward_bars].close;
        let fwd = (exit - entry) / entry;
        let target = if fwd > 0.0 { 1.0 } else if fwd < 0.0 { -1.0 } else { 0.0 };
        let fv = FeatureExtractor::extract(&candles[i - window_bars..i]).normalize();
        out.push((fv.to_array(), target));
    }
    out
}

pub fn gbt_grid_search(data: &[([f64; N_FEATURES], f64)]) -> Option<GbtTuneResult> {
    if data.len() < 20 { return None; }
    let split = (data.len() as f64 * 0.70) as usize;
    let (train, test) = (&data[..split], &data[split..]);
    if test.is_empty() { return None; }

    let n_est_opts = &[3, 5, 8];
    let lr_opts = &[0.05, 0.10, 0.15];
    let depth_opts = &[2, 3];
    let mut best: Option<GbtTuneResult> = None;

    for &n_est in n_est_opts {
        for &lr in lr_opts {
            for &depth in depth_opts {
                let mut gbt = GradientBoostedTrees::new(n_est, lr, depth);
                gbt.train(train);
                if !gbt.is_ready() { continue; }
                let correct = test.iter().filter(|(x, t)| (gbt.predict_raw(x) > 0.0) == (*t > 0.0)).count();
                let acc = correct as f64 / test.len() as f64 * 100.0;
                if best.as_ref().is_none_or(|b| acc > b.oos_accuracy) {
                    best = Some(GbtTuneResult { n_estimators: n_est, learning_rate: lr, max_depth: depth, oos_accuracy: acc });
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod build_set_tests {
    use super::*;

    fn cs(closes: &[f64]) -> Vec<Candle> {
        closes.iter().map(|&c| Candle {
            open: c, high: c + 0.5, low: c - 0.5, close: c, volume: 100.0,
            ..Default::default()
        }).collect()
    }

    #[test]
    fn empty_when_window_or_forward_invalid() {
        let c = cs(&(0..50).map(|i| 100.0 + i as f64).collect::<Vec<_>>());
        assert!(build_training_set(&c, 4, 5).is_empty(), "window<5 → boş");
        assert!(build_training_set(&c, 20, 0).is_empty(), "forward<1 → boş");
    }

    #[test]
    fn empty_when_data_too_short() {
        let c = cs(&[100.0, 101.0, 102.0]);
        assert!(build_training_set(&c, 20, 5).is_empty());
    }

    #[test]
    fn monotonic_up_yields_all_positive_targets() {
        let c = cs(&(0..60).map(|i| 100.0 + i as f64).collect::<Vec<_>>());
        let ds = build_training_set(&c, 20, 5);
        assert!(!ds.is_empty());
        assert!(ds.iter().all(|(_, t)| *t > 0.0),
            "tek yön yukarı → tüm targetler +1 olmalı");
    }

    #[test]
    fn monotonic_down_yields_all_negative_targets() {
        let c = cs(&(0..60).map(|i| 200.0 - i as f64).collect::<Vec<_>>());
        let ds = build_training_set(&c, 20, 5);
        assert!(!ds.is_empty());
        assert!(ds.iter().all(|(_, t)| *t < 0.0));
    }

    #[test]
    fn dataset_size_matches_window_geometry() {
        // 60 mum, window=20, forward=5 → i ∈ [20, 60-5) = [20, 55) → 35 örnek
        let c = cs(&(0..60).map(|i| 100.0 + i as f64).collect::<Vec<_>>());
        let ds = build_training_set(&c, 20, 5);
        assert_eq!(ds.len(), 35);
    }
}
