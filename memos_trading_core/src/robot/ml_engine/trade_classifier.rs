/// Ticaret Örüntüsü Sınıflandırıcısı
///
/// Başarılı geçmiş işlemlerden kazanan/kaybeden örüntüleri öğrenir.
/// Yeni giriş sinyallerini bu filtreden geçirir.
///
/// Öznitelikler (7):
///   [0] hour_norm   — giriş saati / 24.0
///   [1] rsi_norm    — RSI / 100.0
///   [2] atr_norm    — ATR% / 5.0 (0–5% aralığı normalize)
///   [3] vol_ratio   — hacim / ort_hacim, 0–1 (5x clamp)
///   [4] trend_dir   — sma5/sma20 yön, 0–1 (0.5=nötr, >0.5=bullish)
///   [5] body_ratio  — mum gövdesi / toplam aralık, 0–1
///   [6] rr_norm     — TP/SL oranı / 10.0 (0–1)
///
/// Yöntem: Gaussian Naive Bayes
///   - Küçük veri setlerinde (20–200 trade) güvenilir çalışır
///   - Eğitim anında O(n) zaman, tahmin O(1)
///   - Minimum 20 örnek gerektirir, daha azı varsa her sinyale izin verilir

use crate::market_regime::AdxRegime;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Classifier'a beslenen giriş koşullarını özetleyen yapı.
/// atr_pct yüzde cinsinden beklenir (örn. 2.5 = %2.5 ATR).
#[derive(Debug, Clone, Copy)]
pub struct ClassifierInput {
    pub hour:       u32,
    pub rsi:        f64,  // 0–100
    pub atr_pct:    f64,  // ATR yüzdesi (örn. 2.5 = %2.5)
    pub vol_ratio:  f64,  // hacim / ortalama hacim (1.0 = normal)
    pub trend_dir:  f64,  // sma5/sma20 yönü 0–1 (0.5 = nötr)
    pub body_ratio: f64,  // mum gövdesi / (high-low), 0–1
    pub rr:         f64,  // TP/SL oranı (örn. 2.0)
}

impl ClassifierInput {
    /// Geriye dönük uyumluluk: yalnızca temel 3 öznitelikle oluştur.
    pub fn basic(hour: u32, rsi: f64, atr_pct: f64) -> Self {
        Self { hour, rsi, atr_pct, vol_ratio: 1.0, trend_dir: 0.5, body_ratio: 0.5, rr: 2.0 }
    }
}

/// Disk'e kalıcı hale getirilmiş classifier durumu — classifier + eğitim buffer'ı birlikte.
#[derive(Serialize, Deserialize)]
pub struct ClassifierSnapshot {
    pub classifier: TradePatternClassifier,
    pub buffer:     Vec<([f64; N_FEAT], f64)>,
}

const N_FEAT: usize = 7;
const MIN_TRAIN: usize = 20;
const PREDICT_THRESHOLD: f64 = 0.55;

// Cold-start: yeterli örnek yokken ihtiyatlı ATR eşiği
const COLD_START_MAX_ATR: f64 = 5.0;  // ATR % > bu değer → cold-start'ta giriş engellenir (kripto normal ATR ~2-4%)
const COLD_START_MIN_RR:  f64 = 1.5;  // TP/SL oranı < bu değer → cold-start'ta giriş engellenir

// Rejim bazlı P(win) eşikleri
const THRESHOLD_VOLATILE: f64 = 0.60;  // Volatile rejim: en yüksek bariyer
const THRESHOLD_TRENDING: f64 = 0.52;  // Trending: trend yönünde daha kolay
const THRESHOLD_NEUTRAL:  f64 = 0.55;  // Nötr: varsayılan
const THRESHOLD_RANGING:  f64 = 0.50;  // Ranging: mean-reversion → biraz daha cömert

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClassStats {
    mean:     [f64; N_FEAT],
    variance: [f64; N_FEAT],
    count:    usize,
}

impl ClassStats {
    fn zero() -> Self {
        Self { mean: [0.0; N_FEAT], variance: [0.0; N_FEAT], count: 0 }
    }

    /// Gaussian log-likelihood: log P(x | class)
    fn log_likelihood(&self, x: &[f64; N_FEAT]) -> f64 {
        let mut ll = 0.0_f64;
        for i in 0..N_FEAT {
            let v = self.variance[i].max(1e-9);
            let diff = x[i] - self.mean[i];
            ll += -0.5 * (diff * diff / v + v.ln());
        }
        ll
    }
}

/// Gaussian Naive Bayes tabanlı giriş sinyali filtresi.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradePatternClassifier {
    win_stats:  ClassStats,
    loss_stats: ClassStats,
    win_prior:  f64,
    loss_prior: f64,
    pub is_trained: bool,
    pub n_win:  usize,
    pub n_loss: usize,
}

impl Default for TradePatternClassifier {
    fn default() -> Self {
        Self {
            win_stats:  ClassStats::zero(),
            loss_stats: ClassStats::zero(),
            win_prior:  0.5,
            loss_prior: 0.5,
            is_trained: false,
            n_win:  0,
            n_loss: 0,
        }
    }
}

