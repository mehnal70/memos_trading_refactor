//! Destek / Direnç (S/R) Tespiti — Hacim Ağırlıklı Swing Nokta Kümeleme
//!
//! # Algoritma
//! 1. **Swing tespiti** — Her mum için ±`swing_lookback` komşusuna göre
//!    lokal tepe (swing high → direnç adayı) ve dip (swing low → destek adayı) bulunur.
//!    Her noktanın ağırlığı `candle.volume / mean_volume` ile hacim ağırlıklı yapılır.
//!
//! 2. **Kümeleme** — Fiyatı birbirine `cluster_pct` içinde olan noktalar
//!    tek bir bölgeye (zone) birleştirilir; zone orta noktası = hacim ağırlıklı ortalama.
//!
//! 3. **Güç (strength)** — `touch_count × vol_weight`; güçsüz bölgeler atılır.
//!
//! 4. **Bağlam (SrContext)** — Mevcut fiyatın bölgelere uzaklığından
//!    `buy_quality` / `sell_quality` puanları (0–1) türetilir.
//!    Loop bu puanlarla kötü entry noktalarını filtreler.
//!
//! 5. **SL/TP ayarı (opsiyonel)** — `adjust_sl_tp = true` ise yakın destek/direnç
//!    sınırlarından SL ve TP fiyatları hesaplanır; sabit %-tabanlı değerlerin
//!    yerine geçer (daha hassas ve market-aware giriş/çıkış).
//!
//! Tüm fonksiyonlar pure (yan etki yok) — `SrDetector::context()` çağrısıyla
//! robotic_loop.rs'e entegre edilir.

use crate::types::Candle;
use serde::{Deserialize, Serialize};

// ─── Bölge tipi ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneType {
    Support,
    Resistance,
}

// ─── Tek S/R bölgesi ─────────────────────────────────────────────────────────

/// Bir destek veya direnç bölgesi.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrZone {
    /// Bölge alt fiyat sınırı
    pub price_low:   f64,
    /// Bölge üst fiyat sınırı
    pub price_high:  f64,
    /// Hacim ağırlıklı orta nokta
    pub midpoint:    f64,
    pub zone_type:   ZoneType,
    /// touch_count × vol_weight — yüksek = daha güçlü bölge
    pub strength:    f64,
    /// Bu bölgeye ait swing nokta sayısı
    pub touch_count: u32,
    /// Normalize toplam hacim ağırlığı
    pub vol_weight:  f64,
}

impl SrZone {
    /// Verilen fiyat bu bölgenin içinde mi?
    pub fn contains(&self, price: f64) -> bool {
        price >= self.price_low && price <= self.price_high
    }

    /// Fiyatın bölge orta noktasına uzaklığı — pozitif = fiyat üstte.
    pub fn distance_pct(&self, price: f64) -> f64 {
        if self.midpoint > 0.0 {
            (price - self.midpoint) / self.midpoint * 100.0
        } else {
            0.0
        }
    }

    /// TUI gösterimi için özet satır: "DESTEK  95000.0–95500.0  güç=3.2  [×4]"
    pub fn summary(&self) -> String {
        let tag = if self.zone_type == ZoneType::Support { "DESTEK " } else { "DİRENÇ" };
        format!(
            "{} {:.1}–{:.1}  güç={:.1}  [×{}]",
            tag, self.price_low, self.price_high, self.strength, self.touch_count
        )
    }
}

// ─── Mevcut fiyat bağlamı ─────────────────────────────────────────────────────

