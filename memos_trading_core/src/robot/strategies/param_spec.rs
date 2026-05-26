// robot/strategies/param_spec.rs — Strateji parametre uzayı (modüler optimizasyon temeli)
//
// Her strateji `Strategy::param_spec()` ile KENDİ ayarlanabilir parametre uzayını
// bildirir (hangi alan, hangi aralık, hangi adım). Optimizer (HyperOpt / backtest job)
// bu listeyi tüketerek arama ızgarasını/rastgele örneğini üretir — eskiden tüm
// stratejilere uygulanan hardcoded grid (`fast 3-15`...) yerine.
//
// `name` doğrudan `StrategyParams` alan adıdır; örneği parametreye uygulamak tek
// noktadan (`apply_param`) yapılır → yeni bir alan eklemek için tek yer değişir
// (utils.rs grid_search_optimization da bunu kullanır). [[feedback_market_agnostic]]
// tek-kaynak prensibiyle uyumlu.

use crate::core::types::StrategyParams;

/// Parametrenin değer türü. Örneği `StrategyParams`'a uygularken tamsayı alanlar
/// (periyot/bar sayısı) usize'a yuvarlanır; float alanlar aynen geçer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// Tamsayı periyot/bar sayısı (RSI 14, MA fast 5…). usize'a yuvarlanır.
    Int,
    /// Sürekli float eşik/çarpan (Supertrend mult 3.0, BB std_dev 2.0…).
    Float,
    /// Yüzde-eksenli eşik (RSI overbought 70, CCI ±100…). Float gibi taranır;
    /// UI/rapor ayrımı ve gelecekte fiyat/ATR-göreli normalizasyon için işaretlidir.
    Pct,
}

/// Bir stratejinin tek bir ayarlanabilir parametresinin kapalı arama aralığı
/// `[min, max]`, `step` çözünürlükle. `name` = `StrategyParams` alan adı.
#[derive(Debug, Clone)]
pub struct ParamSpec {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub kind: ParamKind,
}

impl ParamSpec {
    /// Tamsayı periyot parametresi (örn. `int("period", 7.0, 21.0, 1.0)`).
    pub fn int(name: &'static str, min: f64, max: f64, step: f64) -> Self {
        Self { name, min, max, step, kind: ParamKind::Int }
    }
    /// Sürekli float parametre (örn. `float("std_dev", 1.5, 4.0, 0.5)`).
    pub fn float(name: &'static str, min: f64, max: f64, step: f64) -> Self {
        Self { name, min, max, step, kind: ParamKind::Float }
    }
    /// Yüzde-eksenli eşik (örn. `pct("overbought", 65.0, 85.0, 5.0)`).
    pub fn pct(name: &'static str, min: f64, max: f64, step: f64) -> Self {
        Self { name, min, max, step, kind: ParamKind::Pct }
    }

    /// Bu spec'in `min..=max` ızgara değerleri (kapalı aralık, `step` adımlı).
    /// Int için tamsayıya yuvarlanmış benzersiz değerler döner.
    pub fn grid_values(&self) -> Vec<f64> {
        let step = if self.step.abs() < 1e-9 { 1.0 } else { self.step.abs() };
        let mut out = Vec::new();
        let mut v = self.min;
        // Kayan nokta birikimini engellemek için tam adım sayısı üzerinden üret.
        let n = ((self.max - self.min) / step).floor() as i64;
        for i in 0..=n.max(0) {
            v = self.min + i as f64 * step;
            out.push(self.quantize(v));
        }
        // Üst sınırı da garanti et (step max'a tam oturmazsa).
        let top = self.quantize(self.max);
        if out.last().map(|&l| (l - top).abs() > 1e-9).unwrap_or(true) {
            out.push(top);
        }
        out.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let _ = v;
        out
    }

    /// `u ∈ [0,1)` birim örneğini bu aralıktaki bir değere eşler (rastgele arama).
    /// PRNG'yi çağıran sağlar → bu modül determinizm/tohum politikasından bağımsız.
    pub fn sample(&self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        self.quantize(self.min + u * (self.max - self.min))
    }

