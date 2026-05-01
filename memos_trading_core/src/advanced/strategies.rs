//! Trading stratejileri - DB bağımlılığı olmayan pure logic
//! 
//! trading_cli'den uyarlanmış, input olarak closes verisini alan stratejiler

use crate::advanced::indicators;
use crate::advanced::risk::TradeAction;

/// Strateji sonucu
#[derive(Debug, Clone)]
pub struct StrategyResult {
    pub action: TradeAction,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub confidence: f64, // 0-100 arasında
    pub reason: String,
}

/// MA Crossover stratejisi
/// 
/// # Parametreler
/// - `closes`: Kapanış fiyatları
/// - `fast_period`: Hızlı MA periyodu
/// - `slow_period`: Yavaş MA periyodu
/// 
/// # Dönüş
/// Son fiyata göre strateji sonucu (Some(StrategyResult) veya None)
pub fn ma_crossover(
    closes: &[f64],
    fast_period: usize,
    slow_period: usize,
) -> Option<StrategyResult> {
    if closes.len() < slow_period {
        return None;
    }

    let fast_mas = indicators::sma(closes, fast_period);
    let slow_mas = indicators::sma(closes, slow_period);

    let current_price = closes[closes.len() - 1];
    let last_fast = fast_mas[fast_mas.len() - 1]?;
    let last_slow = slow_mas[slow_mas.len() - 1]?;

    if fast_mas.len() < 2 || slow_mas.len() < 2 {
        return None;
    }

    let prev_fast = fast_mas[fast_mas.len() - 2]?;
    let prev_slow = slow_mas[slow_mas.len() - 2]?;

    if last_fast > last_slow && prev_fast <= prev_slow {
        // Bullish crossover
        Some(StrategyResult {
            action: TradeAction::Buy,
            entry_price: current_price,
            stop_loss: current_price * 0.98,
            take_profit: current_price * 1.05,
            confidence: 65.0,
            reason: format!("Hızlı MA ({:.2}) Yavaş MA'yı ({:.2}) aştı", last_fast, last_slow),
        })
    } else if last_fast < last_slow && prev_fast >= prev_slow {
        // Bearish crossover
        Some(StrategyResult {
            action: TradeAction::Sell,
            entry_price: current_price,
            stop_loss: current_price * 1.02,
            take_profit: current_price * 0.95,
            confidence: 65.0,
            reason: format!("Hızlı MA ({:.2}) Yavaş MA'nın ({:.2}) altına düştü", last_fast, last_slow),
        })
    } else {
        Some(StrategyResult {
            action: TradeAction::Hold,
            entry_price: current_price,
            stop_loss: 0.0,
            take_profit: 0.0,
            confidence: 0.0,
            reason: "MA'lar kesişmiyor, wait için.".to_string(),
        })
    }
}

/// RSI stratejisi
/// 
/// # Parametreler
/// - `closes`: Kapanış fiyatları
/// - `period`: RSI periyodu (genellikle 14)
/// - `overbought`: Overbought eşiği (genellikle 70)
/// - `oversold`: Oversold eşiği (genellikle 30)
pub fn rsi_strategy(
    closes: &[f64],
    period: usize,
    overbought: f64,
    oversold: f64,
) -> Option<StrategyResult> {
    if closes.len() < period + 1 {
        return None;
    }

    let rsis = indicators::rsi(closes, period);
    let last_rsi = rsis[rsis.len() - 1]?;
    let current_price = closes[closes.len() - 1];

    if last_rsi < oversold {
        Some(StrategyResult {
            action: TradeAction::Buy,
            entry_price: current_price,
            stop_loss: current_price * 0.97,
            take_profit: current_price * 1.05,
            confidence: 70.0,
            reason: format!("RSI oversold ({:.2}) - Geri dönüş bekleniyor", last_rsi),
        })
    } else if last_rsi > overbought {
        Some(StrategyResult {
            action: TradeAction::Sell,
            entry_price: current_price,
            stop_loss: current_price * 1.03,
            take_profit: current_price * 0.95,
            confidence: 70.0,
            reason: format!("RSI overbought ({:.2}) - Düzeltme bekleniyor", last_rsi),
        })
    } else {
        Some(StrategyResult {
            action: TradeAction::Hold,
            entry_price: current_price,
            stop_loss: 0.0,
            take_profit: 0.0,
            confidence: 0.0,
            reason: format!("RSI nötr bölgede ({:.2})", last_rsi),
        })
    }
}

