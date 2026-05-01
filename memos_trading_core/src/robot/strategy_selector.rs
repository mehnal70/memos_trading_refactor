// robot/strategy_selector.rs - Otomatik strateji seçici ve optimizer (ML/AI, hyperopt, fail-safe)
// Türkçe inline açıklamalar, extensible yapı

use crate::robot::strategies::{Strategy, MaCrossoverStrategy, RsiStrategy, MacdStrategy, BollingerStrategy, StochasticStrategy};
use crate::types::{Candle, StrategyParams, Signal};

/// Strateji seçici: performansa, koşullara veya ML/AI modeline göre en iyi stratejiyi seçer
pub struct StrategySelector {
    pub strategies: Vec<Box<dyn Strategy>>,
}

impl StrategySelector {
    pub fn new() -> Self {
        Self {
            strategies: vec![
                Box::new(MaCrossoverStrategy),
                Box::new(RsiStrategy),
                Box::new(MacdStrategy),
                Box::new(BollingerStrategy),
                Box::new(StochasticStrategy),
            ],
        }
    }

    /// Basit: Hepsini dener, en iyi performanslıyı seçer (örnek, backtest ile)
    pub fn select_best(&self, candles: &[Candle], params: &StrategyParams) -> (&dyn Strategy, Signal) {
        let mut best_score = f64::MIN;
        let mut best_strat = &*self.strategies[0];
        let mut best_signal = Signal::Hold;
        for strat in &self.strategies {
            let sig = strat.generate_signal(candles, params, None, None).unwrap_or(Signal::Hold);
            // Burada backtest/ML/AI ile skor hesaplanabilir (ör: son 20 bar karlılığı)
            let score = self.simulate_score(strat.as_ref(), candles, params);
            if score > best_score {
                best_score = score;
                best_strat = strat.as_ref();
                best_signal = sig;
            }
        }
        (best_strat, best_signal)
    }

    /// Dummy skor fonksiyonu: Son sinyalin karlılığına bakar (örnek, gerçek ML/AI ile değiştirilebilir)
    pub fn simulate_score(&self, strat: &dyn Strategy, candles: &[Candle], params: &StrategyParams) -> f64 {
        // Burada ML/AI, hyperopt, backtest, ensemble, fail-safe gibi gelişmiş yöntemler entegre edilebilir
        // Şimdilik: son sinyal BUY ise son 3 bar yükseldiyse +1, SELL ise düştüyse +1, yoksa 0
        let sig = strat.generate_signal(candles, params, None, None).unwrap_or(Signal::Hold);
        if candles.len() < 4 { return 0.0; }
        match sig {
            Signal::Buy => if candles.last().unwrap().close > candles[candles.len()-4].close { 1.0 } else { 0.0 },
            Signal::Sell => if candles.last().unwrap().close < candles[candles.len()-4].close { 1.0 } else { 0.0 },
            Signal::Hold => 0.0,
        }
    }
}

// Gelişmiş: ML/AI model entegrasyonu, hyperopt, ensemble, fail-safe logic buraya eklenebilir.
// Strateji ve parametre arama, otomatik optimizasyon, canlıda adaptasyon için extensible altyapı hazır.
