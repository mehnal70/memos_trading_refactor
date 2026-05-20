// robot/strategies/registry.rs — Faz 4 c2: Strateji plug-in registry.
//
// Tasarım — RiskFilter chain pattern'ı (Faz 4 c1) ile aynı çizgide:
//   - Plug-in noktası: closure-tabanlı factory (`Arc<dyn Fn -> Box<dyn Strategy>>`)
//   - Default kayıt: `default_registry()`; runtime'da `register/with_factory` ile
//     ek strateji enjekte edilebilir.
//   - İsim çözümü: case-insensitive (`make("rsi") == make("RSI")`), alias destekli
//     (örn. "BB" ↔ "BOLLINGER_BANDS").
//   - Bilinmeyen isim → `default_name` (varsayılan: "MA_CROSSOVER") fallback;
//     böylece HyperOpt'tan yanlış strateji ismi gelse bile sistem çökmez ve
//     `make_strategy_pub` davranışı korunur.
//
// Test edilebilirlik: registry MissionControl/AppState olmadan kurulabilir;
// her factory küçük (zero-state) struct ürettiği için clone hızlıdır.

use std::collections::HashMap;
use std::sync::Arc;

use super::base::Strategy;
use super::{
    BollingerBandsStrategy, CciStrategy, DonchianChannelStrategy, EmaCrossoverStrategy,
    FundingRateContrarianStrategy, IctCompositeStrategy, IctFvgStrategy, IctOrderBlockStrategy,
    MaCrossoverStrategy, MacdStrategy, PriceActionStrategy, RsiStrategy, SmcStrategy,
    StochasticRsiStrategy, SupertrendStrategy,
};

/// Bir strateji adına karşılık gelen factory. Her çağrı bağımsız bir
/// `Box<dyn Strategy>` döndürür → paylaşılan mutable state yok.
pub type StrategyFactory = Arc<dyn Fn() -> Box<dyn Strategy> + Send + Sync>;

/// Plug-in registry. İsim → factory eşlemesi, bilinmeyene düşmek için
/// `default_name` tutar.
pub struct StrategyRegistry {
    entries: HashMap<String, StrategyFactory>,
    default_name: String,
}

impl StrategyRegistry {
    /// Boş registry. Genelde `default_registry()` ile başlamak daha uygundur.
    pub fn new(default_name: impl Into<String>) -> Self {
        Self {
            entries: HashMap::new(),
            default_name: canonical(&default_name.into()),
        }
    }

    /// Yeni strateji kaydı. Aynı canonical isim üzerinde önceki kayıt
    /// üzerine yazılır (alias override etmek için kullanılır).
    pub fn register(
        &mut self,
        name: impl Into<String>,
        factory: StrategyFactory,
    ) -> &mut Self {
        self.entries.insert(canonical(&name.into()), factory);
        self
    }

    /// Convenience: zero-state strateji struct'larını closure'a sarmak için.
    /// `register_zst::<RsiStrategy>("RSI")` gibi kullanılır.
    pub fn register_zst<S>(&mut self, name: impl Into<String>) -> &mut Self
    where
        S: Strategy + Default + 'static,
    {
        self.register(name, Arc::new(|| Box::new(S::default()) as Box<dyn Strategy>))
    }

    /// Aynı factory'yi birden çok alias altında kaydeder.
    pub fn register_aliases(
        &mut self,
        aliases: &[&str],
        factory: StrategyFactory,
    ) -> &mut Self {
        for alias in aliases {
            self.register(*alias, factory.clone());
        }
        self
    }

    /// İsim → strateji. Bilinmeyen ad geldiyse `default_name`'e düşer; default
    /// da kayıtsızsa son çare olarak MaCrossoverStrategy döndürür (panik yok).
    pub fn make(&self, name: &str) -> Box<dyn Strategy> {
        let key = canonical(name);
        if let Some(f) = self.entries.get(&key) {
            return f();
        }
        if let Some(f) = self.entries.get(&self.default_name) {
            return f();
        }
        Box::new(MaCrossoverStrategy)
    }

    /// Kayıtlı tüm canonical isimlerin sıralı listesi (deterministik çıktı).
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.entries.keys().cloned().collect();
        v.sort();
        v
    }

    /// Bir ismin kayıtlı olup olmadığını söyler (alias dahil).
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(&canonical(name))
    }

    pub fn default_name(&self) -> &str {
        &self.default_name
    }
}

/// Tüm strateji isimleri tek noktada normalize edilir: trim + uppercase.
/// Böylece `"rsi"`, `"RSI "`, `"Rsi"` aynı slot'a düşer.
fn canonical(name: &str) -> String {
    name.trim().to_uppercase()
}

