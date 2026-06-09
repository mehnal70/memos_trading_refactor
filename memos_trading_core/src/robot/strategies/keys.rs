// robot/strategies/keys.rs — Strateji parametre adlarının TEK-KAYNAK sözlüğü.
//
// Tüm param adları burada `pub const &str` olarak tanımlı: bir typo derleme hatası
// olur (yanlış yazılmış string literal yerine), grep'lenebilir, ve `param_spec`
// bildirimi ↔ `StrategyParams` okuması ↔ optimizer aynı sabite bağlanır.
//
// `StrategyParams` (core/types.rs) bu modüle bağımlı DEĞİL — torba nötr `&str`
// alır/saklar (katman temiz). Burası yalnız adları ve bilinirlik doğrulamasını
// (`is_known`/`intern`) sağlar; `apply_param` bunu typo-güvenliği için kullanır.

// --- Mevcut yapısal parametreler (eski StrategyParams alanları) ---
pub const FAST: &str = "fast";
pub const SLOW: &str = "slow";
pub const PERIOD: &str = "period";
pub const FAST_PERIOD: &str = "fast_period";
pub const SLOW_PERIOD: &str = "slow_period";
pub const SIGNAL_PERIOD: &str = "signal_period";
pub const BB_PERIOD: &str = "bb_period";
pub const OVERBOUGHT: &str = "overbought";
pub const OVERSOLD: &str = "oversold";
pub const STD_DEV: &str = "std_dev";

// --- Eskiden gömülü olup torbaya açılan sabitler (Faz 2) ---
// Hiçbiri varsayılan olarak `param_spec()`'e eklenmez (optimizer tunelemez →
// overfit yok); yalnız config/ParameterStore override edebilir.
pub const HTF_FAST: &str = "htf_fast";
pub const HTF_SLOW: &str = "htf_slow";
pub const SMOOTH_K: &str = "smooth_k";
pub const SMOOTH_D: &str = "smooth_d";
pub const DOJI_RATIO: &str = "doji_ratio";
pub const ENGULF_RATIO: &str = "engulf_ratio";
pub const PIN_RATIO: &str = "pin_ratio";
pub const FVG_PROXIMITY: &str = "fvg_proximity";
pub const FUNDING_THRESHOLD: &str = "funding_threshold";

/// Bilinen tüm param adları — `apply_param` typo-doğrulaması ve testlerin
/// "param_spec adı tanınıyor mu" kontrolü için tek-kaynak.
pub const ALL: &[&str] = &[
    FAST, SLOW, PERIOD, FAST_PERIOD, SLOW_PERIOD, SIGNAL_PERIOD, BB_PERIOD,
    OVERBOUGHT, OVERSOLD, STD_DEV,
    HTF_FAST, HTF_SLOW, SMOOTH_K, SMOOTH_D, DOJI_RATIO, ENGULF_RATIO, PIN_RATIO,
    FVG_PROXIMITY, FUNDING_THRESHOLD,
];

/// Bir param adı bilinen sözlükte mi? `apply_param` bilinmeyen adı sessizce
/// yok sayar (param_spec typo'sunu yakalama semantiği korunur).
#[inline]
pub fn is_known(name: &str) -> bool {
    ALL.contains(&name)
}

/// Bilinen ada karşılık gelen kanonik `&'static str`'i döner (yoksa `None`).
#[inline]
pub fn intern(name: &str) -> Option<&'static str> {
    ALL.iter().copied().find(|&k| k == name)
}
