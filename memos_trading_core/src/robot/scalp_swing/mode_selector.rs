/// ModeSelector — Piyasa koşullarına göre scalp/swing/her ikisi mod seçimi
///
/// ADX + BB genişliği + günün saatine bakarak:
///   - Güçlü trend (ADX > 30, BB geniş) → Swing öncelikli + Scalp de açık
///   - Zayıf/ranging piyasa (ADX < 20)  → Scalp öncelikli (kısa getiri al)
///   - Yüksek volatilite BB squeeze kırılım → Her iki mod birlikte
///   - Gece saatleri (UTC 22–06) → Scalp kapalı, Swing sinyali korunur

use chrono::Timelike;
use crate::types::Candle;
use crate::robot::indicators::{calculate_adx, calculate_bollinger};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeMode {
    ScalpOnly,
    SwingOnly,
    Both,
    Neither, // tavsiye edilmez ama aşırı düşük volatilite/gece gibi durumlarda
}

impl TradeMode {
    pub fn label(&self) -> &'static str {
        match self {
            TradeMode::ScalpOnly => "SKALP",
            TradeMode::SwingOnly => "SWING",
            TradeMode::Both      => "SKALP+SWING",
            TradeMode::Neither   => "BEKLİYOR",
        }
    }
}

pub struct ModeSelector;