/// Hesaplanan mevcut fiyat → S/R bölge ilişkisi.
#[derive(Debug, Clone, Default)]
pub struct SrContext {
    /// Fiyatın altındaki en yakın destek bölgesi.
    pub nearest_support:    Option<SrZone>,
    /// Fiyatın üstündeki en yakın direnç bölgesi.
    pub nearest_resistance: Option<SrZone>,
    /// Fiyat bir destek bölgesinin içinde mi?
    pub in_support_zone:    bool,
    /// Fiyat bir direnç bölgesinin içinde mi?
    pub in_resistance_zone: bool,
    /// Destekten uzaklık % (+ = fiyat destek üstünde, - = destek altına düşmüş).
    pub distance_to_support_pct:    f64,
    /// Direnç'ten uzaklık % (+ = fiyat direnç altında, - = direnç üstüne çıktı).
    pub distance_to_resistance_pct: f64,
    /// Alım entry kalite puanı (0.0–1.0); yüksek = destek yakını = iyi.
    pub buy_quality:  f64,
    /// Satım/short entry kalite puanı (0.0–1.0).
    pub sell_quality: f64,
    /// Hesaplanan tüm bölgeler (TUI / debug için).
    pub all_zones:    Vec<SrZone>,
}

impl SrContext {
    /// S/R'dan türetilmiş dinamik SL fiyatı.
    ///
    /// * Long  → en yakın destek bölgesi altı   (price_low  - buffer)
    /// * Short → en yakın direnç bölgesi üstü   (price_high + buffer)
    /// Bölge bulunamazsa fallback_pct %-bazlı SL kullanılır.
    pub fn sl_price(&self, entry: f64, is_long: bool, fallback_pct: f64, buffer_pct: f64) -> f64 {
        if is_long {
            // En güçlü uygun destek: price_low entry'nin altında olmalı
            self.all_zones.iter()
                .filter(|z| z.zone_type == ZoneType::Support && z.price_low < entry)
                .max_by(|a, b| a.strength.partial_cmp(&b.strength).unwrap_or(std::cmp::Ordering::Equal))
                .map(|z| z.price_low * (1.0 - buffer_pct / 100.0))
                .filter(|&sl| sl > 0.0 && sl < entry)
                .unwrap_or_else(|| entry * (1.0 - fallback_pct / 100.0))
        } else {
            // En güçlü uygun direnç: price_high entry'nin üstünde olmalı
            self.all_zones.iter()
                .filter(|z| z.zone_type == ZoneType::Resistance && z.price_high > entry)
                .max_by(|a, b| a.strength.partial_cmp(&b.strength).unwrap_or(std::cmp::Ordering::Equal))
                .map(|z| z.price_high * (1.0 + buffer_pct / 100.0))
                .filter(|&sl| sl > entry)
                .unwrap_or_else(|| entry * (1.0 + fallback_pct / 100.0))
        }
    }

    /// S/R'dan türetilmiş dinamik TP fiyatı.
    ///
    /// * Long  → direncin hemen altı (price_low - buffer)
    /// * Short → desteğin hemen üstü (price_high + buffer)
    pub fn tp_price(&self, entry: f64, is_long: bool, fallback_pct: f64, buffer_pct: f64) -> f64 {
        if is_long {
            self.nearest_resistance.as_ref()
                .map(|z| z.price_low * (1.0 - buffer_pct / 100.0))
                .filter(|&tp| tp > entry)
                .unwrap_or_else(|| entry * (1.0 + fallback_pct / 100.0))
        } else {
            self.nearest_support.as_ref()
                .map(|z| z.price_high * (1.0 + buffer_pct / 100.0))
                .filter(|&tp| tp < entry)
                .unwrap_or_else(|| entry * (1.0 - fallback_pct / 100.0))
        }
    }

