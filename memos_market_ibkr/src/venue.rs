use std::sync::Arc;

use async_trait::async_trait;
use memos_venue_api::{
    AccountSnapshot, AccountSnapshotQuality, AccountSnapshotStatus, Candle, CandleFallback,
    Exchange, Market, MarketData, OrderExecution, OrderReceipt, OrderRequest, Result, Signal,
    SymbolFilters, Trade, TradeExecutor, TradingProfile, VenueAdapter, VenueFactory, VenueSpec,
};

use crate::protocol;
use crate::{IbkrBridge, IBKR_DEFAULT_ADDR};

pub struct IbkrVenue {
    market: Market,
    profile: memos_venue_api::VenueProfile,
    pub bridge: Arc<IbkrBridge>,
    candle_fallback: Option<Arc<dyn CandleFallback>>,
    execution_enabled: bool,
}

pub struct IbkrVenueFactory {
    bridge_addr: Option<String>,
    candle_fallback: Option<Arc<dyn CandleFallback>>,
}

impl IbkrVenueFactory {
    pub fn from_env(candle_fallback: Arc<dyn CandleFallback>) -> Self {
        Self {
            bridge_addr: std::env::var("IBKR_BRIDGE_ADDR").ok(),
            candle_fallback: Some(candle_fallback),
        }
    }

    pub fn new(
        bridge_addr: Option<String>,
        candle_fallback: Option<Arc<dyn CandleFallback>>,
    ) -> Self {
        Self {
            bridge_addr,
            candle_fallback,
        }
    }
}

impl VenueFactory for IbkrVenueFactory {
    fn build(&self, spec: VenueSpec) -> Result<Arc<dyn VenueAdapter>> {
        if spec.exchange != Exchange::Ibkr {
            return Err(memos_venue_api::VenueError::Config(format!(
                "IBKR factory yalnızca IBKR venue üretir: {}",
                spec.token()
            )));
        }

        let bridge = Arc::new(IbkrBridge::with_defaults(
            self.bridge_addr
                .clone()
                .or_else(|| Some(IBKR_DEFAULT_ADDR.to_string())),
        ));
        let venue = match &self.candle_fallback {
            Some(fallback) => {
                IbkrVenue::with_fallback(spec.market, spec.profile, bridge, fallback.clone())
            }
            None => IbkrVenue::new(spec.market, spec.profile, bridge),
        };
        Ok(Arc::new(venue))
    }
}

impl IbkrVenue {
    pub fn new(
        market: Market,
        profile: memos_venue_api::VenueProfile,
        bridge: Arc<IbkrBridge>,
    ) -> Self {
        Self {
            market,
            profile,
            bridge,
            candle_fallback: None,
            execution_enabled: ibkr_execution_enabled(),
        }
    }

    pub fn with_fallback(
        market: Market,
        profile: memos_venue_api::VenueProfile,
        bridge: Arc<IbkrBridge>,
        candle_fallback: Arc<dyn CandleFallback>,
    ) -> Self {
        Self {
            market,
            profile,
            bridge,
            candle_fallback: Some(candle_fallback),
            execution_enabled: ibkr_execution_enabled(),
        }
    }

    pub fn with_defaults(market: Market, addr: Option<String>) -> Self {
        Self::new(
            market,
            memos_venue_api::VenueProfile::Default,
            Arc::new(IbkrBridge::with_defaults(
                addr.or_else(|| Some(IBKR_DEFAULT_ADDR.to_string())),
            )),
        )
    }

    pub async fn fetch_candles_until(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
        end_timestamp_ms: i64,
    ) -> Result<Vec<Candle>> {
        let request = protocol::req_candles_until(
            self.bridge.next_id(),
            symbol,
            interval,
            limit,
            end_timestamp_ms,
        );
        let body = self.bridge.request(&request).await?;
        protocol::parse_candles(symbol, interval, &body)
    }
}

#[async_trait]
impl MarketData for IbkrVenue {
    async fn fetch_candles(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        let request = protocol::req_candles(self.bridge.next_id(), symbol, interval, limit);
        match self.bridge.request(&request).await {
            Ok(body) => protocol::parse_candles(symbol, interval, &body),
            Err(_) => self
                .candle_fallback
                .as_ref()
                .ok_or_else(|| {
                    memos_venue_api::VenueError::Config(
                        "IBKR bridge erişilemedi ve candle fallback tanımlı değil".to_string(),
                    )
                })?
                .read_candles_for_profile(
                    symbol,
                    interval,
                    Exchange::Ibkr.as_str(),
                    self.market.as_str(),
                    self.profile.as_str(),
                    limit,
                ),
        }
    }

    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        let request = protocol::req_tick(self.bridge.next_id(), symbol);
        match self.bridge.request(&request).await {
            Ok(body) => protocol::parse_tick(symbol, &body),
            Err(_) => {
                let fallback = self.candle_fallback.as_ref().ok_or_else(|| {
                    "IBKR bridge erişilemedi ve fiyat fallback'i tanımlı değil".to_string()
                })?;
                let mut close = None;
                for interval in ["1m", "5m", "15m", "1h", "4h", "1d"] {
                    let Ok(candles) = fallback.read_candles_for_profile(
                        symbol,
                        interval,
                        Exchange::Ibkr.as_str(),
                        self.market.as_str(),
                        self.profile.as_str(),
                        1,
                    ) else {
                        continue;
                    };
                    if let Some(value) = candles
                        .last()
                        .map(|candle| candle.close)
                        .filter(|value| value.is_finite() && *value > 0.0)
                    {
                        close = Some(value);
                        break;
                    }
                }
                let close = close.ok_or_else(|| format!("IBKR fallback fiyat yok [{symbol}]"))?;
                let pad = (close * 0.0001).max(1e-8);
                Ok((close - pad, close + pad))
            }
        }
    }

    async fn symbol_filters(&self, symbol: &str) -> Result<SymbolFilters> {
        let body = self
            .bridge
            .request(&protocol::req_filters(self.bridge.next_id(), symbol))
            .await?;
        protocol::parse_filters(symbol, &body)
    }
}

