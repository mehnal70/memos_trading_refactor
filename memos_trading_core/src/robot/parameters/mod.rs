// robot/parameters/mod.rs — Dinamik parametre store'u.
//
// Faz 2 hedefi: sabit eşikleri (örn. `EDGE_THRESHOLD`, slippage limitleri,
// scalp/swing eşiği) tek bir merkezi store'dan beslemek. HyperOpt + IntelligenceHub
// bu store'a yazar, engine her cycle'da okur. Bu sayede:
//   - Sabit değerler runtime'da güncellenebilir (rejim/öğrenme akışıyla).
//   - HyperOpt sonuçları otomatik propagation.
//   - Test edilebilirlik artar (env yerine direct construct).
//
// Bu commit (c1) iskelet: edge_threshold katmanlarını taşıyor. Sonraki commit'lerde
// (c2) daha çok parametre, (c3) HyperOpt yazımı, (c4) rejim-bazlı katmanlama.

// Faz 2 modülerleştirme: tip tanımları (types) ve merkezi store (store) ayrı
// dosyalara taşındı. Dış API `pub use` ile birebir korunur — call-site değişmedi.
// Env okuyucu helper'lar (parse_env_*, now_epoch_secs) burada kalır: hem store
// child'ı hem test modülü erişir.
mod types;
mod store;

pub mod adaptive;
pub mod symbol_stats;
pub mod trail_feedback;

pub use types::*;
pub use store::*;
pub use symbol_stats::{SymbolStats, compute_symbol_stats};
pub use trail_feedback::{TrailFeedback, PendingTrailObservation};

fn parse_env_f64(key: &str) -> Option<f64> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