    /// Optimum SL/TP çifti: R/R kısıtını karşılayan en uygun S/R bölgelerini seçer.
    ///
    /// Algoritma:
    /// 1. En güçlü uygun bölgeyi SL olarak al (yukarıdaki sl_price gibi).
    /// 2. En yakın TP bölgesini dene; R/R ≥ min_rr sağlanmıyorsa bir sonraki bölgeye geç.
    /// 3. Hiçbir bölge min_rr'ı sağlamıyorsa %-bazlı fallback kullan.
    ///
    /// Döner: `(sl_price, tp_price, actual_rr, log_note)`
    pub fn optimal_sl_tp(
        &self,
        entry:        f64,
        is_long:      bool,
        fallback_sl:  f64,   // %
        fallback_tp:  f64,   // %
        buffer_pct:   f64,   // %
        min_rr:       f64,   // örn. 1.5
    ) -> (f64, f64, f64, String) {
        let sl = self.sl_price(entry, is_long, fallback_sl, buffer_pct);
        let sl_dist = if is_long { (entry - sl).abs() } else { (sl - entry).abs() };
        if sl_dist < 1e-8 {
            let tp = entry * if is_long { 1.0 + fallback_tp / 100.0 } else { 1.0 - fallback_tp / 100.0 };
            return (sl, tp, fallback_tp / fallback_sl, "fallback(sl_dist=0)".into());
        }

        // TP için aday bölgeler: entry'nin ötesindeki bölgeler, uzaklığa göre sıralı
        let tp_zones: Vec<&SrZone> = if is_long {
            let mut v: Vec<&SrZone> = self.all_zones.iter()
                .filter(|z| z.zone_type == ZoneType::Resistance && z.price_low > entry)
                .collect();
            v.sort_by(|a, b| a.price_low.partial_cmp(&b.price_low).unwrap_or(std::cmp::Ordering::Equal));
            v
        } else {
            let mut v: Vec<&SrZone> = self.all_zones.iter()
                .filter(|z| z.zone_type == ZoneType::Support && z.price_high < entry)
                .collect();
            v.sort_by(|a, b| b.price_high.partial_cmp(&a.price_high).unwrap_or(std::cmp::Ordering::Equal));
            v
        };

        // Her TP adayını R/R açısından değerlendir
        for zone in &tp_zones {
            let tp = if is_long {
                zone.price_low * (1.0 - buffer_pct / 100.0)
            } else {
                zone.price_high * (1.0 + buffer_pct / 100.0)
            };
            let tp_dist = if is_long { (tp - entry).abs() } else { (entry - tp).abs() };
            let rr = tp_dist / sl_dist;
            if rr >= min_rr {
                let note = format!("sr_optimal(rr={:.2} sl_str={:.1} tp_str={:.1})",
                    rr,
                    self.all_zones.iter()
                        .filter(|z| (z.price_low - sl / (1.0 - buffer_pct / 100.0)).abs() < sl * 0.01)
                        .map(|z| z.strength).fold(0.0_f64, f64::max),
                    zone.strength
                );
                return (sl, tp, rr, note);
            }
        }

        // Hiçbir bölge yeterli R/R vermiyorsa %-bazlı TP (ama S/R SL'yi koru)
        let tp_pct = (fallback_tp).max(sl_dist / entry * 100.0 * min_rr * 1.1);
        let tp = entry * if is_long { 1.0 + tp_pct / 100.0 } else { 1.0 - tp_pct / 100.0 };
        let rr = (tp_pct / 100.0 * entry) / sl_dist;
        (sl, tp, rr, format!("sr_sl+pct_tp(rr={:.2} tp_zones={})", rr, tp_zones.len()))
    }