    /// Değeri türüne göre normalize eder (Int → en yakın tam sayı, aralığa kırpılı).
    fn quantize(&self, v: f64) -> f64 {
        let v = v.clamp(self.min, self.max);
        match self.kind {
            ParamKind::Int => v.round(),
            ParamKind::Float | ParamKind::Pct => v,
        }
    }
}

/// TEK-KAYNAK: bir `StrategyParams` alanını ADIYLA set eder. Bilinmeyen ad sessizce
/// yok sayılır. utils.rs grid_search_optimization ve HyperOpt spec-araması aynı
/// haritayı kullanır → yeni alan eklerken yalnız burası güncellenir.
pub fn apply_param(p: &mut StrategyParams, name: &str, value: f64) {
    let as_period = || value.round().max(1.0) as usize;
    match name {
        "fast"          => p.fast = Some(as_period()),
        "slow"          => p.slow = Some(as_period()),
        "period"        => p.period = Some(as_period()),
        "fast_period"   => p.fast_period = Some(as_period()),
        "slow_period"   => p.slow_period = Some(as_period()),
        "signal_period" => p.signal_period = Some(as_period()),
        "bb_period"     => p.bb_period = Some(as_period()),
        "overbought"    => p.overbought = Some(value),
        "oversold"      => p.oversold = Some(value),
        "std_dev"       => p.std_dev = Some(value),
        _ => {}
    }
}

/// Bir spec listesi + ona paralel değer vektöründen `StrategyParams` kurar
/// (taban = `default()`). `values.len()` < `specs.len()` ise eksik alanlar
/// default kalır.
pub fn build_params(specs: &[ParamSpec], values: &[f64]) -> StrategyParams {
    let mut p = StrategyParams::default();
    for (spec, &val) in specs.iter().zip(values.iter()) {
        apply_param(&mut p, spec.name, val);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_values_int_kapali_aralik() {
        let s = ParamSpec::int("period", 7.0, 21.0, 7.0);
        assert_eq!(s.grid_values(), vec![7.0, 14.0, 21.0]);
    }

    #[test]
    fn grid_values_ust_siniri_kapsar() {
        // step max'a tam oturmuyor → üst sınır yine eklenir.
        let s = ParamSpec::int("fast", 3.0, 10.0, 3.0); // 3,6,9 + 10
        assert_eq!(s.grid_values(), vec![3.0, 6.0, 9.0, 10.0]);
    }

    #[test]
    fn sample_int_yuvarlanir_ve_kirpilir() {
        let s = ParamSpec::int("period", 5.0, 15.0, 1.0);
        assert_eq!(s.sample(0.0), 5.0);
        assert_eq!(s.sample(1.0), 15.0);
        let v = s.sample(0.5);
        assert!(v.fract() == 0.0, "Int örnek tam sayı olmalı, geldi {v}");
        assert!((5.0..=15.0).contains(&v));
    }

    #[test]
    fn apply_param_alanlara_yazar() {
        let mut p = StrategyParams::default();
        apply_param(&mut p, "fast", 5.4);
        apply_param(&mut p, "slow", 20.0);
        apply_param(&mut p, "std_dev", 2.5);
        apply_param(&mut p, "overbought", 72.0);
        apply_param(&mut p, "bilinmeyen", 9.0); // sessiz yok sayılır
        assert_eq!(p.fast, Some(5)); // 5.4 → round 5
        assert_eq!(p.slow, Some(20));
        assert_eq!(p.std_dev, Some(2.5));
        assert_eq!(p.overbought, Some(72.0));
    }

    #[test]
    fn build_params_paralel_vektor() {
        let specs = vec![
            ParamSpec::int("period", 7.0, 21.0, 1.0),
            ParamSpec::pct("overbought", 65.0, 85.0, 5.0),
        ];
        let p = build_params(&specs, &[14.0, 70.0]);
        assert_eq!(p.period, Some(14));
        assert_eq!(p.overbought, Some(70.0));
    }
}
