// robot/scalp_swing/mode_selector.rs - Otonom İşlem Modu ve Vites Seçici

use chrono::Timelike;
use crate::core::types::Candle;
use crate::core::indicators::{calculate_adx, calculate_bollinger};

/// §61.1: TradeMode - Botun o anki operasyonel modunu belirleyen enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TradeMode {
    ScalpOnly,
    SwingOnly,
    Both,
    Neither, 
}

impl TradeMode {
    pub fn label(&self) -> &'static str {
        match self {
            TradeMode::ScalpOnly => "SKALP",
            TradeMode::SwingOnly => "SWING",
            TradeMode::Both      => "DUAL-MOD",
            TradeMode::Neither   => "BEKLEMEDE",
        }
    }
}

pub struct ModeSelector;

impl ModeSelector {
    /// Piyasa verilerini ve zamanı analiz ederek otonom mod seçimi yapar.
    /// Srivastava ATP Standartları: ADX Rejimi + Volatilite Sıkışması + Zaman Filtresi.
    pub fn select(
        candles:            &[Candle],
        scalp_active_hours: [u32; 2],
    ) -> TradeMode {
        // 1. Teknik Gösterge Analizi (yeni Vec<f64>/struct API, fall-back değerleri ile)
        let adx = calculate_adx(candles, 14).last().copied().unwrap_or(15.0);
        let bb  = calculate_bollinger(candles, 20, 2.0);
        let bb_upper = bb.upper.last().copied().unwrap_or(0.0);
        let bb_mid   = bb.middle.last().copied().unwrap_or(0.0);
        let bb_lower = bb.lower.last().copied().unwrap_or(0.0);

        // Bollinger Band genişliği (Yüzdesel)
        let bb_width = if bb_mid > 0.0 { (bb_upper - bb_lower) / bb_mid } else { 0.02 };

        // 2. Otonom Saat Kontrolü (Midnight Wrap Desteği)
        let current_hour = chrono::Utc::now().hour();
        let [h_start, h_end] = scalp_active_hours;
        let scalp_hour_ok = if h_start < h_end {
            current_hour >= h_start && current_hour < h_end
        } else {
            current_hour >= h_start || current_hour < h_end 
        };

        // 3. Piyasa Durum Kategorizasyonu
        let trending = adx > 30.0;     // Güçlü Trend
        let ranging  = adx < 18.0;     // Yatay Piyasa
        let squeeze  = bb_width < 0.015; // Volatilite Sıkışması
        let high_vol = bb_width > 0.04;  // Aşırı Oynaklık

        // 4. Otonom Karar Matrisi
        match (trending, ranging, scalp_hour_ok) {
            // Trend var ve saat uygun → Her iki fırsatı da kovala
            (true, _, true)  => TradeMode::Both,
            // Trend var ama saat gece → Sadece güvenli Swing işlemleri
            (true, _, false) => TradeMode::SwingOnly,
            // Yatay piyasada saat uygunsa ve volatilite düşükse → Sadece Scalp
            (false, true, true) | (false, false, true) if squeeze || !high_vol => TradeMode::ScalpOnly,
            // Normal koşullarda saat uygunsa → Çift mod
            (false, false, true) => TradeMode::Both,
            // Yatay piyasada gece vakti → Operasyonu durdur (Sermaye koruma)
            (false, _, false) if ranging => TradeMode::Neither,
            // Varsayılan: Saat uygunsa her ikisi, değilse Swing
            _ => if scalp_hour_ok { TradeMode::Both } else { TradeMode::SwingOnly },
        }
    }
}
