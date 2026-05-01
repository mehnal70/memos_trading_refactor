// robot/risk_guardrails.rs - Otonom trading için kritik risk guardrail'leri
// Max daily drawdown, spread/likidite, slipaj monitoring

use chrono::{DateTime, Duration, Utc};

/// Günlük drawdown takibi ve fail-safe
#[derive(Debug, Clone)]
pub struct DrawdownMonitor {
    pub initial_equity: f64,
    pub peak_equity: f64,
    pub current_equity: f64,
    pub max_drawdown_pct: f64,      // Limit: örn. 10%
    pub session_start: DateTime<Utc>,
}

impl DrawdownMonitor {
    /// Yeni monitor - başlangıç equity'si ile
    pub fn new(initial_equity: f64, max_drawdown_pct: f64) -> Self {
        Self {
            initial_equity,
            peak_equity: initial_equity,
            current_equity: initial_equity,
            max_drawdown_pct,
            session_start: Utc::now(),
        }
    }

    /// Equity update - drawdown kontrolü
    pub fn update_equity(&mut self, new_equity: f64) -> DrawdownStatus {
        self.current_equity = new_equity;

        // Peak güncelle
        if new_equity > self.peak_equity {
            self.peak_equity = new_equity;
        }

        // Current drawdown % hesapla
        let current_dd = ((self.peak_equity - new_equity) / self.peak_equity) * 100.0;

        if current_dd >= self.max_drawdown_pct {
            DrawdownStatus::LimitExceeded {
                current_dd,
                limit: self.max_drawdown_pct,
            }
        } else {
            DrawdownStatus::Safe {
                current_dd,
                equity: new_equity,
            }
        }
    }

    /// Session süresi
    pub fn session_duration(&self) -> Duration {
        Utc::now() - self.session_start
    }

    /// Yeni gün başlatması
    pub fn reset_daily(&mut self, new_equity: f64) {
        self.initial_equity = new_equity;
        self.peak_equity = new_equity;
        self.current_equity = new_equity;
        self.session_start = Utc::now();
    }
}

#[derive(Debug, Clone)]
pub enum DrawdownStatus {
    Safe { current_dd: f64, equity: f64 },
    LimitExceeded { current_dd: f64, limit: f64 },
}

/// Order Book depth ve spread kontrolü
#[derive(Debug, Clone)]
pub struct LiquidityMonitor {
    pub max_bid_ask_spread_pct: f64,  // Örn. 0.1% (10 bps)
    pub min_order_book_depth_usd: f64, // Örn. $100k depth gerekli
}

impl LiquidityMonitor {
    /// Yeni likidite monitor
    pub fn new(max_spread_pct: f64, min_depth_usd: f64) -> Self {
        Self {
            max_bid_ask_spread_pct: max_spread_pct,
            min_order_book_depth_usd: min_depth_usd,
        }
    }

    /// Spread ve depth kontrol - satın almak güvenli mi?
    pub fn can_buy(
        &self,
        bid_price: f64,
        ask_price: f64,
        ask_quantity: f64,
    ) -> LiquidityStatus {
        // Spread % hesapla
        let spread_pct = ((ask_price - bid_price) / mid_price(bid_price, ask_price)) * 100.0;

        if spread_pct > self.max_bid_ask_spread_pct {
            return LiquidityStatus::SpreadTooHigh {
                spread_pct,
                limit: self.max_bid_ask_spread_pct,
            };
        }

        // Depth kontrol (ask_quantity * ask_price >= min depth)
        let ask_depth_usd = ask_quantity * ask_price;
        if ask_depth_usd < self.min_order_book_depth_usd {
            return LiquidityStatus::DepthInsufficient {
                available: ask_depth_usd,
                required: self.min_order_book_depth_usd,
            };
        }

        LiquidityStatus::Safe {
            spread_pct,
            depth_usd: ask_depth_usd,
        }
    }

    /// Spread ve depth kontrol - satmak güvenli mi?
    pub fn can_sell(
        &self,
        bid_price: f64,
        ask_price: f64,
        bid_quantity: f64,
    ) -> LiquidityStatus {
        let spread_pct = ((ask_price - bid_price) / mid_price(bid_price, ask_price)) * 100.0;

        if spread_pct > self.max_bid_ask_spread_pct {
            return LiquidityStatus::SpreadTooHigh {
                spread_pct,
                limit: self.max_bid_ask_spread_pct,
            };
        }

        let bid_depth_usd = bid_quantity * bid_price;
        if bid_depth_usd < self.min_order_book_depth_usd {
            return LiquidityStatus::DepthInsufficient {
                available: bid_depth_usd,
                required: self.min_order_book_depth_usd,
            };
        }

        LiquidityStatus::Safe {
            spread_pct,
            depth_usd: bid_depth_usd,
        }
    }
}

