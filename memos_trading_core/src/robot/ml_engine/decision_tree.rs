/// Pure-Rust CART Decision Tree + Gradient Boosted Trees (GBT)
///
/// Kullanım:
///   - DecisionTree: tek ağaç, max_depth ile kontrol edilir
///   - GradientBoostedTrees: n_estimators sığ ağaç ensemble'ı
///
/// Girdi: &[[f64; N]] feature matrisi, &[f64] hedef (regression)
/// Çıktı: f64 tahmin ([-1, +1] aralığında score)

use itertools::Itertools as _;
use super::feature_extractor::FeatureVector;
use super::linear_regressor::N_FEATURES;

// ─── Düğüm ───────────────────────────────────────────────────────────────────

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
                if x[*feature_idx] <= *threshold {
                    left.predict(x)
                } else {
                    right.predict(x)
                }
            }
        }
    }
}

// ─── Yardımcı: MSE ve ortalama ────────────────────────────────────────────────

fn mean(vals: &[f64]) -> f64 {
    if vals.is_empty() { return 0.0; }
    vals.iter().sum::<f64>() / vals.len() as f64
}

fn mse(vals: &[f64]) -> f64 {
    let m = mean(vals);
    vals.iter().map(|v| (v - m).powi(2)).sum::<f64>() / vals.len().max(1) as f64
}

// ─── En iyi bölünmeyi bul ─────────────────────────────────────────────────────

fn best_split(
    data: &[([f64; N_FEATURES], f64)],
    n_features_sample: usize,
) -> Option<(usize, f64)> {
    if data.len() < 4 { return None; }

    let n = data.len();
    let targets: Vec<f64> = data.iter().map(|(_, y)| *y).collect();
    let base_mse = mse(&targets);

    let mut best_gain = 1e-9_f64;
    let mut best_feat = 0usize;
    let mut best_thr  = 0.0f64;

    // Öznitelik alt-örnekleme: rastgele değil, sqrt(N) adımlı deterministik
    let step = (N_FEATURES / n_features_sample).max(1);

    for feat in (0..N_FEATURES).step_by(step) {
        // Threshold adayları: sorted unique değerlerin mid-point'leri (max 20 aday)
        let mut vals: Vec<f64> = data.iter().map(|(x, _)| x[feat]).collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        vals.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

        let step_t = (vals.len() / 20).max(1);
        for wi in (0..vals.len().saturating_sub(1)).step_by(step_t) {
            let thr = (vals[wi] + vals[wi + 1]) / 2.0;

            let (left_y, right_y): (Vec<f64>, Vec<f64>) = data.iter()
                .partition_map(|(x, y)| {
                    if x[feat] <= thr { itertools::Either::Left(*y) }
                    else              { itertools::Either::Right(*y) }
                });

            if left_y.is_empty() || right_y.is_empty() { continue; }

            let gain = base_mse
                - (left_y.len()  as f64 / n as f64) * mse(&left_y)
                - (right_y.len() as f64 / n as f64) * mse(&right_y);

            if gain > best_gain {
                best_gain = gain;
                best_feat = feat;
                best_thr  = thr;
            }
        }
    }

    if best_gain > 1e-9 { Some((best_feat, best_thr)) } else { None }
}

// ─── Ağaç inşası ─────────────────────────────────────────────────────────────

fn build_tree(
    data: &[([f64; N_FEATURES], f64)],
    depth: usize,
    max_depth: usize,
    min_samples: usize,
) -> Node {
    let targets: Vec<f64> = data.iter().map(|(_, y)| *y).collect();
    if depth >= max_depth || data.len() < min_samples {
        return Node::Leaf { value: mean(&targets) };
    }

    let n_feats = ((N_FEATURES as f64).sqrt() as usize).max(1);
    match best_split(data, n_feats) {
        None => Node::Leaf { value: mean(&targets) },
        Some((feat, thr)) => {
            let (left_d, right_d): (Vec<_>, Vec<_>) = data.iter()
                .partition(|(x, _)| x[feat] <= thr);

            if left_d.is_empty() || right_d.is_empty() {
                return Node::Leaf { value: mean(&targets) };
            }

            Node::Split {
                feature_idx: feat,
                threshold:   thr,
                left:  Box::new(build_tree(&left_d, depth + 1, max_depth, min_samples)),
                right: Box::new(build_tree(&right_d, depth + 1, max_depth, min_samples)),
            }
        }
    }
}