#[async_trait]
impl OrderExecution for IbkrVenue {
    async fn submit_order(&self, request: &OrderRequest) -> Result<OrderReceipt> {
        if !self.execution_enabled {
            return Err(
                "IBKR execution kapalı: IBKR_EXECUTION_ENABLED=1 ve IBKR_READ_ONLY=0 gerekli"
                    .into(),
            );
        }

        let body = self
            .bridge
            .request(&protocol::req_order(self.bridge.next_id(), request))
            .await?;
        protocol::parse_order(&request.symbol, &body)
    }

    async fn place_protection_orders(
        &self,
        symbol: &str,
        is_long: bool,
        qty: f64,
        stop_loss: f64,
        take_profit: f64,
    ) -> (Result<OrderReceipt>, Result<OrderReceipt>) {
        if !self.execution_enabled {
            let message =
                "IBKR protection kapalı: IBKR_EXECUTION_ENABLED=1 ve IBKR_READ_ONLY=0 gerekli";
            return (
                Err(memos_venue_api::VenueError::Unsupported(message.to_string())),
                Err(memos_venue_api::VenueError::Unsupported(message.to_string())),
            );
        }
        let body = match self
            .bridge
            .request(&protocol::req_protection(
                self.bridge.next_id(),
                symbol,
                is_long,
                qty,
                stop_loss,
                take_profit,
            ))
            .await
        {
            Ok(body) => body,
            Err(error) => {
                let message = error.to_string();
                return (
                    Err(memos_venue_api::VenueError::Network(message.clone())),
                    Err(memos_venue_api::VenueError::Network(message)),
                );
            }
        };
        match protocol::parse_order(symbol, &body) {
            Ok(receipt) => (Ok(receipt.clone()), Ok(receipt)),
            Err(error) => {
                let message = error.to_string();
                (
                    Err(memos_venue_api::VenueError::Unknown(message.clone())),
                    Err(memos_venue_api::VenueError::Unknown(message)),
                )
            }
        }
    }

    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        if !self.execution_enabled {
            return Err(
                "IBKR cancel_all kapalı: IBKR_EXECUTION_ENABLED=1 ve IBKR_READ_ONLY=0 gerekli"
                    .into(),
            );
        }
        let body = self
            .bridge
            .request(&protocol::req_cancel_all(self.bridge.next_id(), symbol))
            .await?;
        protocol::parse_ack(symbol, &body)
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        if !self.execution_enabled {
            return Err("IBKR leverage ayarı kapalı: execution capability etkin değil".into());
        }
        let body = self
            .bridge
            .request(&protocol::req_set_leverage(
                self.bridge.next_id(),
                symbol,
                leverage,
            ))
            .await?;
        protocol::parse_ack(symbol, &body)
    }

    async fn balance(&self) -> Result<f64> {
        let body = self
            .bridge
            .request(&protocol::req_balance(self.bridge.next_id()))
            .await?;
        protocol::parse_balance(&body)
    }

    async fn account_snapshot(&self) -> Result<AccountSnapshot> {
        let balance = self.balance().await?;
        Ok(AccountSnapshot {
            currency: Some("USD".to_string()),
            balance,
            equity: Some(balance),
            timestamp: chrono::Utc::now(),
            quality: AccountSnapshotQuality::Reported,
            status: AccountSnapshotStatus::Fresh,
        })
    }

    async fn fetch_open_orders(&self) -> core::result::Result<Vec<serde_json::Value>, String> {
        let line = format!(
            "{{\"id\":{},\"cmd\":\"open_orders\",\"symbol\":\"\"}}\n",
            self.bridge.next_id()
        );
        let body = self
            .bridge
            .request(&line)
            .await
            .map_err(|error| format!("IBKR bridge fetch_open_orders hatası: {error}"))?;
        let value = serde_json::from_str::<serde_json::Value>(&body)
            .map_err(|error| format!("IBKR open_orders parse hatası: {error}"))?;
        let orders = value
            .get("open_orders")
            .and_then(|orders| orders.as_array())
            .cloned()
            .unwrap_or_else(|| value.as_array().cloned().unwrap_or_else(|| vec![value]));
        Ok(orders
            .into_iter()
            .map(|mut order| {
                if let Some(symbol) = order.get("symbol").and_then(|value| value.as_str()) {
                    order["symbol"] = serde_json::Value::String(ibkr_symbol(symbol));
                }
                order
            })
            .collect())
    }

    async fn fetch_account_positions(
        &self,
    ) -> core::result::Result<Vec<serde_json::Value>, String> {
        let line = format!(
            "{{\"id\":{},\"cmd\":\"account_positions\",\"symbol\":\"\"}}\n",
            self.bridge.next_id()
        );
        let body = self
            .bridge
            .request(&line)
            .await
            .map_err(|error| format!("IBKR bridge fetch_account_positions hatası: {error}"))?;
        let value = serde_json::from_str::<serde_json::Value>(&body)
            .map_err(|error| format!("IBKR account_positions parse hatası: {error}"))?;
        let positions = value
            .get("account_positions")
            .and_then(|positions| positions.as_array())
            .cloned()
            .unwrap_or_else(|| value.as_array().cloned().unwrap_or_else(|| vec![value]));
        Ok(positions
            .into_iter()
            .map(|mut position| {
                if let Some(symbol) = position.get("symbol").and_then(|value| value.as_str()) {
                    position["symbol"] = serde_json::Value::String(ibkr_symbol(symbol));
                }
                position
            })
            .collect())
    }
}

