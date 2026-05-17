// robot/ml_engine/trade_classifier.rs - Otonom Deneyimsel Sinyal Filtresi (GNB)
//
// Modernizasyon Standartları:
// 1. Match-Guard Rejim Yönetimi: Dinamik olasılık eşikleri hiyerarşik mühürlendi.
// 2. Fonksiyonel İstatistik: Mean/Variance hesaplamaları fold/iteratör zincirine taşındı.
// 3. Kod Tekrarı Süzgeci: Normalizasyon ve Snapshot işlemleri standardize edildi.
// 4. Panic-Free Filesystem: Dosya işlemleri otonom hata yutma ve logging ile sarmalandı.

use crate::robot::logic::market_regime::AdxRegime;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

// --- 1. SABİTLER ---
const N_FEAT: usize = 7;
const MIN_TRAIN: usize = 20;

// Otonom Rejim Eşikleri
const TH_VOLATILE: f64 = 0.60;
const TH_TRENDING: f64 = 0.52;
const TH_NEUTRAL:  f64 = 0.55;
const TH_RANGING:  f64 = 0.50;

const COLD_MAX_ATR: f64 = 5.0;
const COLD_MIN_RR:  f64 = 1.5;

// --- 2. VERİ MODELLERİ ---

#[derive(Debug, Clone, Copy)]
pub struct ClassifierInput {
    pub hour: u32, pub rsi: f64, pub atr_pct: f64,
    pub vol_ratio: f64, pub trend_dir: f64, pub body_ratio: f64, pub rr: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ClassifierSnapshot {
    pub classifier: TradePatternClassifier,
    pub buffer: Vec<([f64; N_FEAT], f64)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ClassStats {
    mean: [f64; N_FEAT],
    variance: [f64; N_FEAT],
    count: usize,
}

impl ClassStats {
    fn zero() -> Self { Self { mean: [0.0; N_FEAT], variance: [1e-4; N_FEAT], count: 0 } }

    fn log_likelihood(&self, x: &[f64; N_FEAT]) -> f64 {
        x.iter().enumerate().map(|(i, &val)| {
            let var = self.variance[i].max(1e-9);
            let diff = val - self.mean[i];
            -0.5 * (diff * diff / var + var.ln())
        }).sum()
    }
}

// --- 3. ANA MOTOR (CLASSIFIER) ---

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TradePatternClassifier {
    win_stats:  ClassStats,
    loss_stats: ClassStats,
    win_prior:  f64,
    loss_prior: f64,
    pub is_trained: bool,
    pub n_win:  usize,
    pub n_loss: usize,
}

impl TradePatternClassifier {
    /// §90.2: Otonom Eğitim - Kazan/Kaybet örüntülerini mühürler.
    pub fn train(&mut self, samples: &[([f64; N_FEAT], f64)]) {
        if samples.len() < MIN_TRAIN { return; }

        let (wins, losses): (Vec<_>, Vec<_>) = samples.iter()
            .partition(|(_, label)| *label > 0.5);

        match (wins.is_empty(), losses.is_empty()) {
            (false, false) => {
                let win_data: Vec<_> = wins.into_iter().map(|(x, _)| x.clone()).collect();
                let loss_data: Vec<_> = losses.into_iter().map(|(x, _)| x.clone()).collect();

                self.win_stats = Self::compute_stats(&win_data);
                self.loss_stats = Self::compute_stats(&loss_data);
                
                let total = (win_data.len() + loss_data.len()) as f64;
                self.win_prior = win_data.len() as f64 / total;
                self.loss_prior = loss_data.len() as f64 / total;
                self.n_win = win_data.len();
                self.n_loss = loss_data.len();
                self.is_trained = true;
            },
            _ => log::warn!("Yetersiz sınıf çeşitliliği, eğitim askıya alındı."),
        }
    }

    fn compute_stats(data: &[[f64; N_FEAT]]) -> ClassStats {
        let n = data.len() as f64;
        let mean = data.iter().fold([0.0; N_FEAT], |mut acc, x| {
            for i in 0..N_FEAT { acc[i] += x[i]; }
            acc
        }).map(|sum| sum / n);

        let variance = data.iter().fold([0.0; N_FEAT], |mut acc, x| {
            for i in 0..N_FEAT { acc[i] += (x[i] - mean[i]).powi(2); }
            acc
        }).map(|sum| (sum / n).max(1e-6));

        ClassStats { mean, variance, count: data.len() }
    }

    pub fn win_probability(&self, x: &[f64; N_FEAT]) -> f64 {
        if !self.is_trained { return 1.0; }
        let log_win = self.win_prior.ln() + self.win_stats.log_likelihood(x);
        let log_loss = self.loss_prior.ln() + self.loss_stats.log_likelihood(x);
        
        let max = log_win.max(log_loss);
        let p_win = (log_win - max).exp();
        let p_loss = (log_loss - max).exp();
        p_win / (p_win + p_loss)
    }

    /// §90.3: Otonom Rejim Veto Yetkisi
    pub fn allows_entry_for_regime(&self, inp: &ClassifierInput, regime: AdxRegime) -> bool {
        if !self.is_trained { return !self.cold_start_blocks(inp.atr_pct, inp.rr); }

        let threshold = match regime {
            AdxRegime::Volatile => TH_VOLATILE,
            AdxRegime::Trending => TH_TRENDING,
            AdxRegime::Neutral  => TH_NEUTRAL,
            AdxRegime::Ranging  => TH_RANGING,
        };

        self.win_probability(&Self::to_features(inp)) >= threshold
    }

    pub fn cold_start_blocks(&self, atr: f64, rr: f64) -> bool {
        !self.is_trained && (atr > COLD_MAX_ATR || rr < COLD_MIN_RR)
    }

    pub fn to_features(i: &ClassifierInput) -> [f64; N_FEAT] {
        [
            i.hour as f64 / 24.0, (i.rsi / 100.0).clamp(0.0, 1.0),
            (i.atr_pct / 5.0).clamp(0.0, 1.0), (i.vol_ratio / 5.0).clamp(0.0, 1.0),
            i.trend_dir.clamp(0.0, 1.0), i.body_ratio.clamp(0.0, 1.0),
            (i.rr / 10.0).clamp(0.0, 1.0),
        ]
    }

    pub fn record_and_maybe_retrain(&mut self, buf: &mut Vec<([f64; N_FEAT], f64)>, inp: &ClassifierInput, pnl: f64) {
        buf.push((Self::to_features(inp), if pnl > 0.0 { 1.0 } else { 0.0 }));
        if buf.len() >= MIN_TRAIN && buf.len() % 10 == 0 { self.train(buf); }
    }

    pub fn save_snapshot(cls: &Self, buf: &[([f64; N_FEAT], f64)], path: &str) {
        let _ = Path::new(path).parent().map(fs::create_dir_all);
        if let Ok(json) = serde_json::to_string_pretty(&ClassifierSnapshot { classifier: cls.clone(), buffer: buf.to_vec() }) {
            let _ = fs::write(path, json);
        }
    }
}



