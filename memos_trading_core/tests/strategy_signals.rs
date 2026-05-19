// Strateji crossing davranışları ve ensemble ağırlıklı oylama birim testleri (D9).
//
// Kapsam:
//   - MacdStrategy crossing — flood yok davranışı
//   - RsiStrategy crossing — yeni OB/OS girişi
//   - StochasticRsiStrategy crossing
//   - PriceActionStrategy doji koruması
//   - StrategyEnsemble confidence-weighted voting
//   - StrategySelector::simulate_score deterministik veride mantıklı skor

use memos_trading_core::core::types::{Candle, Signal, StrategyParams};
use memos_trading_core::robot::strategies::{
    Strategy, MacdStrategy, RsiStrategy, PriceActionStrategy,
    StrategyEnsemble,
};
use memos_trading_core::robot::strategies::oscillator::StochasticRsiStrategy;
use memos_trading_core::robot::strategies::strategy_selector::StrategySelector;

fn bar(open: f64, high: f64, low: f64, close: f64) -> Candle {
    Candle { open, high, low, close, volume: 100.0, ..Default::default() }
}

fn closes(closes: &[f64]) -> Vec<Candle> {
    closes.iter().map(|&c| bar(c, c + 0.5, c - 0.5, c)).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// MACD crossing — eski "snapshot" davranışı flood üretiyordu, yeni davranış
// sadece kesişim barı sinyal verir.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn macd_no_signal_when_lines_drift_apart_without_cross() {
    // Düz monoton yukarı seri — macd > signal'da kalır ama kesişim yok → Hold.
    // (Eski sürüm bu durumda her bar Buy üretirdi.)
    let candles = closes(&(0..60).map(|i| 100.0 + i as f64 * 0.5).collect::<Vec<_>>());
    let strat = MacdStrategy;
    let params = StrategyParams::default();
    // Birkaç bar daha eklenmiş veri — son barda macd > signal ama kesişim yok.
    let sig = strat.generate_signal(&candles, &params, None, None).unwrap();
    assert_eq!(sig, Signal::Hold, "Sürekli yukarı trendde MACD flood yapmamalı");
}

// ─────────────────────────────────────────────────────────────────────────────
// RSI crossing — sadece eşiğe yeni girişte sinyal verir.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn rsi_hold_when_already_overbought() {
    // Sürekli yukarı: RSI 80+ olur ve orada kalır → yeni "giriş" yok → Hold.
    let candles = closes(&(0..50).map(|i| 100.0 + i as f64 * 1.0).collect::<Vec<_>>());
    let strat = RsiStrategy;
    let sig = strat.generate_signal(&candles, &StrategyParams::default(), None, None).unwrap();
    assert_eq!(sig, Signal::Hold, "RSI sabit OB'de kalırsa flood olmamalı");
}

#[test]
fn rsi_never_buys_in_strong_uptrend() {
    // Sürekli yukarı seri — RSI sabit OB'de kalır.
    // Yeni davranış: ne Buy (oversold mantıksız) ne sürekli Sell (bölgede kalış)
    // üretir. Tek istisna OB'ye geçiş anında Sell olabilir; ama hiçbir koşulda Buy değil.
    let series: Vec<f64> = (0..50).map(|i| 100.0 + i as f64 * 0.7).collect();
    let candles = closes(&series);
    let sig = RsiStrategy.generate_signal(&candles, &StrategyParams::default(), None, None).unwrap();
    assert_ne!(sig, Signal::Buy, "Yukarı trendde RSI Buy üretmemeli: {:?}", sig);
}

// ─────────────────────────────────────────────────────────────────────────────
// Stochastic RSI crossing
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn stoch_rsi_no_signal_without_kd_cross() {
    // Düz seri — K ve D yatay; kesişim yok → Hold.
    let candles = closes(&(0..60).map(|_| 100.0).collect::<Vec<_>>());
    let sig = StochasticRsiStrategy.generate_signal(
        &candles, &StrategyParams::default(), None, None,
    ).unwrap();
    assert_eq!(sig, Signal::Hold);
}