fn ibkr_symbol(symbol: &str) -> String {
    if symbol.contains('@') {
        symbol.to_string()
    } else {
        format!("{symbol}@ibkr")
    }
}

fn ibkr_execution_enabled() -> bool {
    let enabled = std::env::var("IBKR_EXECUTION_ENABLED")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    let read_only = std::env::var("IBKR_READ_ONLY")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    enabled && !read_only
}

struct IbkrTradeExecutor {
    bridge: Arc<IbkrBridge>,
}

impl IbkrTradeExecutor {
    fn new(bridge: Arc<IbkrBridge>) -> Self {
        Self { bridge }
    }
}

impl TradeExecutor for IbkrTradeExecutor {
    fn execute(&self, signal: Signal, symbol: &str, amount: f64) -> Result<Trade> {
        if !amount.is_finite() || amount == 0.0 {
            return Err("IBKR miktarı sıfır veya sonlu değil".into());
        }
        let side = match signal {
            Signal::Buy => memos_venue_api::OrderSide::Buy,
            Signal::Sell => memos_venue_api::OrderSide::Sell,
            Signal::Hold => return Err("IBKR HOLD sinyali emir üretemez".into()),
        };
        let request = OrderRequest::market(symbol, side, amount.abs());
        let bridge = Arc::clone(&self.bridge);
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "IBKR execution tokio runtime dışında çağrıldı")?;
        let receipt = runtime.block_on(async move {
            let body = bridge
                .request(&protocol::req_order(bridge.next_id(), &request))
                .await?;
            protocol::parse_order(symbol, &body)
        })?;
        if matches!(
            receipt.status,
            memos_venue_api::OrderStatus::Rejected
                | memos_venue_api::OrderStatus::Canceled
                | memos_venue_api::OrderStatus::Unknown
        ) {
            return Err(format!("IBKR emir kabul edilmedi: {:?}", receipt.status).into());
        }
        Ok(Trade {
            id: receipt.venue_order_id.and_then(|id| id.parse().ok()),
            symbol: symbol.to_string(),
            entry_price: receipt.avg_price,
            exit_price: None,
            amount: receipt.filled_qty.max(amount.abs()),
            entry_time: chrono::Utc::now(),
            exit_time: None,
            pnl: None,
            pnl_pct: None,
            strategy: "IBKR_Otonom".to_string(),
        })
    }

    fn cancel_all(&self, symbol: &str) -> Result<()> {
        let bridge = Arc::clone(&self.bridge);
        let symbol = symbol.to_string();
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "IBKR cancel tokio runtime dışında çağrıldı")?;
        runtime.block_on(async move {
            let body = bridge
                .request(&protocol::req_cancel_all(bridge.next_id(), &symbol))
                .await?;
            protocol::parse_ack(&symbol, &body)
        })
    }
}

impl VenueAdapter for IbkrVenue {
    fn trading_profile(&self) -> TradingProfile {
        TradingProfile {
            max_pyramid_tranches: 2,
            max_atr_pct: 1.5,
        }
    }

    fn exchange(&self) -> Exchange {
        Exchange::Ibkr
    }

    fn market(&self) -> Market {
        self.market
    }

    fn is_delisting_error(&self, error_msg: &str) -> bool {
        let error = error_msg.to_lowercase();
        error.contains("contract")
            || error.contains("symbol")
            || error.contains("security definition")
    }

    fn execution_engine(&self) -> Option<Arc<dyn TradeExecutor>> {
        if self.execution_enabled {
            Some(Arc::new(IbkrTradeExecutor::new(Arc::clone(&self.bridge))))
        } else {
            None
        }
    }
}
