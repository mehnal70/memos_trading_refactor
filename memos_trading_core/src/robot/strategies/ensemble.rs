// robot/strategies/ensemble.rs - Çoklu strateji konsensüs motoru
//
// `StrategyEnsemble` birden çok `Strategy`'yi sırayla çalıştırır, oy yoğunluğuna
// göre tek bir final sinyal üretir. Kendisi de `Strategy` trait'ini uygular —
// yani başka bir ensemble'ın içine bileşen olarak girebilir (composite pattern).
//
// Not: Üyeler iter().map() ile **sequential** çalıştırılır (her strateji ucuz
// hesap; tokio task'a almanın overhead'i kazançtan büyük). Gerçek paralel
// gerekirse rayon::par_iter ile değiştirilebilir.

use crate::core::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::robot::strategies::base::Strategy;
use crate::Result;

/// Tek bir strateji çalışmasının sonucu (consensus için).
#[derive(Debug, Clone)]
pub struct StrategyResult {
    pub strategy_name: String,
    pub signal: Signal,
    pub confidence: f64,
    pub reason: String,
}

impl StrategyResult {
    /// Hold sonucu üreten kısa-yol.
    pub fn hold(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            strategy_name: name.into(),
            signal: Signal::Hold,
            confidence: 0.0,
            reason: reason.into(),
        }
    }
}

/// Bir grup stratejiyi tek bir kararla birleştiren konsensüs motoru.
/// `threshold_ratio`: kararın bağlayıcı olması için gereken oy oranı (0.0–1.0).
pub struct StrategyEnsemble {
    members: Vec<Box<dyn Strategy>>,
    threshold_ratio: f64,
}

impl StrategyEnsemble {
    pub fn new(threshold_ratio: f64) -> Self {
        Self { members: Vec::new(), threshold_ratio: threshold_ratio.clamp(0.0, 1.0) }
    }

    pub fn with_members(mut self, members: Vec<Box<dyn Strategy>>) -> Self {
        self.members = members;
        self
    }

    pub fn add(&mut self, strategy: Box<dyn Strategy>) -> &mut Self {
        self.members.push(strategy);
        self
    }

    pub fn member_count(&self) -> usize { self.members.len() }

    /// Üyelerin tek tek ürettiği detaylı sonuçları döndürür (rapor + log için).
    pub fn evaluate_all(
        &self,
        candles: &[Candle],
        params: &StrategyParams,
        funding_rates: Option<&[FundingRatePoint]>,
        htf_candles: Option<&[Candle]>,
    ) -> Vec<StrategyResult> {
        self.members.iter().map(|s| {
            let name = s.name().to_string();
            match s.generate_signal(candles, params, funding_rates, htf_candles) {
                Ok(sig) => StrategyResult {
                    strategy_name: name,
                    signal: sig,
                    confidence: s.confidence(),
                    reason: "ok".into(),
                },
                Err(e) => StrategyResult::hold(name, format!("err: {}", e)),
            }
        }).collect()
    }

    /// Oy sayımı: (buy, sell, hold).
    fn tally(results: &[StrategyResult]) -> (usize, usize, usize) {
        results.iter().fold((0, 0, 0), |(b, s, h), r| match r.signal {
            Signal::Buy  => (b + 1, s, h),
            Signal::Sell => (b, s + 1, h),
            Signal::Hold => (b, s, h + 1),
        })
    }
}

impl Strategy for StrategyEnsemble {
    fn name(&self) -> &str { "ENSEMBLE" }

    fn generate_signal(
        &self,
        candles: &[Candle],
        params: &StrategyParams,
        funding_rates: Option<&[FundingRatePoint]>,
        htf_candles: Option<&[Candle]>,
    ) -> Result<Signal> {
        if self.members.is_empty() { return Ok(Signal::Hold); }
        let results = self.evaluate_all(candles, params, funding_rates, htf_candles);
        let (buys, sells, _) = Self::tally(&results);
        let total = results.len() as f64;
        let need = (total * self.threshold_ratio).ceil() as usize;

        Ok(match (buys >= need, sells >= need) {
            (true, false) => Signal::Buy,
            (false, true) => Signal::Sell,
            _             => Signal::Hold,
        })
    }
}