// ─────────────────────────────────────────────────────────────────────────────
// PriceAction doji koruması
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn price_action_doji_does_not_trigger_engulfing() {
    // prev: doji (open == close), curr: yeşil mum
    let mut candles = closes(&[100.0; 5]);
    // Düz dolgu, sonra: prev doji (small body), curr büyük yeşil
    let n = candles.len();
    candles[n - 2] = bar(100.0, 101.0, 99.0, 100.001); // body ≈ 0.001 (range 2.0'ın %0.05'i)
    candles[n - 1] = bar(99.5, 102.0, 99.0, 101.5);    // güçlü yeşil
    let sig = PriceActionStrategy.generate_signal(
        &candles, &StrategyParams::default(), None, None,
    ).unwrap();
    assert_ne!(sig, Signal::Buy, "Doji'den sonraki büyük yeşil engulfing sayılmamalı");
}

#[test]
fn price_action_real_engulfing_still_triggers() {
    let candles: Vec<Candle> = vec![
        bar(100.0, 101.0,  99.0, 100.0),
        bar(100.0, 101.0,  99.0, 100.0),
        bar(100.0, 101.0,  99.0, 100.0),
        bar(101.0, 101.5,  99.5, 99.8),   // prev: kırmızı, gerçek mum
        bar(99.5,  103.0,  99.3, 102.5),  // curr: büyük yeşil
    ];
    let sig = PriceActionStrategy.generate_signal(
        &candles, &StrategyParams::default(), None, None,
    ).unwrap();
    assert_eq!(sig, Signal::Buy, "Gerçek bullish engulfing → Buy");
}

// ─────────────────────────────────────────────────────────────────────────────
// StrategyEnsemble — confidence-weighted voting
// ─────────────────────────────────────────────────────────────────────────────

/// Test için sabit sinyal + sabit confidence üreten dummy strateji.
struct FixedStrategy {
    sig: Signal,
    conf: f64,
    label: &'static str,
}

impl Strategy for FixedStrategy {
    fn name(&self) -> &str { self.label }
    fn generate_signal(
        &self,
        _: &[Candle],
        _: &StrategyParams,
        _: Option<&[memos_trading_core::core::types::FundingRatePoint]>,
        _: Option<&[Candle]>,
    ) -> memos_trading_core::Result<Signal> {
        Ok(self.sig)
    }
    fn confidence(&self) -> f64 { self.conf }
}

#[test]
fn ensemble_weighted_low_confidence_buys_lose_to_high_confidence_sell() {
    // 2 Buy üye (conf 0.3 + 0.3 = 0.6), 1 Sell üye (conf 0.9) → ağırlıklı toplam:
    // total_active = 1.5; buy_ratio = 0.4; sell_ratio = 0.6. Threshold 0.55:
    // sell_ratio >= 0.55 → Sell. Eski sayım eşit ağırlıklı olsa 2 Buy / 1 Sell olurdu.
    let mut e = StrategyEnsemble::new(0.55);
    e.add(Box::new(FixedStrategy { sig: Signal::Buy,  conf: 0.3, label: "B1" }));
    e.add(Box::new(FixedStrategy { sig: Signal::Buy,  conf: 0.3, label: "B2" }));
    e.add(Box::new(FixedStrategy { sig: Signal::Sell, conf: 0.9, label: "S1" }));

    let dummy = closes(&[100.0; 5]);
    let sig = e.generate_signal(&dummy, &StrategyParams::default(), None, None).unwrap();
    assert_eq!(sig, Signal::Sell,
        "Yüksek-conf Sell, düşük-conf 2 Buy'a karşı kazanmalı (ağırlıklı)");
}

#[test]
fn ensemble_holds_when_no_majority_weight() {
    let mut e = StrategyEnsemble::new(0.66);
    e.add(Box::new(FixedStrategy { sig: Signal::Buy,  conf: 0.5, label: "B" }));
    e.add(Box::new(FixedStrategy { sig: Signal::Sell, conf: 0.5, label: "S" }));
    let dummy = closes(&[100.0; 5]);
    let sig = e.generate_signal(&dummy, &StrategyParams::default(), None, None).unwrap();
    assert_eq!(sig, Signal::Hold, "Eşit ağırlık + 0.66 eşik → Hold");
}

