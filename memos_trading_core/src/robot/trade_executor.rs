// robot/trade_executor.rs - Trade yürütme impl'leri

use crate::robot::interfaces::TradeExecutor;
use crate::types::{Signal, Trade};
use crate::Result;
use chrono::Utc;
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::{Builder, Handle};
#[cfg(not(target_arch = "wasm32"))]
use tokio::task;

#[cfg(not(target_arch = "wasm32"))]
use crate::robot::binance_executor::BinanceFuturesExecutor;

/// Dummy executor - test/demo
pub struct DummyTradeExecutor;

impl TradeExecutor for DummyTradeExecutor {
    fn execute(&self, signal: Signal, symbol: &str, amount: f64) -> Result<Trade> {
        Ok(Trade {
            id: Some(1),
            symbol: symbol.to_string(),
            entry_price: 100.0,
            exit_price: None,
            amount,
            entry_time: Utc::now(),
            exit_time: None,
            pnl: None,
            pnl_pct: None,
            strategy: format!("Dummy-{:?}", signal),
        })
    }

    fn cancel_all(&self, symbol: &str) -> Result<()> {
        println!("[CANCEL] All orders for {} cancelled (DUMMY).", symbol);
        Ok(())
    }
}

/// Binance Spot + Futures trade executor - paper/live, market-aware
#[cfg(not(target_arch = "wasm32"))]
pub struct BinanceTradeExecutor {
    inner: BinanceFuturesExecutor,
}

#[cfg(not(target_arch = "wasm32"))]
impl BinanceTradeExecutor {
    /// Yeni Binance executor (geriye dönük uyumluluk — futures varsayılan)
    pub fn new(api_key: String, api_secret: String, is_paper: bool) -> Self {
        Self::new_for_market(api_key, api_secret, is_paper, "futures")
    }

    /// Market-aware constructor: market = "spot" | "futures"
    /// Spot için ayrı API key kullanmak isterseniz:
    ///   BINANCE_SPOT_API_KEY / BINANCE_SPOT_API_SECRET set edin.
    ///   Yoksa BINANCE_API_KEY / BINANCE_API_SECRET her iki market'te kullanılır.
    pub fn new_for_market(api_key: String, api_secret: String, is_paper: bool, market: &str) -> Self {
        // Market'e özgü override key varsa kullan
        let (key, secret) = if market == "spot" {
            let sk = std::env::var("BINANCE_SPOT_API_KEY").unwrap_or(api_key.clone());
            let ss = std::env::var("BINANCE_SPOT_API_SECRET").unwrap_or(api_secret.clone());
            (sk, ss)
        } else if market == "futures" {
            let fk = std::env::var("BINANCE_FUTURES_API_KEY").unwrap_or(api_key.clone());
            let fs = std::env::var("BINANCE_FUTURES_API_SECRET").unwrap_or(api_secret.clone());
            (fk, fs)
        } else {
            (api_key, api_secret)
        };
        Self {
            inner: BinanceFuturesExecutor::new_for_market(key, secret, is_paper, market),
        }
    }

    /// Paper mod mu?
    pub fn is_paper(&self) -> bool {
        self.inner.is_paper
    }

    /// Spot mu?
    pub fn is_spot(&self) -> bool {
        self.inner.is_spot
    }

