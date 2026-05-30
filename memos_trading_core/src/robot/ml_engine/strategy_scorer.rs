// robot/strategy_scorer.rs
//
// UCB1 Contextual Bandit — Strateji × Piyasa Rejimi Skorlama Motoru
//
// Her kapanan işlemde güncellenir. Her EVAL_EVERY işlemde bir özerk karar üretir:
//
//   Kural 1 — ATR eşiği:
//     ATR > %2  → Scalp devre dışı (volatilite çok yüksek)
//     ATR < %1.5 → Scalp yeniden aktif
//
//   Kural 2 — Skor bazlı (min 15 örnek):
//     Rejimde ort_pnl < -$20 VE win_rate < %35 → strateji kapat
//     Rejimde ort_pnl > +$10 VE win_rate > %55 → strateji yeniden aç
//
//   UCB1 öneri:
//     best_strategy_for(regime) → en çok kazandıran ya da en az keşfedilen strateji

// ml_engine/strategy_scorer.rs - UCB1 Tabanlı Otonom Strateji × Rejim Performans Puanlayıcı

use crate::robot::logic::market_regime::AdxRegime;
use crate::robot::scalp_swing::TradeType;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

// --- 1. KONFİGÜRASYON VE SABİTLER ---

const N_TYPES:   usize = 3;  // Regular=0, Scalp=1, Swing=2
const N_REGIMES: usize = 4;  // Ranging=0, Neutral=1, Trending=2, Volatile=3
pub const EVAL_EVERY: u32 = 20; // Her 20 işlemde bir özerk değerlendirme

/// UCB1 ödül normalizasyonu: tipik trade PnL'si ±$50 aralığında → [−1, +1]'e indirge.
const REWARD_SCALE: f64 = 50.0;

const TYPE_NAMES:   [&str; N_TYPES]   = ["Regular", "Scalp", "Swing"];
const REGIME_NAMES: [&str; N_REGIMES] = ["Ranging", "Neutral", "Trending", "Volatile"];

// --- 2. YARDIMCI İNDEKSLER ---

fn type_idx(t: TradeType) -> usize {
    match t {
        TradeType::Regular => 0,
        TradeType::Scalp   => 1,
        TradeType::Swing   => 2,
    }
}

pub fn regime_idx(r: AdxRegime) -> usize {
    match r {
        AdxRegime::Ranging  => 0,
        AdxRegime::Neutral  => 1,
        AdxRegime::Trending => 2,
        AdxRegime::Volatile => 3,
    }
}

// --- 3. UCB1 BANDIT KOLU (ARM) YAPISI ---

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Arm {
    pub n:     u32,
    pub total: f64,
    pub wins:  u32,
}

impl Arm {
    pub fn mean(&self) -> f64 {
        if self.n == 0 { 0.0 } else { self.total / self.n as f64 }
    }
    
    pub fn win_rate(&self) -> f64 {
        if self.n == 0 { 0.5 } else { self.wins as f64 / self.n as f64 }
    }

    /// UCB1 Formülü: mean/SCALE + sqrt(2 ln N / n)
    /// Hem kazancı (Exploitation) hem de keşfi (Exploration) dengeler.
    pub fn ucb1(&self, total_n: u32) -> f64 {
        if self.n == 0 { return f64::MAX; }
        let norm_mean = self.mean() / REWARD_SCALE;
        norm_mean + (2.0 * (total_n as f64 + 1.0).ln() / self.n as f64).sqrt()
    }

    pub fn record(&mut self, pnl: f64) {
        self.n += 1;
        self.total += pnl;
        if pnl > 0.0 { self.wins += 1; }
    }
}

// --- 4. ANA STRATEGY SCORER MOTORU ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyScorer {
    /// Matris yapısı: `arms[strateji_idx][rejim_idx]`
    pub arms: [[Arm; N_REGIMES]; N_TYPES],
    pub total_n:    u32,
    pub eval_count: u32,
    pub last_eval_at: u32,

    // Özerk Kontrol Durumu
    pub scalp_disabled: bool,
    pub swing_disabled: bool,
    pub reg_disabled:   bool,
    pub last_reason:    String,
}

impl Default for StrategyScorer {
    fn default() -> Self {
        Self {
            arms:           Default::default(),
            total_n:        0,
            eval_count:     0,
            last_eval_at:   0,
            scalp_disabled: false,
            swing_disabled: false,
            reg_disabled:   false,
            last_reason:    "Başlatıldı".to_string(),
        }
    }
}