    /// S/R tabanlı SL/TP'yi indikatörlerle doğrula ve gerekirse düzelt.
    ///
    /// ## Katmanlar
    /// 1. **ATR(14)** — Volatilite koruyucusu
    ///    - SL mesafesi < 0.8×ATR → genişlet (noise'dan kapanır)
    ///    - SL mesafesi > 3.0×ATR → daralt (çok riskli)
    /// 2. **ADX(14)** — Trend gücü
    ///    - ADX > 25 (trend): TP'ye +0.5×ATR ekle (momentum devam edebilir)
    ///    - ADX < 20 (ranging): TP'yi 2.0×ATR ile sınırla (ortalamaya dönüş)
    /// 3. **BB genişliği** — Ekstra volatilite tamponu
    ///    - BB_width > %4 (yüksek vol): SL'ye +0.3×ATR buffer ekle
    ///
    /// İndikatör verisi yoksa S/R sonucu aynen döner.
    pub fn indicator_adjusted_sl_tp(
        &self,
        candles:     &[Candle],
        entry:       f64,
        is_long:     bool,
        fallback_sl: f64,
        fallback_tp: f64,
        buffer_pct:  f64,
        min_rr:      f64,
    ) -> (f64, f64, f64, String) {
        use crate::robot::indicators::{calculate_atr, calculate_adx, calculate_bollinger};

        // 1. Temel S/R SL/TP
        let (mut sl, mut tp, _rr, sr_note) =
            self.optimal_sl_tp(entry, is_long, fallback_sl, fallback_tp, buffer_pct, min_rr);

        let mut notes: Vec<String> = vec![sr_note];

        // 2. ATR — volatilite uyumu
        if let Some(atr) = calculate_atr(candles, 14) {
            let sl_dist = if is_long { entry - sl } else { sl - entry };

            // SL çok dar → genişlet
            if sl_dist < atr * 0.8 {
                sl = if is_long { entry - atr * 0.8 } else { entry + atr * 0.8 };
                notes.push(format!("atr_sl_floor({:.4})", sl));
            }
            // SL çok geniş → daralt
            if sl_dist > atr * 3.0 {
                sl = if is_long { entry - atr * 3.0 } else { entry + atr * 3.0 };
                notes.push(format!("atr_sl_ceil({:.4})", sl));
            }

            // 3. ADX — ranging piyasada TP daralt (extension kaldırıldı: büyük TP ulaşılamaz)
            if let Some((adx, _, _)) = calculate_adx(candles, 14) {
                if adx < 20.0 {
                    // Ranging: TP max 1.5×ATR mesafede (önceki 2×ATR → 1.5×ATR)
                    let max_tp_dist = atr * 1.5;
                    let tp_dist = if is_long { tp - entry } else { entry - tp };
                    if tp_dist > max_tp_dist {
                        tp = if is_long { entry + max_tp_dist } else { entry - max_tp_dist };
                        notes.push(format!("adx_tp_cap(adx={:.1})", adx));
                    }
                }
            }

            // 4. BB genişliği — yüksek volatilitede SL'ye buffer
            if let Some((upper, _, lower)) = calculate_bollinger(candles, 20, 2.0) {
                let mid = (upper + lower) / 2.0;
                if mid > 0.0 {
                    let bb_width_pct = (upper - lower) / mid * 100.0;
                    if bb_width_pct > 4.0 {
                        // Yüksek vol: SL'ye 0.3×ATR ekstra boşluk
                        sl = if is_long { sl - atr * 0.3 } else { sl + atr * 0.3 };
                        notes.push(format!("bb_sl_buf(bb_w={:.1}%)", bb_width_pct));
                    }
                }
            }
        }

        // Son R/R hesabı
        let sl_dist = if is_long { (entry - sl).abs() } else { (sl - entry).abs() };
        let tp_dist = if is_long { (tp - entry).abs() } else { (entry - tp).abs() };
        let final_rr = if sl_dist > 1e-8 { tp_dist / sl_dist } else { 0.0 };

        // R/R hâlâ min_rr altındaysa TP'yi zorunlu genişlet
        let (sl, tp, final_rr) = if final_rr < min_rr && sl_dist > 1e-8 {
            let new_tp = if is_long { entry + sl_dist * min_rr } else { entry - sl_dist * min_rr };
            notes.push(format!("rr_floor(min={:.1})", min_rr));
            (sl, new_tp, min_rr)
        } else {
            (sl, tp, final_rr)
        };

        (sl, tp, final_rr, notes.join("|"))
    }
}

// ─── Yapılandırma ─────────────────────────────────────────────────────────────