// ─── DecisionTree ─────────────────────────────────────────────────────────────

/// Tek CART regression ağacı.
#[derive(Debug, Clone)]
pub struct DecisionTree {
    root:       Option<Node>,
    max_depth:  usize,
    min_samples: usize,
    pub is_trained: bool,
}

impl DecisionTree {
    pub fn new(max_depth: usize) -> Self {
        Self { root: None, max_depth, min_samples: 4, is_trained: false }
    }

    pub fn train(&mut self, data: &[([f64; N_FEATURES], f64)]) {
        if data.len() < self.min_samples { return; }
        self.root = Some(build_tree(data, 0, self.max_depth, self.min_samples));
        self.is_trained = true;
    }

    pub fn predict_raw(&self, x: &[f64; N_FEATURES]) -> f64 {
        self.root.as_ref().map_or(0.0, |n| n.predict(x))
    }

    pub fn predict(&self, fv: &FeatureVector) -> f64 {
        let norm = fv.normalize();
        let arr  = norm.to_array();
        self.predict_raw(&arr)
    }
}

impl Default for DecisionTree {
    fn default() -> Self { Self::new(4) }
}

// ─── Gradient Boosted Trees ───────────────────────────────────────────────────

/// Basit Gradient Boosted Regression Trees (GBRT).
/// Her yineleme önceki toplamın residual'ını öğrenir.
#[derive(Debug, Clone)]
pub struct GradientBoostedTrees {
    trees:          Vec<DecisionTree>,
    learning_rate:  f64,
    n_estimators:   usize,
    max_depth:      usize,
    pub is_trained: bool,
    base_score:     f64,
}

impl GradientBoostedTrees {
    /// * `n_estimators`: ağaç sayısı (3–10 önerilir; hız/doğruluk dengesi)
    /// * `learning_rate`: shrinkage (0.05–0.2 önerilir)
    /// * `max_depth`: ağaç derinliği (2–3 yeterli)
    pub fn new(n_estimators: usize, learning_rate: f64, max_depth: usize) -> Self {
        Self {
            trees:         Vec::with_capacity(n_estimators),
            learning_rate,
            n_estimators,
            max_depth,
            is_trained:   false,
            base_score:   0.0,
        }
    }

    /// Hızlı başlangıç: 5 ağaç, lr=0.1, depth=3
    pub fn with_defaults() -> Self { Self::new(5, 0.1, 3) }

    pub fn train(&mut self, data: &[([f64; N_FEATURES], f64)]) {
        if data.len() < 8 { return; }

        // Base score: hedeflerin ortalaması
        let targets: Vec<f64> = data.iter().map(|(_, y)| *y).collect();
        self.base_score = mean(&targets);

        // Residuals: F_0 = base_score; r_i = y_i - F_0
        let mut residuals: Vec<f64> = targets.iter().map(|y| y - self.base_score).collect();

        self.trees.clear();
        for _ in 0..self.n_estimators {
            let train_data: Vec<([f64; N_FEATURES], f64)> = data.iter()
                .zip(residuals.iter())
                .map(|((x, _), r)| (*x, *r))
                .collect();

            let mut tree = DecisionTree::new(self.max_depth);
            tree.train(&train_data);

            // Yeni residuals: r_i = r_i - lr * tree.predict(x_i)
            for (j, (x, _)) in data.iter().enumerate() {
                residuals[j] -= self.learning_rate * tree.predict_raw(x);
            }

            self.trees.push(tree);
        }
        self.is_trained = true;
    }

    /// Ham tahmin (bias dahil)
    pub fn predict_raw(&self, x: &[f64; N_FEATURES]) -> f64 {
        let boosted: f64 = self.trees.iter()
            .map(|t| self.learning_rate * t.predict_raw(x))
            .sum();
        (self.base_score + boosted).clamp(-1.0, 1.0)
    }

    /// FeatureVector'dan tahmin
    pub fn predict(&self, fv: &FeatureVector) -> f64 {
        let norm = fv.normalize();
        let arr  = norm.to_array();
        self.predict_raw(&arr)
    }

    /// Kaç örnek eğitildi (trees boş değilse trained kabul edilir)
    pub fn is_ready(&self) -> bool { self.is_trained && !self.trees.is_empty() }
}

impl Default for GradientBoostedTrees {
    fn default() -> Self { Self::with_defaults() }
}

