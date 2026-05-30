//! Uyarlamalı (öz-ayarlayan) trade parametreleri — `config/adaptive_params.json`
//!
//! # Amaç
//! Geçmiş performans istatistiklerine göre parametreleri otomatik olarak günceller
//! ve diske kaydeder. Böylece her oturum kendi performansından ders çıkarır.
//!
//! # Parametreler
//! | Alan | Varsayılan | Açıklama |
//! |------|-----------|----------|
//! | `short_htf_block`          | true   | HTF Bullish iken SHORT tamamen engelle |
//! | `short_min_composite_score`| 0.40   | SHORT için min kompozit skor eşiği |
//! | `sl_atr_multiplier`        | 1.5    | SL mesafesi = ATR × bu çarpan (0 = sabit %) |
//! | `tp_atr_multiplier`        | 3.0    | TP mesafesi = ATR × bu çarpan (0 = sabit %) |
//! | `max_concurrent_shorts`    | 2      | Aynı anda açık olabilecek SHORT sayısı |
//! | `adx_trend_threshold`      | 25.0   | ADX > bu eşik = güçlü trend, karşı yön engellenir |
//! | `futures_short_min_conf`   | 0.45   | Futures SHORT için min ML confidence |
//! | `trailing_sl_activation_pct`| 3.50  | TSL aktif olmadan önce gerekli min kâr % (>= trailing_pct+1) |
//! | `max_trade_loss_pct`       | 0.50   | Tek trade için max kayıp / sermaye % |
//! | `short_loss_streak_pause`  | 3      | Bu kadar ardışık SHORT kaybından sonra SHORT duraklat |
//! | `long_htf_block`           | true   | HTF Bearish iken LONG tamamen engelle |
//! | `max_daily_sl_per_symbol`  | 2      | Bir sembolde günlük max SL — aşılınca sembol gece yarısına bloke |
//! | `max_consecutive_losses`   | 5      | Global ardışık kayıp sayısı → tüm giriş duraklat |
//!
//! # Otomatik Ayarlama
//! `auto_adjust()` her N trade kapandığında çağrılır:
//! - Win rate < %30 → `futures_short_min_conf` +0.05, `short_min_composite_score` +0.05
//! - Win rate > %55 → Eşikleri kademeli olarak gevşet
//! - R/R < 1.5 → `tp_atr_multiplier` +0.2
//! - Ardışık SHORT kaybı ≥ `short_loss_streak_pause` → `short_htf_block` = true (zorla aç)

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

// ── Yapı ──────────────────────────────────────────────────────────────────────

