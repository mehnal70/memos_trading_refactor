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

use crate::market_regime::AdxRegime;
use crate::robot::scalp_swing::TradeType;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const N_TYPES:   usize = 3;  // Regular=0, Scalp=1, Swing=2
const N_REGIMES: usize = 4;  // Ranging=0, Neutral=1, Trending=2, Volatile=3
pub const EVAL_EVERY: u32 = 20; // Her N işlemde bir özerk değerlendirme

// UCB1 ödül normalizasyonu: tipik trade PnL'si ±$50 aralığında → [−1, +1]'e indirge.
// Standart UCB1 [0,1] ödül varsayar; REWARD_SCALE olmadan exploration terimi domine eder.
const REWARD_SCALE: f64 = 50.0;

const TYPE_NAMES:   [&str; N_TYPES]   = ["Regular", "Scalp", "Swing"];
const REGIME_NAMES: [&str; N_REGIMES] = ["Ranging", "Neutral", "Trending", "Volatile"];

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

// ── UCB1 bandit kolu ──────────────────────────────────────────────────────────

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
    // UCB1 — normalize edilmiş ödül: mean/REWARD_SCALE + sqrt(2 ln N / n)
    // Önceki 10.0 çarpanı exploration'ı aşırı ödüllendiriyor, az örnekli kol her zaman kazanıyordu.
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

// ── Ana yapı ─────────────────────────────────────────────────────────────────

/// UCB1 tabanlı strateji × rejim bandit motoru + özerk strateji kontrolü.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyScorer {
    /// `arms[strateji_idx][rejim_idx]`
    pub arms: [[Arm; N_REGIMES]; N_TYPES],
    pub total_n:    u32,
    pub eval_count: u32,
    pub last_eval_at: u32,

    // ── Özerk kontrol durumu ──────────────────────────────────────────────
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
            last_reason:    String::new(),
        }
    }
}

impl StrategyScorer {
    /// Kapanan işlemi kaydeder; her EVAL_EVERY sonrası `evaluate()` çağrılabilir.
    pub fn record(&mut self, trade_type: TradeType, regime: AdxRegime, pnl: f64) {
        self.arms[type_idx(trade_type)][regime_idx(regime)].record(pnl);
        self.total_n += 1;
    }

    pub fn should_evaluate(&self) -> bool {
        self.total_n > 0 && self.total_n % EVAL_EVERY == 0
    }

