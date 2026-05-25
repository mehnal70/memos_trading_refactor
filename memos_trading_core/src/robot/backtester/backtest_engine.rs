// backtest_engine.rs - Yüksek Performanslı Otonom Simülasyon Motoru

use crate::core::types::{Candle, StrategyParams};
use crate::Result;
use crate::MemosTradingError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// --- 1. YAPILANDIRMA VE VERİ MODELLERİ ---

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BacktestConfig {
    pub symbol: String,
    pub interval: String,
    pub initial_balance: f64,
    pub max_position_size: f64,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub strategy_name: String,
    pub strategy_params: Option<StrategyParams>,
    #[serde(default = "default_commission")]
    pub commission_pct: f64,
    // Pozisyon Yönetimi (B1/B2/B3)
    pub breakeven_at_rr: Option<f64>,
    pub atr_trail_mult: Option<f64>,
    pub partial_tp_ratio: Option<f64>,
    pub position_profile: Option<String>,
    pub security_profile: Option<String>,
}

fn default_commission() -> f64 { 0.001 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedTrade {
    pub symbol: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub entry_time: String,
    pub exit_time: String,
    pub amount: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub duration_minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub symbol: String,
    pub strategy: String,
    pub total_trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub total_pnl_pct: f64,
    pub max_drawdown_pct: f64,
    pub profit_factor: f64,
    pub sharpe_ratio: f64,
    pub trades: Vec<SimulatedTrade>,
}

// --- 2. ANA SİMÜLASYON MOTORU ---

struct BacktestPos {
    entry_price: f64,
    entry_idx: usize,
    entry_ts: DateTime<Utc>,
    qty: f64,
    sl_price: f64,
    tp_price: f64,
    risk_distance: f64,
    best_price: f64,
    trailing_pct: Option<f64>,
    trailing_sl: Option<f64>,
    breakeven_triggered: bool,
    partial_tp_triggered: bool,
}

pub struct Backtester {
    config: BacktestConfig,
    trades: Vec<SimulatedTrade>,
    balance_history: Vec<(DateTime<Utc>, f64)>,
}

impl Backtester {
    pub fn new(config: BacktestConfig) -> Self {
        Self {
            config,
            trades: Vec::with_capacity(100),
            balance_history: Vec::with_capacity(1000),
        }
    }

    pub fn run(&mut self, candles: &[Candle]) -> Result<BacktestResult> {
        if candles.is_empty() {
            return Err(MemosTradingError::Strategy("Mum verisi yok".to_owned()));
        }

        let mut balance = self.config.initial_balance;
        let mut pos: Option<BacktestPos> = None;
        let mut max_balance = balance;
        let mut max_drawdown: f64 = 0.0;

        // Mumları zaman sırasına sok (Emanet kopyalama yerine referans kullanımı)
        let mut sorted = candles.to_vec();
        sorted.sort_by_key(|c| c.timestamp);

        for (idx, candle) in sorted.iter().enumerate() {
            let mut close_signal = false;
            let mut trade_net = 0.0;

            if let Some(ref mut p) = pos {
                // B2: Trailing Stop Güncelleme
                if let Some(trail_pct) = p.trailing_pct {
                    p.best_price = p.best_price.max(candle.close);
                    let new_trail = p.best_price * (1.0 - trail_pct / 100.0);
                    p.trailing_sl = Some(p.trailing_sl.unwrap_or(0.0).max(new_trail));
                }

                let eff_sl = p.trailing_sl.unwrap_or(p.sl_price).max(p.sl_price);

                // B1: Breakeven (Başabaş) Kontrolü
                if !p.breakeven_triggered {
                    if let Some(be_rr) = self.config.breakeven_at_rr {
                        if candle.close - p.entry_price >= be_rr * p.risk_distance {
                            p.sl_price = p.entry_price;
                            p.breakeven_triggered = true;
                        }
                    }
                }

                // B3: Kısmi Kar Al (Partial TP)
                if !p.partial_tp_triggered {
                    if let Some(ratio) = self.config.partial_tp_ratio {
                        let partial_threshold = p.entry_price + (p.tp_price - p.entry_price) * 0.5;
                        if candle.close >= partial_threshold {
                            let p_qty = p.qty * ratio;
                            let fee = p_qty * (p.entry_price + candle.close) * self.config.commission_pct;
                            let net = (candle.close - p.entry_price) * p_qty - fee;
                            
                            self.trades.push(self.create_sim_trade(p, candle, p_qty, net));
                            balance += net;
                            p.qty -= p_qty;
                            p.partial_tp_triggered = true;
                        }
                    }
                }

                // Tam Çıkış Kontrolü (SL veya TP)
                if candle.close >= p.tp_price || candle.close <= eff_sl {
                    let fee = p.qty * (p.entry_price + candle.close) * self.config.commission_pct;
                    trade_net = (candle.close - p.entry_price) * p.qty - fee;
                    self.trades.push(self.create_sim_trade(p, candle, p.qty, trade_net));
                    close_signal = true;
                }
            }

            if close_signal {
                balance += trade_net;
                pos = None;
            }

            // Stratejik Giriş Kontrolü
            if pos.is_none() && Self::should_open(&sorted, idx, &self.config) {
                let entry = candle.close;
                let sl = entry * (1.0 - self.config.stop_loss_pct / 100.0);
                let trail_pct = self.config.atr_trail_mult.map(|m| Self::calc_atr_pct(&sorted[..=idx]) * m);
                
                pos = Some(BacktestPos {
                    entry_price: entry,
                    entry_idx: idx,
                    entry_ts: candle.timestamp,
                    qty: self.config.max_position_size,
                    sl_price: sl,
                    tp_price: entry * (1.0 + self.config.take_profit_pct / 100.0),
                    risk_distance: (entry - sl).abs().max(f64::EPSILON),
                    best_price: entry,
                    trailing_pct: trail_pct,
                    trailing_sl: None,
                    breakeven_triggered: false,
                    partial_tp_triggered: false,
                });
            }

            // Risk & Bakiye Takibi
            max_balance = max_balance.max(balance);
            let current_val = balance + pos.as_ref().map_or(0.0, |p| (candle.close - p.entry_price) * p.qty);
            max_drawdown = max_drawdown.max((max_balance - current_val) / max_balance * 100.0);
            self.balance_history.push((candle.timestamp, balance));
        }

        self.finalize_result(balance, max_drawdown)
    }

    fn create_sim_trade(&self, p: &BacktestPos, c: &Candle, qty: f64, net: f64) -> SimulatedTrade {
        SimulatedTrade {
            symbol: c.symbol.clone(),
            entry_price: p.entry_price,
            exit_price: c.close,
            entry_time: p.entry_ts.to_rfc3339(),
            exit_time: c.timestamp.to_rfc3339(),
            amount: qty,
            pnl: net,
            pnl_pct: (net / (p.entry_price * qty + f64::EPSILON)) * 100.0,
            duration_minutes: (c.timestamp - p.entry_ts).num_minutes(),
        }
    }

    fn finalize_result(&self, balance: f64, max_dd: f64) -> Result<BacktestResult> {
        let total_pnl = balance - self.config.initial_balance;
        let win_count = self.trades.iter().filter(|t| t.pnl > 0.0).count();

        // Profit factor = brüt kâr / brüt zarar (gerçek hesap; eski hardcode 1.5 idi).
        // Zarar yokken kâr varsa anlamlı bir tavan (999) — INF JSON'da sorun çıkarır.
        let gross_profit: f64 = self.trades.iter().filter(|t| t.pnl > 0.0).map(|t| t.pnl).sum();
        let gross_loss: f64   = self.trades.iter().filter(|t| t.pnl < 0.0).map(|t| -t.pnl).sum();
        let profit_factor = if gross_loss > f64::EPSILON { gross_profit / gross_loss }
            else if gross_profit > 0.0 { 999.0 } else { 0.0 };

        // Per-trade Sharpe = ortalama getiri / getiri std (gerçek hesap; eski hardcode 2.0).
        // sqrt(n) ölçeklemesi YOK — A/B karşılaştırmasında trade sayısı farklı olabilir,
        // bu yüzden trade-başına risk-ayarlı getiri daha adil bir kıyas metriğidir.
        let rets: Vec<f64> = self.trades.iter().map(|t| t.pnl_pct).collect();
        let n = rets.len();
        let sharpe_ratio = if n >= 2 {
            let mean = rets.iter().sum::<f64>() / n as f64;
            let var = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
            let sd = var.sqrt();
            if sd > f64::EPSILON { mean / sd } else { 0.0 }
        } else { 0.0 };

        Ok(BacktestResult {
            symbol: self.config.symbol.clone(),
            strategy: self.config.strategy_name.clone(),
            total_trades: self.trades.len(),
            win_rate: (win_count as f64 / self.trades.len().max(1) as f64) * 100.0,
            total_pnl,
            total_pnl_pct: (total_pnl / self.config.initial_balance) * 100.0,
            max_drawdown_pct: max_dd,
            profit_factor,
            sharpe_ratio,
            trades: self.trades.clone(),
        })
    }

    // --- 3. TEKNİK ANALİZ VE STRATEJİ MATRİSİ ---

    fn should_open(candles: &[Candle], idx: usize, cfg: &BacktestConfig) -> bool {
        if idx < 20 { return false; }
        let current = &candles[idx];
        
        match cfg.strategy_name.as_str() {
            "RSI" => {
                let rsi = Self::calc_rsi(candles, idx, 14);
                rsi < 30.0
            },
            "EMA_CROSS" => {
                let e_fast = Self::calc_ema(candles, idx, 9);
                let e_slow = Self::calc_ema(candles, idx, 21);
                e_fast > e_slow
            },
            "PRICE_ACTION" => {
                let prev = &candles[idx-1];
                current.close > prev.high && current.close > current.open // Simple Breakout
            },
            _ => current.close > Self::calc_sma(candles, idx, 20),
        }
    }

    fn calc_sma(candles: &[Candle], idx: usize, p: usize) -> f64 {
        let start = idx.saturating_sub(p - 1);
        candles[start..=idx].iter().map(|c| c.close).sum::<f64>() / p as f64
    }

    fn calc_ema(candles: &[Candle], idx: usize, p: usize) -> f64 {
        let alpha = 2.0 / (p as f64 + 1.0);
        let mut ema = candles[0].close;
        for i in 1..=idx {
            ema = (candles[i].close - ema) * alpha + ema;
        }
        ema
    }

    fn calc_rsi(candles: &[Candle], idx: usize, p: usize) -> f64 {
        if idx < p { return 50.0; }
        let mut gains = 0.0;
        let mut losses = 0.0;
        for i in (idx - p + 1)..=idx {
            let diff = candles[i].close - candles[i-1].close;
            if diff > 0.0 { gains += diff; } else { losses += diff.abs(); }
        }
        if losses == 0.0 { return 100.0; }
        100.0 - (100.0 / (1.0 + (gains / losses)))
    }

    fn calc_atr_pct(candles: &[Candle]) -> f64 {
        let n = candles.len();
        if n < 2 { return 1.0; }
        let tr = (candles[n-1].high - candles[n-1].low)
            .max((candles[n-1].high - candles[n-2].close).abs());
        (tr / candles[n-1].close) * 100.0
    }
}