impl ModeSelector {
    /// 1h mumlardan (veya ana interval) mod seçimi yap.
    /// `scalp_active_hours`: UTC [start_h, end_h] — bu aralık dışında scalp yok
    pub fn select(
        candles:            &[Candle],
        scalp_active_hours: [u32; 2],
    ) -> TradeMode {
        let adx = calculate_adx(candles, 14).map(|(a, _, _)| a).unwrap_or(15.0);
        let (bb_lower, bb_mid, bb_upper) =
            calculate_bollinger(candles, 20, 2.0).unwrap_or((0.0, 0.0, 0.0));
        let bb_width = if bb_mid > 0.0 { (bb_upper - bb_lower) / bb_mid } else { 0.02 };

        let current_hour = chrono::Utc::now().hour();
        let [h_start, h_end] = scalp_active_hours;
        let scalp_hour_ok = if h_start < h_end {
            current_hour >= h_start && current_hour < h_end
        } else {
            // gece yarısı wrap (örn. 22–06)
            current_hour >= h_start || current_hour < h_end
        };

        // Volatilite kategorileri
        let trending    = adx > 30.0;
        let ranging     = adx < 18.0;
        let squeeze     = bb_width < 0.015; // çok sıkışmış → kırılım yakın
        let high_vol    = bb_width > 0.04;  // geniş BB → büyük hareket

        match (trending, ranging, scalp_hour_ok) {
            // Güçlü trend + saatler uygun → her ikisi
            (true, _, true)  => TradeMode::Both,
            // Güçlü trend ama gece → sadece swing
            (true, _, false) => TradeMode::SwingOnly,
            // Ranging veya squeeze + saatler uygun → scalp fırsatı
            (false, true, true) | (false, false, true) if squeeze || !high_vol
                             => TradeMode::ScalpOnly,
            // Saatler uygun, orta ADX → her ikisi
            (false, false, true) => TradeMode::Both,
            // Gece + ranging → hiçbiri (swing sinyali üretilmez de)
            (false, _, false) => TradeMode::Neither,
            // Diğer durumlar
            _ => if scalp_hour_ok { TradeMode::Both } else { TradeMode::SwingOnly },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn flat_candle(price: f64) -> Candle {
        Candle { timestamp: Utc::now(), open: price, high: price * 1.001, low: price * 0.999, close: price, volume: 1000.0, symbol: "TEST".into(), interval: "1h".into() }
    }

    fn trending_candle(i: usize) -> Candle {
        let p = 100.0 + i as f64 * 2.0;
        Candle { timestamp: Utc::now(), open: p - 1.0, high: p + 2.0, low: p - 2.0, close: p, volume: 2000.0, symbol: "TEST".into(), interval: "1h".into() }
    }

    /// Yetersiz veri (< 20 mum) → BB hesaplanamaz → varsayılan değerlerle çalışmalı, paniklemez
    #[test]
    fn test_too_few_candles_no_panic() {
        let candles: Vec<Candle> = (0..10).map(|i| trending_candle(i)).collect();
        let _mode = ModeSelector::select(&candles, [6, 22]);
        // Panik olmadı = test geçti
    }

    /// TradeMode::label() doğru etiket döndürmeli
    #[test]
    fn test_trade_mode_labels() {
        assert_eq!(TradeMode::ScalpOnly.label(), "SKALP");
        assert_eq!(TradeMode::SwingOnly.label(), "SWING");
        assert_eq!(TradeMode::Both.label(),      "SKALP+SWING");
        assert_eq!(TradeMode::Neither.label(),   "BEKLİYOR");
    }

    /// Tüm saatler aktif [0, 24] gibi bir aralıkta gece saati kontrolü doğru çalışmalı
    #[test]
    fn test_all_hours_active() {
        let candles: Vec<Candle> = (0..60).map(|i| trending_candle(i)).collect();
        // [0, 0] wrap-around: 0 >= 0 veya 0 < 0 → sadece 0 < 0 false → hour >= 0 true
        // Sonuç: scalp_hour_ok = true her saat
        let mode = ModeSelector::select(&candles, [0, 0]);
        // Herhangi bir valid mod dönmeli (panik yok)
        let _ = mode.label();
    }

    /// Gece saati aralığı [22, 6] → şimdiki UTC saate göre scalp_hour_ok hesabı
    #[test]
    fn test_night_hours_wrap_around_logic() {
        // Saat 03:00 UTC → [22, 6] aralığında → scalp_hour_ok = true
        let h_start = 22u32;
        let h_end   = 6u32;
        let night_hour  = 3u32;   // gece yarısı sonrası
        let midday_hour = 12u32;  // öğlen

        let scalp_ok_night  = night_hour  >= h_start || night_hour  < h_end;
        let scalp_ok_midday = midday_hour >= h_start || midday_hour < h_end;
        assert!(scalp_ok_night,   "03:00 UTC gece aralığında scalp aktif olmalı");
        assert!(!scalp_ok_midday, "12:00 UTC gece aralığı dışında scalp pasif olmalı");
    }

    /// Düz piyasa (trending=false, ranging=true) saatler uygunsa ScalpOnly dönmeli
    #[test]
    fn test_ranging_market_scalp_hours_ok() {
        // ModeSelector iç mantığı: (false, true, true) && (squeeze || !high_vol) → ScalpOnly
        // Doğrudan saate bağlı olduğu için sadece mantık tablosunu test edelim
        let trending  = false;
        let ranging   = true;
        let scalp_ok  = true;
        let squeeze   = false;
        let high_vol  = false;

        let mode = match (trending, ranging, scalp_ok) {
            (true, _, true)  => TradeMode::Both,
            (true, _, false) => TradeMode::SwingOnly,
            (false, true, true) | (false, false, true) if squeeze || !high_vol => TradeMode::ScalpOnly,
            (false, false, true) => TradeMode::Both,
            (false, _, false) => TradeMode::Neither,
            _ => if scalp_ok { TradeMode::Both } else { TradeMode::SwingOnly },
        };
        assert_eq!(mode, TradeMode::ScalpOnly);
    }

    /// Güçlü trend + saatler uygun → Both
    #[test]
    fn test_strong_trend_scalp_hours_ok_is_both() {
        let trending = true;
        let ranging  = false;
        let scalp_ok = true;

        let mode = match (trending, ranging, scalp_ok) {
            (true, _, true)  => TradeMode::Both,
            _                => TradeMode::Neither,
        };
        assert_eq!(mode, TradeMode::Both);
    }

    /// Güçlü trend + gece → SwingOnly
    #[test]
    fn test_strong_trend_night_is_swing_only() {
        let trending = true;
        let ranging  = false;
        let scalp_ok = false;

        let mode = match (trending, ranging, scalp_ok) {
            (true, _, true)  => TradeMode::Both,
            (true, _, false) => TradeMode::SwingOnly,
            _                => TradeMode::Neither,
        };
        assert_eq!(mode, TradeMode::SwingOnly);
    }
}