/// S/R dedektörü yapılandırması — JSON'dan hot-reload edilebilir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrDetectorConfig {
    /// Filtre etkin mi?
    pub enabled:          bool,
    /// Swing doğrulama için her yöndeki mum sayısı (önerilen: 3–7).
    pub swing_lookback:   usize,
    /// Bu % içindeki noktalar aynı bölgeye birleştirilir (önerilen: 0.3–1.0).
    pub cluster_pct:      f64,
    /// Bu strength'in altındaki bölgeler atılır (önerilen: 1.0–2.0).
    pub min_strength:     f64,
    /// Saklanacak maksimum bölge sayısı.
    pub max_zones:        usize,
    /// Alım sinyali için minimum quality puanı (0.0 = filtre yok).
    pub min_buy_quality:  f64,
    /// Satım sinyali için minimum quality puanı.
    pub min_sell_quality: f64,
    /// S/R sınırlarına göre SL/TP otomatik hesaplansın mı?
    pub adjust_sl_tp:     bool,
    /// SL/TP hesabında bölge sınırına eklenecek buffer (%).
    pub sl_tp_buffer_pct: f64,
    /// Hacim ağırlığı kullanılsın mı? (false = tüm noktalar eşit ağırlık)
    pub vol_weighted:     bool,
    /// S/R SL'nin global SL'yi en fazla bu % kadar genişletmesine izin ver.
    /// 0.0 = sınırsız. Örn: 1.5 → S/R SL'si %1.5'i geçemez.
    #[serde(default = "SrDetectorConfig::default_max_sl_adjust_pct")]
    pub max_sl_adjust_pct: f64,
    /// S/R TP'sinin bu %'yi geçmesini engelle. 0.0 = sınırsız.
    /// Örn: 15.0 → S/R TP'si %15'i geçemez (ADX/BB genişleme etkisi kısıtlanır).
    #[serde(default = "SrDetectorConfig::default_max_tp_adjust_pct")]
    pub max_tp_adjust_pct: f64,
}

impl SrDetectorConfig {
    fn default_max_sl_adjust_pct() -> f64 { 1.5 }
    fn default_max_tp_adjust_pct() -> f64 { 4.0 }
}

impl Default for SrDetectorConfig {
    fn default() -> Self {
        Self {
            enabled:           true,
            swing_lookback:    5,
            cluster_pct:       0.5,
            min_strength:      1.0,
            max_zones:         8,
            min_buy_quality:   0.0,
            min_sell_quality:  0.0,
            adjust_sl_tp:      true,
            sl_tp_buffer_pct:  0.2,
            vol_weighted:      true,
            max_sl_adjust_pct: 1.5,
            max_tp_adjust_pct: 4.0,
        }
    }
}

// ─── SrDetector ───────────────────────────────────────────────────────────────

pub struct SrDetector {
    pub config: SrDetectorConfig,
}

impl SrDetector {
    pub fn new(config: SrDetectorConfig) -> Self {
        Self { config }
    }

    /// Mum verisinden S/R bölgelerini hesapla (güce göre sıralı, en güçlü önce).
    pub fn detect(&self, candles: &[Candle]) -> Vec<SrZone> {
        let lb = self.config.swing_lookback;
        if candles.len() < lb * 2 + 1 {
            return vec![];
        }

        let mean_vol = mean_volume(candles);

        let mut highs: Vec<(f64, f64)> = vec![]; // (fiyat, hacim_ağırlığı)
        let mut lows:  Vec<(f64, f64)> = vec![];

        for i in lb..(candles.len() - lb) {
            let c = &candles[i];
            let vw = if self.config.vol_weighted && mean_vol > 0.0 {
                c.volume / mean_vol
            } else {
                1.0
            };

            // Swing High: candles[i].high >= tüm [i-lb..i] ve [i+1..i+lb] high'ları
            let is_sh = (1..=lb).all(|j| {
                c.high >= candles[i - j].high && c.high >= candles[i + j].high
            });
            if is_sh { highs.push((c.high, vw)); }

            // Swing Low: candles[i].low <= tüm komşuların low'ları
            let is_sl = (1..=lb).all(|j| {
                c.low <= candles[i - j].low && c.low <= candles[i + j].low
            });
            if is_sl { lows.push((c.low, vw)); }
        }

        let mut zones = Vec::new();
        zones.extend(cluster_points(highs, self.config.cluster_pct, ZoneType::Resistance));
        zones.extend(cluster_points(lows,  self.config.cluster_pct, ZoneType::Support));

        zones.retain(|z| z.strength >= self.config.min_strength);
        zones.sort_by(|a, b| b.strength.partial_cmp(&a.strength)
            .unwrap_or(std::cmp::Ordering::Equal));
        zones.truncate(self.config.max_zones);
        zones
    }