// ─── GBT HyperParam Grid Search ───────────────────────────────────────────────

/// Tek bir grid-search sonucu
#[derive(Debug, Clone)]
pub struct GbtTuneResult {
    pub n_estimators:  usize,
    pub learning_rate: f64,
    pub max_depth:     usize,
    /// OOS doğruluk oranı (%) — en iyi parametreyi seçmek için kullanılır
    pub oos_accuracy:  f64,
}

/// GBT hyperparameter grid search
///
/// Veriyi 70/30 train/test olarak böler; her kombinasyon için OOS doğruluk
/// (tahmin yönü doğru mu?) hesaplar ve en iyi sonucu döndürür.
///
/// Izgara küçük tutulur (toplam ~18 kombinasyon) — batch eğitim süresini etkilemez.
pub fn gbt_grid_search(
    data: &[([f64; N_FEATURES], f64)],
) -> Option<GbtTuneResult> {
    if data.len() < 20 { return None; }

    let split     = (data.len() as f64 * 0.70) as usize;
    let train     = &data[..split];
    let test      = &data[split..];
    if test.is_empty() { return None; }

    let n_est_opts:  &[usize] = &[3, 5, 8];
    let lr_opts:     &[f64]   = &[0.05, 0.10, 0.15];
    let depth_opts:  &[usize] = &[2, 3];   // 18 kombinasyon

    let mut best: Option<GbtTuneResult> = None;

    for &n_est in n_est_opts {
        for &lr in lr_opts {
            for &depth in depth_opts {
                let mut gbt = GradientBoostedTrees::new(n_est, lr, depth);
                gbt.train(train);
                if !gbt.is_ready() { continue; }

                let correct = test.iter().filter(|(x, t)| {
                    let pred = gbt.predict_raw(x);
                    (pred > 0.0) == (*t > 0.0)
                }).count();
                let acc = correct as f64 / test.len() as f64 * 100.0;

                let better = best.as_ref().map_or(true, |b| acc > b.oos_accuracy);
                if better {
                    best = Some(GbtTuneResult {
                        n_estimators:  n_est,
                        learning_rate: lr,
                        max_depth:     depth,
                        oos_accuracy:  acc,
                    });
                }
            }
        }
    }

    best
}

// ─── Testler ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::ml_engine::feature_extractor::FeatureExtractor;
    use crate::types::Candle;
    use chrono::Utc;

    fn candles() -> Vec<Candle> {
        let mut p = 100.0f64;
        (0..50).map(|i| {
            p += (i as f64 * 0.3) % 2.0 - 0.5;
            Candle {
                symbol: "T".into(), interval: "1h".into(),
                timestamp: Utc::now() + chrono::Duration::hours(i),
                open: p, high: p + 2.0, low: p - 1.0, close: p + 0.5,
                volume: 1000.0 + i as f64 * 30.0,
            }
        }).collect()
    }

    fn make_data(n: usize) -> Vec<([f64; N_FEATURES], f64)> {
        let c = candles();
        (5..n.min(c.len()))
            .map(|i| {
                let fv = FeatureExtractor::extract(&c[..=i]);
                let arr = fv.normalize().to_array();
                let target = if c[i].close > c[i - 1].close { 1.0 } else { -1.0 };
                (arr, target)
            })
            .collect()
    }

    #[test]
    fn decision_tree_trains_and_predicts() {
        let data = make_data(40);
        let mut dt = DecisionTree::new(3);
        dt.train(&data);
        assert!(dt.is_trained);
        let pred = dt.predict_raw(&data[0].0);
        assert!(pred >= -1.0 && pred <= 1.0);
    }

    #[test]
    fn gbt_trains_and_predicts() {
        let data = make_data(40);
        let mut gbt = GradientBoostedTrees::new(5, 0.1, 2);
        gbt.train(&data);
        assert!(gbt.is_ready());
        let pred = gbt.predict_raw(&data[0].0);
        assert!(pred >= -1.0 && pred <= 1.0);
    }

    #[test]
    fn gbt_feature_vector_predict() {
        let c = candles();
        let fv = FeatureExtractor::extract(&c);
        let data = make_data(40);
        let mut gbt = GradientBoostedTrees::with_defaults();
        gbt.train(&data);
        let score = gbt.predict(&fv);
        assert!(score >= -1.0 && score <= 1.0);
    }
}
