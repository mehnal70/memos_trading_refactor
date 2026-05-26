// robot/strategy_selector.rs - Otomatik strateji seçici ve mini-backtest tabanlı skor
//
// `simulate_score`: walk-forward mantığında her bar için stratejiyi çalıştırır,
// bir sonraki barın getirisini sinyal yönüne göre toplayarak ortalama
// işlem getirisini döndürür. Eski "son 3 bar yükseldi mi" dummy'sinin yerine.
//
// Faz 4 c2: Aday strateji listesi artık `StrategyRegistry`'den geliyor. Varsayılan
// `new()` kompakt bir aday seti üretir (MA/RSI/MACD/BB); özel kullanım için
// `from_registry(names…)` veya `with_strategies(vec)` ile başka kombinasyonlar
// kurulabilir → engine ya da test buradan plug-in zincirini değiştirir,
// kaynak kod modifikasyonu gerekmez.

use crate::core::types::{Candle, StrategyParams, Signal};
use crate::robot::strategies::{default_registry, Strategy, StrategyRegistry};

/// Strateji seçici: walk-forward skoru en yüksek stratejiyi seçer.
pub struct StrategySelector {
    pub strategies: Vec<Box<dyn Strategy>>,
    /// `strategies` ile paralel REGISTRY adları (kurulumda verilen). resolver bu adla
    /// çağrılır → ParameterStore anahtarlarıyla tutarlı (örn. "BB", impl adı
    /// "BOLLINGER_BANDS" değil). with_strategies'te `strat.name()`'e düşer.
    pub names: Vec<String>,
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
    /// Varsayılan kompakt aday seti (MA / RSI / MACD / BB) registry'den çözülür.
    pub fn new() -> Self {
        Self::from_registry(
            &default_registry(),
            &["MA_CROSSOVER", "RSI", "MACD", "BB"],
        )
    }

    /// Verilen registry'den belirtilen isimleri toplar. Bilinmeyen isim
    /// registry'nin fallback davranışına düşer — selector çağrı yerinde
    /// yeniden panik üretmez.
    pub fn from_registry(registry: &StrategyRegistry, names: &[&str]) -> Self {
        let strategies = names.iter().map(|n| registry.make(n)).collect();
        let names = names.iter().map(|n| n.to_string()).collect();
        Self { strategies, names, lookback: 30, min_trades: 3 }
    }

    /// Hazır strateji vektörüyle kurma — testlerde özel/dummy strateji
    /// enjekte etmek için kullanışlı. Registry adı yok → resolver impl `name()` ile çağrılır.
    pub fn with_strategies(strategies: Vec<Box<dyn Strategy>>) -> Self {
        let names = strategies.iter().map(|s| s.name().to_string()).collect();
        Self { strategies, names, lookback: 30, min_trades: 3 }
    }

    /// Hepsini dener, walk-forward skoru en yüksek stratejiyi ve onun şu anki
    /// sinyalini döndürür. Skor "ortalama işlem getirisi" (bir sonraki bar return'ü).
    pub fn select_best(&self, candles: &[Candle], params: &StrategyParams) -> (&dyn Strategy, Signal) {
        // Tek params tüm adaylara — resolver'sız geriye-uyum yolu.
        self.select_best_resolved(candles, |_| *params)
    }

    /// `select_best`'in resolver'lı sürümü: her aday KENDİ resolve'lu paramıyla
    /// skorlanır (ParameterStore'un strateji-bazlı en iyi seti). Backtest job'ın
    /// param_spec araması ile bulduğu paramlar artık SEÇİM aşamasına da girer —
    /// eskiden tüm adaylar default param ile yarışıyordu. `resolve(name)` ilgili
    /// stratejinin paramını döndürür (yoksa default).
    pub fn select_best_resolved(
        &self,
        candles: &[Candle],
        resolve: impl Fn(&str) -> StrategyParams,
    ) -> (&dyn Strategy, Signal) {
        let (idx, sig) = self.best_index_resolved(candles, resolve);
        (self.strategies[idx].as_ref(), sig)
    }

    /// `select_best_resolved`'in ad-döndüren sürümü: seçilen stratejinin REGISTRY
    /// adını (kurulumda verilen, örn. "BB") + sinyalini döndürür. Canlı motor bu adı
    /// hem make_strategy_pub'a hem resolve_strategy_params'a verir → PS anahtarlarıyla
    /// tutarlı (impl adı "BOLLINGER_BANDS" ile sapma olmaz).
    pub fn select_best_name_resolved(
        &self,
        candles: &[Candle],
        resolve: impl Fn(&str) -> StrategyParams,
    ) -> (String, Signal) {
        let (idx, sig) = self.best_index_resolved(candles, resolve);
        (self.names[idx].clone(), sig)
    }

    /// Ortak çekirdek: her adayı KENDİ registry-adıyla resolve edilen paramıyla
    /// skorlar, en yüksek skorlunun indeksini + güncel sinyalini döndürür.
    fn best_index_resolved(
        &self,
        candles: &[Candle],
        resolve: impl Fn(&str) -> StrategyParams,
    ) -> (usize, Signal) {
        let mut best_score = f64::NEG_INFINITY;
        let mut best_idx = 0usize;
        let mut best_signal = Signal::Hold;
        for (i, strat) in self.strategies.iter().enumerate() {
            let p = resolve(&self.names[i]); // registry adı → PS tutarlı
            let sig = strat.generate_signal(candles, &p, None, None).unwrap_or(Signal::Hold);
            let score = self.simulate_score(strat.as_ref(), candles, &p);
            if score > best_score {
                best_score = score;
                best_idx = i;
                best_signal = sig;
            }
        }
        (best_idx, best_signal)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::strategies::default_registry;
    use chrono::{TimeZone, Utc};
    use std::cell::RefCell;

    fn wave(n: usize) -> Vec<Candle> {
        (0..n).map(|i| {
            let close = 100.0 + (i as f64 * 0.4).sin() * 10.0;
            Candle {
                timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64 * 3600, 0).unwrap(),
                open: close, high: close + 1.0, low: close - 1.0, close,
                volume: 1_000.0, symbol: "T".into(), interval: "1h".into(),
            }
        }).collect()
    }

    #[test]
    fn select_best_resolved_her_adayi_kendi_paramiyla_cozer() {
        let sel = StrategySelector::from_registry(&default_registry(), &["RSI", "MACD", "BB"]);
        let candles = wave(120);
        let istenen = RefCell::new(Vec::<String>::new());
        let (best, _sig) = sel.select_best_resolved(&candles, |name| {
            istenen.borrow_mut().push(name.to_string());
            StrategyParams::default()
        });
        // Resolver HER aday için bir kez çağrıldı (per-candidate param çözümü).
        let mut got = istenen.into_inner();
        got.sort();
        assert_eq!(got, vec!["BB", "MACD", "RSI"],
            "her aday kendi adıyla resolve edilmeli: {got:?}");
        assert!(["RSI", "MACD", "BB"].contains(&best.name()));
    }

    #[test]
    fn select_best_default_resolver_ile_uyumlu() {
        // select_best (tek-param) == select_best_resolved (sabit resolver) — geriye uyum.
        let sel = StrategySelector::from_registry(&default_registry(), &["RSI", "MACD"]);
        let candles = wave(120);
        let p = StrategyParams::default();
        let (a, _) = sel.select_best(&candles, &p);
        let (b, _) = sel.select_best_resolved(&candles, |_| p);
        assert_eq!(a.name(), b.name());
    }
}
