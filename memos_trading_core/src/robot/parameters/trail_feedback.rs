// robot/parameters/trail_feedback.rs — Trailing-stop outcome feedback loop
//
// Phase B (strategy_trail_targets) statik tabloyu kuruyor; Phase C bu tabloyu
// trade gözleminden öğrenip per-(sembol, strateji) düzeltir.
//
// Akış:
//   1) Pozisyon TRAILING_STOP ile kapanır → exit_price, is_long, t_exit yakalanır.
//   2) ~60 saniye sonra current_price kontrol edilir:
//        - LONG: price > exit × (1 + threshold) → "early exit" (trail çok sıkıymış)
//        - SHORT: price < exit × (1 - threshold) → "early exit"
//        - aksi: trail doğruydu (gerçek dönüş yakalandı).
//   3) `record_outcome(was_early)` sayaçları günceller; her N=20 outcome'da bir
//      early_exit_rate'e göre target_override patch'i ayarlar:
//        - rate > 50% → widen ×1.2 (cap 5.0%)
//        - rate < 20% → narrow ×0.9 (floor 0.2%)
//        - aksi: dokunma.
//
// Patch en sıkı/en gevşek uçlarda clamp'lenir; runaway feedback yok.

use serde::{Deserialize, Serialize};

/// Bir (sym, strategy) kombinasyonu için trailing-stop outcome istatistikleri.
/// `target_override` Some olduğunda Phase B default'unu by-pass eder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrailFeedback {
    /// Toplam gözlemlenmiş trailing-stop sayısı.
    pub total_count: u32,
    /// Bunlardan kaç tanesi "early exit" (trail çok sıkı, sonra fiyat dönmedi).
    pub early_count: u32,
    /// Adjustment penceresinde son uygulanan override yüzdesi (entry'den uzaklık).
    /// None = henüz override yok, default tablo geçerli.
    pub target_override: Option<f64>,
    /// Son adjust epoch saniyesi. UI/debug izleme için.
    pub last_adjusted: u64,
}

/// Adjustment pencere boyutu — bu kadar outcome toplanınca bir patch denemesi.
pub const FEEDBACK_WINDOW: u32 = 20;
/// Early-exit threshold: exit fiyatından ne kadar fark "early" sayılır (%0.1).
pub const EARLY_EXIT_THRESHOLD: f64 = 0.001;
/// Adjust katsayıları (multipl/böl).
pub const WIDEN_FACTOR: f64 = 1.2;
pub const NARROW_FACTOR: f64 = 0.9;
/// Patch clamp aralığı — target_pct mantıklı sınırlarda kalır.
pub const TARGET_MIN_PCT: f64 = 0.2;
pub const TARGET_MAX_PCT: f64 = 5.0;
/// Rate eşikleri — adjustment kararı için.
pub const WIDEN_RATE_THRESHOLD: f64 = 0.50;  // >50% early → widen
pub const NARROW_RATE_THRESHOLD: f64 = 0.20; // <20% early → narrow

impl TrailFeedback {
    pub fn new() -> Self { Self::default() }

    /// Tek bir trailing outcome'ı işler. `was_early` = trail tetiklendi ama fiyat
    /// pozisyonun lehine geri döndü (sıkı trail). `base_target` = Phase B'den gelen
    /// strateji default'u (adjustment'ın referansı).
    ///
    /// Döner: target_override değiştiyse Some(new_target), aksi None.
    pub fn record_outcome(&mut self, was_early: bool, base_target: f64) -> Option<f64> {
        self.total_count += 1;
        if was_early { self.early_count += 1; }

        // Adjustment penceresi tamamlandı mı?
        if self.total_count < FEEDBACK_WINDOW { return None; }
        if !self.total_count.is_multiple_of(FEEDBACK_WINDOW) { return None; }

        let rate = self.early_count as f64 / self.total_count as f64;
        // Mevcut override (yoksa base'den başla).
        let current = self.target_override.unwrap_or(base_target);

        let new_target = if rate > WIDEN_RATE_THRESHOLD {
            (current * WIDEN_FACTOR).min(TARGET_MAX_PCT)
        } else if rate < NARROW_RATE_THRESHOLD {
            (current * NARROW_FACTOR).max(TARGET_MIN_PCT)
        } else {
            return None;  // stable range, dokunma
        };

        // No-op koruması (clamp aynı değeri verirse override yazma)
        if (new_target - current).abs() < 1e-6 { return None; }

        let now = crate::core::time::now_epoch_secs();
        self.target_override = Some(new_target);
        self.last_adjusted = now;
        Some(new_target)
    }

    /// Early-exit oranı (0.0..=1.0). Diagnostik için.
    pub fn early_rate(&self) -> f64 {
        if self.total_count == 0 { 0.0 }
        else { self.early_count as f64 / self.total_count as f64 }
    }
}

/// Trail çıkışı sonrası gözlem birimi — close_paper_position queue'ya bunu yazar,
/// processor 60sn sonra olgunluk kontrolü yapıp outcome belirler.
#[derive(Debug, Clone)]
pub struct PendingTrailObservation {
    pub symbol:        String,
    pub strategy:      String,
    pub is_long:       bool,
    pub exit_price:    f64,
    pub exit_epoch:    u64,
}

impl PendingTrailObservation {
    /// Olgunluk: exit'ten en az `min_age_secs` (default 60) geçmiş olmalı.
    pub fn is_mature(&self, min_age_secs: u64) -> bool {
        let now = crate::core::time::now_epoch_secs();
        now.saturating_sub(self.exit_epoch) >= min_age_secs
    }