impl TradePatternClassifier {
    /// `samples`: (features, label) where label = 1.0 kazanç, 0.0 kayıp
    pub fn train(&mut self, samples: &[([f64; N_FEAT], f64)]) {
        if samples.len() < MIN_TRAIN { return; }

        let wins:  Vec<[f64; N_FEAT]> = samples.iter().filter(|(_, l)| *l > 0.5).map(|(x, _)| *x).collect();
        let losses: Vec<[f64; N_FEAT]> = samples.iter().filter(|(_, l)| *l <= 0.5).map(|(x, _)| *x).collect();

        if wins.is_empty() || losses.is_empty() { return; }

        self.win_stats  = Self::compute_stats(&wins);
        self.loss_stats = Self::compute_stats(&losses);

        let total = (wins.len() + losses.len()) as f64;
        self.win_prior  = wins.len()  as f64 / total;
        self.loss_prior = losses.len() as f64 / total;
        self.n_win  = wins.len();
        self.n_loss = losses.len();
        self.is_trained = true;
    }

    fn compute_stats(data: &[[f64; N_FEAT]]) -> ClassStats {
        let n = data.len() as f64;
        let mut mean = [0.0f64; N_FEAT];
        for x in data { for i in 0..N_FEAT { mean[i] += x[i]; } }
        for i in 0..N_FEAT { mean[i] /= n; }
        let mut var = [0.0f64; N_FEAT];
        for x in data {
            for i in 0..N_FEAT {
                let d = x[i] - mean[i];
                var[i] += d * d;
            }
        }
        for i in 0..N_FEAT { var[i] = (var[i] / n).max(1e-6); }
        ClassStats { mean, variance: var, count: data.len() }
    }

    /// P(win | features) — 0.0–1.0
    pub fn win_probability(&self, x: &[f64; N_FEAT]) -> f64 {
        if !self.is_trained { return 1.0; }
        let log_win  = self.win_prior.ln()  + self.win_stats.log_likelihood(x);
        let log_loss = self.loss_prior.ln() + self.loss_stats.log_likelihood(x);
        let max = log_win.max(log_loss);
        let p_win  = (log_win  - max).exp();
        let p_loss = (log_loss - max).exp();
        p_win / (p_win + p_loss)
    }

    /// Sinyale izin ver mi? Eğitilmemişse her zaman true döner.
    pub fn allows_entry(&self, inp: &ClassifierInput) -> bool {
        if !self.is_trained { return true; }
        let x = Self::to_features(inp);
        self.win_probability(&x) >= PREDICT_THRESHOLD
    }

    /// Cold-start koruması: classifier henüz eğitilmemişken ihtiyatlı heuristikler uygular.
    /// `atr_pct` yüzde cinsinden (örn. 2.5 = %2.5), `rr` = TP/SL oranı.
    /// `true` döner → giriş engellendi (yüksek ATR veya yetersiz RR)
    pub fn cold_start_blocks(&self, atr_pct: f64, rr: f64) -> bool {
        if self.is_trained { return false; } // eğitildiyse bu guard devre dışı
        atr_pct > COLD_START_MAX_ATR || rr < COLD_START_MIN_RR
    }

    /// Rejim bazlı P(win) filtresi.
    /// Volatile rejimde eşik 0.60, Ranging'de 0.50, diğerlerinde orta değerler.
    pub fn allows_entry_for_regime(&self, inp: &ClassifierInput, regime: AdxRegime) -> bool {
        if !self.is_trained { return true; }
        let threshold = match regime {
            AdxRegime::Volatile => THRESHOLD_VOLATILE,
            AdxRegime::Trending => THRESHOLD_TRENDING,
            AdxRegime::Neutral  => THRESHOLD_NEUTRAL,
            AdxRegime::Ranging  => THRESHOLD_RANGING,
        };
        let x = Self::to_features(inp);
        self.win_probability(&x) >= threshold
    }

    /// Normalize edilmiş 7-öznitelik vektörü oluştur.
    pub fn to_features(inp: &ClassifierInput) -> [f64; N_FEAT] {
        [
            inp.hour as f64 / 24.0,
            inp.rsi.clamp(0.0, 100.0) / 100.0,
            inp.atr_pct.clamp(0.0, 5.0) / 5.0,
            inp.vol_ratio.clamp(0.0, 5.0) / 5.0,
            inp.trend_dir.clamp(0.0, 1.0),
            inp.body_ratio.clamp(0.0, 1.0),
            inp.rr.clamp(0.0, 10.0) / 10.0,
        ]
    }

    /// Kapatılan işlemden eğitim örneği üret ve buffer'a ekle.
    /// Buffer MIN_TRAIN'e ulaşınca (veya ikili sayısı değişince) train() çağrılır.
    pub fn record_and_maybe_retrain(
        &mut self,
        buffer: &mut Vec<([f64; N_FEAT], f64)>,
        inp: &ClassifierInput,
        pnl: f64,
    ) {
        let label = if pnl > 0.0 { 1.0 } else { 0.0 };
        buffer.push((Self::to_features(inp), label));
        // Her 10 yeni örnek sonrası yeniden eğit (en az MIN_TRAIN birikince ilk kez)
        if buffer.len() >= MIN_TRAIN && buffer.len() % 10 == 0 {
            self.train(buffer);
        }
    }

    /// Classifier + buffer'ı birlikte diske kaydet.
    pub fn save_snapshot(classifier: &TradePatternClassifier, buffer: &[([f64; N_FEAT], f64)], path: &str) {
        if let Some(parent) = Path::new(path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        let snap = ClassifierSnapshot { classifier: classifier.clone(), buffer: buffer.to_vec() };
        if let Ok(json) = serde_json::to_string_pretty(&snap) {
            let _ = fs::write(path, json);
        }
    }

    /// Classifier + buffer'ı diskten yükle — dosya yoksa None döner.
    pub fn load_snapshot(path: &str) -> Option<ClassifierSnapshot> {
        let content = fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }
}