/// Uyarlamalı trade parametreleri — disk'e kaydedilir, loop her döngüde uygular.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveTradeParams {
    // ── SHORT korumaları ─────────────────────────────────────────────────────
    /// HTF Bullish iken SHORT sinyalini tamamen engelle (htf_filter'dan bağımsız, her zaman aktif).
    #[serde(default = "default_true")]
    pub short_htf_block: bool,

    /// SHORT girişi için min kompozit skor (sinyal güç eşiği).
    /// Strateji rankinginden gelen skor bu değerin altındaysa SELL engellenir.
    #[serde(default = "default_short_min_score")]
    pub short_min_composite_score: f64,

    /// Ardışık bu kadar SHORT kaybından sonra SHORT girişleri otomatik duraklat.
    /// 0 = duraklama yok.
    #[serde(default = "default_short_loss_streak_pause")]
    pub short_loss_streak_pause: u32,

    // ── ATR tabanlı SL/TP ────────────────────────────────────────────────────
    /// SL mesafesi = ATR(14) × bu çarpan. 0.0 = devre dışı (sabit % kullan).
    #[serde(default = "default_sl_atr_mult")]
    pub sl_atr_multiplier: f64,

    /// TP mesafesi = ATR(14) × bu çarpan. 0.0 = devre dışı (sabit % kullan).
    #[serde(default = "default_tp_atr_mult")]
    pub tp_atr_multiplier: f64,

    // ── Pozisyon limitleri ───────────────────────────────────────────────────
    /// Aynı anda açık olabilecek max SHORT pozisyon sayısı.
    /// 0 = sınırsız.
    #[serde(default = "default_max_concurrent_shorts")]
    pub max_concurrent_shorts: u32,

    /// Aynı anda açık olabilecek max LONG pozisyon sayısı (global korelasyon limiti).
    /// 0 = sınırsız. Her sembol en fazla 1 LONG açabilir; bu limit farklı sembollerin
    /// toplam LONG sayısını kısıtlar. Önerilen: 5 (her sembolde 1, toplamda 5 farklı).
    #[serde(default = "default_max_concurrent_longs")]
    pub max_concurrent_longs: u32,

    /// Tek trade'de riske atılabilecek max sermaye yüzdesi.
    /// Örn. 0.5 → sermayenin %0.5'inden fazla risk alınamaz.
    #[serde(default = "default_max_trade_loss_pct")]
    pub max_trade_loss_pct: f64,

    // ── Güç/trend filtreleri ─────────────────────────────────────────────────
    /// ADX bu eşiğin üstündeyse güçlü trend var demektir.
    /// Güçlü trende karşı yönde işlem engellenir.
    #[serde(default = "default_adx_threshold")]
    pub adx_trend_threshold: f64,

    /// Futures SHORT için min ML confidence eşiği (standart eşikten yüksek).
    /// 0.0 = standart eşik geçerli (ek kısıtlama yok).
    #[serde(default = "default_futures_short_min_conf")]
    pub futures_short_min_conf: f64,

    // ── Trailing SL ──────────────────────────────────────────────────────────
    /// Trailing SL, kâr bu yüzdenin üstüne çıkmadan aktif olmaz.
    /// ⚠ MATEMATİKSEL ZORUNLULUK: bu değer >= (trailing_pct + 1.0) olmalı.
    /// Aksi hâlde TSL aktive anında best_price*(1-tpct) < entry → anlık kapanış riski.
    /// position_manager.rs'de `act = raw_act.max(tpct + 1.0)` ile de güvence altına alınmıştır.
    /// Örn. 3.50 → %3.5 kâr olmadan TSL devreye girmez (varsayılan tpct=2.5 için güvenli).
    #[serde(default = "default_trailing_sl_activation_pct")]
    pub trailing_sl_activation_pct: f64,

    // ── LONG korumaları ─────────────────────────────────────────────────────
    /// HTF Bearish iken BUY sinyalini tamamen engelle (short_htf_block'un LONG karşılığı).
    #[serde(default = "default_true")]
    pub long_htf_block: bool,

    /// Bir sembole aynı gün bu kadar SL yenirse o sembol gece yarısına bloke olur.
    /// 0 = devre dışı.
    #[serde(default = "default_max_daily_sl")]
    pub max_daily_sl_per_symbol: u32,

    /// Bu kadar ardışık global kayıptan sonra tüm yeni girişler duraklat (SHORT + LONG).
    /// 0 = devre dışı.
    #[serde(default = "default_max_consecutive_losses")]
    pub max_consecutive_losses: u32,

    // ── Meta ─────────────────────────────────────────────────────────────────
    /// Son otomatik ayarlama UTC zaman damgası (ISO 8601).
    #[serde(default)]
    pub last_adjusted_at: String,

    /// Kaç trade'de bir otomatik ayarlama yapılsın (0 = hiç).
    #[serde(default = "default_adjust_every_n")]
    pub adjust_every_n_trades: u32,

    /// Mevcut ardışık SHORT kaybı sayacı — otomatik ayarlama için izlenir.
    #[serde(default)]
    pub short_loss_streak_current: u32,
}

// ── Serde default fonksiyonları ───────────────────────────────────────────────