    fn run_async<F, T>(&self, fut: F) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        if let Ok(handle) = Handle::try_current() {
            task::block_in_place(|| handle.block_on(fut))
        } else {
            let runtime = Builder::new_current_thread().enable_all().build()?;
            runtime.block_on(fut)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl TradeExecutor for BinanceTradeExecutor {
    fn execute(&self, signal: Signal, symbol: &str, amount: f64) -> Result<Trade> {
        let side = match signal {
            Signal::Buy => "BUY",
            Signal::Sell => "SELL",
            Signal::Hold => return Err("No trade for HOLD signal".into()),
        };

        // TUI raw mode'da println! ekranı bozar; log_msg string'i hata ayıklama için
        // saklanır ama doğrudan terminale yazılmaz.
        // Emir bilgisi SharedLogger aracılığıyla AppState.log'a akar.
        let _log_msg = self.inner.log_order(symbol, side, amount, 0.0);

        if self.inner.is_paper {
            // Paper: dummy trade döndür
            Ok(Trade {
                id: Some(1),
                symbol: symbol.to_string(),
                entry_price: 0.0,
                exit_price: None,
                amount,
                entry_time: Utc::now(),
                exit_time: None,
                pnl: None,
                pnl_pct: None,
                strategy: format!("BinanceTestnet-{:?}", signal),
            })
        } else {
            let order_response = self.run_async(self.inner.place_market_order(symbol, side, amount))?;
            let order_id = order_response
                .get("orderId")
                .and_then(|v| v.as_u64());
            let entry_price = order_response
                .get("avgPrice")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);

            Ok(Trade {
                id: order_id,
                symbol: symbol.to_string(),
                entry_price,
                exit_price: None,
                amount,
                entry_time: Utc::now(),
                exit_time: None,
                pnl: None,
                pnl_pct: None,
                strategy: format!("BinanceLive-{:?}", signal),
            })
        }
    }

    fn cancel_all(&self, symbol: &str) -> Result<()> {
        if self.inner.is_paper {
            Ok(())
        } else {
            self.run_async(self.inner.cancel_all_orders(symbol))?;
            Ok(())
        }
    }

    fn fetch_open_symbols(&self) -> Vec<String> {
        if self.inner.is_paper {
            return vec![];
        }
        match self.run_async(self.inner.get_positions("")) {
            Ok(positions) => positions
                .iter()
                .filter_map(|p| {
                    let qty = p
                        .get("positionAmt")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    if qty.abs() > 1e-9 {
                        p.get("symbol")
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    /// POST_ONLY (GTX/LIMIT_MAKER) limit emir.
    /// Paper modda anında başarı döner. Live modda:
    ///   1. POST_ONLY limit emir gönderir.
    ///   2. EXPIRED/REJECTED → taker olurdu, Err döner.
    ///   3. 500ms'de bir fill durumu sorgulanır, `timeout_ms` içinde dolmazsa emir iptal edilir.
    fn execute_limit(&self, signal: Signal, symbol: &str, amount: f64, price: f64, timeout_ms: u64) -> Result<Trade> {
        let side = match signal {
            Signal::Buy  => "BUY",
            Signal::Sell => "SELL",
            Signal::Hold => return Err("execute_limit: HOLD sinyali için emir yok".into()),
        };
        let _log = self.inner.log_order(symbol, side, amount, price);

        if self.inner.is_paper {
            return Ok(Trade {
                id: Some(1),
                symbol: symbol.to_string(),
                entry_price: price,
                exit_price: None,
                amount,
                entry_time: Utc::now(),
                exit_time: None,
                pnl: None,
                pnl_pct: None,
                strategy: format!("PaperLimitMaker-{:?}", signal),
            });
        }

        // 1. POST_ONLY emir gönder
        let resp = self.run_async(self.inner.place_post_only_limit_order(symbol, side, amount, price))?;

        let order_id = match resp.get("orderId").and_then(|v| v.as_u64()) {
            Some(id) => id,
            None => {
                let st = resp.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                return Err(format!("PostOnly emir kabul edilmedi — status={st} [{symbol}] @ {price:.4}").into());
            }
        };

        // GTX hemen EXPIRED → would-be taker
        match resp.get("status").and_then(|v| v.as_str()).unwrap_or("") {
            "EXPIRED" | "REJECTED" => {
                return Err(format!("GTX emir anında iptal (taker olurdu) [{symbol}] @ {price:.4}").into());
            }
            "FILLED" => {
                let fill_price = resp.get("avgPrice")
                    .and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(price);
                return Ok(Trade {
                    id: Some(order_id), symbol: symbol.to_string(), entry_price: fill_price,
                    exit_price: None, amount, entry_time: Utc::now(), exit_time: None,
                    pnl: None, pnl_pct: None, strategy: format!("BinanceLimitMaker-{:?}", signal),
                });
            }
            _ => {}
        }

        // 2. Fill polling (500ms aralıklı)
        let poll = std::time::Duration::from_millis(500);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            std::thread::sleep(poll);
            let st_resp = self.run_async(self.inner.get_order_status(symbol, order_id))?;
            match st_resp.get("status").and_then(|v| v.as_str()).unwrap_or("UNKNOWN") {
                "FILLED" => {
                    let fill_price = st_resp.get("avgPrice")
                        .and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(price);
                    return Ok(Trade {
                        id: Some(order_id), symbol: symbol.to_string(), entry_price: fill_price,
                        exit_price: None, amount, entry_time: Utc::now(), exit_time: None,
                        pnl: None, pnl_pct: None, strategy: format!("BinanceLimitMaker-{:?}", signal),
                    });
                }
                "CANCELED" | "EXPIRED" | "REJECTED" => {
                    return Err(format!("Limit emir iptal/expired [{symbol}] @ {price:.4}").into());
                }
                _ => {
                    if std::time::Instant::now() >= deadline {
                        let _ = self.run_async(self.inner.cancel_order(symbol, order_id));
                        return Err(format!("Limit emir timeout ({timeout_ms}ms) → iptal [{symbol}] @ {price:.4}").into());
                    }
                }
            }
        }
    }

    fn fetch_book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        self.run_async(self.inner.fetch_book_ticker(symbol))
    }
}