#[derive(Debug, Clone)]
pub enum LiquidityStatus {
    Safe { spread_pct: f64, depth_usd: f64 },
    SpreadTooHigh { spread_pct: f64, limit: f64 },
    DepthInsufficient { available: f64, required: f64 },
}

/// Slipaj detectoru - execution vs mid-price sapma
#[derive(Debug, Clone)]
pub struct SlippageDetector {
    pub max_slippage_pct: f64,         // Örn. 0.5%
    pub cooldown_seconds: i64,         // Slipaj sonrası bekleme
    pub last_slippage_time: Option<DateTime<Utc>>,
}

impl SlippageDetector {
    /// Yeni slippage detector
    pub fn new(max_slippage_pct: f64, cooldown_seconds: i64) -> Self {
        Self {
            max_slippage_pct,
            cooldown_seconds,
            last_slippage_time: None,
        }
    }

    /// Slippage hesapla ve kontrol
    pub fn check_slippage(
        &mut self,
        execution_price: f64,
        mid_price: f64,
    ) -> SlippageStatus {
        let slippage_pct = ((execution_price - mid_price).abs() / mid_price) * 100.0;

        if slippage_pct > self.max_slippage_pct {
            self.last_slippage_time = Some(Utc::now());
            return SlippageStatus::ExcessiveSlippage {
                slippage_pct,
                limit: self.max_slippage_pct,
            };
        }

        SlippageStatus::Acceptable { slippage_pct }
    }

    /// Cooldown aktif mi? (slippage sonrası bekleme)
    pub fn is_in_cooldown(&self) -> bool {
        if let Some(last_time) = self.last_slippage_time {
            let elapsed = (Utc::now() - last_time).num_seconds();
            elapsed < self.cooldown_seconds
        } else {
            false
        }
    }

    /// Cooldown'un kalan süresi
    pub fn cooldown_remaining_secs(&self) -> i64 {
        if let Some(last_time) = self.last_slippage_time {
            let elapsed = (Utc::now() - last_time).num_seconds();
            (self.cooldown_seconds - elapsed).max(0)
        } else {
            0
        }
    }
}

#[derive(Debug, Clone)]
pub enum SlippageStatus {
    Acceptable { slippage_pct: f64 },
    ExcessiveSlippage { slippage_pct: f64, limit: f64 },
}

/// Yardımcı: mid-price hesapla
fn mid_price(bid: f64, ask: f64) -> f64 {
    (bid + ask) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drawdown_monitor_safe() {
        let mut monitor = DrawdownMonitor::new(10000.0, 10.0);
        let status = monitor.update_equity(9500.0); // 5% DD
        if let DrawdownStatus::Safe { current_dd, .. } = status {
            assert!(current_dd < 10.0);
        } else {
            panic!("Should be safe");
        }
    }

    #[test]
    fn test_drawdown_monitor_limit_exceeded() {
        let mut monitor = DrawdownMonitor::new(10000.0, 10.0);
        monitor.peak_equity = 10000.0;
        let status = monitor.update_equity(8900.0); // 11% DD
        if let DrawdownStatus::LimitExceeded { current_dd, .. } = status {
            assert!(current_dd > 10.0);
        } else {
            panic!("Should exceed limit");
        }
    }

    #[test]
    fn test_liquidity_monitor_safe() {
        let liquidity = LiquidityMonitor::new(0.1, 10000.0);
        let status = liquidity.can_buy(100.0, 100.05, 200.0); // Spread: 0.05%, Depth: $20k
        if let LiquidityStatus::Safe { .. } = status {
            // OK
        } else {
            panic!("Should be safe");
        }
    }

    #[test]
    fn test_liquidity_monitor_spread_high() {
        let liquidity = LiquidityMonitor::new(0.1, 10000.0);
        let status = liquidity.can_buy(100.0, 101.0, 200.0); // Spread: 1%
        if let LiquidityStatus::SpreadTooHigh { .. } = status {
            // OK
        } else {
            panic!("Should detect high spread");
        }
    }

    #[test]
    fn test_slippage_detector_acceptable() {
        let mut detector = SlippageDetector::new(0.5, 60);
        let status = detector.check_slippage(100.2, 100.0); // 0.2% slippage
        if let SlippageStatus::Acceptable { .. } = status {
            // OK
        } else {
            panic!("Should be acceptable");
        }
    }

    #[test]
    fn test_slippage_detector_excessive() {
        let mut detector = SlippageDetector::new(0.5, 60);
        let status = detector.check_slippage(100.8, 100.0); // 0.8% slippage
        if let SlippageStatus::ExcessiveSlippage { .. } = status {
            // OK
            assert!(detector.is_in_cooldown());
        } else {
            panic!("Should detect excessive slippage");
        }
    }
}
