//! Mum sentezleyici: 1m mumlardan 5m, 15m, 1h, 1d gibi üst zaman dilimi mumları üretir.
//! Her interval için ayrı bir accumulator tutulur. Tam N adet 1m mum birikince
//! sentezlenmiş mum emit edilir ve accumulator sıfırlanır.
//! Eski implementasyonda her 1m gelişinde sliding window ile mum üretiliyordu —
//! bu, 1d için günde 1440 farklı timestamp'e sahip 1440 ayrı DB satırı üretiyordu.

use crate::types::Candle;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Interval {
    M1, M5, M15, M30, H1, H4, D1
}

impl Interval {
    pub fn as_str(&self) -> &'static str {
        match self {
            Interval::M1  => "1m",
            Interval::M5  => "5m",
            Interval::M15 => "15m",
            Interval::M30 => "30m",
            Interval::H1  => "1h",
            Interval::H4  => "4h",
            Interval::D1  => "1d",
        }
    }
    pub fn minutes(&self) -> usize {
        match self {
            Interval::M1  => 1,
            Interval::M5  => 5,
            Interval::M15 => 15,
            Interval::M30 => 30,
            Interval::H1  => 60,
            Interval::H4  => 240,
            Interval::D1  => 1440,
        }
    }
    pub fn all() -> &'static [Interval] {
        &[Interval::M1, Interval::M5, Interval::M15, Interval::M30,
          Interval::H1, Interval::H4, Interval::D1]
    }
}

/// Sembol başına üst zaman dilimi sentezleyici.
/// Her interval için ayrı bir 1m accumulator tutar.
/// N adet 1m birikince tam bir üst mum sentezlenir, emit edilir, accumulator sıfırlanır.
pub struct CandleSynth<'a> {
    pub symbol:      String,
    /// Her interval için biriken 1m mumları (henüz tamamlanmamış üst mum)
    accumulators:    HashMap<Interval, Vec<Candle>>,
    pub on_candle:   Box<dyn Fn(&Candle) + Send + Sync + 'a>,
}

impl<'a> CandleSynth<'a> {
    pub fn new(symbol: &str, on_candle: Box<dyn Fn(&Candle) + Send + Sync + 'a>) -> Self {
        let mut accumulators = HashMap::new();
        for &intv in Interval::all() {
            if intv != Interval::M1 {
                accumulators.insert(intv, Vec::new());
            }
        }
        Self { symbol: symbol.to_string(), accumulators, on_candle }
    }

    /// Yeni 1m mum geldiğinde çağrılır.
    /// Her interval'ın accumulator'una ekler; N dolduğunda emit et ve sıfırla.
    /// Dönen Vec<Candle>: bu tick'te kapanan tüm üst mum dönemleri.
    /// Callback (on_candle) geriye dönük uyumluluk için hâlâ çağrılır.
    pub fn push_1m(&mut self, candle: Candle) -> Vec<Candle> {
        let mut emitted: Vec<Candle> = Vec::new();
        for &target in &[Interval::M5, Interval::M15, Interval::M30,
                         Interval::H1, Interval::H4, Interval::D1]
        {
            let acc = self.accumulators.get_mut(&target).unwrap();
            acc.push(candle.clone());

            if acc.len() >= target.minutes() {
                // Tam period tamamlandı — sentezle
                let open   = acc.first().unwrap().open;
                let close  = acc.last().unwrap().close;
                let high   = acc.iter().map(|c| c.high).fold(f64::MIN, f64::max);
                let low    = acc.iter().map(|c| c.low ).fold(f64::MAX, f64::min);
                let volume = acc.iter().map(|c| c.volume).sum();
                let ts     = acc.first().unwrap().timestamp;

                let synthesized = Candle {
                    timestamp: ts,
                    open, high, low, close, volume,
                    symbol:   self.symbol.clone(),
                    interval: target.as_str().to_string(),
                };
                // Callback: evolution trigger, cache güncelleme vs. hâlâ çalışır
                (self.on_candle)(&synthesized);
                // Çağırana hangi interval'ların kapandığını ilet
                emitted.push(synthesized);

                // Accumulator'u sıfırla — bir sonraki period için hazır
                acc.clear();
            }
        }
        emitted
    }
}