#[test]
fn ensemble_ignores_hold_in_active_total() {
    // 1 Buy (conf 0.9), 2 Hold (conf 0.5). Active total = 0.9 (sadece Buy).
    // buy_ratio = 1.0 → 0.6 eşiği aşar → Buy. Hold ağırlığı paya katılsa
    // ratio = 0.9 / 1.9 ≈ 0.47 < 0.6 → Hold olurdu.
    let mut e = StrategyEnsemble::new(0.60);
    e.add(Box::new(FixedStrategy { sig: Signal::Buy,  conf: 0.9, label: "B" }));
    e.add(Box::new(FixedStrategy { sig: Signal::Hold, conf: 0.5, label: "H1" }));
    e.add(Box::new(FixedStrategy { sig: Signal::Hold, conf: 0.5, label: "H2" }));
    let dummy = closes(&[100.0; 5]);
    let sig = e.generate_signal(&dummy, &StrategyParams::default(), None, None).unwrap();
    assert_eq!(sig, Signal::Buy, "Hold ağırlıkları aktif paya katılmamalı");
}

#[test]
fn ensemble_empty_members_returns_hold() {
    let e = StrategyEnsemble::new(0.5);
    let dummy = closes(&[100.0; 5]);
    let sig = e.generate_signal(&dummy, &StrategyParams::default(), None, None).unwrap();
    assert_eq!(sig, Signal::Hold);
}

// ─────────────────────────────────────────────────────────────────────────────
// StrategySelector::simulate_score
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn selector_score_zero_when_strategy_always_holds() {
    struct AlwaysHold;
    impl Strategy for AlwaysHold {
        fn name(&self) -> &str { "HOLD" }
        fn generate_signal(
            &self, _: &[Candle], _: &StrategyParams,
            _: Option<&[memos_trading_core::core::types::FundingRatePoint]>,
            _: Option<&[Candle]>,
        ) -> memos_trading_core::Result<Signal> { Ok(Signal::Hold) }
    }
    let candles = closes(&(0..50).map(|i| 100.0 + i as f64).collect::<Vec<_>>());
    let sel = StrategySelector::new();
    let score = sel.simulate_score(&AlwaysHold, &candles, &StrategyParams::default());
    assert_eq!(score, 0.0);
}

#[test]
fn selector_score_positive_for_aligned_strategy() {
    // Sürekli artan seri; "Always Buy" stratejisi her bar pozitif getiri toplar.
    struct AlwaysBuy;
    impl Strategy for AlwaysBuy {
        fn name(&self) -> &str { "BUY" }
        fn generate_signal(
            &self, _: &[Candle], _: &StrategyParams,
            _: Option<&[memos_trading_core::core::types::FundingRatePoint]>,
            _: Option<&[Candle]>,
        ) -> memos_trading_core::Result<Signal> { Ok(Signal::Buy) }
    }
    let candles = closes(&(0..50).map(|i| 100.0 + i as f64).collect::<Vec<_>>());
    let sel = StrategySelector::new();
    let score = sel.simulate_score(&AlwaysBuy, &candles, &StrategyParams::default());
    assert!(score > 0.0, "Yukarı trendde Buy stratejisi pozitif ortalama getiri vermeli: {}", score);
}

#[test]
fn selector_score_negative_for_misaligned_strategy() {
    // Yukarı trendde "Always Sell" → her bar negatif getiri.
    struct AlwaysSell;
    impl Strategy for AlwaysSell {
        fn name(&self) -> &str { "SELL" }
        fn generate_signal(
            &self, _: &[Candle], _: &StrategyParams,
            _: Option<&[memos_trading_core::core::types::FundingRatePoint]>,
            _: Option<&[Candle]>,
        ) -> memos_trading_core::Result<Signal> { Ok(Signal::Sell) }
    }
    let candles = closes(&(0..50).map(|i| 100.0 + i as f64).collect::<Vec<_>>());
    let sel = StrategySelector::new();
    let score = sel.simulate_score(&AlwaysSell, &candles, &StrategyParams::default());
    assert!(score < 0.0, "Yukarı trendde Sell stratejisi negatif olmalı: {}", score);
}