/// Bool env okuyucu — kabul edilen değerler: "1"/"0", "true"/"false",
/// "yes"/"no", "on"/"off" (case-insensitive). Tanımsız veya tanınmayan
/// değerlerde None döner → çağıran default değeri korur.
fn parse_env_bool(key: &str) -> Option<bool> {
    let v = std::env::var(key).ok()?;
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on"  => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Sistemden epoch saniyesini okur; SystemTime hatasında 0 döner (cooldown
/// kapanır → güvenli taraf: hiç drift atma değil, eski davranışla aynı).
fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SymbolStats fixture: belirli yaşa sahip tazeleyici. Now-offset saniye.
    fn make_stats(noise_pct: f64, sample: usize, age_secs: u64) -> SymbolStats {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        SymbolStats {
            noise_floor_pct: noise_pct,
            p90_range_pct:   noise_pct * 1.5,
            sample_size:     sample,
            last_updated:    now.saturating_sub(age_secs),
        }
    }

    #[test]
    fn resolve_atr_mult_returns_default_when_no_stats() {
        let s = ParameterStore::default();
        let m = s.resolve_atr_mult("BTCUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 2.0).abs() < 1e-9, "stats yokken default döner");
    }

    #[test]
    fn resolve_atr_mult_uses_strategy_target_for_trend() {
        let mut s = ParameterStore::default();
        // SUPERTREND target=1.2, noise=0.05 → mult = 24
        s.update_symbol_stats("ETHUSDT", "1m", make_stats(0.05, 100, 60));
        let m = s.resolve_atr_mult("ETHUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 24.0).abs() < 1e-9, "mult = 1.2/0.05 = 24, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_uses_strategy_target_for_meanrev() {
        let mut s = ParameterStore::default();
        // BB target=0.5, noise=0.05 → mult = 10
        s.update_symbol_stats("ETHUSDT", "1m", make_stats(0.05, 100, 60));
        let m = s.resolve_atr_mult("ETHUSDT", "1m", "BB", 2.0);
        assert!((m - 10.0).abs() < 1e-9, "BB mean-rev: mult = 0.5/0.05 = 10, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_falls_back_to_default_target_for_unknown_strategy() {
        let mut s = ParameterStore::default();
        // default target=0.7, noise=0.05 → mult = 14
        s.update_symbol_stats("ETHUSDT", "1m", make_stats(0.05, 100, 60));
        let m = s.resolve_atr_mult("ETHUSDT", "1m", "FANCY_UNKNOWN", 2.0);
        assert!((m - 14.0).abs() < 1e-9, "unknown → default target 0.7, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_clamps_at_max_for_extreme_low_noise() {
        let mut s = ParameterStore::default();
        // Noise = %0.001, target SUPERTREND=1.2 → mult = 1200 → clamp 30
        s.update_symbol_stats("BTCUSDT", "1m", make_stats(0.001, 100, 0));
        let m = s.resolve_atr_mult("BTCUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 30.0).abs() < 1e-9, "MAX_MULT clamp 30, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_clamps_at_min_for_very_wide_noise() {
        let mut s = ParameterStore::default();
        // Noise = %5 (çok yüksek), target=1.2 → mult = 0.24 → clamp 1.5
        s.update_symbol_stats("XYZUSDT", "1h", make_stats(5.0, 100, 0));
        let m = s.resolve_atr_mult("XYZUSDT", "1h", "SUPERTREND", 2.0);
        assert!((m - 1.5).abs() < 1e-9, "MIN_MULT clamp 1.5, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_falls_back_when_stats_stale() {
        let mut s = ParameterStore::default();
        s.update_symbol_stats("BTCUSDT", "1m", make_stats(0.025, 100, 10 * 3600));
        let m = s.resolve_atr_mult("BTCUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 2.0).abs() < 1e-9, "stale → default 2.0, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_falls_back_when_sample_too_small() {
        let mut s = ParameterStore::default();
        s.update_symbol_stats("BTCUSDT", "1m", make_stats(0.025, 30, 60));
        let m = s.resolve_atr_mult("BTCUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 2.0).abs() < 1e-9, "low sample → default, gerçek {}", m);
    }

    #[test]
    fn target_trail_pct_picks_strategy_specific_default() {
        let s = ParameterStore::default();
        // Default tablodan
        assert!((s.target_trail_pct_for_strategy("SUPERTREND") - 1.2).abs() < 1e-9);
        assert!((s.target_trail_pct_for_strategy("BB") - 0.5).abs() < 1e-9);
        assert!((s.target_trail_pct_for_strategy("MA_CROSSOVER") - 1.5).abs() < 1e-9);
        // Bilinmeyen → default
        assert!((s.target_trail_pct_for_strategy("FANCY_FOO") - 0.7).abs() < 1e-9);
    }

    #[test]
    fn purge_stale_keeps_fresh_drops_old() {
        let mut s = ParameterStore::default();
        s.update_symbol_stats("FRESH", "1m", make_stats(0.5, 100, 60));
        s.update_symbol_stats("STALE", "1m", make_stats(0.5, 100, 7 * 3600));
        s.purge_stale_symbol_stats(6 * 3600);
        assert!(s.symbol_stats.contains_key(&("FRESH".into(), "1m".into())));
        assert!(!s.symbol_stats.contains_key(&("STALE".into(), "1m".into())));
    }

    #[test]
    fn default_edge_thresholds_match_legacy_constants() {
        // Faz 1 öncesi Engine::dynamic_edge_threshold sabitleri: 0.20 / 0.35 / 0.55.
        // Default ParameterStore aynı değerleri korumalı (geriye uyum).
        let s = ParameterStore::default();
        assert_eq!(s.edge_thresholds.cold, 0.20);
        assert_eq!(s.edge_thresholds.warm, 0.35);
        assert_eq!(s.edge_thresholds.hot,  0.55);
        assert_eq!(s.edge_thresholds.cold_until, 0.05);
        assert_eq!(s.edge_thresholds.warm_until, 0.30);
    }

    #[test]
    fn edge_threshold_picks_correct_tier_for_each_confidence_zone() {
        let s = ParameterStore::default();
        // cold: ml < 0.05
        assert!((s.edge_threshold(0.0)  - 0.20).abs() < 1e-9);
        assert!((s.edge_threshold(0.04) - 0.20).abs() < 1e-9);
        // warm: 0.05 ≤ ml < 0.30
        assert!((s.edge_threshold(0.05) - 0.35).abs() < 1e-9);
        assert!((s.edge_threshold(0.20) - 0.35).abs() < 1e-9);
        assert!((s.edge_threshold(0.29) - 0.35).abs() < 1e-9);
        // hot: ml ≥ 0.30
        assert!((s.edge_threshold(0.30) - 0.55).abs() < 1e-9);
        assert!((s.edge_threshold(0.99) - 0.55).abs() < 1e-9);
    }

    #[test]
    fn from_env_overrides_individual_thresholds() {
        std::env::set_var("EDGE_THRESHOLD_HOT", "0.70");
        std::env::set_var("EDGE_WARM_UNTIL",    "0.50");
        let s = ParameterStore::from_env();
        std::env::remove_var("EDGE_THRESHOLD_HOT");
        std::env::remove_var("EDGE_WARM_UNTIL");
        assert!((s.edge_thresholds.hot - 0.70).abs() < 1e-9);
        assert!((s.edge_thresholds.warm_until - 0.50).abs() < 1e-9);
        // Diğer alanlar default'ta kalmalı.
        assert_eq!(s.edge_thresholds.cold, 0.20);
        assert_eq!(s.edge_thresholds.warm, 0.35);
    }

    #[test]
    fn from_env_with_garbage_falls_back_to_default() {
        std::env::set_var("EDGE_THRESHOLD_COLD", "not_a_number");
        let s = ParameterStore::from_env();
        std::env::remove_var("EDGE_THRESHOLD_COLD");
        assert_eq!(s.edge_thresholds.cold, 0.20);
    }

    // ─── Multi-TF (Faz B) ────────────────────────────────────────────────

    #[test]
    fn multi_tf_default_enabled() {
        let s = ParameterStore::default();
        assert!(s.multi_tf.enabled, "default'ta multi-TF açık olmalı");
        assert_eq!(s.multi_tf.min_required, 30);
        assert!(s.multi_tf.download_htf);
    }

    #[test]
    fn from_env_multi_tf_disabled_via_env() {
        std::env::set_var("MULTI_TF_ENABLED", "false");
        std::env::set_var("MULTI_TF_DOWNLOAD", "0");
        std::env::set_var("MULTI_TF_MIN_REQUIRED", "50");
        let s = ParameterStore::from_env();
        std::env::remove_var("MULTI_TF_ENABLED");
        std::env::remove_var("MULTI_TF_DOWNLOAD");
        std::env::remove_var("MULTI_TF_MIN_REQUIRED");
        assert!(!s.multi_tf.enabled);
        assert!(!s.multi_tf.download_htf);
        assert_eq!(s.multi_tf.min_required, 50);
    }

    #[test]
    fn parse_env_bool_accepts_common_forms() {
        std::env::set_var("MTF_TEST_BOOL", "yes");
        assert_eq!(parse_env_bool("MTF_TEST_BOOL"), Some(true));
        std::env::set_var("MTF_TEST_BOOL", "OFF");
        assert_eq!(parse_env_bool("MTF_TEST_BOOL"), Some(false));
        std::env::set_var("MTF_TEST_BOOL", "garbage");
        assert_eq!(parse_env_bool("MTF_TEST_BOOL"), None);
        std::env::remove_var("MTF_TEST_BOOL");
        assert_eq!(parse_env_bool("MTF_TEST_BOOL"), None);
    }

    // ─── Leverage (Otonom katman) ────────────────────────────────────────

    fn enabled_lev_store() -> ParameterStore {
        let mut s = ParameterStore::default();
        s.leverage.enabled = true;
        s.leverage.base = 3.0;
        s.leverage.max = 10.0;
        s.leverage.conf_boost_threshold = 0.70;
        s.leverage.vol_floor_pct = 1.0;
        s
    }

    #[test]
    fn resolve_leverage_returns_one_when_disabled() {
        // Default artık enabled=true (otonom davranış); kapalıya çekip kontrol.
        let mut s = ParameterStore::default();
        s.leverage.enabled = false;
        assert_eq!(s.resolve_leverage("StrongUptrend", 0.9, 0.7, Some(0.5)), 1.0);
    }

    #[test]
    fn resolve_leverage_default_is_autonomous() {
        let s = ParameterStore::default();
        assert!(s.leverage.enabled, "default'ta otonom leverage açık");
        assert_eq!(s.leverage.base, 3.0);
        assert_eq!(s.leverage.max, 10.0);
        // StrongUptrend + yüksek conf + iyi wr → 3.0 * 1.3 * 1.2 * 1.15 = 5.382
        let lev = s.resolve_leverage("StrongUptrend", 0.85, 0.7, Some(0.3));
        assert!(lev > 1.0 && lev <= 10.0, "dinamik aralık, got {}", lev);
    }

    #[test]
    fn resolve_leverage_uses_regime_factor() {
        let s = enabled_lev_store();
        // base=3.0, ranging=×1.0, ml=0.5 (no boost), wr=0.5 (neutral), no vol → 3.0
        let lev = s.resolve_leverage("Ranging", 0.5, 0.5, Some(0.5));
        assert!((lev - 3.0).abs() < 1e-9, "ranging+neutral → base, got {}", lev);
    }

    #[test]
    fn resolve_leverage_high_vol_halves() {
        let s = enabled_lev_store();
        // base=3.0 × 0.5 = 1.5
        let lev = s.resolve_leverage("HighVolatility", 0.5, 0.5, None);
        assert!((lev - 1.5).abs() < 1e-9, "highvol → ×0.5, got {}", lev);
    }

    #[test]
    fn resolve_leverage_strong_trend_with_conf_and_wins() {
        let s = enabled_lev_store();
        // 3.0 × 1.3 (strong up) × 1.2 (conf>0.70) × 1.15 (wr≥0.6) = 5.382
        let lev = s.resolve_leverage("StrongUptrend", 0.85, 0.7, Some(0.3));
        assert!((lev - 5.382).abs() < 1e-3, "boost stack: 3×1.3×1.2×1.15, got {}", lev);
    }

    #[test]
    fn resolve_leverage_clamps_to_max() {
        let mut s = enabled_lev_store();
        s.leverage.base = 8.0;
        // 8.0 × 1.3 × 1.2 × 1.15 = 14.35 → clamp to max 10.0
        let lev = s.resolve_leverage("StrongDowntrend", 0.9, 0.7, Some(0.3));
        assert!((lev - 10.0).abs() < 1e-9, "clamp to max, got {}", lev);
    }

    #[test]
    fn resolve_leverage_clamps_to_floor_one() {
        let s = enabled_lev_store();
        // 3.0 × 0.5 (highvol) × 0.75 (wr=0.3) × 0.7 (high noise) = 0.7875 → clamp 1.0
        let lev = s.resolve_leverage("HighVolatility", 0.5, 0.3, Some(2.5));
        assert!((lev - 1.0).abs() < 1e-9, "floor 1.0 koruması, got {}", lev);
    }

    #[test]
    fn resolve_leverage_zero_winrate_treated_neutral() {
        let s = enabled_lev_store();
        // wr=0.0 ("veri yok") cezalandırılmamalı: base=3.0 × ranging=1.0 = 3.0
        let lev_zero = s.resolve_leverage("Ranging", 0.5, 0.0, None);
        let lev_neut = s.resolve_leverage("Ranging", 0.5, 0.5, None);
        assert!((lev_zero - lev_neut).abs() < 1e-9, "0.0 nötr olmalı, zero={} neut={}", lev_zero, lev_neut);
    }

    #[test]
    fn resolve_leverage_noise_floor_optional() {
        let s = enabled_lev_store();
        // None → vol faktörü uygulanmaz; Some(altında) → uygulanmaz; Some(üstünde) → ×0.7
        let lev_none  = s.resolve_leverage("Ranging", 0.5, 0.5, None);
        let lev_under = s.resolve_leverage("Ranging", 0.5, 0.5, Some(0.5));
        let lev_over  = s.resolve_leverage("Ranging", 0.5, 0.5, Some(2.0));
        assert!((lev_none - lev_under).abs() < 1e-9);
        assert!(lev_over < lev_none, "yüksek noise → düşük lev");
    }

    #[test]
    fn from_env_leverage_override_chain() {
        std::env::set_var("LEVERAGE_ENABLED",        "true");
        std::env::set_var("LEVERAGE_BASE",           "5.0");
        std::env::set_var("LEVERAGE_MAX",            "20.0");
        std::env::set_var("LEVERAGE_CONF_THRESHOLD", "0.80");
        std::env::set_var("LEVERAGE_VOL_FLOOR_PCT",  "1.5");
        let s = ParameterStore::from_env();
        std::env::remove_var("LEVERAGE_ENABLED");
        std::env::remove_var("LEVERAGE_BASE");
        std::env::remove_var("LEVERAGE_MAX");
        std::env::remove_var("LEVERAGE_CONF_THRESHOLD");
        std::env::remove_var("LEVERAGE_VOL_FLOOR_PCT");
        assert!(s.leverage.enabled);
        assert!((s.leverage.base - 5.0).abs() < 1e-9);
        assert!((s.leverage.max - 20.0).abs() < 1e-9);
        assert!((s.leverage.conf_boost_threshold - 0.80).abs() < 1e-9);
        assert!((s.leverage.vol_floor_pct - 1.5).abs() < 1e-9);
    }

    #[test]
    fn default_partial_fill_matches_legacy_constants() {
        let s = ParameterStore::default();
        assert_eq!(s.partial_fill.overfill_tolerance, 0.001);
        assert_eq!(s.partial_fill.cum_tolerance,      0.001);
        assert_eq!(s.partial_fill.max_slippage_pct,   1.0);
    }

    #[test]
    fn default_scalp_swing_and_sr_update_match_legacy() {
        let s = ParameterStore::default();
        assert_eq!(s.scalp_swing_threshold_min, 60);
        assert_eq!(s.sr_update_every_secs,      30);
    }

    #[test]
    fn default_trade_risk_matches_legacy_fallbacks() {
        let s = ParameterStore::default();
        assert_eq!(s.trade_risk.take_profit_pct,   3.0);
        assert_eq!(s.trade_risk.stop_loss_pct,     1.5);
        assert_eq!(s.trade_risk.max_position_size, 0.5);
    }

    #[test]
    fn apply_optimization_writes_trade_risk_fields() {
        let mut s = ParameterStore::default();
        s.apply_optimization(4.5, 2.0, 0.75);
        assert!((s.trade_risk.take_profit_pct - 4.5).abs() < 1e-9);
        assert!((s.trade_risk.stop_loss_pct   - 2.0).abs() < 1e-9);
        assert!((s.trade_risk.max_position_size - 0.75).abs() < 1e-9);
    }

    #[test]
    fn apply_optimization_rejects_invalid_values() {
        let mut s = ParameterStore::default();
        s.apply_optimization(-1.0, 0.0, 2.0); // hepsi geçersiz
        // Default'ta kalmalı
        assert_eq!(s.trade_risk.take_profit_pct,   3.0);
        assert_eq!(s.trade_risk.stop_loss_pct,     1.5);
        assert_eq!(s.trade_risk.max_position_size, 0.5);
    }

    #[test]
    fn apply_optimization_partial_keeps_unspecified_alone() {
        let mut s = ParameterStore::default();
        // Sadece TP geçerli, SL=0 (skip), max_pos > 1 (skip).
        s.apply_optimization(5.0, 0.0, 1.5);
        assert_eq!(s.trade_risk.take_profit_pct,   5.0);
        assert_eq!(s.trade_risk.stop_loss_pct,     1.5); // default kaldı
        assert_eq!(s.trade_risk.max_position_size, 0.5); // default kaldı
    }

    #[test]
    fn observe_regime_first_call_does_not_report_change() {
        let mut s = ParameterStore::default();
        let changed = s.observe_regime("Ranging");
        assert!(!changed, "ilk gözlem değişim sayılmamalı");
        assert_eq!(s.last_observed_regime.as_deref(), Some("Ranging"));
        // Patch yazılmamış olmalı (ilk gözlem)
        assert!(s.regime_overrides.is_empty());
    }

    #[test]
    fn observe_regime_change_triggers_tighten_and_reports_true() {
        let mut s = ParameterStore::default();
        s.observe_regime_with_now("Ranging", 1000); // ilk gözlem, seed
        // Hysteresis: yeni rejim için ardışık 3 cycle gerek.
        assert!(!s.observe_regime_with_now("HighVolatility", 1001), "1. tur henüz drift sayılmaz");
        assert!(!s.observe_regime_with_now("HighVolatility", 1002), "2. tur henüz drift sayılmaz");
        let changed = s.observe_regime_with_now("HighVolatility", 1003);
        assert!(changed, "3. ardışık görüşte drift confirmed olmalı");
        let patch = s.regime_overrides.get("HighVolatility")
            .expect("HV patch yazılmalı");
        assert!(patch.edge_thresholds.is_some());
        assert!(patch.trade_risk.is_some());
        // Base 0.50 → 0.50 * 0.70 = 0.35
        assert!(patch.trade_risk.unwrap().max_position_size < 0.50);
    }

    #[test]
    fn observe_regime_same_regime_back_to_back_no_tighten() {
        let mut s = ParameterStore::default();
        s.observe_regime_with_now("StrongUptrend", 1000); // seed
        let changed = s.observe_regime_with_now("StrongUptrend", 1001);
        assert!(!changed);
        assert!(s.regime_overrides.is_empty());
    }

    #[test]
    fn observe_regime_oscillation_does_not_drift() {
        // Rejim her tur A↔B arasında salınıyor: hiçbir aday DRIFT_CONFIRMATION_TURNS'a
        // ulaşamaz → drift yok, patch yazılmaz.
        let mut s = ParameterStore::default();
        s.observe_regime_with_now("Ranging", 1000); // seed
        for t in 1..30 {
            let r = if t % 2 == 0 { "HighVolatility" } else { "Ranging" };
            let changed = s.observe_regime_with_now(r, 1000 + t);
            assert!(!changed, "sallanan rejim drift sayılmamalı (t={}): {}", t, r);
        }
        assert!(s.regime_overrides.is_empty(),
            "sallanma sırasında patch yazılmamalı: {:?}", s.regime_overrides);
    }

    #[test]
    fn observe_regime_cooldown_suppresses_back_to_back_drifts() {
        // İlk drift confirmed olsun.
        let mut s = ParameterStore::default();
        s.observe_regime_with_now("Ranging", 1000); // seed
        for t in 1..=3 { s.observe_regime_with_now("HighVolatility", 1000 + t); }
        assert_eq!(s.last_observed_regime.as_deref(), Some("HighVolatility"));
        let first_drift_at = s.last_drift_at_secs;
        assert!(first_drift_at >= 1003);
        let patches_after_first = s.regime_overrides.len();

        // Hemen ardından yeni bir rejime geçiş — cooldown içinde olduğu için drift
        // confirmed olmaz, sayım toplansa bile tighten/log yok.
        for t in 4..=10 {
            let changed = s.observe_regime_with_now("StrongUptrend", 1000 + t);
            assert!(!changed, "cooldown içinde drift bastırılmalı (t={})", t);
        }
        assert_eq!(s.regime_overrides.len(), patches_after_first,
            "cooldown içinde yeni patch yazılmamalı");

        // Cooldown sonrası bir tur daha → confirmed.
        let after_cd = first_drift_at + DRIFT_COOLDOWN_SECS + 1;
        let changed = s.observe_regime_with_now("StrongUptrend", after_cd);
        assert!(changed, "cooldown bittikten sonra aday onaylanmalı");
        assert!(s.regime_overrides.contains_key("StrongUptrend"));
    }

    #[test]
    fn feedback_record_appends_and_bounds_window() {
        let mut fb = RegimeFeedback::default();
        for i in 0..(RegimeFeedback::WINDOW + 5) {
            fb.record(i as f64);
        }
        assert_eq!(fb.recent_pnl.len(), RegimeFeedback::WINDOW);
        assert_eq!(fb.total_trades, (RegimeFeedback::WINDOW + 5) as u32);
        // Kuyruğun en yeni elemanı son record olmalı (push_back)
        assert_eq!(*fb.recent_pnl.back().unwrap(), (RegimeFeedback::WINDOW + 4) as f64);
    }

    #[test]
    fn feedback_win_rate_counts_only_positive_pnl() {
        let mut fb = RegimeFeedback::default();
        for v in [1.0, -2.0, 0.5, -1.0, 0.0] { fb.record(v); }
        // 5 trade'den 2'si > 0 → win_rate 0.4
        assert!((fb.win_rate() - 0.4).abs() < 1e-9);
    }

    #[test]
    fn apply_feedback_holds_off_until_window_fills() {
        let mut s = ParameterStore::default();
        // İlk 9 kayıp — WINDOW=10 dolmadığı için tighten yok.
        for _ in 0..9 {
            assert!(!s.apply_trade_feedback("Ranging", -1.0));
        }
        assert!(s.regime_overrides.is_empty(),
            "WINDOW dolmadan tighten olmamalı");
    }

    #[test]
    fn apply_feedback_tightens_after_low_winrate() {
        let mut s = ParameterStore::default();
        // 10 trade, 8'i kayıp → win_rate 0.2, eşik 0.40 altında → tighten.
        for v in [-1.0, -1.0, -1.0, 0.5, -1.0, -1.0, -1.0, 0.3, -1.0, -1.0] {
            s.apply_trade_feedback("Ranging", v);
        }
        let patch = s.regime_overrides.get("Ranging").expect("tighten patch yazılmalı");
        let e = patch.edge_thresholds.expect("edge tightened");
        let r = patch.trade_risk.expect("risk tightened");
        // Base 0.20 → 0.20*1.15 = 0.23
        assert!(e.cold > 0.20);
        // Base 0.5 → 0.5*0.70 = 0.35
        assert!(r.max_position_size < 0.5);
        // Base 3.0 → 3.0*0.85 = 2.55
        assert!(r.take_profit_pct < 3.0);
    }

    #[test]
    fn apply_feedback_no_tighten_when_winrate_high() {
        let mut s = ParameterStore::default();
        // 10 trade, 7'si kazanç → win_rate 0.7 > 0.40, tighten yok.
        for v in [1.0, 1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, 1.0] {
            s.apply_trade_feedback("Ranging", v);
        }
        assert!(s.regime_overrides.get("Ranging").is_none(),
            "yüksek win_rate'te patch yazılmamalı");
    }

    #[test]
    fn regime_with_no_override_falls_back_to_base() {
        let s = ParameterStore::default();
        // Hiç override yok → base ile aynı
        let er = s.edge_thresholds_for("Ranging");
        assert_eq!(er.cold, s.edge_thresholds.cold);
        assert_eq!(er.hot,  s.edge_thresholds.hot);
        let tr = s.trade_risk_for("StrongUptrend");
        assert_eq!(tr.take_profit_pct,   s.trade_risk.take_profit_pct);
        assert_eq!(tr.max_position_size, s.trade_risk.max_position_size);
    }

    #[test]
    fn regime_override_only_replaces_specified_fields() {
        let mut s = ParameterStore::default();
        // HighVolatility için sadece edge eşiklerini sıkılaştır; trade_risk patch yok.
        let strict_edges = EdgeThresholds { cold: 0.50, warm: 0.65, hot: 0.80,
            cold_until: 0.05, warm_until: 0.30 };
        s.set_regime_patch("HighVolatility",
            RegimePatch::empty().with_edge(strict_edges));

        // HighVolatility için edge override aktif
        assert!((s.edge_threshold_for("HighVolatility", 0.0)  - 0.50).abs() < 1e-9);
        assert!((s.edge_threshold_for("HighVolatility", 0.99) - 0.80).abs() < 1e-9);
        // Ama trade_risk hâlâ base
        let tr = s.trade_risk_for("HighVolatility");
        assert_eq!(tr.take_profit_pct, 3.0);

        // Patch'siz başka bir rejim base'i kullanır
        assert!((s.edge_threshold_for("Ranging", 0.99) - 0.55).abs() < 1e-9);
    }

    #[test]
    fn regime_directional_policy_overrides_fallback() {
        let mut s = ParameterStore::default();
        // Policy yokken her rejim env fallback'e düşer (sparse, sıfır regresyon).
        assert!(s.regime_directional_for("Ranging", true));
        assert!(!s.regime_directional_for("Ranging", false));
        // HighVolatility için policy: disiplin AÇIK; StrongUptrend için KAPALI.
        s.set_regime_patch("HighVolatility",
            RegimePatch::empty().with_policy(RegimePolicy { regime_directional: Some(true) }));
        s.set_regime_patch("StrongUptrend",
            RegimePatch::empty().with_policy(RegimePolicy { regime_directional: Some(false) }));
        // Policy fallback'i EZER (her iki fallback değerinde de).
        assert!(s.regime_directional_for("HighVolatility", false), "policy=true fallback=false'u ezmeli");
        assert!(!s.regime_directional_for("StrongUptrend", true), "policy=false fallback=true'yu ezmeli");
        // Policy'siz rejim hâlâ fallback.
        assert!(s.regime_directional_for("Ranging", true));
        // is_empty: yalnız policy taşıyan patch boş sayılmaz.
        assert!(!RegimePatch::empty().with_policy(RegimePolicy { regime_directional: Some(true) }).is_empty());
    }

    #[test]
    fn interval_for_uses_map_else_fallback() {
        let mut s = ParameterStore::default();
        // Boş map → her sembol fallback (config.interval).
        assert_eq!(s.interval_for("BTCUSDT", "1h"), "1h");
        // Map'e yaz → o sembol map'ten, diğeri fallback.
        s.symbol_interval.insert("BTCUSDT".into(), "15m".into());
        assert_eq!(s.interval_for("BTCUSDT", "1h"), "15m");
        assert_eq!(s.interval_for("ETHUSDT", "1h"), "1h");
    }

    #[test]
    fn regime_trade_risk_override_only_when_set() {
        let mut s = ParameterStore::default();
        // Ranging için pos boyutunu kıs, TP daralt.
        let tight = TradeRiskParams { take_profit_pct: 1.5, stop_loss_pct: 0.8, max_position_size: 0.25 };
        s.set_regime_patch("Ranging", RegimePatch::empty().with_trade_risk(tight));

        let r = s.trade_risk_for("Ranging");
        assert_eq!(r.take_profit_pct,   1.5);
        assert_eq!(r.max_position_size, 0.25);
        // Diğer rejimler base'de kalmalı
        let u = s.trade_risk_for("StrongUptrend");
        assert_eq!(u.take_profit_pct,   3.0);
        assert_eq!(u.max_position_size, 0.5);
    }

    #[test]
    fn from_env_overrides_partial_fill_and_scalp_and_sr() {
        std::env::set_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT", "2.5");
        std::env::set_var("SCALP_SWING_THRESHOLD_MIN",     "15");
        std::env::set_var("SR_UPDATE_EVERY_SECS",          "10");
        let s = ParameterStore::from_env();
        std::env::remove_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT");
        std::env::remove_var("SCALP_SWING_THRESHOLD_MIN");
        std::env::remove_var("SR_UPDATE_EVERY_SECS");
        assert!((s.partial_fill.max_slippage_pct - 2.5).abs() < 1e-9);
        assert_eq!(s.scalp_swing_threshold_min, 15);
        assert_eq!(s.sr_update_every_secs,      10);
        // Diğer alanlar default'ta kalmalı
        assert_eq!(s.partial_fill.overfill_tolerance, 0.001);
    }
}
