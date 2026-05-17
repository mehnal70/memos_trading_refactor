use crate::core::types::Trade;
use chrono::{DateTime, Utc};
use std::collections::VecDeque;

/// Gerçek zamanlı alım-satım metrikleri
#[derive(Debug, Clone)]
pub struct TradingMetrics {
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub total_pnl_pct: f64,
    pub average_win: f64,
    pub average_loss: f64,
    pub profit_factor: f64, // Toplam kar / Toplam kayıp
    pub consecutive_wins: usize,
    pub consecutive_losses: usize,
    pub max_consecutive_wins: usize,
    pub max_consecutive_losses: usize,
}

impl Default for TradingMetrics {
    fn default() -> Self {
        Self {
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate: 0.0,
            total_pnl: 0.0,
            total_pnl_pct: 0.0,
            average_win: 0.0,
            average_loss: 0.0,
            profit_factor: 0.0,
            consecutive_wins: 0,
            consecutive_losses: 0,
            max_consecutive_wins: 0,
            max_consecutive_losses: 0,
        }
    }
}

impl TradingMetrics {
    /// Yeni metrikler oluştur
    pub fn new() -> Self {
        Self::default()
    }

    /// Trade'ler listesinden metrikleri hesapla
    pub fn from_trades(trades: &[Trade]) -> Self {
        let mut metrics = TradingMetrics::default();
        metrics.total_trades = trades.len();

        let mut total_wins = 0.0;
        let mut total_losses = 0.0;
        let mut total_pnl = 0.0;
        let mut winning_count = 0;
        let mut losing_count = 0;
        let mut consecutive_wins = 0;
        let mut consecutive_losses = 0;
        let mut max_consecutive_wins = 0;
        let mut max_consecutive_losses = 0;

        for trade in trades {
            if let Some(pnl) = trade.pnl {
                total_pnl += pnl;

                if pnl > 0.0 {
                    winning_count += 1;
                    total_wins += pnl;
                    consecutive_wins += 1;
                    consecutive_losses = 0;
                    max_consecutive_wins = max_consecutive_wins.max(consecutive_wins);
                } else if pnl < 0.0 {
                    losing_count += 1;
                    total_losses += pnl.abs();
                    consecutive_losses += 1;
                    consecutive_wins = 0;
                    max_consecutive_losses = max_consecutive_losses.max(consecutive_losses);
                }
            }
        }

        metrics.winning_trades = winning_count;
        metrics.losing_trades = losing_count;
        metrics.consecutive_wins = consecutive_wins;
        metrics.consecutive_losses = consecutive_losses;
        metrics.max_consecutive_wins = max_consecutive_wins;
        metrics.max_consecutive_losses = max_consecutive_losses;
        metrics.total_pnl = total_pnl;

        if metrics.total_trades > 0 {
            metrics.win_rate = (winning_count as f64 / metrics.total_trades as f64) * 100.0;
        }

        if winning_count > 0 {
            metrics.average_win = total_wins / winning_count as f64;
        }

        if losing_count > 0 {
            metrics.average_loss = total_losses / losing_count as f64;
        }

        if total_losses > 0.0 {
            metrics.profit_factor = total_wins / total_losses;
        }

        metrics
    }

    /// Başlangıç balance'ına göre toplam return hesapla
    pub fn calculate_total_return_pct(&self, initial_balance: f64) -> f64 {
        if initial_balance > 0.0 {
            (self.total_pnl / initial_balance) * 100.0
        } else {
            0.0
        }
    }
}

/// Zaman serisi metrikleri (performans trend analizi)
#[derive(Debug, Clone)]
pub struct EquityTrend {
    history: VecDeque<(DateTime<Utc>, f64)>, // (timestamp, equity)
    max_history_size: usize,
}

impl EquityTrend {
    /// Yeni trend oluştur
    pub fn new(max_size: usize) -> Self {
        Self {
            history: VecDeque::new(),
            max_history_size: max_size,
        }
    }

    /// Equity snapshot ekle
    pub fn record_equity(&mut self, equity: f64) {
        self.history.push_back((Utc::now(), equity));

        // Eski kayıtları kaldır
        while self.history.len() > self.max_history_size {
            self.history.pop_front();
        }
    }

    /// Son equity'yi getir
    pub fn latest_equity(&self) -> Option<f64> {
        self.history.back().map(|(_, e)| *e)
    }

    /// Trend boyunca average equity
    pub fn average_equity(&self) -> f64 {
        if self.history.is_empty() {
            return 0.0;
        }

        let sum: f64 = self.history.iter().map(|(_, e)| e).sum();
        sum / self.history.len() as f64
    }

    /// Volatility (standart sapma)
    pub fn volatility(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }

        let avg = self.average_equity();
        let variance: f64 = self.history
            .iter()
            .map(|(_, e)| (e - avg).powi(2))
            .sum::<f64>()
            / self.history.len() as f64;

        variance.sqrt()
    }

    /// Geçmiş kayıtları getir
    pub fn history(&self) -> &VecDeque<(DateTime<Utc>, f64)> {
        &self.history
    }
}
