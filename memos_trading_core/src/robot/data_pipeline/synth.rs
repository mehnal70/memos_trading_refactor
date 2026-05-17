// robot/data_pipeline/synth.rs - Yüksek Performanslı Mum Sentezleyici
//
// Görev: 1 dakikalık (M1) baz mumları kullanarak otonom olarak üst zaman dilimlerini
// (M5, M15, H1, H4 vb.) sentezler ve callback üzerinden sisteme besler.

use crate::core::types::Candle;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Interval {
    M1, M5, M15, M30, H1, H4, D1
}

impl Interval {
    pub fn as_str(&self) -> &'static str {
        match self {
            Interval::M1 => "1m", Interval::M5 => "5m", Interval::M15 => "15m",
            Interval::M30 => "30m", Interval::H1 => "1h", Interval::H4 => "4h", Interval::D1 => "1d",
        }
    }

    #[inline]
    pub fn minutes(&self) -> usize {
        match self {
            Interval::M1 => 1, Interval::M5 => 5, Interval::M15 => 15,
            Interval::M30 => 30, Interval::H1 => 60, Interval::H4 => 240, Interval::D1 => 1440,
        }
    }

    pub fn all_upper() -> &'static [Interval] {
        &[Interval::M5, Interval::M15, Interval::M30, Interval::H1, Interval::H4, Interval::D1]
    }
}

/// Bellek dostu akümülatör: Mum listesi tutmak yerine istatistik günceller.
#[derive(Debug, Clone)]
struct CandleAcc {
    count: usize,
    open: f64,
    high: f64,
    low: f64,
    volume: f64,
    start_ts: chrono::DateTime<chrono::Utc>,
}

impl CandleAcc {
    fn new(first: &Candle) -> Self {
        Self {
            count: 1,
            open: first.open,
            high: first.high,
            low: first.low,
            volume: first.volume,
            start_ts: first.timestamp,
        }
    }

    fn update(&mut self, candle: &Candle) {
        self.count += 1;
        self.high = self.high.max(candle.high);
        self.low = self.low.min(candle.low);
        self.volume += candle.volume;
    }
}

pub struct CandleSynth<'a> {
    pub symbol: String,
    accumulators: HashMap<Interval, Option<CandleAcc>>,
    pub on_candle: Box<dyn Fn(&Candle) + Send + Sync + 'a>,
}

impl<'a> CandleSynth<'a> {
    pub fn new(symbol: &str, on_candle: Box<dyn Fn(&Candle) + Send + Sync + 'a>) -> Self {
        let mut accumulators = HashMap::with_capacity(7);
        for &intv in Interval::all_upper() {
            accumulators.insert(intv, None);
        }
        Self {
            symbol: symbol.to_owned(),
            accumulators,
            on_candle,
        }
    }

    /// 1 dakikalık mumu sisteme iter ve sentezlenen üst mumları döner.
    pub fn push_1m(&mut self, candle: &Candle) -> Vec<Candle> {
        let mut emitted = Vec::with_capacity(1);

        for &target in Interval::all_upper() {
            let target_mins = target.minutes();
            let acc_opt = self.accumulators.get_mut(&target).expect("Interval in map");

            match acc_opt {
                None => {
                    *acc_opt = Some(CandleAcc::new(candle));
                }
                Some(acc) => {
                    acc.update(candle);
                    
                    if acc.count >= target_mins {
                        let synthesized = Candle {
                            timestamp: acc.start_ts,
                            open: acc.open,
                            high: acc.high,
                            low: acc.low,
                            close: candle.close,
                            volume: acc.volume,
                            symbol: self.symbol.clone(),
                            interval: target.as_str().to_owned(),
                        };

                        (self.on_candle)(&synthesized);
                        emitted.push(synthesized);
                        *acc_opt = None; // Sıfırla
                    }
                }
            }
        }
        emitted
    }
}
