// robot/backtest_scheduler.rs - Otomatik backtest scheduler ve param optimization
// Periyodik olarak geçmiş veri ile backtest çalıştırır, parametreleri optimize eder, canary mode ile live'e geçer

use chrono::{DateTime, Duration, Utc};
use crate::core::types::{Candle, StrategyParams, Signal};
use crate::Result;

/// Backtest sonuçları
#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub total_trades: usize,
    pub win_rate: f64,                  // 0-100
    pub sharpe_ratio: f64,              // Risk-adjusted return
    pub max_drawdown_pct: f64,          // Max loss from peak
    pub profit_factor: f64,             // Gross profit / Gross loss
    pub net_pnl: f64,                   // Total P&L
    pub params: StrategyParams,         // Kullanılan parametreler
    pub timestamp: DateTime<Utc>,       // Backtest zamanı
}

impl BacktestResult {
    /// Backtest sonuçları canlı trade için uygun mu?
    pub fn is_production_ready(&self) -> bool {
        self.sharpe_ratio > 1.0
            && self.max_drawdown_pct < 20.0
            && self.win_rate > 45.0
            && self.profit_factor > 1.5
    }

    /// Canary mode kontrol (paper→live geçişi güvenli mi?)
    pub fn can_go_live(&self) -> CanaryStatus {
        match (
            self.sharpe_ratio > 1.0,
            self.max_drawdown_pct < 20.0,
            self.win_rate > 45.0,
            self.profit_factor > 1.5,
        ) {
            (true, true, true, true) => CanaryStatus::ReadyForLive,
            (true, true, true, false) => CanaryStatus::WaitForProfitability,
            (true, true, false, _) => CanaryStatus::WaitForWinRate,
            (true, false, _, _) => CanaryStatus::WaitForDrawdownControl,
            (false, _, _, _) => CanaryStatus::WaitForSharpeRatio,
        }
    }

    /// Önceki sonuçtan daha iyi mi?
    pub fn is_better_than(&self, other: &BacktestResult) -> bool {
        // Composite score: Sharpe * Win Rate / (1 + DD%)
        let self_score = (self.sharpe_ratio * self.win_rate) / (1.0 + self.max_drawdown_pct);
        let other_score = (other.sharpe_ratio * other.win_rate) / (1.0 + other.max_drawdown_pct);
        self_score > other_score
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CanaryStatus {
    ReadyForLive,
    WaitForSharpeRatio,
    WaitForDrawdownControl,
    WaitForWinRate,
    WaitForProfitability,
}

/// Scheduler konfigürasyonu
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub backtest_interval_secs: u64,   // Ne sıklıkta backtest çalıştır (örn. 3600 = 1h)
    pub lookback_days: u64,             // Kaç gün geçmiş veri kullan (örn. 30)
    pub min_candles_for_backtest: usize,// Min candle sayısı (örn. 100)
    pub max_consecutive_losses: usize,  // Fail-safe: max kayıp trade sayısı
    pub paper_duration_before_live: u64,// Paper mod süresi (secs, örn. 86400 = 1 gün)
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            backtest_interval_secs: 3600,   // 1 saat
            lookback_days: 30,
            min_candles_for_backtest: 100,
            max_consecutive_losses: 5,
            paper_duration_before_live: 86400, // 1 gün
        }
    }
}

/// Backtest scheduler
pub struct BacktestScheduler {
    pub config: SchedulerConfig,
    pub last_backtest_time: Option<DateTime<Utc>>,
    pub last_result: Option<BacktestResult>,
    pub best_result: Option<BacktestResult>,
    pub current_mode: TradingMode,
    pub mode_switch_time: Option<DateTime<Utc>>,
    pub consecutive_losses: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TradingMode {
    Disabled,  // Sistemi kapat
    Paper,     // Paper trading (güvenli test)
    Live,      // Live trading (canlı para)
}

impl BacktestScheduler {
    /// Yeni scheduler
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            last_backtest_time: None,
            last_result: None,
            best_result: None,
            current_mode: TradingMode::Paper, // Varsayılan paper
            mode_switch_time: Some(Utc::now()),
            consecutive_losses: 0,
        }
    }

    /// Backtest çalıştırılması gerekli mi?
    pub fn should_run_backtest(&self) -> bool {
        match self.last_backtest_time {
            None => true, // İlk kez
            Some(last_time) => {
                let elapsed = Utc::now() - last_time;
                elapsed > Duration::seconds(self.config.backtest_interval_secs as i64)
            }
        }
    }

    /// Backtest sonucu kaydederken mode geçişi kontrol et
    pub fn process_result(&mut self, result: BacktestResult) {
        self.last_backtest_time = Some(Utc::now());

        // Best result güncelle
        if let Some(ref best) = self.best_result {
            if result.is_better_than(best) {
                self.best_result = Some(result.clone());
            }
        } else {
            self.best_result = Some(result.clone());
        }

        // Canary status kontrol
        match result.can_go_live() {
            CanaryStatus::ReadyForLive => {
                // Paper mode'dan live'e geçiş hazırlığı
                if self.current_mode == TradingMode::Paper {
                    if let Some(switch_time) = self.mode_switch_time {
                        let elapsed = Utc::now() - switch_time;
                        if elapsed > Duration::seconds(self.config.paper_duration_before_live as i64) {
                            self.current_mode = TradingMode::Live;
                            println!("[SCHEDULER] ✓ Paper validation passed! Switching to LIVE");
                        }
                    }
                }
            }
            _ => {
                // Eğer live mode'daysa, sonuç kötü ise paper'a geri dön
                if self.current_mode == TradingMode::Live {
                    println!("[SCHEDULER] ⚠ Metrics degrading, reverting to PAPER");
                    self.current_mode = TradingMode::Paper;
                    self.mode_switch_time = Some(Utc::now());
                }
            }
        }

        self.last_result = Some(result);
    }

    /// Trade sonucu kaydediyor (win/loss için loss counter)
    pub fn record_trade_outcome(&mut self, is_win: bool) {
        if is_win {
            self.consecutive_losses = 0;
        } else {
            self.consecutive_losses += 1;
            if self.consecutive_losses >= self.config.max_consecutive_losses {
                println!("[SCHEDULER] FAILSAFE: {} consecutive losses, disabling trading", self.consecutive_losses);
                self.current_mode = TradingMode::Disabled;
            }
        }
    }

    /// Mode bilgisi
    pub fn status(&self) -> String {
        format!(
            "[Scheduler] Mode: {:?}, Last BT: {:?}, Sharpe: {:.2}, DD: {:.1}%, Consecutive Losses: {}",
            self.current_mode,
            self.last_backtest_time.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()),
            self.last_result.as_ref().map(|r| r.sharpe_ratio).unwrap_or(0.0),
            self.last_result.as_ref().map(|r| r.max_drawdown_pct).unwrap_or(0.0),
            self.consecutive_losses,
        )
    }
}

