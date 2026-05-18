// src/robot/logic/sr_detector.rs - Srivastava ATP Otonom Pazar Seviye Dedektörü
// Srivastava ATP - İşlevsel Çarklar Odası (Support & Resistance Engine)

use crate::prelude::*;
use serde::{Serialize, Deserialize};

// =============================================================================
// 🛡️ Bölge Tipleri — Destek / Direnç Anayasal Veri Modeli
// =============================================================================

/// Bir S/R bölgesinin tipi (Destek mi Direnç mi).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZoneType {
    Support,
    Resistance,
}

/// Tek bir destek veya direnç bölgesinin anayasal hali.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrZone {
    pub price_low:   f64,
    pub price_high:  f64,
    pub midpoint:    f64,
    pub zone_type:   ZoneType,
    pub strength:    f64,
    pub touch_count: u32,
    pub vol_weight:  f64,
}

impl SrZone {
    /// Fiyat bu bölgenin [price_low, price_high] aralığında mı?
    pub fn contains(&self, price: f64) -> bool {
        price >= self.price_low && price <= self.price_high
    }

    /// Fiyatın bölgenin orta noktasından % uzaklığı (işaretli):
    /// pozitif → fiyat midpoint'in üzerinde (destekten yukarı),
    /// negatif → fiyat midpoint'in altında (dirence kadar yukarı).
    pub fn distance_pct(&self, price: f64) -> f64 {
        if price.abs() < f64::EPSILON { return 0.0; }
        (price - self.midpoint) / price * 100.0
    }
}