fn default_true()                    -> bool  { true }
fn default_short_min_score()         -> f64   { 0.40 }
fn default_short_loss_streak_pause() -> u32   { 5 }
fn default_sl_atr_mult()             -> f64   { 1.0 }
fn default_tp_atr_mult()             -> f64   { 1.2 }
fn default_max_concurrent_shorts()   -> u32   { 2 }
fn default_max_concurrent_longs()    -> u32   { 5 }
fn default_max_trade_loss_pct()      -> f64   { 0.5 }
fn default_adx_threshold()           -> f64   { 25.0 }
fn default_futures_short_min_conf()  -> f64   { 0.45 }
fn default_trailing_sl_activation_pct() -> f64 { 3.50 }
fn default_adjust_every_n()          -> u32   { 20 }
fn default_max_daily_sl()            -> u32   { 2 }
fn default_max_consecutive_losses()  -> u32   { 8 }

impl Default for AdaptiveTradeParams {
    fn default() -> Self {
        Self {
            short_htf_block:            default_true(),
            short_min_composite_score:  default_short_min_score(),
            short_loss_streak_pause:    default_short_loss_streak_pause(),
            sl_atr_multiplier:          default_sl_atr_mult(),
            tp_atr_multiplier:          default_tp_atr_mult(),
            max_concurrent_shorts:      default_max_concurrent_shorts(),
            max_concurrent_longs:       default_max_concurrent_longs(),
            max_trade_loss_pct:         default_max_trade_loss_pct(),
            adx_trend_threshold:        default_adx_threshold(),
            futures_short_min_conf:     default_futures_short_min_conf(),
            trailing_sl_activation_pct: default_trailing_sl_activation_pct(),
            long_htf_block:             default_true(),
            max_daily_sl_per_symbol:    default_max_daily_sl(),
            max_consecutive_losses:     default_max_consecutive_losses(),
            last_adjusted_at:           String::new(),
            adjust_every_n_trades:      default_adjust_every_n(),
            short_loss_streak_current:  0,
        }
    }
}

// ── IO ────────────────────────────────────────────────────────────────────────

