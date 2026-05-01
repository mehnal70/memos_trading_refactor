use crate::types::{Candle, StrategyParams};
use crate::Result;
use crate::MemosTradingError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Backtest yapılandırması
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestConfig {
    pub symbol: String,
    pub interval: String,
    pub initial_balance: f64,
    pub max_position_size: f64,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub strategy_name: String,
    pub position_profile: Option<String>,  // Profil bilgisi (opsiyonel)
    pub security_profile: Option<String>,  // Güvenlik profili (opsiyonel)
    /// Strateji parametreleri (RSI period/OB/OS vb.) — None ise varsayılan kullanılır
    #[serde(default)]
    pub strategy_params: Option<StrategyParams>,
    /// Komisyon oranı (giriş + çıkış her biri için, ör: 0.001 = %0.1)
    #[serde(default = "default_commission")]
    pub commission_pct: f64,
    // ── Pozisyon yönetimi parametreleri (B1/B2/B3) ───────────────────────────
    /// B1: Kâr bu R katına ulaşınca SL giriş fiyatına taşı (None = devre dışı)
    #[serde(default)]
    pub breakeven_at_rr: Option<f64>,
    /// B2: ATR trailing çarpanı (None = trailing yok)
    #[serde(default)]
    pub atr_trail_mult: Option<f64>,
    /// B3: TP'de kapatılacak pozisyon oranı (None = tam kapat)
    #[serde(default)]
    pub partial_tp_ratio: Option<f64>,
}

fn default_commission() -> f64 { 0.001 }

/// Pozisyon yönetimi optimizasyon sonucu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PosOptResult {
    pub breakeven_at_rr:  Option<f64>,
    pub atr_trail_mult:   Option<f64>,
    pub partial_tp_ratio: Option<f64>,
    pub score:            f64,
    pub win_rate:         f64,
    pub total_pnl_pct:    f64,
    pub profit_factor:    f64,
    pub total_trades:     usize,
}

/// Engine içi pozisyon takibi — B1/B2/B3 simülasyonu için
struct BacktestPos {
    entry_price:          f64,
    entry_idx:            usize,
    qty:                  f64,
    sl_price:             f64,
    tp_price:             f64,
    risk_distance:        f64,   // |entry - original_sl|
    best_price:           f64,
    trailing_pct:         Option<f64>,
    trailing_sl:          Option<f64>,
    breakeven_triggered:  bool,
    partial_tp_triggered: bool,
}

/// Tek bir trade'in simulasyon sonucu
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

/// Backtest sonuçları
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub symbol: String,
    pub strategy: String,
    pub period_start: String,
    pub period_end: String,
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub total_pnl_pct: f64,
    pub max_drawdown_pct: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub profit_factor: f64,
    pub sharpe_ratio: f64,
    pub trades: Vec<SimulatedTrade>,
    pub position_profile: Option<String>,  // Hangi profille çalıştırıldı
    pub security_profile: Option<String>,  // Güvenlik profili
}

/// Profil karşılaştırma sonuçları
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileComparisonResult {
    pub symbol: String,
    pub strategy: String,
    pub period_start: String,
    pub period_end: String,
    pub profiles: Vec<ProfilePerformance>,
    pub best_profile: String,
    pub best_metric: String,  // "win_rate", "total_pnl", "profit_factor", etc.
}

/// Tek bir profilin performans metrikleri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilePerformance {
    pub profile_name: String,
    pub total_trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub total_pnl_pct: f64,
    pub max_drawdown_pct: f64,
    pub profit_factor: f64,
    pub sharpe_ratio: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
}

/// Backtesting engine
pub struct Backtester {
    config: BacktestConfig,
    trades: Vec<SimulatedTrade>,
    balance_history: Vec<(DateTime<Utc>, f64)>,
}

impl Backtester {
    pub fn new(config: BacktestConfig) -> Self {
        Self {
            config,
            trades: Vec::new(),
            balance_history: Vec::new(),
        }
    }