    /// Özerk değerlendirme — ATR eşiği + skor eşiği kurallarını uygular.
    /// Değişiklik varsa `last_reason` güncellenir; boş ise değişiklik yok.
    pub fn evaluate(&mut self, current_atr_pct: Option<f64>, current_regime: AdxRegime) {
        self.eval_count   += 1;
        self.last_eval_at  = self.total_n;
        self.last_reason   = String::new();
        let mut changes: Vec<String> = vec![];
        let ri = regime_idx(current_regime);

        // ── Kural 1: ATR volatilite eşiği ────────────────────────────────────
        // NOT: ATR'ye dayalı kalıcı global kapatma kaldırıldı — bir sembolün ATR'si
        // diğer tüm semboller için scalp'i devre dışı bırakıyordu. Volatilite kontrolü
        // artık process_scalp_swing içinde her sembol için yerel olarak yapılır.
        let _ = current_atr_pct;

        // ── Kural 2: Skor bazlı (minimum 15 örnek — 5 çok az, istatistiksel olarak anlamsız) ──
        const MIN_SAMPLES: u32 = 15;
        for ti in 0..N_TYPES {
            let arm = &self.arms[ti][ri];
            if arm.n < MIN_SAMPLES { continue; }
            let losing  = arm.mean() < -20.0 && arm.win_rate() < 0.35;
            let winning = arm.mean() >  10.0 && arm.win_rate() > 0.55;
            match ti {
                1 => { // Scalp
                    if losing && !self.scalp_disabled {
                        self.scalp_disabled = true;
                        changes.push(format!("Scalp/{}: pnl={:.1} win={:.0}% → kapatıldı",
                            REGIME_NAMES[ri], arm.mean(), arm.win_rate() * 100.0));
                    } else if winning && self.scalp_disabled {
                        self.scalp_disabled = false;
                        changes.push(format!("Scalp/{}: pnl={:.1} win={:.0}% → yeniden aktif",
                            REGIME_NAMES[ri], arm.mean(), arm.win_rate() * 100.0));
                    }
                }
                2 => { // Swing
                    if losing && !self.swing_disabled {
                        self.swing_disabled = true;
                        changes.push(format!("Swing/{}: pnl={:.1} win={:.0}% → kapatıldı",
                            REGIME_NAMES[ri], arm.mean(), arm.win_rate() * 100.0));
                    } else if winning && self.swing_disabled {
                        self.swing_disabled = false;
                        changes.push(format!("Swing/{}: pnl={:.1} win={:.0}% → yeniden aktif",
                            REGIME_NAMES[ri], arm.mean(), arm.win_rate() * 100.0));
                    }
                }
                0 => { // Regular
                    if losing && !self.reg_disabled {
                        self.reg_disabled = true;
                        changes.push(format!("Regular/{}: pnl={:.1} win={:.0}% → kapatıldı",
                            REGIME_NAMES[ri], arm.mean(), arm.win_rate() * 100.0));
                    } else if winning && self.reg_disabled {
                        self.reg_disabled = false;
                        changes.push(format!("Regular/{}: pnl={:.1} win={:.0}% → yeniden aktif",
                            REGIME_NAMES[ri], arm.mean(), arm.win_rate() * 100.0));
                    }
                }
                _ => {}
            }
        }
        if !changes.is_empty() {
            self.last_reason = changes.join(" | ");
        }
    }

    /// Mevcut rejimde UCB1 skoru en yüksek stratejiyi öner.
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

    /// Tüm strateji × rejim özeti (log/TUI için).
    pub fn summary(&self) -> String {
        let lines: Vec<String> = (0..N_TYPES).flat_map(|ti| {
            (0..N_REGIMES).filter_map(move |ri| {
                let arm = &self.arms[ti][ri];
                if arm.n == 0 { return None; }
                Some(format!(
                    "{}/{}: n={} win={:.0}% avg={:.1}",
                    TYPE_NAMES[ti], REGIME_NAMES[ri],
                    arm.n, arm.win_rate() * 100.0, arm.mean()
                ))
            })
        }).collect();
        if lines.is_empty() { "Veri yok (ilk değerlendirme için en az 5 işlem gerekli)".to_string() }
        else { lines.join("  |  ") }
    }

    /// Verilen strateji bu anda özerk kontrol tarafından devre dışı mı?
    pub fn is_disabled(&self, t: TradeType) -> bool {
        match t {
            TradeType::Regular => self.reg_disabled,
            TradeType::Scalp   => self.scalp_disabled,
            TradeType::Swing   => self.swing_disabled,
        }
    }

    /// Disk'ten yükle — dosya yoksa varsayılan döner (cold start).
    pub fn load(path: &str) -> Self {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        let mut s: Self = serde_json::from_str(&content).unwrap_or_default();
        // Migrasyon: eski sürümler scalp_disabled'ı global ATR kuralıyla açıyordu.
        // Bu kural kaldırıldı (per-symbol guard'a taşındı). Eski snapshot'tan gelen
        // ATR-kaynaklı disable bayrağını temizle ki tüm semboller engellenmesin.
        if s.last_reason.contains("ATR=") && s.last_reason.contains("Scalp") {
            s.scalp_disabled = false;
            s.last_reason = String::new();
        }
        s
    }

    /// Diske kaydet — config/ yoksa oluştur.
    pub fn save(&self, path: &str) {
        if let Some(parent) = Path::new(path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}
