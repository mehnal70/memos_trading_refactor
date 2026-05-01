// robot/backtester/walk_forward.rs
//
// Walk-Forward Test (Kayan Pencere Testi)
//
// Klasik backtest "curve fitting" riski taşır: parametreler tüm veriye göre
// seçilir, dolayısıyla geçmişi ezberler. Walk-forward bu sorunu çözer:
//
//   ┌─────────────────────────────────────────────────────────┐
//   │  Window 1: [  IN-SAMPLE  ][OOS]                         │
//   │  Window 2:      [  IN-SAMPLE  ][OOS]                    │
//   │  Window 3:           [  IN-SAMPLE  ][OOS]               │
//   └─────────────────────────────────────────────────────────┘
//
// Her pencerede:
//   1. In-sample bölümünde ParameterOptimizer ile en iyi TP/SL bulunur.
//   2. Bu parametreler out-of-sample (OOS) bölümünde test edilir.
//   3. Sadece OOS sonuçları raporlanır → "gerçek dünya" tahmini.
//
// Aggregate OOS metrikleri tüm pencerelerin ortalamasıdır.

use serde::{Deserialize, Serialize};
use crate::types::Candle;
use crate::robot::backtester::{Backtester, BacktestConfig};

/// Walk-forward test yapılandırması
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardConfig {
    /// In-sample pencere uzunluğu (mum sayısı)
    pub in_sample_bars: usize,
    /// Out-of-sample pencere uzunluğu (mum sayısı)
    pub out_of_sample_bars: usize,
    /// Her adımda kaç mum ilerleneceği (step < oos → örtüşen pencereler)
    pub step_bars: usize,
    /// Başlangıç bakiyesi
    pub initial_balance: f64,
    /// Optimize edilecek strateji
    pub strategy_name: String,
    /// Sembol ve interval
    pub symbol: String,
    pub interval: String,
    /// Komisyon oranı
    pub commission_pct: f64,
}

impl Default for WalkForwardConfig {
    fn default() -> Self {
        Self {
            in_sample_bars:     200,
            out_of_sample_bars:  50,
            step_bars:           50,
            initial_balance:  10_000.0,
            strategy_name:    "RSI".to_string(),
            symbol:           "BTCUSDT".to_string(),
            interval:         "1h".to_string(),
            commission_pct:    0.001,
        }
    }
}

/// Tek bir pencerenin OOS sonucu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowResult {
    pub window_idx:          usize,
    pub in_sample_start:     usize,
    pub in_sample_end:       usize,
    pub oos_start:           usize,
    pub oos_end:             usize,
    /// In-sample'da seçilen parametreler
    pub best_tp_pct:         f64,
    pub best_sl_pct:         f64,
    /// OOS test sonuçları
    pub oos_trades:          usize,
    pub oos_win_rate:        f64,
    pub oos_pnl_pct:         f64,
    pub oos_profit_factor:   f64,
    pub oos_max_dd_pct:      f64,
    pub oos_sharpe:          f64,
}

/// Tüm walk-forward testinin özet sonucu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardResult {
    pub config:               WalkForwardConfig,
    pub windows:              Vec<WindowResult>,
    pub total_windows:        usize,
    pub profitable_windows:   usize,

    // ── Aggregate OOS metrikleri (pencerelerin ortalaması) ──────────────
    pub avg_oos_win_rate:     f64,
    pub avg_oos_pnl_pct:      f64,
    pub avg_oos_profit_factor:f64,
    pub avg_oos_max_dd_pct:   f64,
    pub avg_oos_sharpe:       f64,

    /// Istikrar skoru: pencerelerin kaçı kârlı (0.0–1.0)
    pub consistency_score:    f64,
    /// Ortalama OOS trades
    pub avg_oos_trades:       f64,
}

/// Walk-Forward Test çalıştırıcısı
pub struct WalkForwardTester {
    pub config: WalkForwardConfig,
}

impl WalkForwardTester {
    pub fn new(config: WalkForwardConfig) -> Self {
        Self { config }
    }