    /// Tarihsel mum'larla backtest çalıştır
    pub fn run(&mut self, candles: &[Candle]) -> Result<BacktestResult> {
        if candles.is_empty() {
            return Err(MemosTradingError::Strategy(
                "Hiç mum verisi sağlanmadı".to_string(),
            ));
        }

        let mut balance = self.config.initial_balance;
        let mut pos: Option<BacktestPos> = None;
        let mut max_balance = balance;
        let mut max_drawdown = 0.0;

        // Mum'ları sırala
        let mut sorted_candles = candles.to_vec();
        sorted_candles.sort_by_key(|c| c.timestamp);

        for (idx, candle) in sorted_candles.iter().enumerate() {
            // ── Açık pozisyon yönetimi ──────────────────────────────────────────
            let mut close_pos       = false;
            let mut partial_trade:  Option<SimulatedTrade> = None;
            let mut partial_pnl     = 0.0;
            let mut close_trade:    Option<(SimulatedTrade, f64)> = None;

            if let Some(ref mut p) = pos {
                // B2: trailing stop güncelle
                if let Some(trail_pct) = p.trailing_pct {
                    if candle.close > p.best_price { p.best_price = candle.close; }
                    let new_trail = p.best_price * (1.0 - trail_pct / 100.0);
                    p.trailing_sl = Some(p.trailing_sl.unwrap_or(0.0_f64).max(new_trail));
                }
                let eff_sl = p.trailing_sl.unwrap_or(p.sl_price).max(p.sl_price);

                // B1: breakeven tetikle
                if !p.breakeven_triggered {
                    if let Some(be_rr) = self.config.breakeven_at_rr {
                        if candle.close - p.entry_price >= be_rr * p.risk_distance {
                            p.sl_price = p.entry_price;
                            p.breakeven_triggered = true;
                        }
                    }
                }

                let pnl_pct = (candle.close - p.entry_price) / p.entry_price * 100.0;
                // Kısmi TP eşiği: TP fiyatının tam yolunun yarısı
                let partial_tp_price = p.entry_price + (p.tp_price - p.entry_price) * 0.5;

                // B3: kısmi TP — TP yarı yoluna ulaşınca pozisyonun `ratio` kadarını kapat
                if !p.partial_tp_triggered {
                    if let Some(ratio) = self.config.partial_tp_ratio {
                        if candle.close >= partial_tp_price {
                            let partial_qty = p.qty * ratio;
                            let fee = partial_qty * (p.entry_price + candle.close) * self.config.commission_pct;
                            let gross = (candle.close - p.entry_price) * partial_qty;
                            let net   = gross - fee;
                            let net_pct = pnl_pct - fee / (p.entry_price * partial_qty + f64::EPSILON) * 100.0;
                            partial_trade = Some(SimulatedTrade {
                                symbol:           candle.symbol.clone(),
                                entry_price:      p.entry_price,
                                exit_price:       candle.close,
                                entry_time:       sorted_candles[p.entry_idx].timestamp.to_rfc3339(),
                                exit_time:        candle.timestamp.to_rfc3339(),
                                amount:           partial_qty,
                                pnl:              net,
                                pnl_pct:          net_pct,
                                duration_minutes: Self::calculate_duration(&sorted_candles[p.entry_idx], candle),
                            });
                            partial_pnl           = net;
                            p.qty                -= partial_qty;
                            p.partial_tp_triggered = true;
                            // Kısmi TP sonrası SL'yi breakeveN'e çek
                            if p.sl_price < p.entry_price {
                                p.sl_price          = p.entry_price;
                                p.breakeven_triggered = true;
                            }
                        }
                    }
                }

                // TP veya SL kontrolü — tp_price fiyat karşılaştırması (yüzde dönüşüm hatası yok)
                if candle.close >= p.tp_price || candle.close <= eff_sl {
                    let exit    = candle.close;
                    let fee     = p.qty * (p.entry_price + exit) * self.config.commission_pct;
                    let gross   = (exit - p.entry_price) * p.qty;
                    let net     = gross - fee;
                    let net_pct = pnl_pct - fee / (p.entry_price * p.qty + f64::EPSILON) * 100.0;
                    close_trade = Some((SimulatedTrade {
                        symbol:           candle.symbol.clone(),
                        entry_price:      p.entry_price,
                        exit_price:       exit,
                        entry_time:       sorted_candles[p.entry_idx].timestamp.to_rfc3339(),
                        exit_time:        candle.timestamp.to_rfc3339(),
                        amount:           p.qty,
                        pnl:              net,
                        pnl_pct:          net_pct,
                        duration_minutes: Self::calculate_duration(&sorted_candles[p.entry_idx], candle),
                    }, net));
                    close_pos = true;
                }
            }

            // Kısmi TP uygula (borrow bitti)
            if let Some(trade) = partial_trade {
                self.trades.push(trade);
                balance += partial_pnl;
                if balance > max_balance { max_balance = balance; }
            }

            // Tam kapat
            if close_pos {
                if let Some((trade, net)) = close_trade {
                    balance += net;
                    if balance > max_balance { max_balance = balance; }
                    self.trades.push(trade);
                }
                pos = None;
            }

            // Unrealized drawdown hesapla
            if let Some(ref p) = pos {
                let unrealized  = (candle.close - p.entry_price) * p.qty;
                let current_bal = balance + unrealized;
                let dd_pct      = (max_balance - current_bal) / max_balance * 100.0;
                if dd_pct > max_drawdown { max_drawdown = dd_pct; }
            }

            // Stratejiye göre giriş sinyali üret
            if pos.is_none() && Self::should_open_position(&sorted_candles, idx, &self.config.strategy_name, self.config.strategy_params.as_ref()) {
                let entry     = candle.close;
                let sl        = entry * (1.0 - self.config.stop_loss_pct / 100.0);
                let risk      = (entry - sl).abs().max(f64::EPSILON);
                let trail_pct = self.config.atr_trail_mult.map(|mult|
                    Self::calc_atr_pct(&sorted_candles[..=idx]) * mult
                );
                pos = Some(BacktestPos {
                    entry_price:          entry,
                    entry_idx:            idx,
                    qty:                  self.config.max_position_size,
                    sl_price:             sl,
                    tp_price:             entry * (1.0 + self.config.take_profit_pct / 100.0),
                    risk_distance:        risk,
                    best_price:           entry,
                    trailing_pct:         trail_pct,
                    trailing_sl:          None,
                    breakeven_triggered:  false,
                    partial_tp_triggered: false,
                });
            }

            // Balance history kaydet
            self.balance_history.push((candle.timestamp, balance));
        }

        // Açık pozisyon varsa kapat
        if let Some(p) = pos {
            let last_candle = &sorted_candles[sorted_candles.len() - 1];
            let fee         = p.qty * (p.entry_price + last_candle.close) * self.config.commission_pct;
            let gross_pnl   = (last_candle.close - p.entry_price) * p.qty;
            let pnl         = gross_pnl - fee;
            let pnl_pct     = (last_candle.close - p.entry_price) / p.entry_price * 100.0
                - fee / (p.entry_price * p.qty + f64::EPSILON) * 100.0;

            let trade = SimulatedTrade {
                symbol:           last_candle.symbol.clone(),
                entry_price:      p.entry_price,
                exit_price:       last_candle.close,
                entry_time:       sorted_candles[p.entry_idx].timestamp.to_rfc3339(),
                exit_time:        last_candle.timestamp.to_rfc3339(),
                amount:           p.qty,
                pnl,
                pnl_pct,
                duration_minutes: Self::calculate_duration(&sorted_candles[p.entry_idx], last_candle),
            };

            balance += pnl;
            self.trades.push(trade);
        }

        // Sonuçları hesapla
        let total_pnl = balance - self.config.initial_balance;
        let total_pnl_pct = (total_pnl / self.config.initial_balance) * 100.0;

        let winning_trades = self.trades.iter().filter(|t| t.pnl > 0.0).count();
        let losing_trades = self.trades.iter().filter(|t| t.pnl < 0.0).count();
        let win_rate = if self.trades.is_empty() {
            0.0
        } else {
            (winning_trades as f64 / self.trades.len() as f64) * 100.0
        };

        let avg_win = self.trades
            .iter()
            .filter(|t| t.pnl > 0.0)
            .map(|t| t.pnl)
            .sum::<f64>()
            / (winning_trades.max(1) as f64);

        let avg_loss = self.trades
            .iter()
            .filter(|t| t.pnl < 0.0)
            .map(|t| t.pnl.abs())
            .sum::<f64>()
            / (losing_trades.max(1) as f64);

        let profit_factor = if avg_loss > 0.0 && losing_trades > 0 {
            (avg_win * winning_trades as f64) / (avg_loss * losing_trades as f64)
        } else {
            0.0
        };

        let sharpe_ratio = Self::calculate_sharpe_ratio(&self.balance_history);

        Ok(BacktestResult {
            symbol: self.config.symbol.clone(),
            strategy: self.config.strategy_name.clone(),
            period_start: sorted_candles.first().unwrap().timestamp.to_rfc3339(),
            period_end: sorted_candles.last().unwrap().timestamp.to_rfc3339(),
            total_trades: self.trades.len(),
            winning_trades,
            losing_trades,
            win_rate,
            total_pnl,
            total_pnl_pct,
            max_drawdown_pct: max_drawdown,
            avg_win,
            avg_loss,
            profit_factor,
            sharpe_ratio,
            trades: self.trades.clone(),
            position_profile: self.config.position_profile.clone(),
            security_profile: self.config.security_profile.clone(),
        })
    }

