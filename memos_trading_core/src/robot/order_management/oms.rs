// robot/order_management/oms.rs - Akıllı Emir Yönetim Sistemi (OMS)

use crate::core::types::{Signal, Trade, Market};
use crate::robot::engines::executor::RoboticTradeExecutor; // Artık mod.rs üzerinden bulabilir
use crate::robot::order_management::paper_executor::ExecutionCostConfig;
use std::time::Duration;

pub struct OrderManagementSystem;

impl OrderManagementSystem {
    /// Giriş emirlerini "Best_Bid + 1 tick" seviyesine yerleştirerek maker komisyon avantajı sağlar.
    /// Re-quoting mantığı ve Spread Guard burada mühürlenmiştir.
    pub async fn smart_limit_entry(
        executor_wrapper: &RoboticTradeExecutor<'_>,
        signal: Signal,
        symbol: &str,
        qty: f64,
        base_price: f64,
        max_spread_bps: Option<f64>,
    ) -> Vec<crate::Result<Trade>> {
        const MAX_ATTEMPTS: u32 = 3;
        const PER_ATTEMPT_TIMEOUT_MS: u64 = 2000;
        const BIP: f64 = 0.0001; 

        let is_long = matches!(signal, Signal::Buy);
        let current_price = base_price;

        for attempt in 1..=MAX_ATTEMPTS {
            let (best_bid, best_ask) = executor_wrapper.executor
                .fetch_book_ticker(symbol)
                .unwrap_or((0.0, 0.0));

            let (bid, ask) = if best_bid > 0.0 && best_ask > 0.0 && best_ask > best_bid {
                (best_bid, best_ask)
            } else {
                (current_price * (1.0 - 0.0002), current_price * (1.0 + 0.0002))
            };

            // Spread Guard
            if let Some(max_bps) = max_spread_bps {
                let mid = (bid + ask) / 2.0;
                if mid > 0.0 {
                    let spread_bps = (ask - bid) / mid * 10_000.0;
                    if spread_bps > max_bps {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                }
            }

            let tick = bid.max(ask) * BIP;
            let limit_price = if is_long { bid + tick } else { ask - tick };

            let result = executor_wrapper.executor.execute_limit(
                signal.clone(), symbol, qty, limit_price, PER_ATTEMPT_TIMEOUT_MS,
            );

            match result {
                Ok(trade) => return vec![Ok(trade)],
                Err(_e) => {
                    if attempt >= MAX_ATTEMPTS {
                        return vec![Err(format!("SmartLimit {} deneme başarısız [{}]", MAX_ATTEMPTS, symbol).into())];
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        vec![Err("OMS: Logic Error".into())]
    }

    /// Giriş veya çıkış fiyatını spread + slippage + market impact ile otonom ayarlar.
    pub fn adjust_price_for_costs(base: f64, qty: f64, is_buy: bool, ec: &ExecutionCostConfig) -> f64 {
        let notional = base * qty;
        let adj = ec.market_impact_pct(notional) / 100.0
                + ec.spread_pct / 200.0 
                + ec.slippage_pct / 100.0;
        if is_buy { base * (1.0 + adj) } else { base * (1.0 - adj) }
    }
}