    /// Ana test fonksiyonu
    pub fn run(&self, candles: &[Candle]) -> Option<WalkForwardResult> {
        let total = candles.len();
        let window_size = self.config.in_sample_bars + self.config.out_of_sample_bars;

        if total < window_size {
            return None;
        }

        let mut windows: Vec<WindowResult> = Vec::new();
        let mut start = 0usize;

        while start + window_size <= total {
            let is_end  = start + self.config.in_sample_bars;
            let oos_end = is_end + self.config.out_of_sample_bars;

            let in_sample = &candles[start..is_end];
            let oos       = &candles[is_end..oos_end];

            // In-sample: grid search (4×4 = 16 kombinasyon, hızlı)
            let (best_tp, best_sl) = self.quick_optimize(in_sample);

            // OOS: seçilen parametrelerle backtest
            let oos_result = self.run_backtest(oos, best_tp, best_sl);

            windows.push(WindowResult {
                window_idx:        windows.len(),
                in_sample_start:   start,
                in_sample_end:     is_end,
                oos_start:         is_end,
                oos_end,
                best_tp_pct:       best_tp,
                best_sl_pct:       best_sl,
                oos_trades:        oos_result.0,
                oos_win_rate:      oos_result.1,
                oos_pnl_pct:       oos_result.2,
                oos_profit_factor: oos_result.3,
                oos_max_dd_pct:    oos_result.4,
                oos_sharpe:        oos_result.5,
            });

            start += self.config.step_bars;
        }

        if windows.is_empty() {
            return None;
        }

        let n = windows.len() as f64;
        let profitable = windows.iter().filter(|w| w.oos_pnl_pct > 0.0).count();

        let avg = |f: fn(&WindowResult) -> f64| -> f64 {
            windows.iter().map(f).sum::<f64>() / n
        };

        Some(WalkForwardResult {
            total_windows:         windows.len(),
            profitable_windows:    profitable,
            avg_oos_win_rate:      avg(|w| w.oos_win_rate),
            avg_oos_pnl_pct:       avg(|w| w.oos_pnl_pct),
            avg_oos_profit_factor: avg(|w| w.oos_profit_factor),
            avg_oos_max_dd_pct:    avg(|w| w.oos_max_dd_pct),
            avg_oos_sharpe:        avg(|w| w.oos_sharpe),
            avg_oos_trades:        avg(|w| w.oos_trades as f64),
            // En az 3 pencere olmadan oran istatistiksel anlamsız → 0.0 dön
            consistency_score:     if windows.len() >= 3 { profitable as f64 / windows.len() as f64 } else { 0.0 },
            config:                self.config.clone(),
            windows,
        })
    }

    // ── Yardımcı: hızlı grid search (in-sample) ──────────────────────────

    fn quick_optimize(&self, candles: &[Candle]) -> (f64, f64) {
        // 8×8 = 64 kombinasyon — daha ince ızgara, daha iyi parametre kalitesi
        let tp_values = [2.5, 3.5, 5.0, 6.5, 8.0, 10.0, 12.0, 15.0];
        let sl_values = [1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.5, 5.5];

        let mut best_score = f64::NEG_INFINITY;
        let mut best_tp    = 5.0;
        let mut best_sl    = 2.0;

        for &tp in &tp_values {
            for &sl in &sl_values {
                if tp <= sl { continue; } // RR < 1 — geçersiz
                let r = self.run_backtest(candles, tp, sl);
                // Kompozit skor: Sharpe 40% + PnL 35% + WinRate 25%
                let score = r.5 * 0.40 + r.2 * 0.35 + r.1 * 0.25;
                if score > best_score {
                    best_score = score;
                    best_tp = tp;
                    best_sl = sl;
                }
            }
        }

        (best_tp, best_sl)
    }

    /// Backtest çalıştır → (trades, win_rate, pnl_pct, pf, max_dd, sharpe)
    fn run_backtest(&self, candles: &[Candle], tp: f64, sl: f64) -> (usize, f64, f64, f64, f64, f64) {
        let cfg = BacktestConfig {
            symbol:           self.config.symbol.clone(),
            interval:         self.config.interval.clone(),
            initial_balance:  self.config.initial_balance,
            max_position_size: 1.0,
            take_profit_pct:  tp,
            stop_loss_pct:    sl,
            strategy_name:    self.config.strategy_name.clone(),
            position_profile: None,
            security_profile: None,
            strategy_params:  None,
            commission_pct:   self.config.commission_pct,
            breakeven_at_rr:  None,
            atr_trail_mult:   None,
            partial_tp_ratio: None,
        };

        match Backtester::new(cfg).run(candles) {
            Ok(r) => (r.total_trades, r.win_rate, r.total_pnl_pct,
                      r.profit_factor, r.max_drawdown_pct, r.sharpe_ratio),
            Err(_) => (0, 0.0, 0.0, 0.0, 0.0, 0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_candles(n: usize) -> Vec<Candle> {
        let mut price = 100.0f64;
        (0..n).map(|i| {
            price += (i as f64 * 0.3) % 4.0 - 1.5;
            Candle {
                symbol:    "BTC".into(),
                interval:  "1h".into(),
                timestamp: Utc::now() + chrono::Duration::hours(i as i64),
                open:  price,
                high:  price + 2.0,
                low:   price - 1.0,
                close: price + 0.5,
                volume: 500.0 + i as f64 * 3.0,
            }
        }).collect()
    }

    #[test]
    fn test_walk_forward_runs() {
        let cfg = WalkForwardConfig {
            in_sample_bars:     60,
            out_of_sample_bars: 20,
            step_bars:          20,
            initial_balance:    10_000.0,
            strategy_name:      "RSI".into(),
            symbol:             "BTC".into(),
            interval:           "1h".into(),
            commission_pct:     0.001,
        };
        let candles = make_candles(200);
        let result  = WalkForwardTester::new(cfg).run(&candles);
        assert!(result.is_some(), "walk-forward sonuç dönmeli");
        let r = result.unwrap();
        assert!(r.total_windows >= 2,  "en az 2 pencere bekleniyor");
        assert!(r.consistency_score >= 0.0 && r.consistency_score <= 1.0);
        assert!(r.avg_oos_win_rate >= 0.0 && r.avg_oos_win_rate <= 100.0);
    }

    #[test]
    fn test_walk_forward_insufficient_data() {
        let cfg = WalkForwardConfig::default(); // 200+50 bar gerekir
        let candles = make_candles(100);        // yetersiz
        assert!(WalkForwardTester::new(cfg).run(&candles).is_none());
    }
}