    /// Farklı profiller ile backtest çalıştır ve karşılaştır
    pub fn compare_profiles(
        base_config: BacktestConfig,
        candles: &[Candle],
        profiles: &[&str],
    ) -> Result<ProfileComparisonResult> {
        if candles.is_empty() {
            return Err(MemosTradingError::Strategy(
                "Hiç mum verisi sağlanmadı".to_string(),
            ));
        }

        if profiles.is_empty() {
            return Err(MemosTradingError::Strategy(
                "Hiç profil sağlanmadı".to_string(),
            ));
        }

        let mut results = Vec::new();

        // Her profil için backtest çalıştır
        for profile_name in profiles {
            let mut config = base_config.clone();
            config.position_profile = Some(profile_name.to_string());

            // Profil bazlı parametreleri ayarla
            match *profile_name {
                "Conservative" => {
                    config.take_profit_pct = 5.0;
                    config.stop_loss_pct = 1.0;
                    config.max_position_size = base_config.initial_balance * 0.05;
                }
                "Balanced" => {
                    config.take_profit_pct = 8.0;
                    config.stop_loss_pct = 2.0;
                    config.max_position_size = base_config.initial_balance * 0.10;
                }
                "Aggressive" => {
                    config.take_profit_pct = 15.0;
                    config.stop_loss_pct = 3.0;
                    config.max_position_size = base_config.initial_balance * 0.20;
                }
                "Scalper" => {
                    config.take_profit_pct = 2.0;
                    config.stop_loss_pct = 0.5;
                    config.max_position_size = base_config.initial_balance * 0.15;
                }
                "SwingTrading" => {
                    config.take_profit_pct = 20.0;
                    config.stop_loss_pct = 5.0;
                    config.max_position_size = base_config.initial_balance * 0.08;
                }
                _ => {
                    // Bilinmeyen profil, base config'i kullan
                    config.position_profile = Some("Custom".to_string());
                }
            }

            let mut backtester = Backtester::new(config);
            let result = backtester.run(candles)?;

            results.push(ProfilePerformance {
                profile_name: profile_name.to_string(),
                total_trades: result.total_trades,
                win_rate: result.win_rate,
                total_pnl: result.total_pnl,
                total_pnl_pct: result.total_pnl_pct,
                max_drawdown_pct: result.max_drawdown_pct,
                profit_factor: result.profit_factor,
                sharpe_ratio: result.sharpe_ratio,
                avg_win: result.avg_win,
                avg_loss: result.avg_loss,
            });
        }

        // En iyi profili bul (profit_factor bazlı)
        let best_profile = results
            .iter()
            .max_by(|a, b| a.profit_factor.partial_cmp(&b.profit_factor).unwrap())
            .map(|p| p.profile_name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        Ok(ProfileComparisonResult {
            symbol: base_config.symbol.clone(),
            strategy: base_config.strategy_name.clone(),
            period_start: candles.first().unwrap().timestamp.to_rfc3339(),
            period_end: candles.last().unwrap().timestamp.to_rfc3339(),
            profiles: results,
            best_profile,
            best_metric: "profit_factor".to_string(),
        })
    }

    /// Basit EMA hesapla
    fn calculate_ema(prices: &[f64], period: usize) -> f64 {
        if prices.len() < period {
            return prices.iter().sum::<f64>() / prices.len() as f64;
        }

        let multiplier = 2.0 / (period as f64 + 1.0);
        let sma = prices.iter().take(period).sum::<f64>() / period as f64;

        prices
            .iter()
            .skip(period)
            .fold(sma, |ema, &price| {
                ema + multiplier * (price - ema)
            })
    }

    /// Basit SMA hesapla
    fn calculate_sma(prices: &[f64], period: usize) -> f64 {
        if prices.is_empty() {
            return 0.0;
        }
        if prices.len() < period {
            return prices.iter().sum::<f64>() / prices.len() as f64;
        }
        let slice = &prices[prices.len() - period..];
        slice.iter().sum::<f64>() / period as f64
    }

    /// RSI hesapla
    fn calculate_rsi(prices: &[f64], period: usize) -> f64 {
        if prices.len() <= period {
            return 50.0;
        }

        let mut gains = 0.0;
        let mut losses = 0.0;
        let start = prices.len() - period;

        for idx in start..prices.len() {
            if idx == 0 {
                continue;
            }
            let change = prices[idx] - prices[idx - 1];
            if change > 0.0 {
                gains += change;
            } else {
                losses += change.abs();
            }
        }

        if losses == 0.0 {
            return 100.0;
        }

        let rs = gains / losses;
        100.0 - (100.0 / (1.0 + rs))
    }

    /// Bollinger bantlarını hesapla
    fn calculate_bollinger(prices: &[f64], period: usize, std_mult: f64) -> (f64, f64, f64) {
        if prices.is_empty() {
            return (0.0, 0.0, 0.0);
        }

        let window = if prices.len() >= period {
            &prices[prices.len() - period..]
        } else {
            prices
        };

        let mean = window.iter().sum::<f64>() / window.len() as f64;
        let variance = window
            .iter()
            .map(|p| (p - mean).powi(2))
            .sum::<f64>()
            / window.len() as f64;
        let std_dev = variance.sqrt();

        let upper = mean + std_mult * std_dev;
        let lower = mean - std_mult * std_dev;
        (upper, mean, lower)
    }

    /// MACD ve sinyal hesapla (parametreli)
    fn calculate_macd_signal_params(prices: &[f64], fast: usize, slow: usize, signal: usize) -> (f64, f64) {
        if prices.len() < slow {
            return (0.0, 0.0);
        }
        let mut macd_series = Vec::new();
        for idx in slow..=prices.len() {
            let window = &prices[..idx];
            let ema_fast = Self::calculate_ema(window, fast);
            let ema_slow = Self::calculate_ema(window, slow);
            macd_series.push(ema_fast - ema_slow);
        }
        if macd_series.is_empty() {
            return (0.0, 0.0);
        }
        let macd_value = *macd_series.last().unwrap_or(&0.0);
        let signal_value = Self::calculate_ema(&macd_series, signal);
        (macd_value, signal_value)
    }

    /// Strateji adı normalize et
    fn normalize_strategy_name(strategy_name: &str) -> String {
        strategy_name.trim().to_uppercase().replace('-', "_")
    }

    /// Stratejiye göre giriş kararı üret
    fn should_open_position(candles: &[Candle], idx: usize, strategy_name: &str, params: Option<&StrategyParams>) -> bool {
        if candles.is_empty() || idx >= candles.len() {
            return false;
        }

        let closes: Vec<f64> = candles[..=idx].iter().map(|c| c.close).collect();
        let strategy = Self::normalize_strategy_name(strategy_name);

        match strategy.as_str() {
            "RSI" => {
                let period   = params.and_then(|p| p.period).unwrap_or(14);
                let oversold = params.and_then(|p| p.oversold).unwrap_or(30.0);
                if closes.len() < period + 1 {
                    return false;
                }
                let rsi = Self::calculate_rsi(&closes, period);
                rsi < oversold
            }
            "BOLLINGER" | "BOLLINGER_BANDS" => {
                let period  = params.and_then(|p| p.period).unwrap_or(20);
                let std_dev = params.and_then(|p| p.std_dev).unwrap_or(2.0);
                if closes.len() < period {
                    return false;
                }
                let (_, _, lower) = Self::calculate_bollinger(&closes, period, std_dev);
                let last_close = *closes.last().unwrap_or(&0.0);
                last_close <= lower
            }
            "MACD" => {
                let fast   = params.and_then(|p| p.fast_period).unwrap_or(12);
                let slow   = params.and_then(|p| p.slow_period).unwrap_or(26);
                let signal = params.and_then(|p| p.signal_period).unwrap_or(9);
                if closes.len() < slow + signal {
                    return false;
                }
                let prev_closes = &closes[..closes.len() - 1];
                let (prev_macd, prev_sig) = Self::calculate_macd_signal_params(prev_closes, fast, slow, signal);
                let (curr_macd, curr_sig) = Self::calculate_macd_signal_params(&closes, fast, slow, signal);
                prev_macd <= prev_sig && curr_macd > curr_sig
            }
            "EMA" | "EMA_CROSSOVER" => {
                // İki EMA kesişimi: hızlı EMA yavaş EMA'yı aşağıdan yukarıya geçtiğinde giriş
                let fast = params.and_then(|p| p.fast_period).unwrap_or(9);
                let slow = params.and_then(|p| p.slow_period).unwrap_or(21);
                if closes.len() < slow + 1 { return false; }
                let prev = &closes[..closes.len() - 1];
                let fast_prev = Self::calculate_ema(prev, fast);
                let slow_prev = Self::calculate_ema(prev, slow);
                let fast_curr = Self::calculate_ema(&closes, fast);
                let slow_curr = Self::calculate_ema(&closes, slow);
                fast_prev <= slow_prev && fast_curr > slow_curr // golden cross
            }
            "DONCHIAN" | "DONCHIAN_CHANNEL" => {
                // Donchian kanalı breakout: fiyat önceki N mumun en yüksek kapanışını geçerse giriş
                let period = params.and_then(|p| p.period).unwrap_or(20);
                if closes.len() < period + 1 { return false; }
                let window = &closes[closes.len() - period - 1..closes.len() - 1];
                let upper = window.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                *closes.last().unwrap_or(&0.0) > upper
            }
            "WILLIAMS" | "WILLIAMS_R" => {
                // Williams %R < -80 → aşırı satım → alış sinyali
                let period = params.and_then(|p| p.period).unwrap_or(14);
                if closes.len() < period { return false; }
                let window = &closes[closes.len() - period..];
                let highest = window.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let lowest  = window.iter().cloned().fold(f64::INFINITY,     f64::min);
                let last    = *closes.last().unwrap_or(&0.0);
                if (highest - lowest).abs() < 1e-10 { return false; }
                let wr = (highest - last) / (highest - lowest) * -100.0;
                wr < -80.0
            }
            "CCI" => {
                // CCI < -100 → aşırı satım → alış sinyali
                let period = params.and_then(|p| p.period).unwrap_or(20);
                if closes.len() < period { return false; }
                let window = &closes[closes.len() - period..];
                let mean = window.iter().sum::<f64>() / period as f64;
                let mean_dev = window.iter().map(|&c| (c - mean).abs()).sum::<f64>() / period as f64;
                let last = *closes.last().unwrap_or(&0.0);
                if mean_dev < 1e-10 { return false; }
                let cci = (last - mean) / (0.015 * mean_dev);
                cci < -100.0
            }
            "STOCH_RSI" => {
                // Stochastic RSI < 20 → aşırı satım bölgesi → alış sinyali
                let rsi_period   = params.and_then(|p| p.period).unwrap_or(14);
                let stoch_period = 14usize;
                let min_len = rsi_period + stoch_period;
                if closes.len() < min_len { return false; }
                // RSI serisi oluştur
                let rsi_vals: Vec<f64> = (rsi_period + 1..=closes.len())
                    .map(|i| Self::calculate_rsi(&closes[..i], rsi_period))
                    .collect();
                if rsi_vals.len() < stoch_period { return false; }
                let win = &rsi_vals[rsi_vals.len() - stoch_period..];
                let rsi_high = win.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let rsi_low  = win.iter().cloned().fold(f64::INFINITY,     f64::min);
                let last_rsi = *rsi_vals.last().unwrap_or(&50.0);
                if (rsi_high - rsi_low).abs() < 1e-10 { return false; }
                let stoch_rsi = (last_rsi - rsi_low) / (rsi_high - rsi_low) * 100.0;
                stoch_rsi < 20.0
            }
            "SUPERTREND" => {
                // ATR tabanlı Supertrend: trend yönü 1 (yukarı) → alış sinyali
                let period = params.and_then(|p| p.period).unwrap_or(10);
                let mult   = params.and_then(|p| p.std_dev).unwrap_or(3.0);
                if idx < period + 1 { return false; }
                matches!(
                    crate::robot::indicators::calculate_supertrend(&candles[..=idx], period, mult),
                    Some((1, _))
                )
            }
            "PRICE_ACTION" => {
                // Bullish engulfing veya pin bar formasyonu → alış sinyali
                if idx < 2 { return false; }
                let prev = &candles[idx - 1];
                let curr = &candles[idx];
                let prev_body  = (prev.close - prev.open).abs();
                let curr_body  = (curr.close - curr.open).abs();
                let curr_upper = curr.high - curr.close.max(curr.open);
                let curr_lower = curr.close.min(curr.open) - curr.low;
                // Bullish engulfing
                let bull_engulf = prev.close < prev.open
                    && curr.close > curr.open
                    && curr_body > prev_body * 1.1
                    && curr.open <= prev.close
                    && curr.close >= prev.open;
                // Bullish pin bar: uzun alt gölge, küçük üst gölge
                let bull_pin = curr_body > 1e-10
                    && curr_lower >= curr_body * 2.0
                    && curr_upper < curr_body * 0.5;
                bull_engulf || bull_pin
            }
            "ICT_FVG" => {
                // ICT Fair Value Gap: Bullish FVG bölgesinde fiyat → alış sinyali
                let lookback = params.and_then(|p| p.period).unwrap_or(5).max(3);
                if idx < lookback + 1 { return false; }
                let current_price = candles[idx].close;
                let mut found = false;
                for i in 2..lookback.min(idx) {
                    let left  = &candles[idx - i - 1];
                    let right = &candles[idx - i + 1];
                    // Bullish FVG: left candle'ın high'ı < right candle'ın low'u
                    if left.high < right.low {
                        if current_price >= left.high && current_price <= right.low {
                            found = true; break;
                        }
                        let fvg_mid = (left.high + right.low) / 2.0;
                        if fvg_mid > 1e-10
                            && (current_price - fvg_mid).abs() / fvg_mid < 0.01
                            && current_price < fvg_mid
                        {
                            found = true; break;
                        }
                    }
                }
                found
            }
            "SMC" => {
                // Smart Money Concepts — Break of Structure (BOS) yukarı → alış sinyali
                let swing_lb = params.and_then(|p| p.period).unwrap_or(10).max(3);
                if idx < swing_lb * 2 + 2 { return false; }
                let current = &candles[idx];
                let window = &candles[idx - swing_lb..idx];
                let swing_high = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                current.close > swing_high
            }
            "ICT_OB" => {
                // Order Block: bullish market structure içinde son kırmızı muma dönüş
                let swing_lb = params.and_then(|p| p.period).unwrap_or(10).max(5);
                if idx < swing_lb * 2 + 3 { return false; }
                let current = &candles[idx];
                let window      = &candles[idx - swing_lb..idx];
                let prev_window = &candles[idx - swing_lb * 2..idx - swing_lb];
                let swing_high  = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                let prev_high   = prev_window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                let bullish_ms  = swing_high > prev_high;
                if !bullish_ms { return false; }
                // Son kırmızı mumu bul
                candles[idx - swing_lb..idx.saturating_sub(1)]
                    .iter()
                    .rev()
                    .find(|c| c.close < c.open)
                    .map(|ob| {
                        let ob_mid = (ob.high + ob.low) / 2.0;
                        let in_ob  = current.close >= ob.low && current.close <= ob.high;
                        let near_ob = ob_mid > 1e-10
                            && (current.close - ob_mid).abs() / ob_mid < 0.008
                            && current.close < ob_mid;
                        in_ob || near_ob
                    })
                    .unwrap_or(false)
            }
            "ICT_SWEEP" => {
                // Liquidity Sweep: swing low altına geçiş + geri dönüş → bullish
                let lookback = params.and_then(|p| p.period).unwrap_or(20).max(5);
                if idx < lookback + 3 { return false; }
                let current = &candles[idx];
                let prev    = &candles[idx - 1];
                let window  = &candles[idx - 1 - lookback..idx - 1];
                let swing_low = window.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
                // Önceki mum sweep etti, bu mum geri döndü
                prev.low < swing_low
                    && current.close > swing_low
                    && current.close > prev.open
            }
            "ICT_KILLZONE" => {
                // Killzone: Londra (07-10 UTC) veya NY (12-15 UTC) + FVG
                use chrono::Timelike;
                let lookback = params.and_then(|p| p.period).unwrap_or(6).max(3);
                if idx < lookback + 2 { return false; }
                let hour = candles[idx].timestamp.hour();
                let in_kz = (7..10).contains(&hour) || (12..15).contains(&hour);
                if !in_kz { return false; }
                let current_price = candles[idx].close;
                let mut found = false;
                for i in 2..lookback.min(idx) {
                    let left  = &candles[idx - i - 1];
                    let right = &candles[idx - i + 1];
                    if left.high < right.low {
                        if current_price >= left.high && current_price <= right.low { found = true; break; }
                        let fvg_mid = (left.high + right.low) / 2.0;
                        if fvg_mid > 1e-10
                            && (current_price - fvg_mid).abs() / fvg_mid < 0.012
                            && current_price < fvg_mid
                        { found = true; break; }
                    }
                }
                found
            }
            "ICT_OTE" => {
                // OTE: Bullish BOS sonrası %62-79 Fibonacci retrace'e giriş
                let swing_lb = params.and_then(|p| p.period).unwrap_or(15).max(5);
                if idx < swing_lb * 2 + 2 { return false; }
                let price = candles[idx].close;
                let window = &candles[idx - swing_lb..idx];
                let swing_high = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                let swing_low  = window.iter().map(|c| c.low ).fold(f64::INFINITY, f64::min);
                let range      = swing_high - swing_low;
                if range < 1e-10 { return false; }
                let prev_window = &candles[idx - swing_lb * 2..idx - swing_lb];
                let prev_high   = prev_window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                let bullish_bos = swing_high > prev_high;
                if !bullish_bos { return false; }
                let ote_low  = swing_high - range * 0.79;
                let ote_high = swing_high - range * 0.62;
                price >= ote_low && price <= ote_high
            }
            "ICT_COMPOSITE" => {
                // Composite: Market Structure + Premium/Discount + (FVG veya OB)
                let swing_lb = params.and_then(|p| p.period).unwrap_or(20).max(8);
                if idx < swing_lb * 2 + 6 { return false; }
                let current = &candles[idx];
                let window = &candles[idx - swing_lb..idx];
                let swing_high = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                let swing_low  = window.iter().map(|c| c.low ).fold(f64::INFINITY, f64::min);
                let range = swing_high - swing_low;
                if range < 1e-10 { return false; }
                let prev_window = &candles[idx - swing_lb * 2..idx - swing_lb];
                let prev_high   = prev_window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                let prev_low    = prev_window.iter().map(|c| c.low ).fold(f64::INFINITY, f64::min);
                let bullish_ms  = swing_high > prev_high && swing_low >= prev_low;
                if !bullish_ms { return false; }
                let equilibrium = (swing_high + swing_low) / 2.0;
                let in_discount = current.close < equilibrium;
                if !in_discount { return false; }
                // FVG veya OB teyidi
                let fvg_lb = 6usize.min(idx.saturating_sub(1));
                let mut has_bull_fvg = false;
                for i in 2..fvg_lb {
                    if idx < i + 2 { break; }
                    let left  = &candles[idx - i - 1];
                    let right = &candles[idx - i + 1];
                    if left.high < right.low
                        && current.close >= left.high
                        && current.close <= right.low
                    { has_bull_fvg = true; break; }
                }
                let ob_lb = swing_lb.min(idx.saturating_sub(1));
                let ob_hit = candles[idx - ob_lb..idx.saturating_sub(1)]
                    .iter().rev()
                    .find(|c| c.close < c.open)
                    .map(|ob| current.close >= ob.low && current.close <= ob.high)
                    .unwrap_or(false);
                has_bull_fvg || ob_hit
            }
            _ => {
                // Fallback: MA Crossover (EMA5 > EMA10 ve fiyat SMA5 üzerinde)
                if closes.len() < 10 {
                    return false;
                }
                let ema5 = Self::calculate_ema(&closes, 5);
                let ema10 = Self::calculate_ema(&closes, 10);
                let sma5 = Self::calculate_sma(&closes, 5);
                ema5 > ema10 && closes.last().copied().unwrap_or(0.0) >= sma5
            }
        }
    }

    /// Sharpe ratio hesapla
    fn calculate_sharpe_ratio(balance_history: &[(DateTime<Utc>, f64)]) -> f64 {
        if balance_history.len() < 2 {
            return 0.0;
        }

        // Returns hesapla
        let returns: Vec<f64> = balance_history
            .windows(2)
            .map(|w| (w[1].1 - w[0].1) / w[0].1)
            .collect();

        if returns.is_empty() {
            return 0.0;
        }

        let mean_return = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance = returns
            .iter()
            .map(|r| (r - mean_return).powi(2))
            .sum::<f64>()
            / returns.len() as f64;

        let std_dev = variance.sqrt();
        if std_dev == 0.0 {
            0.0
        } else {
            (mean_return / std_dev) * (252.0_f64.sqrt()) // Annualized
        }
    }

    /// Trade süresi hesapla (dakika)
    fn calculate_duration(entry_candle: &Candle, exit_candle: &Candle) -> i64 {
        let duration = exit_candle.timestamp - entry_candle.timestamp;
        duration.num_minutes()
    }

    /// 14-periyot ATR değerini close bazında % olarak hesapla.
    /// Yetersiz veri varsa 1.0 döner (sıfır bölme koruması).
    fn calc_atr_pct(candles: &[Candle]) -> f64 {
        const N: usize = 14;
        if candles.len() < 2 { return 1.0; }
        let window = &candles[candles.len().saturating_sub(N)..];
        let atr: f64 = window.windows(2).map(|w| {
            let tr = (w[1].high - w[1].low)
                .max((w[1].high - w[0].close).abs())
                .max((w[1].low  - w[0].close).abs());
            tr
        }).sum::<f64>() / (window.len().max(1) as f64);
        let last_close = candles.last().map(|c| c.close).unwrap_or(1.0);
        if last_close > 0.0 { atr / last_close * 100.0 } else { 1.0 }
    }

    /// Sonuçları al
    pub fn get_results(&self) -> &[SimulatedTrade] {
        &self.trades
    }

    /// B1/B2/B3 parametrelerini grid search ile optimize et.
    ///
    /// 5×5×4 = 100 kombinasyon çalıştırır; kompozit skor:
    /// `pnl*0.35 + win_rate*0.30 + profit_factor.min(10)*0.20 - max_drawdown*0.15`
    ///
    /// Minimum 3 trade gerektiren bir kombinasyon bulunamazsa `None` döner.
    pub fn optimize_position_management(
        base_config: &BacktestConfig,
        candles:     &[Candle],
    ) -> Option<PosOptResult> {
        const BREAKEVEN_GRID: &[Option<f64>] = &[None, Some(0.3), Some(0.5), Some(0.7), Some(1.0)];
        const ATR_TRAIL_GRID: &[Option<f64>] = &[None, Some(1.0), Some(1.5), Some(2.0), Some(3.0)];
        const PARTIAL_TP_GRID: &[Option<f64>] = &[None, Some(0.3), Some(0.5), Some(0.7)];

        let mut best: Option<PosOptResult> = None;

        for &be in BREAKEVEN_GRID {
            for &at in ATR_TRAIL_GRID {
                for &pt in PARTIAL_TP_GRID {
                    let mut cfg           = base_config.clone();
                    cfg.breakeven_at_rr   = be;
                    cfg.atr_trail_mult    = at;
                    cfg.partial_tp_ratio  = pt;

                    let mut bt = Backtester::new(cfg);
                    let Ok(result) = bt.run(candles) else { continue };
                    if result.total_trades < 3 { continue; }

                    let score = result.total_pnl_pct * 0.35
                        + result.win_rate           * 0.30
                        + result.profit_factor.min(10.0) * 0.20
                        - result.max_drawdown_pct   * 0.15;

                    let candidate = PosOptResult {
                        breakeven_at_rr:  be,
                        atr_trail_mult:   at,
                        partial_tp_ratio: pt,
                        score,
                        win_rate:         result.win_rate,
                        total_pnl_pct:    result.total_pnl_pct,
                        profit_factor:    result.profit_factor,
                        total_trades:     result.total_trades,
                    };

                    if best.as_ref().map_or(true, |b| score > b.score) {
                        best = Some(candidate);
                    }
                }
            }
        }

        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_candles() -> Vec<Candle> {
        vec![
            Candle {
                symbol: "BTC".to_string(),
                interval: "1h".to_string(),
                timestamp: Utc::now(),
                open: 100.0,
                high: 102.0,
                low: 99.0,
                close: 101.0,
                volume: 1000.0,
            },
            Candle {
                symbol: "BTC".to_string(),
                interval: "1h".to_string(),
                timestamp: Utc::now() + chrono::Duration::hours(1),
                open: 101.0,
                high: 103.0,
                low: 100.0,
                close: 102.0,
                volume: 1100.0,
            },
            Candle {
                symbol: "BTC".to_string(),
                interval: "1h".to_string(),
                timestamp: Utc::now() + chrono::Duration::hours(2),
                open: 102.0,
                high: 110.0,
                low: 102.0,
                close: 109.0,
                volume: 1200.0,
            },
        ]
    }

    #[test]
    fn test_backtester_creation() {
        let config = BacktestConfig {
            symbol: "BTC".to_string(),
            interval: "1h".to_string(),
            initial_balance: 1000.0,
            max_position_size: 1.0,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            strategy_name: "MA_Crossover".to_string(),
            position_profile: None,
            security_profile: None,
            commission_pct: 0.001,
            strategy_params: None,
            breakeven_at_rr: None,
            atr_trail_mult: None,
            partial_tp_ratio: None,
        };

        let backtester = Backtester::new(config);
        assert_eq!(backtester.trades.len(), 0);
    }

    #[test]
    fn test_backtester_run() {
        let config = BacktestConfig {
            symbol: "BTC".to_string(),
            interval: "1h".to_string(),
            initial_balance: 1000.0,
            max_position_size: 1.0,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            strategy_name: "MA_Crossover".to_string(),
            position_profile: None,
            security_profile: None,
            commission_pct: 0.001,
            strategy_params: None,
            breakeven_at_rr: None,
            atr_trail_mult: None,
            partial_tp_ratio: None,
        };

        let mut backtester = Backtester::new(config);
        let candles = create_test_candles();
        let result = backtester.run(&candles);

        assert!(result.is_ok());
    }

    #[test]
    fn test_backtester_empty_candles() {
        let config = BacktestConfig {
            symbol: "BTC".to_string(),
            interval: "1h".to_string(),
            initial_balance: 1000.0,
            max_position_size: 1.0,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            strategy_name: "MA_Crossover".to_string(),
            position_profile: None,
            security_profile: None,
            commission_pct: 0.001,
            strategy_params: None,
            breakeven_at_rr: None,
            atr_trail_mult: None,
            partial_tp_ratio: None,
        };

        let mut backtester = Backtester::new(config);
        let result = backtester.run(&[]);

        assert!(result.is_err());
    }

    #[test]
    fn test_ema_calculation() {
        let prices = vec![100.0, 102.0, 104.0, 106.0, 108.0];
        let ema = Backtester::calculate_ema(&prices, 3);
        assert!(ema > 0.0);
    }

    #[test]
    fn test_simulated_trade_creation() {
        let trade = SimulatedTrade {
            symbol: "BTC".to_string(),
            entry_price: 100.0,
            exit_price: 110.0,
            entry_time: Utc::now().to_rfc3339(),
            exit_time: Utc::now().to_rfc3339(),
            amount: 1.0,
            pnl: 10.0,
            pnl_pct: 10.0,
            duration_minutes: 60,
        };

        assert_eq!(trade.symbol, "BTC");
        assert_eq!(trade.pnl, 10.0);
    }

    #[test]
    fn test_backtest_result_serialization() {
        let result = BacktestResult {
            symbol: "BTC".to_string(),
            strategy: "MA_Crossover".to_string(),
            period_start: Utc::now().to_rfc3339(),
            period_end: Utc::now().to_rfc3339(),
            total_trades: 10,
            winning_trades: 7,
            losing_trades: 3,
            win_rate: 70.0,
            total_pnl: 500.0,
            total_pnl_pct: 50.0,
            max_drawdown_pct: 5.0,
            avg_win: 100.0,
            avg_loss: 50.0,
            profit_factor: 4.67,
            sharpe_ratio: 1.5,
            trades: vec![],
            position_profile: Some("Balanced".to_string()),
            security_profile: Some("Production".to_string()),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"symbol\":\"BTC\""));
        assert!(json.contains("\"position_profile\""));
    }

    #[test]
    fn test_profile_comparison() {
        use chrono::Utc;

        // Test verileri oluştur
        let candles = vec![
            Candle {
                symbol: "BTCUSDT".to_string(),
                timestamp: Utc::now(),
                open: 100.0,
                high: 110.0,
                low: 95.0,
                close: 105.0,
                volume: 1000.0,
                interval: "1h".to_string(),
            },
            Candle {
                symbol: "BTCUSDT".to_string(),
                timestamp: Utc::now() + chrono::Duration::hours(1),
                open: 105.0,
                high: 115.0,
                low: 100.0,
                close: 110.0,
                volume: 1200.0,
                interval: "1h".to_string(),
            },
            Candle {
                symbol: "BTCUSDT".to_string(),
                timestamp: Utc::now() + chrono::Duration::hours(2),
                open: 110.0,
                high: 120.0,
                low: 105.0,
                close: 115.0,
                volume: 1500.0,
                interval: "1h".to_string(),
            },
        ];

        let base_config = BacktestConfig {
            symbol: "BTCUSDT".to_string(),
            interval: "1h".to_string(),
            initial_balance: 10000.0,
            max_position_size: 1.0,
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            strategy_name: "MA_Crossover".to_string(),
            position_profile: None,
            security_profile: None,
            commission_pct: 0.001,
            strategy_params: None,
            breakeven_at_rr: None,
            atr_trail_mult: None,
            partial_tp_ratio: None,
        };

        let profiles = vec!["Conservative", "Balanced", "Aggressive"];
        let result = Backtester::compare_profiles(base_config, &candles, &profiles);

        assert!(result.is_ok());
        let comparison = result.unwrap();
        assert_eq!(comparison.profiles.len(), 3);
        assert!(!comparison.best_profile.is_empty());
        assert_eq!(comparison.best_metric, "profit_factor");
    }

    // ── Yeni strateji testleri ─────────────────────────────────────────────

    /// PRICE_ACTION: Bullish engulfing formasyon tespiti
    /// Senaryo: önceki mum kırmızı (close<open), mevcut mum büyük yeşil (body > prev*1.1)
    #[test]
    fn test_price_action_bullish_engulf() {
        let base = Utc::now();
        // 30 adet nötr mum (min buffer) + formasyon
        let mut candles: Vec<Candle> = (0..30).map(|i| Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(i),
            open: 100.0, high: 101.0, low: 99.0, close: 100.5, volume: 500.0,
        }).collect();
        // idx=30: önceki mum — kırmızı (close < open)
        candles.push(Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(30),
            open: 105.0, high: 106.0, low: 99.0, close: 100.0, volume: 800.0,
        });
        // idx=31: bullish engulfing — yeşil, gövde çok daha büyük
        candles.push(Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(31),
            open: 99.5, high: 108.0, low: 99.0, close: 107.0, volume: 1500.0,
        });
        let signal = Backtester::should_open_position(&candles, 31, "PRICE_ACTION", None);
        assert!(signal, "Bullish engulfing → sinyal bekleniyor");
    }

    /// PRICE_ACTION: Düz mumlar → sinyal olmamalı
    #[test]
    fn test_price_action_no_signal() {
        let base = Utc::now();
        let candles: Vec<Candle> = (0..20).map(|i| Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(i),
            open: 100.0, high: 100.5, low: 99.5, close: 100.2, volume: 500.0,
        }).collect();
        let signal = Backtester::should_open_position(&candles, 19, "PRICE_ACTION", None);
        assert!(!signal, "Düz mumlar → sinyal beklenmez");
    }

    /// SMC: Fiyat önceki swing high'ı kırdığında BOS sinyali
    /// Senaryo: 25 mum boyunca swing high=103.0, son mumun close=104.0 → BOS yukarı
    #[test]
    fn test_smc_bos_buy() {
        let base = Utc::now();
        // 25 nötr mum (swing penceresi için yeterli)
        let mut candles: Vec<Candle> = (0..25).map(|i| Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(i),
            open: 100.0, high: 103.0, low: 97.0, close: 101.0, volume: 500.0,
        }).collect();
        // Son mum swing high'ı (103.0) yukarı kırar
        candles.push(Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(25),
            open: 101.0, high: 105.0, low: 100.0, close: 104.0, volume: 1200.0,
        });
        let idx = candles.len() - 1;
        let signal = Backtester::should_open_position(&candles, idx, "SMC", None);
        assert!(signal, "BOS yukarı kırılım → sinyal bekleniyor");
    }

    /// SMC: Fiyat swing high altında → sinyal olmamalı
    #[test]
    fn test_smc_no_signal() {
        let base = Utc::now();
        let candles: Vec<Candle> = (0..26).map(|i| Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(i),
            open: 100.0, high: 103.0, low: 97.0, close: 101.0, volume: 500.0,
        }).collect();
        let idx = candles.len() - 1;
        let signal = Backtester::should_open_position(&candles, idx, "SMC", None);
        assert!(!signal, "Close < swing_high → sinyal beklenmez");
    }

    /// ICT_FVG: Bullish FVG bölgesinde fiyat → sinyal
    /// Senaryo: left.high=100, right.low=104 → boşluk var; current_price=102 bölge içinde
    #[test]
    fn test_ict_fvg_bullish() {
        let base = Utc::now();
        // Buffer mumlar
        let mut candles: Vec<Candle> = (0..10).map(|i| Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(i),
            open: 98.0, high: 100.0, low: 96.0, close: 99.0, volume: 500.0,
        }).collect();
        // FVG triad: left → mid → right
        // left: high=100
        candles.push(Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(10),
            open: 98.0, high: 100.0, low: 96.0, close: 99.0, volume: 500.0,
        });
        // mid: hızlı yukarı mum (boşluğu oluşturur)
        candles.push(Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(11),
            open: 101.0, high: 108.0, low: 100.5, close: 107.0, volume: 2000.0,
        });
        // right: low=104 (left.high=100 < right.low=104 → bullish FVG)
        candles.push(Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(12),
            open: 106.0, high: 109.0, low: 104.0, close: 108.0, volume: 1500.0,
        });
        // Son mum: fiyat FVG bölgesine geri döndü (close=102, 100..104 içinde)
        candles.push(Candle {
            symbol: "TEST".to_string(), interval: "1h".to_string(),
            timestamp: base + chrono::Duration::hours(13),
            open: 105.0, high: 106.0, low: 101.0, close: 102.0, volume: 800.0,
        });
        let idx = candles.len() - 1;
        let signal = Backtester::should_open_position(&candles, idx, "ICT_FVG", None);
        assert!(signal, "Fiyat Bullish FVG içinde → sinyal bekleniyor");
    }

    /// SUPERTREND: Düz yükselen seride uptrend → sinyal üretmeli
    #[test]
    fn test_supertrend_uptrend() {
        let base = Utc::now();
        // 30 mum boyunca fiyat düzenli artıyor → Supertrend uptrend'de olmalı
        let candles: Vec<Candle> = (0..30).map(|i| {
            let p = 100.0 + i as f64 * 0.5;
            Candle {
                symbol: "TEST".to_string(), interval: "1h".to_string(),
                timestamp: base + chrono::Duration::hours(i),
                open: p - 0.2, high: p + 0.5, low: p - 0.5, close: p, volume: 1000.0,
            }
        }).collect();
        let idx = candles.len() - 1;
        let signal = Backtester::should_open_position(&candles, idx, "SUPERTREND", None);
        // Güçlü uptrend'de Supertrend = 1 (buy) beklenir
        assert!(signal, "Güçlü uptrend → Supertrend buy sinyali bekleniyor");
    }

    /// SUPERTREND: Sert düşüş serisinde downtrend → sinyal olmamalı
    #[test]
    fn test_supertrend_downtrend_no_signal() {
        let base = Utc::now();
        let candles: Vec<Candle> = (0..30).map(|i| {
            let p = 200.0 - i as f64 * 2.0; // sert düşüş
            Candle {
                symbol: "TEST".to_string(), interval: "1h".to_string(),
                timestamp: base + chrono::Duration::hours(i),
                open: p + 0.3, high: p + 1.0, low: p - 1.5, close: p, volume: 1000.0,
            }
        }).collect();
        let idx = candles.len() - 1;
        let signal = Backtester::should_open_position(&candles, idx, "SUPERTREND", None);
        assert!(!signal, "Sert düşüş → Supertrend buy sinyali beklenmez");
    }
}