    /// `current_price` (price_poll sonrası taze) ile outcome hesaplar.
    /// LONG'da exit sonrası fiyat yukarı recovery → early (trail çok sıkıydı).
    /// SHORT'ta exit sonrası fiyat aşağı recovery → early.
    pub fn evaluate(&self, current_price: f64) -> bool {
        if self.exit_price <= 0.0 || current_price <= 0.0 { return false; }
        if self.is_long {
            (current_price - self.exit_price) / self.exit_price > EARLY_EXIT_THRESHOLD
        } else {
            (self.exit_price - current_price) / self.exit_price > EARLY_EXIT_THRESHOLD
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_outcome_widens_target_when_early_rate_high() {
        let mut fb = TrailFeedback::new();
        // 20 outcome, hepsi early → rate 100% > 50% → widen
        for _ in 0..FEEDBACK_WINDOW {
            fb.record_outcome(true, 1.0);
        }
        assert_eq!(fb.total_count, FEEDBACK_WINDOW);
        assert_eq!(fb.early_count, FEEDBACK_WINDOW);
        assert_eq!(fb.target_override, Some(1.2),
            "1.0 × 1.2 = 1.2 widen, gerçek {:?}", fb.target_override);
    }

    #[test]
    fn record_outcome_narrows_when_early_rate_low() {
        let mut fb = TrailFeedback::new();
        // 20 outcome, hepsi doğru çıkış → rate 0% < 20% → narrow
        for _ in 0..FEEDBACK_WINDOW {
            fb.record_outcome(false, 1.0);
        }
        assert_eq!(fb.target_override, Some(0.9),
            "1.0 × 0.9 = 0.9 narrow, gerçek {:?}", fb.target_override);
    }

    #[test]
    fn record_outcome_no_change_in_dead_zone() {
        let mut fb = TrailFeedback::new();
        // 20 outcome, 7 early (35% → dead zone [20%, 50%])
        for i in 0..FEEDBACK_WINDOW {
            fb.record_outcome(i < 7, 1.0);
        }
        assert_eq!(fb.target_override, None, "dead zone'da patch yok");
    }

    #[test]
    fn record_outcome_clamps_at_max() {
        let mut fb = TrailFeedback::new();
        fb.target_override = Some(4.5); // zaten yüksek
        // 20 early → ×1.2 = 5.4 → clamp 5.0
        for _ in 0..FEEDBACK_WINDOW {
            fb.record_outcome(true, 1.0);
        }
        assert_eq!(fb.target_override, Some(TARGET_MAX_PCT),
            "MAX cap 5.0, gerçek {:?}", fb.target_override);
    }

    #[test]
    fn record_outcome_clamps_at_min() {
        let mut fb = TrailFeedback::new();
        fb.target_override = Some(0.21); // floor'a yakın
        // 20 right exit → ×0.9 = 0.189 → clamp 0.2
        for _ in 0..FEEDBACK_WINDOW {
            fb.record_outcome(false, 1.0);
        }
        assert_eq!(fb.target_override, Some(TARGET_MIN_PCT),
            "MIN cap 0.2, gerçek {:?}", fb.target_override);
    }

    #[test]
    fn record_outcome_returns_none_before_window_complete() {
        let mut fb = TrailFeedback::new();
        // 19 outcome (< 20 window) → adjustment yok
        for _ in 0..FEEDBACK_WINDOW - 1 {
            let result = fb.record_outcome(true, 1.0);
            assert!(result.is_none(), "pencere dolmadan dönme");
        }
    }

    #[test]
    fn evaluate_long_early_when_price_recovers_up() {
        let obs = PendingTrailObservation {
            symbol: "ETHUSDT".into(), strategy: "SUPERTREND".into(),
            is_long: true, exit_price: 2000.0, exit_epoch: 0,
        };
        // Fiyat exit'in 0.2% üstüne çıktı → early
        assert!(obs.evaluate(2004.0), "LONG +0.2% → early");
        // Fiyat aşağıya devam → doğru çıkış
        assert!(!obs.evaluate(1995.0), "LONG -0.25% → doğru exit");
    }

    #[test]
    fn evaluate_short_early_when_price_recovers_down() {
        let obs = PendingTrailObservation {
            symbol: "ETHUSDT".into(), strategy: "SUPERTREND".into(),
            is_long: false, exit_price: 2000.0, exit_epoch: 0,
        };
        // Fiyat exit'in 0.2% altına düştü → early (SHORT için kâr yönü)
        assert!(obs.evaluate(1996.0), "SHORT -0.2% → early");
        // Fiyat yukarı devam → doğru çıkış (SHORT zarar yönünde)
        assert!(!obs.evaluate(2005.0), "SHORT +0.25% → doğru exit");
    }

    #[test]
    fn evaluate_rejects_zero_prices() {
        let obs = PendingTrailObservation {
            symbol: "X".into(), strategy: "Y".into(),
            is_long: true, exit_price: 0.0, exit_epoch: 0,
        };
        assert!(!obs.evaluate(100.0), "exit_price=0 → false (sentinel)");
        let obs2 = PendingTrailObservation { exit_price: 100.0, ..obs };
        assert!(!obs2.evaluate(0.0), "current=0 → false");
    }

    #[test]
    fn early_rate_zero_when_no_outcomes() {
        let fb = TrailFeedback::new();
        assert_eq!(fb.early_rate(), 0.0);
    }
}