/// MACD stratejisi
pub fn macd_strategy(
    closes: &[f64],
    fast_period: usize,
    slow_period: usize,
    signal_period: usize,
) -> Option<StrategyResult> {
    if closes.len() < slow_period + signal_period {
        return None;
    }

    let (macd_line, signal_line, _hist) = indicators::macd(
        closes,
        fast_period,
        slow_period,
        signal_period,
    );

    let current_price = closes[closes.len() - 1];
    let last_macd = macd_line[macd_line.len() - 1]?;
    let last_signal = signal_line[signal_line.len() - 1]?;

    if macd_line.len() < 2 || signal_line.len() < 2 {
        return None;
    }

    let prev_macd = macd_line[macd_line.len() - 2]?;
    let prev_signal = signal_line[signal_line.len() - 2]?;

    if prev_macd <= prev_signal && last_macd > last_signal {
        // Bullish crossover
        Some(StrategyResult {
            action: TradeAction::Buy,
            entry_price: current_price,
            stop_loss: current_price * 0.98,
            take_profit: current_price * 1.06,
            confidence: 72.0,
            reason: format!("MACD signal çizgisini aştı ({:.4})", last_macd - last_signal),
        })
    } else if prev_macd >= prev_signal && last_macd < last_signal {
        // Bearish crossover
        Some(StrategyResult {
            action: TradeAction::Sell,
            entry_price: current_price,
            stop_loss: current_price * 1.02,
            take_profit: current_price * 0.94,
            confidence: 72.0,
            reason: format!("MACD signal çizgisinin altına düştü ({:.4})", last_signal - last_macd),
        })
    } else {
        Some(StrategyResult {
            action: TradeAction::Hold,
            entry_price: current_price,
            stop_loss: 0.0,
            take_profit: 0.0,
            confidence: 0.0,
            reason: "MACD sinyal yok".to_string(),
        })
    }
}

/// Bollinger Bands stratejisi
pub fn bollinger_bands_strategy(
    closes: &[f64],
    period: usize,
    std_dev: f64,
) -> Option<StrategyResult> {
    if closes.len() < period {
        return None;
    }

    let (lower_band, middle_band, upper_band) =
        indicators::bollinger_bands(closes, period, std_dev);

    let current_price = closes[closes.len() - 1];
    let last_lower = lower_band[lower_band.len() - 1]?;
    let last_upper = upper_band[upper_band.len() - 1]?;
    let last_middle = middle_band[middle_band.len() - 1]?;

    if current_price < last_lower {
        Some(StrategyResult {
            action: TradeAction::Buy,
            entry_price: current_price,
            stop_loss: last_lower * 0.99,
            take_profit: last_middle * 1.01,
            confidence: 68.0,
            reason: format!(
                "Fiyat alt banta ({:.2}) dokundu - Geri dönüş bekleniyor",
                last_lower
            ),
        })
    } else if current_price > last_upper {
        Some(StrategyResult {
            action: TradeAction::Sell,
            entry_price: current_price,
            stop_loss: last_upper * 1.01,
            take_profit: last_middle * 0.99,
            confidence: 68.0,
            reason: format!(
                "Fiyat üst banta ({:.2}) dokundu - Düzeltme bekleniyor",
                last_upper
            ),
        })
    } else {
        Some(StrategyResult {
            action: TradeAction::Hold,
            entry_price: current_price,
            stop_loss: 0.0,
            take_profit: 0.0,
            confidence: 0.0,
            reason: "Fiyat bantlar arasında - Sinyal yok".to_string(),
        })
    }
}

