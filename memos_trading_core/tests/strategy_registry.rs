// Faz 4 c2 — StrategyRegistry entegrasyon testleri.
//
// Bu paket registry'nin canon davranışını ve onunla birlikte değişen iki
// tüketiciyi (optimizer::make_strategy_pub ve strategies::StrategySelector)
// uçtan uca doğrular. Registry'nin kendi içindeki unit testler
// `src/robot/strategies/registry.rs` içinde; buradakiler kütüphane sınırı
// dışından bakar.

use memos_trading_core::core::types::{Candle, Signal, StrategyParams};
use memos_trading_core::robot::logic::optimizer::make_strategy_pub;
use memos_trading_core::robot::strategies::{
    default_registry, Strategy, StrategyRegistry,
};
use memos_trading_core::robot::strategies::strategy_selector::StrategySelector;
use std::sync::Arc;

fn bar(c: f64) -> Candle {
    Candle { open: c, high: c + 0.5, low: c - 0.5, close: c, volume: 100.0, ..Default::default() }
}

fn closes(xs: &[f64]) -> Vec<Candle> {
    xs.iter().map(|&c| bar(c)).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Registry → make / canonical / fallback
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn registry_make_produces_runnable_strategy() {
    let r = default_registry();
    let strat = r.make("RSI");
    let candles = closes(&(0..60).map(|i| 100.0 + i as f64 * 0.5).collect::<Vec<_>>());
    let sig = strat.generate_signal(&candles, &StrategyParams::default(), None, None).unwrap();
    // RSI sinyali bir Signal varyantı olmalı (Hold dahil) — ama panik yok.
    assert!(matches!(sig, Signal::Buy | Signal::Sell | Signal::Hold));
}

#[test]
fn registry_alias_and_case_resolve_to_same_struct_name() {
    let r = default_registry();
    let bb_long  = r.make("BOLLINGER_BANDS");
    let bb_short = r.make("bb");
    assert_eq!(bb_long.name(), bb_short.name(),
        "BB alias ve case-insensitive aynı stratejiye düşmeli");
}

#[test]
fn unknown_name_falls_back_to_registry_default() {
    let r = default_registry();
    let unk = r.make("BU_AD_KAYITLI_DEGIL");
    let def = r.make(r.default_name());
    assert_eq!(unk.name(), def.name(),
        "Bilinmeyen ad default_name'e düşmeli (MA_CROSSOVER)");
}

// ─────────────────────────────────────────────────────────────────────────────
// optimizer::make_strategy_pub artık registry'ye delege ediyor
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn make_strategy_pub_uses_default_registry_for_aliases() {
    // "DEFAULT" alias'ı MA_CROSSOVER'a çözülmeli — geri uyumluluk garantisi.
    let s1 = make_strategy_pub("DEFAULT");
    let s2 = make_strategy_pub("MA_CROSSOVER");
    assert_eq!(s1.name(), s2.name());
}

#[test]
fn make_strategy_pub_falls_back_on_unknown_legacy_name() {
    // Eski ml_engine selector "SUPERTREND_MACD" gibi sahte adlar üretiyordu;
    // registry fallback'i sayesinde sistem MA_CROSSOVER ile çalışmaya devam eder.
    let legacy = make_strategy_pub("SUPERTREND_MACD");
    let default = make_strategy_pub("MA_CROSSOVER");
    assert_eq!(legacy.name(), default.name());
}

// ─────────────────────────────────────────────────────────────────────────────
// strategies::StrategySelector::from_registry — özelleşmiş aday seti
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn selector_from_registry_loads_requested_names_only() {
    let r = default_registry();
    let sel = StrategySelector::from_registry(&r, &["RSI", "MACD"]);
    assert_eq!(sel.strategies.len(), 2);
    // Boş veride bile select_best panik etmez.
    let (_strat, sig) = sel.select_best(&closes(&[100.0; 5]), &StrategyParams::default());
    assert!(matches!(sig, Signal::Buy | Signal::Sell | Signal::Hold));
}

#[test]
fn selector_with_strategies_accepts_custom_chain() {
    // Test/özel kullanım: kendi stratejilerimle selector kurabiliyor muyum?
    struct AlwaysBuy;
    impl Strategy for AlwaysBuy {
        fn generate_signal(
            &self,
            _candles: &[Candle],
            _params: &StrategyParams,
            _funding: Option<&[memos_trading_core::core::types::FundingRatePoint]>,
            _htf: Option<&[Candle]>,
        ) -> memos_trading_core::Result<Signal> { Ok(Signal::Buy) }
        fn name(&self) -> &str { "always_buy" }
    }

    let sel = StrategySelector::with_strategies(vec![Box::new(AlwaysBuy)]);
    // Yeterli bar olmayan kısa seri → simulate_score 0 döner, ama Strategy yine
    // sinyal üretir. select_best en az tek stratejide döner.
    let candles = closes(&(0..60).map(|i| 100.0 + i as f64).collect::<Vec<_>>());
    let (strat, sig) = sel.select_best(&candles, &StrategyParams::default());
    assert_eq!(strat.name(), "always_buy");
    assert_eq!(sig, Signal::Buy);
}

// ─────────────────────────────────────────────────────────────────────────────
// Runtime'da yeni strateji enjeksiyonu
// ─────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────
// canonical_pool — backtest pool'unun otomatik genişlemesi
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn canonical_pool_includes_all_unique_strategies_without_aliases() {
    let r = default_registry();
    let pool = r.canonical_pool();
    // Bilinen canonical adların hepsi pool'da
    for n in &[
        "MA_CROSSOVER", "EMA_CROSSOVER", "MACD", "SUPERTREND",
        "RSI", "STOCH_RSI", "CCI",
        "BB", "DONCHIAN",
        "PRICE_ACTION", "ICT_FVG", "SMC", "ICT_OB", "ICT_COMPOSITE",
        "FUNDING_CONTRARIAN",
    ] {
        assert!(pool.iter().any(|x| x == n), "canonical_pool'da eksik: {n}");
    }
    // Alias'lar pool dışında — yine make() ile çözülürler ama backtest tek bir
    // strateji üzerinde mükerrer iş yapmaz.
    for a in &["MA", "DEFAULT", "EMA", "STOCHASTIC_RSI", "BOLLINGER_BANDS"] {
        assert!(!pool.iter().any(|x| x == a),
            "canonical_pool alias içeremez: {a}");
    }
    // En az 15 benzersiz strateji
    assert!(pool.len() >= 15, "pool ≥ 15 strateji bekleniyor, gelen: {}", pool.len());
}

#[test]
fn canonical_pool_grows_when_new_strategy_is_registered() {
    let mut r = default_registry();
    let before = r.canonical_pool().len();
    // Yeni strateji: ICT_BREAKER_BLOCK varsayımı (gerçek struct gerekmez —
    // closure içinde mevcut bir struct'ı sarmak yeterli; pool sayısının
    // arttığını doğruluyoruz).
    r.register("ICT_BREAKER_BLOCK",
        std::sync::Arc::new(|| Box::new(
            memos_trading_core::robot::strategies::MaCrossoverStrategy
        ) as Box<dyn Strategy>));
    let after = r.canonical_pool();
    assert_eq!(after.len(), before + 1);
    assert!(after.iter().any(|x| x == "ICT_BREAKER_BLOCK"));
}

#[test]
fn runtime_registration_adds_new_strategy() {
    struct EchoSell;
    impl Strategy for EchoSell {
        fn generate_signal(
            &self,
            _candles: &[Candle],
            _params: &StrategyParams,
            _funding: Option<&[memos_trading_core::core::types::FundingRatePoint]>,
            _htf: Option<&[Candle]>,
        ) -> memos_trading_core::Result<Signal> { Ok(Signal::Sell) }
        fn name(&self) -> &str { "echo_sell" }
    }

    let mut r: StrategyRegistry = default_registry();
    r.register("ECHO_SELL", Arc::new(|| Box::new(EchoSell) as Box<dyn Strategy>));
    assert!(r.contains("echo_sell"));
    let s = r.make("ECHO_SELL");
    assert_eq!(s.name(), "echo_sell");
}