impl AdaptiveTradeParams {
    /// Dosyadan yükle. Yoksa varsayılan döner (diske kaydetmez).
    pub fn load(path: &str) -> Self {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Diske kaydet. Birden fazla olası yol dener (binary CWD değişebilir).
    pub fn save(&self, path: &str) {
        if let Some(parent) = Path::new(path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    // ── Otomatik ayarlama ─────────────────────────────────────────────────────

    /// Performans istatistiklerine göre parametreleri güncelle.
    ///
    /// Çağrı koşulu: `session_closed % adjust_every_n_trades == 0 && session_closed > 0`
    ///
    /// # Kurallar
    /// - Win rate < %30  → giriş eşiklerini sıkılaştır
    /// - Win rate > %55  → giriş eşiklerini kademeli gevşet
    /// - R/R < 1.5       → TP mesafesini artır
    /// - Yüksek loss_streak → max_consecutive_losses eşiğini geriye çekme (kilitlenme önle)
    /// - Ortalama kazanç küçükse → trailing_sl_activation_pct düşür
    /// - Stabilize olduysa → eşikleri normalize et
    pub fn auto_adjust(
        &mut self,
        session_closed:    usize,
        session_wins:      usize,
        loss_streak:       usize,
        session_rr:        f64,
        short_loss_streak: u32,
        // Kazanan trade başına ortalama kâr % (ör. 1.2 = %1.2). TSL aktivasyon için.
        avg_win_pct:       f64,
        path:              &str,
    ) {
        // 0 = devre dışı anlamına gelir; 0 gelmişse config bozuk → fallback 20
        let every_n = if self.adjust_every_n_trades == 0 { 20 } else { self.adjust_every_n_trades };
        if session_closed == 0 || !session_closed.is_multiple_of(every_n as usize) { return; }

        let win_rate = session_wins as f64 / session_closed as f64 * 100.0;
        let mut changed = false;

        // ── 1. Win rate çok düşük → filtreleri sıkılaştır ────────────────────
        if win_rate < 30.0 {
            let prev = self.futures_short_min_conf;
            self.futures_short_min_conf = (self.futures_short_min_conf + 0.05).min(0.75);
            let prev2 = self.short_min_composite_score;
            self.short_min_composite_score = (self.short_min_composite_score + 0.05).min(0.80);
            if self.futures_short_min_conf != prev || self.short_min_composite_score != prev2 {
                changed = true;
            }
        }

        // ── 2. Win rate yüksek → filtreleri kademeli gevşet ──────────────────
        if win_rate > 55.0 {
            let prev = self.futures_short_min_conf;
            self.futures_short_min_conf = (self.futures_short_min_conf - 0.02).max(0.20);
            let prev2 = self.short_min_composite_score;
            self.short_min_composite_score = (self.short_min_composite_score - 0.02).max(0.25);
            if self.futures_short_min_conf != prev || self.short_min_composite_score != prev2 {
                changed = true;
            }
        }

        // ── 3. R/R / TP ayarı ────────────────────────────────────────────────
        // Düşük R/R + düşük kazanma oranı → TP çok uzak, ulaşılamıyor → küçült.
        // Düşük R/R + iyi kazanma oranı   → kazanç küçük ama sık, TP hafifçe artır.
        // Yüksek R/R                       → normalize et (küçült).
        if session_rr < 1.5 {
            if win_rate < 40.0 {
                // TP ulaşılamıyor → çarpanı küçült
                let prev = self.tp_atr_multiplier;
                self.tp_atr_multiplier = (self.tp_atr_multiplier - 0.1).max(0.8);
                if self.tp_atr_multiplier != prev { changed = true; }
            } else if win_rate >= 40.0 && self.tp_atr_multiplier < 2.0 {
                // Kazanmalar oluyor ama kazanç küçük → hafifçe artır
                let prev = self.tp_atr_multiplier;
                self.tp_atr_multiplier = (self.tp_atr_multiplier + 0.1).min(2.0);
                if self.tp_atr_multiplier != prev { changed = true; }
            }
        }

        // R/R iyileştiyse → TP çarpanını normalize et
        if session_rr > 2.5 && self.tp_atr_multiplier > 1.5 {
            let prev = self.tp_atr_multiplier;
            self.tp_atr_multiplier = (self.tp_atr_multiplier - 0.1).max(1.2);
            if self.tp_atr_multiplier != prev { changed = true; }
        }

        // ── 4. max_consecutive_losses: streak görülen maksimuma göre ayarla ──
        // Kural: eşik = max(8, gözlemlenen_peak_streak + 3)
        // → sistem hiçbir zaman kendi gördüğü streak'te kilitlenmesin
        // → ama çok yüksek de kalmasın (normalize: streak=0 iken eşik yavaş düşer)
        if loss_streak > 0 {
            let observed_peak = loss_streak.max(short_loss_streak as usize);
            let needed = (observed_peak + 3).max(8) as u32;
            if needed > self.max_consecutive_losses {
                self.max_consecutive_losses = needed.min(20); // max tavan: 20
                changed = true;
            }
        } else if win_rate > 45.0 && self.max_consecutive_losses > 8 {
            // Performans normalleşti → eşiği yavaşça geri getir
            self.max_consecutive_losses = (self.max_consecutive_losses - 1).max(8);
            changed = true;
        }

        // ── 5. trailing_sl_activation_pct: ort. kazanç %'sine göre ayarla ───
        // Kural: TSL aktivasyon = ort_kazanç × 0.40 (kârın %40'ini bekle — daha güvenli)
        // Alt sınır: 2.5% (position_manager'daki tpct+1.0 zorlamasıyla etkin minimum ~3.5%)
        // Üst sınır: 6.0% (çok geç aktivasyon kaybettirir)
        if avg_win_pct > 0.1 {
            let target = (avg_win_pct * 0.40).clamp(2.5, 6.0);
            let current = self.trailing_sl_activation_pct;
            // Kademeli yaklaş — ani değişim olmasın
            if (target - current).abs() > 0.1 {
                let new_val = if target > current {
                    (current + 0.1).min(target)
                } else {
                    (current - 0.1).max(target)
                };
                self.trailing_sl_activation_pct = (new_val * 10.0).round() / 10.0;
                changed = true;
            }
        }

        // ── 6. Ardışık SHORT kaybı — htf_block zorla aç ─────────────────────
        if short_loss_streak >= self.short_loss_streak_pause && self.short_loss_streak_pause > 0
            && !self.short_htf_block {
                self.short_htf_block = true;
                changed = true;
            }
        // SHORT streak temizlendi → htf_block geri kapat (win_rate makul ise)
        if short_loss_streak == 0 && win_rate > 40.0 && self.short_htf_block {
            self.short_htf_block = false;
            changed = true;
        }

        // ── 7. Kayıp serisi sayacını güncelle ─────────────────────────────────
        if loss_streak > self.short_loss_streak_current as usize {
            self.short_loss_streak_current = loss_streak as u32;
        } else if loss_streak == 0 {
            self.short_loss_streak_current = 0;
        }

        if changed {
            self.last_adjusted_at = chrono::Utc::now().to_rfc3339();
            self.save(path);
        }
    }

    /// SHORT ATR-tabanlı SL mesafesi döner.
    /// `atr_pct`: mevcut mum tabanlı ATR yüzdesi (örn. 0.8 → %0.8)
    /// `fallback_pct`: ATR geçersizse kullanılacak sabit SL yüzdesi
    pub fn sl_pct(&self, atr_pct: Option<f64>, fallback_pct: f64) -> f64 {
        if self.sl_atr_multiplier > 0.0 {
            if let Some(atr) = atr_pct.filter(|v| *v > 0.0) {
                return (atr * self.sl_atr_multiplier).max(fallback_pct * 0.5);
            }
        }
        fallback_pct
    }

    /// SHORT ATR-tabanlı TP mesafesi döner.
    pub fn tp_pct(&self, atr_pct: Option<f64>, fallback_pct: f64) -> f64 {
        if self.tp_atr_multiplier > 0.0 {
            if let Some(atr) = atr_pct.filter(|v| *v > 0.0) {
                return (atr * self.tp_atr_multiplier).max(fallback_pct * 0.5);
            }
        }
        fallback_pct
    }
}

// ── Varsayılan JSON içeriği ───────────────────────────────────────────────────

/// Yoksa `config/adaptive_params.json`'ı varsayılan değerlerle oluştur.
pub fn ensure_default_file(path: &str) {
    if !Path::new(path).exists() {
        AdaptiveTradeParams::default().save(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let p = AdaptiveTradeParams::default();
        assert!(p.short_htf_block);
        assert!(p.long_htf_block);
        assert_eq!(p.max_concurrent_shorts, 2);
        assert_eq!(p.max_daily_sl_per_symbol, 2);
        assert_eq!(p.max_consecutive_losses, 8);
        assert!((p.sl_atr_multiplier - 1.0).abs() < 1e-9);
        assert!((p.tp_atr_multiplier - 1.2).abs() < 1e-9);
        assert!((p.trailing_sl_activation_pct - 3.5).abs() < 1e-9); // default 3.5
    }

    #[test]
    fn test_sl_tp_pct() {
        let p = AdaptiveTradeParams::default();
        // ATR varsa çarpan uygulanır
        let sl = p.sl_pct(Some(0.4), 0.5); // 0.4 × 1.0 = 0.4 → fallback max(0.4, 0.5*0.5=0.25) = 0.4
        assert!((sl - 0.4).abs() < 1e-9);
        let tp = p.tp_pct(Some(0.4), 1.2); // (0.4 × 1.2).max(1.2 × 0.5) = 0.48.max(0.6) = 0.6
        assert!((tp - 0.6).abs() < 1e-9);
        // ATR yoksa fallback
        assert_eq!(p.sl_pct(None, 0.5), 0.5);
    }

    #[test]
    fn test_auto_adjust_tightens_on_low_win_rate() {
        let mut p = AdaptiveTradeParams { adjust_every_n_trades: 20, ..Default::default() };
        let prev_conf = p.futures_short_min_conf;
        p.auto_adjust(20, 5, 0, 1.0, 0, 0.0, "/tmp/adaptive_params_test.json");
        // win_rate = 25% < 30% → conf arttı
        assert!(p.futures_short_min_conf > prev_conf);
        let _ = std::fs::remove_file("/tmp/adaptive_params_test.json");
    }
}