/// Strateji motoru - birden fazla stratejinin oy birliğini hesapla
pub struct StrategyEngine {
    results: Vec<StrategyResult>,
}

impl StrategyEngine {
    pub fn new() -> Self {
        Self {
            results: vec![],
        }
    }

    pub fn add_result(&mut self, result: StrategyResult) {
        self.results.push(result);
    }

    /// Tüm stratejilerin oy birliğini hesapla
    pub fn consensus(&self) -> Option<StrategyResult> {
        if self.results.is_empty() {
            return None;
        }

        // Satın alma ve satış oylarını say
        let buy_count = self
            .results
            .iter()
            .filter(|r| r.action == TradeAction::Buy)
            .count();
        let sell_count = self
            .results
            .iter()
            .filter(|r| r.action == TradeAction::Sell)
            .count();

        let total = self.results.len();
        let consensus_threshold = (total as f64 * 0.6) as usize; // %60 oy gerekli

        if buy_count >= consensus_threshold {
            let avg_entry = self
                .results
                .iter()
                .filter(|r| r.action == TradeAction::Buy)
                .map(|r| r.entry_price)
                .sum::<f64>()
                / buy_count as f64;
            let avg_confidence = self
                .results
                .iter()
                .filter(|r| r.action == TradeAction::Buy)
                .map(|r| r.confidence)
                .sum::<f64>()
                / buy_count as f64;

            return Some(StrategyResult {
                action: TradeAction::Buy,
                entry_price: avg_entry,
                stop_loss: avg_entry * 0.97,
                take_profit: avg_entry * 1.05,
                confidence: avg_confidence,
                reason: format!("{} / {} strateji BUY verdi", buy_count, total),
            });
        } else if sell_count >= consensus_threshold {
            let avg_entry = self
                .results
                .iter()
                .filter(|r| r.action == TradeAction::Sell)
                .map(|r| r.entry_price)
                .sum::<f64>()
                / sell_count as f64;
            let avg_confidence = self
                .results
                .iter()
                .filter(|r| r.action == TradeAction::Sell)
                .map(|r| r.confidence)
                .sum::<f64>()
                / sell_count as f64;

            return Some(StrategyResult {
                action: TradeAction::Sell,
                entry_price: avg_entry,
                stop_loss: avg_entry * 1.03,
                take_profit: avg_entry * 0.95,
                confidence: avg_confidence,
                reason: format!("{} / {} strateji SELL verdi", sell_count, total),
            });
        }

        // Oy birliği yok -> Hold
        Some(StrategyResult {
            action: TradeAction::Hold,
            entry_price: self.results[0].entry_price,
            stop_loss: 0.0,
            take_profit: 0.0,
            confidence: 0.0,
            reason: format!(
                "Oy birliği yok: BUY={}, SELL={}, HOLD={}",
                buy_count,
                sell_count,
                total - buy_count - sell_count
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ma_crossover() {
        let closes = vec![
            100.0, 101.0, 102.0, 103.0, 104.0, 105.0, 106.0, 107.0, 108.0, 109.0,
        ];
        let result = ma_crossover(&closes, 3, 5);
        assert!(result.is_some());
    }

    #[test]
    fn test_rsi_strategy() {
        let closes = vec![
            44.0, 44.34, 44.09, 43.61, 44.33, 44.83, 45.10, 45.42, 45.84, 46.08,
            45.89, 46.03, 45.61, 46.28, 46.00, 45.50, 46.00, 46.00, 46.00, 46.00,
        ];
        let result = rsi_strategy(&closes, 14, 70.0, 30.0);
        assert!(result.is_some());
    }
}