    /// Mevcut fiyat için S/R bağlamını hesapla.
    pub fn context(&self, candles: &[Candle], current_price: f64) -> SrContext {
        let zones = self.detect(candles);

        // En yakın destek: fiyatın altındaki (midpoint ≤ price), en yakın olanı
        let nearest_support = zones.iter()
            .filter(|z| z.zone_type == ZoneType::Support && z.midpoint <= current_price)
            .min_by(|a, b| {
                let da = (current_price - a.midpoint).abs();
                let db = (current_price - b.midpoint).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned();

        // En yakın direnç: fiyatın üstündeki (midpoint ≥ price), en yakın olanı
        let nearest_resistance = zones.iter()
            .filter(|z| z.zone_type == ZoneType::Resistance && z.midpoint >= current_price)
            .min_by(|a, b| {
                let da = (a.midpoint - current_price).abs();
                let db = (b.midpoint - current_price).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned();

        let in_support_zone    = zones.iter().any(|z|
            z.zone_type == ZoneType::Support    && z.contains(current_price));
        let in_resistance_zone = zones.iter().any(|z|
            z.zone_type == ZoneType::Resistance && z.contains(current_price));

        let distance_to_support_pct = nearest_support.as_ref()
            .map(|z| z.distance_pct(current_price))
            .unwrap_or(f64::MAX);
        let distance_to_resistance_pct = nearest_resistance.as_ref()
            .map(|z| z.distance_pct(current_price).abs())
            .unwrap_or(f64::MAX);

        let buy_quality  = compute_buy_quality(
            in_support_zone, in_resistance_zone,
            distance_to_support_pct, distance_to_resistance_pct,
        );
        let sell_quality = compute_sell_quality(
            in_resistance_zone, in_support_zone,
            distance_to_resistance_pct, distance_to_support_pct,
        );

        SrContext {
            nearest_support,
            nearest_resistance,
            in_support_zone,
            in_resistance_zone,
            distance_to_support_pct,
            distance_to_resistance_pct,
            buy_quality,
            sell_quality,
            all_zones: zones,
        }
    }
}

// ─── Yardımcı fonksiyonlar ────────────────────────────────────────────────────

fn mean_volume(candles: &[Candle]) -> f64 {
    if candles.is_empty() { return 1.0; }
    let sum: f64 = candles.iter().map(|c| c.volume).sum();
    sum / candles.len() as f64
}

/// Nokta listesini yakın fiyatlara göre kümelere ayırır, her kümeden bir SrZone üretir.
fn cluster_points(
    mut points: Vec<(f64, f64)>,
    cluster_pct: f64,
    zone_type: ZoneType,
) -> Vec<SrZone> {
    if points.is_empty() { return vec![]; }

    // Fiyata göre sırala
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut zones: Vec<SrZone> = vec![];
    let mut cluster: Vec<(f64, f64)> = vec![points[0]];

    for &(price, vw) in &points[1..] {
        let ref_price = cluster[0].0;
        if ref_price > 0.0 && (price - ref_price).abs() / ref_price * 100.0 <= cluster_pct {
            cluster.push((price, vw));
        } else {
            zones.push(zone_from_cluster(&cluster, zone_type));
            cluster = vec![(price, vw)];
        }
    }
    if !cluster.is_empty() {
        zones.push(zone_from_cluster(&cluster, zone_type));
    }
    zones
}

fn zone_from_cluster(cluster: &[(f64, f64)], zone_type: ZoneType) -> SrZone {
    let total_vw: f64 = cluster.iter().map(|(_, vw)| vw).sum();
    let midpoint = if total_vw > 0.0 {
        cluster.iter().map(|(p, vw)| p * vw).sum::<f64>() / total_vw
    } else {
        cluster.iter().map(|(p, _)| p).sum::<f64>() / cluster.len() as f64
    };

    let prices: Vec<f64> = cluster.iter().map(|(p, _)| *p).collect();
    let raw_low  = prices.iter().cloned().fold(f64::MAX, f64::min);
    let raw_high = prices.iter().cloned().fold(f64::MIN, f64::max);

    // Minimum bölge genişliği: midpoint'in ±0.1%'i
    let half_min = midpoint * 0.001;
    let price_low  = raw_low.min(midpoint - half_min);
    let price_high = raw_high.max(midpoint + half_min);

    let touch_count = cluster.len() as u32;
    let strength    = total_vw * touch_count as f64;

    SrZone {
        price_low,
        price_high,
        midpoint,
        zone_type,
        strength,
        touch_count,
        vol_weight: total_vw,
    }
}

// ─── Kalite puanı hesabı ──────────────────────────────────────────────────────

/// Alım entry kalite puanı.
///
/// | Koşul                              | Puan |
/// |------------------------------------|------|
/// | Direnç bölgesi içinde              | 0.05 |
/// | Destek bölgesi içinde              | 0.90 |
/// | Desteğe < 0.5% uzaklık            | 0.75 |
/// | Desteğe 0.5–1.5% uzaklık          | 0.55 |
/// | Desteğe 1.5–3.0% uzaklık          | 0.35 |
/// | Destek yok / > 3% uzaklık         | 0.15 |
/// | Destek altına düştü (kırılım ?)    | 0.20 |
fn compute_buy_quality(
    in_support:              bool,
    in_resistance:           bool,
    dist_support_pct:        f64,  // + = fiyat destek üstünde, - = altına düştü
    dist_resistance_pct:     f64,  // fiyat direnç altında ise pozitif
) -> f64 {
    // Direnç bölgesindeyiz → kesinlikle alma
    if in_resistance { return 0.05; }
    // Destek bölgesindeyiz → ideal giriş
    if in_support    { return 0.90; }
    // Destek altına düştük → tehlikeli
    if dist_support_pct < 0.0 { return 0.15; }

    // Birincil kriter: destekten ne kadar uzaktayız?
    // Alım için destek yakınlığı kritiktir — destek uzaksa kalite düşer.
    if dist_support_pct == f64::MAX {
        // Destek yok: direnç durumuna göre nötr puan
        return if dist_resistance_pct == f64::MAX { 0.55 }
               else if dist_resistance_pct < 1.5   { 0.20 }
               else { 0.40 };
    }
    if dist_support_pct > 3.0 { return 0.15; } // destekten çok uzak → kötü giriş
    if dist_support_pct > 1.5 { return 0.30; }

    // İkincil kriter: önde ne kadar direnç var?
    if dist_resistance_pct == f64::MAX { return 0.70; }
    if dist_resistance_pct < 0.5  { return 0.10; }
    if dist_resistance_pct < 1.5  { return 0.40; }
    if dist_resistance_pct < 3.0  { return 0.60; }
    0.75
}

/// Satım/short entry kalite puanı — primary kriter önde ne kadar destek var.
fn compute_sell_quality(
    in_resistance:           bool,
    in_support:              bool,
    dist_resistance_pct:     f64,  // + = fiyat direnç altında
    dist_support_pct:        f64,
) -> f64 {
    // Destek bölgesindeyiz → short için kötü
    if in_support    { return 0.05; }
    // Direnç bölgesindeyiz → ideal short girişi
    if in_resistance { return 0.90; }
    // Direnç üstüne çıktık → tehlikeli short
    if dist_resistance_pct < 0.0 { return 0.15; }

    // Ana kriter: önde ne kadar destek var?
    if dist_support_pct == f64::MAX {
        return if dist_resistance_pct == f64::MAX { 0.55 } else { 0.70 };
    }
    if dist_support_pct < 0.5  { return 0.10; }
    if dist_support_pct < 1.5  { return 0.40; }
    if dist_support_pct < 3.0  { return 0.60; }
    0.75
}

// ─── Testler ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Candle;
    use chrono::Utc;

    fn c(high: f64, low: f64, vol: f64) -> Candle {
        let close = (high + low) / 2.0;
        Candle {
            timestamp: Utc::now(),
            open: close, high, low, close,
            volume: vol,
            symbol: "TEST".into(),
            interval: "1h".into(),
        }
    }

    fn baseline_candles(n: usize, base_h: f64, base_l: f64) -> Vec<Candle> {
        (0..n).map(|_| c(base_h, base_l, 1.0)).collect()
    }

    #[test]
    fn detects_swing_high() {
        // 15 tane düz mum, ortada belirgin tepe
        let mut candles = baseline_candles(15, 100.0, 99.0);
        candles[7] = c(115.0, 114.0, 5.0); // swing high
        let det = SrDetector::new(SrDetectorConfig { swing_lookback: 3, ..Default::default() });
        let zones = det.detect(&candles);
        let res_zones: Vec<_> = zones.iter().filter(|z| z.zone_type == ZoneType::Resistance).collect();
        assert!(!res_zones.is_empty(), "direnç bölgesi tespit edilemedi");
        assert!((res_zones[0].midpoint - 115.0).abs() < 1.0);
    }

    #[test]
    fn detects_swing_low() {
        let mut candles = baseline_candles(15, 100.0, 99.0);
        candles[7] = c(86.0, 84.0, 5.0); // swing low
        let det = SrDetector::new(SrDetectorConfig { swing_lookback: 3, ..Default::default() });
        let zones = det.detect(&candles);
        let sup_zones: Vec<_> = zones.iter().filter(|z| z.zone_type == ZoneType::Support).collect();
        assert!(!sup_zones.is_empty(), "destek bölgesi tespit edilemedi");
        // Swing low algoritması c.low=84.0 ekler → midpoint ≈ 84.0
        assert!((sup_zones[0].midpoint - 84.0).abs() < 1.0);
    }

    #[test]
    fn quality_in_support_zone() {
        let q = compute_buy_quality(true, false, 0.1, 5.0);
        assert!(q > 0.8);
    }

    #[test]
    fn quality_in_resistance_blocks_buy() {
        let q = compute_buy_quality(false, true, 3.0, 0.2);
        assert!(q < 0.15);
    }

    #[test]
    fn quality_far_from_support_is_low() {
        let q = compute_buy_quality(false, false, 5.0, 10.0);
        assert!(q < 0.20);
    }

    #[test]
    fn cluster_merges_close_levels() {
        let points = vec![(100.0_f64, 1.0), (100.3, 1.0), (100.5, 1.0), (105.0, 1.0)];
        // cluster_pct = 1.0 → ilk 3 birleşmeli
        let zones = cluster_points(points, 1.0, ZoneType::Support);
        assert_eq!(zones.len(), 2);
    }

    #[test]
    fn sl_price_long_uses_support() {
        let zone = SrZone {
            price_low: 95.0, price_high: 95.5, midpoint: 95.25,
            zone_type: ZoneType::Support, strength: 3.0, touch_count: 3, vol_weight: 3.0,
        };
        let ctx = SrContext {
            nearest_support: Some(zone.clone()),
            all_zones: vec![zone], // sl_price() all_zones'u kullanır
            ..Default::default()
        };
        let sl = ctx.sl_price(100.0, true, 2.0, 0.1);
        // 95.0 * (1 - 0.001) ≈ 94.905
        assert!(sl < 95.0 && sl > 94.0);
    }
}
