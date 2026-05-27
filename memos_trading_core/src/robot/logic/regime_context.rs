// robot/logic/regime_context.rs — Piyasa Rejim Bağlamı (Adım 1 izolasyonu)
//
// Hedef mimari: "AI/regime tespiti yalnız Adım 1'de, GENİŞ zaman diliminde (HTF),
// SEYREK çalışır ve sisteme bir Regime Context sağlar; hızlı olması gerekmez. Edge/
// sizing/trigger (Adım 2-5) ise hızlı matematik matrisinin işidir." Bu modül o
// bağlamı izole eder:
//   - `RegimeContext` : üretilmiş rejim + meta (kaynak TF, zaman, dedektör, ADX/ATR).
//   - `RegimeDetector`: pluggable tespit sözleşmesi — bugün `MathRegimeDetector`
//     (ADX/momentum, tek-kaynak `classify_market_regime`), yarın `OnnxRegimeDetector`.
//   - Cache (`RegimeCache`): per-sembol son bağlam; cycle hot-path her 500ms yeniden
//     hesaplamak yerine TTL içinde buradan OKUR (seyreklik). Bayatsa dedektör HTF
//     mumlarından yeniden üretir.
//
// Böylece regime üretimi cycle'dan ayrışır; ileride ONNX eklemek = yeni bir
// RegimeDetector kolu (Engine'e ya da hot-path'e dokunmadan).

use std::collections::HashMap;
use crate::core::types::Candle;
use crate::evolution::MarketRegime;
use crate::robot::logic::market_regime::{classify_market_regime, compute_adx_from_candles, compute_atr_pct};

/// Bir sembol için üretilmiş rejim bağlamı (cache değeri).
#[derive(Debug, Clone)]
pub struct RegimeContext {
    pub regime: MarketRegime,
    pub adx: f64,
    pub atr_pct: f64,
    /// Hangi TF'den üretildi (örn. "4h" HTF, "1m" base) — gözlemlenebilirlik.
    pub source_interval: String,
    /// epoch ms (TTL tazelik kontrolü).
    pub computed_at_ms: u64,
    /// Üreten dedektör adı ("math" | "onnx" | …) — telemetri/log için.
    pub detector: &'static str,
}

impl RegimeContext {
    /// `now_ms` anında bu bağlam hâlâ taze mi? `ttl_ms == 0` → asla taze (her
    /// çağrı yeniden hesaplar = legacy per-cycle davranış). Saf → birim test edilir.
    pub fn is_fresh(&self, now_ms: u64, ttl_ms: u64) -> bool {
        ttl_ms > 0 && now_ms.saturating_sub(self.computed_at_ms) < ttl_ms
    }
}

/// Per-sembol rejim bağlamı önbelleği (BrainBox'ta `Arc<RwLock<_>>` olarak tutulur).
pub type RegimeCache = HashMap<String, RegimeContext>;

/// Pluggable rejim tespiti. `Send + Sync` — paralel sembol cycle'ından erişilir.
pub trait RegimeDetector: Send + Sync {
    /// Verilen mum dizisinden rejimi üretir. `candles` tercihen HTF (4h/1d) dilimidir;
    /// hızlı olması beklenmez (seyrek çağrılır).
    fn detect(&self, candles: &[Candle]) -> MarketRegime;
    fn name(&self) -> &'static str;
}

/// Matematiksel (ADX + momentum) dedektör — tek-kaynak `classify_market_regime`.
/// Mevcut/varsayılan davranış; ONNX gelene kadar üretimde bu çalışır.
pub struct MathRegimeDetector;

impl RegimeDetector for MathRegimeDetector {
    fn detect(&self, candles: &[Candle]) -> MarketRegime {
        classify_market_regime(candles)
    }
    fn name(&self) -> &'static str { "math" }
}

/// Aktif rejim dedektörünü döndürür. `default_registry()` deseninin rejim karşılığı:
/// env `REGIME_DETECTOR` ile seçilir (şimdilik yalnız "math"; "onnx" ileride bir kol
/// olarak eklenecek). Bilinmeyen değer → math'e düşer (log uyarısı çağırana bırakıldı).
pub fn default_regime_detector() -> Box<dyn RegimeDetector> {
    match std::env::var("REGIME_DETECTOR").ok().as_deref() {
        // Gelecek: Some("onnx") => Box::new(OnnxRegimeDetector::from_env()),
        _ => Box::new(MathRegimeDetector),
    }
}

/// Bir dedektörle `RegimeContext` üretir (cache yazımı çağırana ait). `adx`/`atr_pct`
/// gözlemlenebilirlik için ayrıca hesaplanır (rejim kararını değiştirmez).
pub fn build_context(
    detector: &dyn RegimeDetector,
    candles: &[Candle],
    source_interval: &str,
    now_ms: u64,
) -> RegimeContext {
    RegimeContext {
        regime: detector.detect(candles),
        adx: compute_adx_from_candles(candles),
        atr_pct: compute_atr_pct(candles),
        source_interval: source_interval.to_string(),
        computed_at_ms: now_ms,
        detector: detector.name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn candles(closes: &[f64]) -> Vec<Candle> {
        closes.iter().enumerate().map(|(i, &c)| Candle {
            timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64 * 60, 0).unwrap(),
            open: c, high: c * 1.01, low: c * 0.99, close: c,
            volume: 1.0, symbol: "T".into(), interval: "1m".into(),
        }).collect()
    }

    #[test]
    fn math_detector_parity_with_classify() {
        // MathRegimeDetector, tek-kaynak classify_market_regime ile birebir aynı.
        let up: Vec<f64> = (0..60).map(|i| 100.0 + i as f64).collect();
        let cs = candles(&up);
        assert_eq!(MathRegimeDetector.detect(&cs), classify_market_regime(&cs));
        assert_eq!(MathRegimeDetector.name(), "math");
    }

    #[test]
    fn is_fresh_respects_ttl() {
        let ctx = RegimeContext {
            regime: MarketRegime::Ranging, adx: 20.0, atr_pct: 1.0,
            source_interval: "4h".into(), computed_at_ms: 1_000, detector: "math",
        };
        assert!(ctx.is_fresh(1_500, 1_000), "Δ500 < ttl1000 → taze");
        assert!(!ctx.is_fresh(2_500, 1_000), "Δ1500 ≥ ttl1000 → bayat");
        assert!(!ctx.is_fresh(1_000, 0), "ttl=0 → asla taze (legacy per-cycle)");
    }

    #[test]
    fn build_context_populates_meta() {
        let up: Vec<f64> = (0..60).map(|i| 100.0 + i as f64).collect();
        let ctx = build_context(&MathRegimeDetector, &candles(&up), "4h", 42);
        assert_eq!(ctx.source_interval, "4h");
        assert_eq!(ctx.computed_at_ms, 42);
        assert_eq!(ctx.detector, "math");
    }
}
