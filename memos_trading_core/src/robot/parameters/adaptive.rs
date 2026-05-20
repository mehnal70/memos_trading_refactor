// robot/parameters/adaptive.rs — Faz 3: rejim-bazlı adaptive patch heuristic'leri.
//
// Engine her cycle'da `Engine::classify_regime` ile mevcut rejimi sınıflandırıyor.
// İlgili rejim için ParameterStore'da henüz patch yoksa bu modüldeki heuristic
// (varsayılan) patch yerleştirilir — böylece "Ranging" piyasada darTP/SL,
// "HighVolatility" piyasada sıkı edge eşikleri otomatik devreye girer.
//
// Bu patch'ler manuel/HyperOpt tarafından override edilebilir; ilk kez görülen
// rejim için akıllı bir başlangıç noktası sağlamak amaçlı. Faz 3 sonraki commit'leri
// learn_from_exit + DriftDetector feedback'iyle bu patch'leri rafine edecek.

use super::{EdgeThresholds, RegimePatch, TradeRiskParams};

/// Verilen rejim string'i için (MarketRegime::as_str()) varsayılan patch.
/// Bilinmeyen rejimler için boş patch döner (base parametreler kullanılır).
///
/// Heuristic özet:
/// - **Ranging**: Trend belirsiz — daha sıkı edge, kısa TP/dar SL, küçük pozisyon.
/// - **HighVolatility**: Geniş hareket — sıkı edge filtresi, geniş TP/SL,
///   ama küçük pozisyon (drawdown koruması).
/// - **LowVolatility**: Düşük hareket — gevşek edge (sinyal kıt), orta TP/SL.
/// - **StrongUptrend / StrongDowntrend**: Net trend — gevşek edge (fırsatı kaçırma),
///   normal pozisyon.
/// - **WeakUptrend / WeakDowntrend / Unknown**: Base parametreleri kullan (boş patch).
pub fn default_patch_for_regime(regime: &str) -> RegimePatch {
    match regime {
        "Ranging" => RegimePatch::empty()
            .with_edge(EdgeThresholds {
                cold: 0.30, warm: 0.45, hot: 0.65,
                cold_until: 0.05, warm_until: 0.30,
            })
            .with_trade_risk(TradeRiskParams {
                take_profit_pct:   1.5,
                stop_loss_pct:     0.8,
                max_position_size: 0.35,
            }),
        "HighVolatility" => RegimePatch::empty()
            .with_edge(EdgeThresholds {
                cold: 0.45, warm: 0.55, hot: 0.70,
                cold_until: 0.05, warm_until: 0.30,
            })
            .with_trade_risk(TradeRiskParams {
                take_profit_pct:   4.0,
                stop_loss_pct:     2.0,
                max_position_size: 0.25,
            }),
        "LowVolatility" => RegimePatch::empty()
            .with_edge(EdgeThresholds {
                cold: 0.15, warm: 0.25, hot: 0.40,
                cold_until: 0.05, warm_until: 0.30,
            })
            .with_trade_risk(TradeRiskParams {
                take_profit_pct:   2.5,
                stop_loss_pct:     1.2,
                max_position_size: 0.50,
            }),
        "StrongUptrend" | "StrongDowntrend" => RegimePatch::empty()
            .with_edge(EdgeThresholds {
                cold: 0.15, warm: 0.30, hot: 0.50,
                cold_until: 0.05, warm_until: 0.30,
            })
            .with_trade_risk(TradeRiskParams {
                take_profit_pct:   3.5,
                stop_loss_pct:     1.5,
                max_position_size: 0.50,
            }),
        // WeakUptrend / WeakDowntrend / Unknown → base parametreler.
        _ => RegimePatch::empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranging_has_tighter_edge_and_smaller_position() {
        let p = default_patch_for_regime("Ranging");
        let e = p.edge_thresholds.expect("Ranging edge override olmalı");
        let r = p.trade_risk.expect("Ranging trade_risk override olmalı");
        // Ranging cold base'den (0.20) yüksek olmalı — trend yokken sıkı filtre
        assert!(e.cold > 0.20);
        // Pozisyon base'den (0.5) küçük
        assert!(r.max_position_size < 0.5);
        // Kısa TP
        assert!(r.take_profit_pct < 3.0);
    }

    #[test]
    fn high_volatility_has_strict_edge_and_small_position() {
        let p = default_patch_for_regime("HighVolatility");
        let e = p.edge_thresholds.expect("HV edge");
        let r = p.trade_risk.expect("HV trade_risk");
        assert!(e.hot >= 0.65);
        assert!(r.max_position_size <= 0.30);
    }

    #[test]
    fn low_volatility_relaxes_edge_thresholds() {
        let p = default_patch_for_regime("LowVolatility");
        let e = p.edge_thresholds.expect("LV edge");
        // Cold base'den (0.20) düşük → sinyal kıt, gevşek
        assert!(e.cold < 0.20);
    }

    #[test]
    fn strong_trends_share_same_patch() {
        let up = default_patch_for_regime("StrongUptrend");
        let dn = default_patch_for_regime("StrongDowntrend");
        assert_eq!(up.edge_thresholds.map(|e| e.cold), dn.edge_thresholds.map(|e| e.cold));
        assert_eq!(up.trade_risk.map(|t| t.take_profit_pct),
                   dn.trade_risk.map(|t| t.take_profit_pct));
    }

    #[test]
    fn weak_and_unknown_regimes_yield_empty_patch() {
        for r in ["WeakUptrend", "WeakDowntrend", "Unknown", "Custom123"] {
            assert!(default_patch_for_regime(r).is_empty(),
                "rejim {} için boş patch beklenir", r);
        }
    }
}