/// Basit backtest simülatörü (dummy)
pub fn simulate_backtest(
    candles: &[Candle],
    params: &StrategyParams,
    initial_capital: f64,
) -> Result<BacktestResult> {
    if candles.len() < 10 {
        return Err("Insufficient candles for backtest".into());
    }

    // Dummy: basit sinyal üretim
    let mut trades = 0;
    let mut wins = 0;
    let mut total_pnl = 0.0;
    let mut peak_equity = initial_capital;
    let mut max_drawdown = 0.0;

    let mut last_signal = Signal::Hold;
    for (i, _candle) in candles.iter().enumerate().skip(2) {
        // Basit MA çapraz sinyal (dummy)
        let short_ma = candles[i - 1].close;
        let long_ma = candles[i - 2].close;

        let signal = if short_ma > long_ma {
            Signal::Buy
        } else {
            Signal::Sell
        };

        if signal != last_signal && signal != Signal::Hold {
            trades += 1;
            // Dummy P&L: 0.5% veya -0.3%
            let pnl = if i % 3 == 0 { 0.005 } else { -0.003 };
            total_pnl += pnl * initial_capital;

            if pnl > 0.0 {
                wins += 1;
            }

            last_signal = signal;
        }

        let current_equity = initial_capital + total_pnl;
        if current_equity > peak_equity {
            peak_equity = current_equity;
        }
        let dd = ((peak_equity - current_equity) / peak_equity) * 100.0;
        if dd > max_drawdown {
            max_drawdown = dd;
        }
    }

    let win_rate = if trades > 0 {
        (wins as f64 / trades as f64) * 100.0
    } else {
        0.0
    };

    // Dummy Sharpe (normal return / volatility)
    let sharpe_ratio = if max_drawdown > 0.0 {
        (total_pnl / initial_capital) / (max_drawdown / 100.0).max(0.01)
    } else {
        0.0
    };

    let profit_factor = if total_pnl < 0.0 { 0.1 } else { 2.0 };

    Ok(BacktestResult {
        total_trades: trades,
        win_rate,
        sharpe_ratio: sharpe_ratio.max(0.1), // Min 0.1
        max_drawdown_pct: max_drawdown,
        profit_factor,
        net_pnl: total_pnl,
        params: params.clone(),
        timestamp: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_backtest_result_ready() {
        let result = BacktestResult {
            total_trades: 100,
            win_rate: 55.0,
            sharpe_ratio: 1.5,
            max_drawdown_pct: 10.0,
            profit_factor: 2.0,
            net_pnl: 1000.0,
            params: StrategyParams::default(),
            timestamp: Utc::now(),
        };
        assert!(result.is_production_ready());
    }

    #[test]
    fn test_backtest_result_poor_sharpe() {
        let result = BacktestResult {
            total_trades: 100,
            win_rate: 55.0,
            sharpe_ratio: 0.5, // Too low
            max_drawdown_pct: 10.0,
            profit_factor: 2.0,
            net_pnl: 1000.0,
            params: StrategyParams::default(),
            timestamp: Utc::now(),
        };
        assert!(!result.is_production_ready());
        assert_eq!(result.can_go_live(), CanaryStatus::WaitForSharpeRatio);
    }

    #[test]
    fn test_scheduler_should_run_backtest() {
        let config = SchedulerConfig {
            backtest_interval_secs: 10,
            ..Default::default()
        };
        let scheduler = BacktestScheduler::new(config);
        assert!(scheduler.should_run_backtest()); // First time
    }

    #[test]
    fn test_scheduler_consecutive_losses() {
        let config = SchedulerConfig {
            max_consecutive_losses: 3,
            ..Default::default()
        };
        let mut scheduler = BacktestScheduler::new(config);
        assert_eq!(scheduler.current_mode, TradingMode::Paper);

        scheduler.record_trade_outcome(false);
        scheduler.record_trade_outcome(false);
        assert_eq!(scheduler.current_mode, TradingMode::Paper);

        scheduler.record_trade_outcome(false); // 3rd loss
        assert_eq!(scheduler.current_mode, TradingMode::Disabled);
    }

    #[test]
    fn test_scheduler_win_resets_loss_counter() {
        let config = SchedulerConfig {
            max_consecutive_losses: 5,
            ..Default::default()
        };
        let mut scheduler = BacktestScheduler::new(config);

        scheduler.record_trade_outcome(false);
        scheduler.record_trade_outcome(false);
        assert_eq!(scheduler.consecutive_losses, 2);

        scheduler.record_trade_outcome(true);
        assert_eq!(scheduler.consecutive_losses, 0);
    }
}