/// Projenin varsayılan strateji kümesi. `make_strategy_pub` ve
/// `StrategySelector` bu registry'yi kullanır. Yeni bir strateji eklemek
/// istenirse buraya satır eklemek yeterli — engine tarafında değişiklik
/// gerekmez.
pub fn default_registry() -> StrategyRegistry {
    let mut r = StrategyRegistry::new("MA_CROSSOVER");

    // Trend ailesi
    r.register_aliases(
        &["MA", "MA_CROSSOVER", "DEFAULT"],
        Arc::new(|| Box::new(MaCrossoverStrategy) as Box<dyn Strategy>),
    );
    r.register_aliases(
        &["EMA", "EMA_CROSSOVER"],
        Arc::new(|| Box::new(EmaCrossoverStrategy) as Box<dyn Strategy>),
    );
    r.register("MACD", Arc::new(|| Box::new(MacdStrategy) as Box<dyn Strategy>));
    r.register(
        "SUPERTREND",
        Arc::new(|| Box::new(SupertrendStrategy) as Box<dyn Strategy>),
    );

    // Osilatör ailesi
    r.register("RSI", Arc::new(|| Box::new(RsiStrategy) as Box<dyn Strategy>));
    r.register_aliases(
        &["STOCH_RSI", "STOCHASTIC_RSI"],
        Arc::new(|| Box::new(StochasticRsiStrategy) as Box<dyn Strategy>),
    );
    r.register("CCI", Arc::new(|| Box::new(CciStrategy) as Box<dyn Strategy>));

    // Volatilite & kanal
    r.register_aliases(
        &["BB", "BOLLINGER_BANDS"],
        Arc::new(|| Box::new(BollingerBandsStrategy) as Box<dyn Strategy>),
    );
    r.register(
        "DONCHIAN",
        Arc::new(|| Box::new(DonchianChannelStrategy) as Box<dyn Strategy>),
    );

    // Price action + SMC ailesi
    r.register(
        "PRICE_ACTION",
        Arc::new(|| Box::new(PriceActionStrategy) as Box<dyn Strategy>),
    );
    r.register(
        "ICT_FVG",
        Arc::new(|| Box::new(IctFvgStrategy) as Box<dyn Strategy>),
    );
    r.register("SMC", Arc::new(|| Box::new(SmcStrategy) as Box<dyn Strategy>));
    r.register(
        "ICT_OB",
        Arc::new(|| Box::new(IctOrderBlockStrategy) as Box<dyn Strategy>),
    );
    r.register(
        "ICT_COMPOSITE",
        Arc::new(|| Box::new(IctCompositeStrategy) as Box<dyn Strategy>),
    );

    // Funding (perpetual)
    r.register(
        "FUNDING_CONTRARIAN",
        Arc::new(|| Box::new(FundingRateContrarianStrategy::default()) as Box<dyn Strategy>),
    );

    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_contains_canonical_names() {
        let r = default_registry();
        for n in &[
            "RSI", "MACD", "BB", "SUPERTREND", "EMA", "STOCH_RSI", "CCI",
            "PRICE_ACTION", "ICT_FVG", "SMC", "ICT_OB", "ICT_COMPOSITE",
            "MA_CROSSOVER", "DONCHIAN", "FUNDING_CONTRARIAN",
        ] {
            assert!(r.contains(n), "kayıtlı olması gereken strateji: {n}");
        }
    }

    #[test]
    fn make_is_case_insensitive_and_alias_aware() {
        let r = default_registry();
        let a = r.make("rsi").name().to_string();
        let b = r.make("RSI").name().to_string();
        assert_eq!(a, b);

        // BB ↔ BOLLINGER_BANDS aynı strateji
        let x = r.make("BB").name().to_string();
        let y = r.make("BOLLINGER_BANDS").name().to_string();
        assert_eq!(x, y);
    }

    #[test]
    fn unknown_name_falls_back_to_default() {
        let r = default_registry();
        let fallback = r.make("BILINMEYEN_STRATEJI");
        let default  = r.make(r.default_name().to_string().as_str());
        assert_eq!(fallback.name(), default.name());
    }

    #[test]
    fn custom_factory_can_be_registered_at_runtime() {
        struct DummyStrat;
        impl Strategy for DummyStrat {
            fn generate_signal(
                &self,
                _candles: &[crate::core::types::Candle],
                _params: &crate::core::types::StrategyParams,
                _funding: Option<&[crate::core::types::FundingRatePoint]>,
                _htf: Option<&[crate::core::types::Candle]>,
            ) -> crate::Result<crate::core::types::Signal> {
                Ok(crate::core::types::Signal::Hold)
            }
            fn name(&self) -> &str { "dummy" }
        }

        let mut r = default_registry();
        r.register("DUMMY", Arc::new(|| Box::new(DummyStrat) as Box<dyn Strategy>));
        assert!(r.contains("dummy"));
        assert_eq!(r.make("DUMMY").name(), "dummy");
    }

    #[test]
    fn names_listing_is_sorted_and_includes_aliases() {
        let r = default_registry();
        let names = r.names();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        // Hem alias hem canonical aynı listede:
        assert!(names.iter().any(|n| n == "BB"));
        assert!(names.iter().any(|n| n == "BOLLINGER_BANDS"));
    }
}