impl StrategyScorer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Kapanan bir işlemin sonucunu otonom kaydeder.
    pub fn record(&mut self, trade_type: TradeType, regime: AdxRegime, pnl: f64) {
        self.arms[type_idx(trade_type)][regime_idx(regime)].record(pnl);
        self.total_n += 1;
    }

    /// Değerlendirme periyodu (EVAL_EVERY) geldi mi?
    pub fn should_evaluate(&self) -> bool {
        self.total_n > 0 && self.total_n.is_multiple_of(EVAL_EVERY)
    }

    /// Özerk Karar Mekanizması: İstatistiksel olarak başarısız kolları budar.
    pub fn evaluate(&mut self, _current_atr_pct: Option<f64>, current_regime: AdxRegime) {
        self.eval_count   += 1;
        self.last_eval_at  = self.total_n;
        self.last_reason   = String::new();
        let mut changes: Vec<String> = vec![];
        let ri = regime_idx(current_regime);

        // Minimum güvenilir örnek sayısı
        const MIN_SAMPLES: u32 = 15;

        for ti in 0..N_TYPES {
            let arm = &self.arms[ti][ri];
            if arm.n < MIN_SAMPLES { continue; }

            // Budama Koşulları: Ortalama PnL < -20 USD ve Kazanma Oranı < %35
            let losing  = arm.mean() < -20.0 && arm.win_rate() < 0.35;
            // Yeniden Etkinleştirme: Ortalama PnL > 10 USD ve Kazanma Oranı > %55
            let winning = arm.mean() >  10.0 && arm.win_rate() > 0.55;

            match ti {
                1 => { // Scalp
                    if losing && !self.scalp_disabled {
                        self.scalp_disabled = true;
                        changes.push(format!("Scalp/{}: Kapatıldı (pnl={:.1})", REGIME_NAMES[ri], arm.mean()));
                    } else if winning && self.scalp_disabled {
                        self.scalp_disabled = false;
                        changes.push(format!("Scalp/{}: Aktif Edildi", REGIME_NAMES[ri]));
                    }
                }
                2 => { // Swing
                    if losing && !self.swing_disabled {
                        self.swing_disabled = true;
                        changes.push(format!("Swing/{}: Kapatıldı (pnl={:.1})", REGIME_NAMES[ri], arm.mean()));
                    } else if winning && self.swing_disabled {
                        self.swing_disabled = false;
                        changes.push(format!("Swing/{}: Aktif Edildi", REGIME_NAMES[ri]));
                    }
                }
                0 => { // Regular
                    if losing && !self.reg_disabled {
                        self.reg_disabled = true;
                        changes.push(format!("Regular/{}: Kapatıldı (pnl={:.1})", REGIME_NAMES[ri], arm.mean()));
                    } else if winning && self.reg_disabled {
                        self.reg_disabled = false;
                        changes.push(format!("Regular/{}: Aktif Edildi", REGIME_NAMES[ri]));
                    }
                }
                _ => {}
            }
        }
        if !changes.is_empty() {
            self.last_reason = changes.join(" | ");
        }
    }

    /// Mevcut rejim için UCB1 skoru en yüksek (en karlı/potansiyelli) motoru önerir.
    pub fn best_strategy_for(&self, regime: AdxRegime) -> TradeType {
        let ri = regime_idx(regime);
        let types = [TradeType::Regular, TradeType::Scalp, TradeType::Swing];
        types.into_iter()
            .enumerate()
            .max_by(|(a, _), (b, _)| {
                let sa = self.arms[*a][ri].ucb1(self.total_n);
                let sb = self.arms[*b][ri].ucb1(self.total_n);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, t)| t)
            .unwrap_or(TradeType::Regular)
    }

    /// Verilen motor tipi otonom olarak engellenmiş mi?
    pub fn is_disabled(&self, t: TradeType) -> bool {
        match t {
            TradeType::Regular => self.reg_disabled,
            TradeType::Scalp   => self.scalp_disabled,
            TradeType::Swing   => self.swing_disabled,
        }
    }

    pub fn summary(&self) -> String {
        let lines: Vec<String> = (0..N_TYPES).flat_map(|ti| {
            (0..N_REGIMES).filter_map(move |ri| {
                let arm = &self.arms[ti][ri];
                if arm.n == 0 { return None; }
                Some(format!("{}/{}: n={} win={:.0}% avg={:.1}", 
                    TYPE_NAMES[ti], REGIME_NAMES[ri], arm.n, arm.win_rate() * 100.0, arm.mean()))
            })
        }).collect();
        if lines.is_empty() { "Eğitim verisi birikiyor...".to_string() } else { lines.join("  |  ") }
    }

    /// Veriyi diskten yükler ve eski/hatalı bayrakları temizler.
    pub fn load(path: &str) -> Self {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        let mut s: Self = serde_json::from_str(&content).unwrap_or_default();
        // Legacy Cleanup: Eski global ATR kurallarını sıfırla
        if s.last_reason.contains("ATR=") {
            s.scalp_disabled = false;
            s.last_reason = "Migrasyon: Global ATR temizlendi".to_string();
        }
        s
    }

    pub fn save(&self, path: &str) {
        if let Some(parent) = Path::new(path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}
