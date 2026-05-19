// robot/strategy_selector.rs - Otomatik strateji seçici ve mini-backtest tabanlı skor
//
// `simulate_score`: walk-forward mantığında her bar için stratejiyi çalıştırır,
// bir sonraki barın getirisini sinyal yönüne göre toplayarak ortalama
// işlem getirisini döndürür. Eski "son 3 bar yükseldi mi" dummy'sinin yerine.

use crate::core::types::{Candle, StrategyParams, Signal};
use crate::robot::strategies::{
    Strategy, MaCrossoverStrategy, RsiStrategy, MacdStrategy, BollingerBandsStrategy,
};

/// Strateji seçici: walk-forward skoru en yüksek stratejiyi seçer.
pub struct StrategySelector {
    pub strategies: Vec<Box<dyn Strategy>>,
    /// Walk-forward penceresi — son `lookback` bar üzerinde strateji çalıştırılır,
    /// her bar için bir sonraki barın getirisi pozitif/negatif yönüne göre sayılır.
    pub lookback: usize,
    /// İşlem sayısı bu eşiğin altındaysa skor 0 (yeterli aktivite yok).
    pub min_trades: usize,
}

impl Default for StrategySelector {
    fn default() -> Self { Self::new() }
}

impl StrategySelector {
    pub fn new() -> Self {
        Self {
            strategies: vec![
                Box::new(MaCrossoverStrategy),
                Box::new(RsiStrategy),
                Box::new(MacdStrategy),
                Box::new(BollingerBandsStrategy),
            ],
            lookback: 30,
            min_trades: 3,
        }
    }

    /// Hepsini dener, walk-forward skoru en yüksek stratejiyi ve onun şu anki
    /// sinyalini döndürür. Skor "ortalama işlem getirisi" (bir sonraki bar return'ü).
    pub fn select_best(&self, candles: &[Candle], params: &StrategyParams) -> (&dyn Strategy, Signal) {
        let mut best_score = f64::NEG_INFINITY;
        let mut best_strat = &*self.strategies[0];
        let mut best_signal = Signal::Hold;
        for strat in &self.strategies {
            let sig = strat.generate_signal(candles, params, None, None).unwrap_or(Signal::Hold);
            let score = self.simulate_score(strat.as_ref(), candles, params);
            if score > best_score {
                best_score = score;
                best_strat = strat.as_ref();
                best_signal = sig;
            }
        }
        (best_strat, best_signal)
    }

    /// Walk-forward mini-backtest: son `lookback` bar boyunca her i bar'ı için
    /// strateji `candles[..=i]` üzerinde çalıştırılır, sinyal yönüne göre
    /// candles[i] → candles[i+1] yüzdesel getirisi toplanır.
    /// Sonuç: ortalama işlem getirisi (per trade). Az işlemde 0.
    pub fn simulate_score(&self, strat: &dyn Strategy, candles: &[Candle], params: &StrategyParams) -> f64 {
        let n = candles.len();
        if n < self.lookback + 2 { return 0.0; }

        let start = n - self.lookback - 1;
        let mut total_return = 0.0;
        let mut trades = 0usize;

        for i in start..n - 1 {
            let slice = &candles[..=i];
            let sig = strat.generate_signal(slice, params, None, None).unwrap_or(Signal::Hold);
            let entry = candles[i].close;
            if entry <= 0.0 { continue; }
            let ret = (candles[i + 1].close - entry) / entry;
            match sig {
                Signal::Buy  => { total_return += ret;  trades += 1; }
                Signal::Sell => { total_return += -ret; trades += 1; }
                Signal::Hold => {}
            }
        }

        if trades < self.min_trades { 0.0 } else { total_return / trades as f64 }
    }
}