/// Belirli bir anlık fiyat için bağlam matrisi:
/// en yakın destek/direnç, bölgenin içinde mi, % uzaklıklar ve entry kalite puanları.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrContext {
    pub nearest_support:            Option<SrZone>,
    pub nearest_resistance:         Option<SrZone>,
    pub in_support_zone:            bool,
    pub in_resistance_zone:         bool,
    pub distance_to_support_pct:    f64,
    pub distance_to_resistance_pct: f64,
    pub buy_quality:                f64,
    pub sell_quality:               f64,
    pub all_zones:                  Vec<SrZone>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrDetectorConfig {
    pub enabled:          bool,
    pub swing_lookback:   usize,
    pub cluster_pct:      f64,
    pub min_strength:     f64,
    pub max_zones:        usize,
    pub min_buy_quality:  f64,
    pub min_sell_quality: f64,
    pub adjust_sl_tp:     bool,
    pub sl_tp_buffer_pct: f64,
    pub vol_weighted:     bool,
    #[serde(default = "SrDetectorConfig::default_max_sl_adjust_pct")]
    pub max_sl_adjust_pct: f64,
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

pub struct SrDetector {
    pub config: SrDetectorConfig,
}

impl SrDetector {
    pub fn new(config: SrDetectorConfig) -> Self {
        Self { config }
    }

    /// Mum verisinden S/R bölgelerini hesaplar (güce göre sıralı, en güçlü önce).
    pub fn detect(&self, candles: &[Candle]) -> Vec<SrZone> {
        let lb = self.config.swing_lookback;
        if candles.len() < lb * 2 + 1 { return vec![]; }

        let mean_vol = mean_volume(candles);
        let mut highs: Vec<(f64, f64)> = vec![]; 
        let mut lows:  Vec<(f64, f64)> = vec![];

        for i in lb..(candles.len() - lb) {
            let c = &candles[i];
            let vw = if self.config.vol_weighted && mean_vol > 0.0 { c.volume / mean_vol } else { 1.0 };

            let is_sh = (1..=lb).all(|j| c.high >= candles[i - j].high && c.high >= candles[i + j].high);
            if is_sh { highs.push((c.high, vw)); }

            let is_sl = (1..=lb).all(|j| c.low <= candles[i - j].low && c.low <= candles[i + j].low);
            if is_sl { lows.push((c.low, vw)); }
        }

        let mut zones = Vec::new();
        zones.extend(cluster_points(highs, self.config.cluster_pct, ZoneType::Resistance));
        zones.extend(cluster_points(lows,  self.config.cluster_pct, ZoneType::Support));

        zones.retain(|z| z.strength >= self.config.min_strength);
        zones.sort_by(|a, b| b.strength.partial_cmp(&a.strength).unwrap_or(std::cmp::Ordering::Equal));
        zones.truncate(self.config.max_zones);
        zones
    }

    /// Mevcut fiyat için anlık S/R bağlam matrisini (`SrContext`) hasat eder.
    pub fn context(&self, candles: &[Candle], current_price: f64) -> SrContext {
        let zones = self.detect(candles);

        let nearest_support = zones.iter()
            .filter(|z| z.zone_type == ZoneType::Support && z.midpoint <= current_price)
            .min_by(|a, b| (current_price - a.midpoint).abs().partial_cmp(&(current_price - b.midpoint).abs()).unwrap_or(std::cmp::Ordering::Equal))
            .cloned();

        let nearest_resistance = zones.iter()
            .filter(|z| z.zone_type == ZoneType::Resistance && z.midpoint >= current_price)
            .min_by(|a, b| (a.midpoint - current_price).abs().partial_cmp(&(b.midpoint - current_price).abs()).unwrap_or(std::cmp::Ordering::Equal))
            .cloned();

        let in_support_zone    = zones.iter().any(|z| z.zone_type == ZoneType::Support    && z.contains(current_price));
        let in_resistance_zone = zones.iter().any(|z| z.zone_type == ZoneType::Resistance && z.contains(current_price));

        let distance_to_support_pct = nearest_support.as_ref().map(|z| z.distance_pct(current_price)).unwrap_or(f64::MAX);
        let distance_to_resistance_pct = nearest_resistance.as_ref().map(|z| z.distance_pct(current_price).abs()).unwrap_or(f64::MAX);

        // --- 🧬 6. KISIM: SİZİN ÖZGÜN KALİTE MATEMATİĞİNİZ BURAYA ENJEKTE EDİLDİ ---
        let buy_quality  = compute_buy_quality(in_support_zone, in_resistance_zone, distance_to_support_pct, distance_to_resistance_pct);
        let sell_quality = compute_sell_quality(in_resistance_zone, in_support_zone, distance_to_resistance_pct, distance_to_support_pct);

        SrContext {
            nearest_support, nearest_resistance, in_support_zone, in_resistance_zone,
            distance_to_support_pct, distance_to_resistance_pct, buy_quality, sell_quality, all_zones: zones,
        }
    }
}

// =============================================================================
// --- İÇ YARDIMCI MATEMATİKSEL İŞÇİLER (SİZİN EKSİKSİZ METOTLARINIZ) ---
// =============================================================================

fn mean_volume(candles: &[Candle]) -> f64 {
    if candles.is_empty() { return 1.0; }
    candles.iter().map(|c| c.volume).sum::<f64>() / candles.len() as f64
}

/// Alım entry kalite puanı — destek yakınlığı durumlarına göre süzgeç
fn compute_buy_quality(
    in_support:              bool,
    in_resistance:           bool,
    dist_support_pct:        f64,  
    dist_resistance_pct:     f64,  
) -> f64 {
    if in_resistance { return 0.05; }
    if in_support    { return 0.90; }
    if dist_support_pct < 0.0 { return 0.15; }

    if dist_support_pct == f64::MAX {
        return if dist_resistance_pct == f64::MAX { 0.55 }
               else if dist_resistance_pct < 1.5   { 0.20 }
               else { 0.40 };
    }
    if dist_support_pct > 3.0 { return 0.15; } 
    if dist_support_pct > 1.5 { return 0.30; }

    if dist_resistance_pct == f64::MAX { return 0.70; }
    if dist_resistance_pct < 0.5  { return 0.10; }
    if dist_resistance_pct < 1.5  { return 0.40; }
    if dist_resistance_pct < 3.0  { return 0.60; }
    0.75
}

/// Satım/short entry kalite puanı — direnç yakınlığı durumlarına göre süzgeç
fn compute_sell_quality(
    in_resistance:           bool,
    in_support:              bool,
    dist_resistance_pct:     f64,  
    dist_support_pct:        f64,
) -> f64 {
    if in_support    { return 0.05; }
    if in_resistance { return 0.90; }
    if dist_resistance_pct < 0.0 { return 0.15; }

    if dist_support_pct == f64::MAX {
        return if dist_resistance_pct == f64::MAX { 0.55 } else { 0.70 };
    }
    if dist_support_pct < 0.5  { return 0.10; }
    if dist_support_pct < 1.5  { return 0.40; }
    if dist_support_pct < 3.0  { return 0.60; }
    0.75
}

fn cluster_points(mut points: Vec<(f64, f64)>, cluster_pct: f64, zone_type: ZoneType) -> Vec<SrZone> {
    if points.is_empty() { return vec![]; }
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
    if !cluster.is_empty() { zones.push(zone_from_cluster(&cluster, zone_type)); }
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

    let half_min = midpoint * 0.001;
    let price_low  = raw_low.min(midpoint - half_min);
    let price_high = raw_high.max(midpoint + half_min);
    let touch_count = cluster.len() as u32;

    SrZone { price_low, price_high, midpoint, zone_type, strength: total_vw * touch_count as f64, touch_count, vol_weight: total_vw }
}
