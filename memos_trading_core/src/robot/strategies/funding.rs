// robot/strategies/funding.rs - Funding rate kontrar stratejisi (perpetual'lar için)
//
// Aşırı pozitif funding (long'lar premium ödüyor) → kalabalık LONG, contrarian SELL.
// Aşırı negatif funding (short'lar premium ödüyor) → kalabalık SHORT, contrarian BUY.

use crate::core::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::robot::strategies::base::Strategy;
use crate::Result;

pub struct FundingRateContrarianStrategy {
    pub threshold: f64,
}

impl Default for FundingRateContrarianStrategy {
    fn default() -> Self { Self { threshold: 0.0005 } } // %0.05
}

impl Strategy for FundingRateContrarianStrategy {
    fn name(&self) -> &str { "FUNDING_CONTRARIAN" }
    fn generate_signal(
        &self,
        _candles: &[Candle],
        _params: &StrategyParams,
        funding_rates: Option<&[FundingRatePoint]>,
        _htf: Option<&[Candle]>,
    ) -> Result<Signal> {
        let rates = match funding_rates {
            Some(r) if !r.is_empty() => r,
            _ => return Ok(Signal::Hold),
        };
        let last = match rates.last() {
            Some(l) => l,
            None => return Ok(Signal::Hold),
        };

        let raw = if last.funding_rate >=  self.threshold { Signal::Sell }
                  else if last.funding_rate <= -self.threshold { Signal::Buy }
                  else { Signal::Hold };
        Ok(raw)
    }
}
