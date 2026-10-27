//! `BinanceVenue` — `VenueAdapter`'ın Binance (spot + futures) implementasyonu.
//!
//! Mevcut, savaşta-denenmiş somut tipleri **sarar**, yeniden yazmaz:
//!   * mum/fiyat → [`BinanceFetcher`] (`MarketFetcher`)
//!   * emir/kaldıraç/bakiye/filtre → [`BinanceFuturesExecutor`]
//!
//! Böylece Faz 0 davranış-koruyan kalır: aynı kod, tek arayüz arkasında. Kirli `serde_json::Value`
//! yanıtları burada `OrderReceipt`'e normalleşir; motor borsa-bağımsız tipleri görür.
//!
//! İki kuruluş modu: [`BinanceVenue::data_only`] (auth'suz — yalnız public veri: mum) ve
//! [`BinanceVenue::with_executor`] (auth'lu — veri + yürütme). Mum çekme (`fetch_candles`)
//! executor'a bağlı değildir (public klines); book/filtre/emir executor ister.

use async_trait::async_trait;
use std::sync::Arc;

use crate::executor::{BinanceFuturesExecutor, BinanceTradeExecutor};
use crate::fetcher::BinanceFetcher;
use memos_venue_api::{
    AccountSnapshot, AccountSnapshotQuality, AccountSnapshotStatus, Candle, Exchange, Market,
    MarketData, OrderExecution, OrderKind, OrderReceipt, OrderRequest, Result, SymbolFilters,
    TradeExecutor, TradingProfile, VenueAdapter,
};

pub struct BinanceVenue {
    market: Market,
    fetcher: BinanceFetcher,
    /// Auth'lu executor — yürütme + book/filtre için. `None` = data-only venue (yalnız mum).
    executor: Option<Arc<BinanceFuturesExecutor>>,
}

impl BinanceVenue {
    /// Auth'lu venue: veri + yürütme. `executor` market'iyle (Spot/Futures) tutarlı olmalı.
    pub fn with_executor(market: Market, executor: Arc<BinanceFuturesExecutor>) -> Self {
        return Self {
            market,
            fetcher: BinanceFetcher::new_for_host(&executor.base_url),
            executor: Some(executor),
        };
    }

    /// Data-only venue: yalnız public mum çekme (auth/keys gerekmez). Book/filtre/emir
    /// çağrıldığında açık hata döner. Fiyat-poll/screener gibi salt-veri yolları için.
    pub fn data_only(market: Market) -> Self {
        let testnet = std::env::var("BINANCE_TESTNET")
            .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
        let host = if testnet {
            if market == Market::Spot {
                "https://testnet.binance.vision"
            } else {
                "https://testnet.binancefuture.com"
            }
        } else if market == Market::Spot {
            "https://api.binance.com"
        } else {
            "https://fapi.binance.com"
        };
        return Self {
            market,
            fetcher: BinanceFetcher::new_for_host(host),
            executor: None,
        };
    }

    /// Executor'a erişim — data-only venue'da açık hata.
    fn exec(&self) -> Result<&BinanceFuturesExecutor> {
        return self.executor.as_deref().ok_or_else(|| {
            "BinanceVenue data-only: executor (auth) gerekiyor"
                .to_string()
                .into()
        });
    }

    fn ensure_execution_symbol(symbol: &str) -> Result<()> {
        let requested = symbol.trim().trim_end_matches("@binance").to_ascii_uppercase();
        let raw = std::env::var("BINANCE_EXECUTION_SYMBOL_ALLOWLIST")
            .or_else(|_| std::env::var("TRADE_SYMBOL"))
            .map_err(|_| "Binance execution sembol allowlist tanımlı değil".to_string())?;
        let allowed = raw
            .split(',')
            .map(|item| item.trim().trim_end_matches("@binance").to_ascii_uppercase())
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        if allowed.is_empty() || !allowed.iter().any(|item| item == &requested) {
            return Err(format!(
                "Binance execution sembol allowlist reddi: {requested}"
            )
            .into());
        }
        Ok(())
    }
}

#[async_trait]
impl MarketData for BinanceVenue {
    async fn fetch_candles(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        return self
            .fetcher
            .fetch_latest_market(symbol, interval, self.market.as_str(), limit)
            .await
            .map_err(Into::into);
    }

    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        return Ok(self.exec()?.fetch_book_ticker(symbol).await?);
    }

    async fn symbol_filters(&self, symbol: &str) -> Result<SymbolFilters> {
        return Ok(self.exec()?.ensure_filters(symbol).await?);
    }
}

#[async_trait]
impl OrderExecution for BinanceVenue {
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderReceipt> {
        Self::ensure_execution_symbol(&req.symbol)?;
        let exec = self.exec()?;
        let side = req.side.as_binance();
        let raw = match req.kind {
            OrderKind::Market => exec.place_market_order(&req.symbol, side, req.qty).await?,
            OrderKind::PostOnlyLimit { price } => {
                exec.place_post_only_limit_order(&req.symbol, side, req.qty, price)
                    .await?
            }
        };
        return Ok(OrderReceipt::from_binance(raw));
    }

    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        Self::ensure_execution_symbol(symbol)?;
        let _ = self.exec()?.cancel_all_orders(symbol).await?;
        return Ok(());
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        let _ = self.exec()?.set_leverage(symbol, leverage).await?;
        return Ok(());
    }

    async fn balance(&self) -> Result<f64> {
        return Ok(self.exec()?.get_balance().await?);
    }

    async fn account_snapshot(&self) -> Result<AccountSnapshot> {
        let response = self.exec()?.get_account_snapshot().await?;
        let (balance, equity, currency) = if self.market == Market::Spot {
            let usdt = response["balances"]
                .as_array()
                .and_then(|balances| balances.iter().find(|asset| asset["asset"] == "USDT"))
                .ok_or_else(|| "Binance spot USDT bakiyesi bulunamadı".to_string())?;
            let free = usdt["free"]
                .as_str()
                .and_then(|value| value.parse::<f64>().ok())
                .ok_or_else(|| "Binance spot free USDT bakiyesi geçersiz".to_string())?;
            let locked = usdt["locked"]
                .as_str()
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.0);
            (free, Some(free + locked), "USDT")
        } else {
            let wallet = response["totalWalletBalance"]
                .as_str()
                .and_then(|value| value.parse::<f64>().ok())
                .ok_or_else(|| "Binance futures totalWalletBalance bulunamadı".to_string())?;
            let equity = response["totalMarginBalance"]
                .as_str()
                .and_then(|value| value.parse::<f64>().ok())
                .ok_or_else(|| "Binance futures totalMarginBalance bulunamadı".to_string())?;
            (wallet, Some(equity), "USDT")
        };

        Ok(AccountSnapshot {
            currency: Some(currency.to_string()),
            balance,
            equity,
            timestamp: chrono::Utc::now(),
            quality: AccountSnapshotQuality::Reported,
            status: AccountSnapshotStatus::Fresh,
        })
    }

    async fn apply_filters(
        &self,
        symbol: &str,
        qty: f64,
        price: f64,
    ) -> core::result::Result<f64, String> {
        self.exec()
            .map_err(|e| e.to_string())?
            .apply_filters(symbol, qty, price)
            .await
    }

    async fn round_price(&self, symbol: &str, price: f64) -> core::result::Result<f64, String> {
        let filters = self
            .exec()
            .map_err(|e| e.to_string())?
            .ensure_filters(symbol)
            .await
            .map_err(|e| e.to_string())?;
        Ok(filters.round_price(price))
    }

    async fn smart_limit_entry(
        &self,
        symbol: &str,
        side: memos_venue_api::OrderSide,
        qty: f64,
        timeout_ms: u64,
        max_attempts: u32,
        max_spread_bps: f64,
        fallback_market: bool,
    ) -> Result<(OrderReceipt, bool)> {
        Self::ensure_execution_symbol(symbol)?;
        let exec = self.exec()?;
        match exec
            .place_smart_limit_entry(
                symbol,
                side.as_binance(),
                qty,
                timeout_ms,
                max_attempts,
                max_spread_bps,
            )
            .await
        {
            Ok(raw) => Ok((OrderReceipt::from_binance(raw), true)),
            Err(maker_err) if fallback_market => {
                if maker_err.is_exchange_block() {
                    return Err(maker_err);
                }
                let raw = exec
                    .place_market_order(symbol, side.as_binance(), qty)
                    .await
                    .map_err(|market_err| {
                        if market_err.is_exchange_block() {
                            market_err
                        } else {
                            memos_venue_api::VenueError::Api(format!(
                                "maker başarısız ({maker_err}); market fallback başarısız ({market_err})"
                            ))
                        }
                    })?;
                Ok((OrderReceipt::from_binance(raw), false))
            }
            Err(err) => Err(err),
        }
    }

    async fn place_protection_orders(
        &self,
        symbol: &str,
        is_long: bool,
        qty: f64,
        stop_loss: f64,
        take_profit: f64,
    ) -> (Result<OrderReceipt>, Result<OrderReceipt>) {
        if let Err(err) = Self::ensure_execution_symbol(symbol) {
            return (Err(err), Err("Binance protection sembol allowlist reddi".into()));
        }
        let exec = match self.exec() {
            Ok(exec) => exec,
            Err(err) => {
                return (
                    Err(err),
                    Err(memos_venue_api::VenueError::Unsupported(
                        "executor yok".into(),
                    )),
                )
            }
        };
        let (sl, tp) = exec
            .place_protection_orders(symbol, is_long, qty, stop_loss, take_profit)
            .await;
        (
            sl.map(OrderReceipt::from_binance),
            tp.map(OrderReceipt::from_binance),
        )
    }

    async fn cancel_order(&self, symbol: &str, venue_order_id: &str) -> Result<()> {
        Self::ensure_execution_symbol(symbol)?;
        let id = venue_order_id
            .trim_matches('"')
            .parse::<u64>()
            .map_err(|_| {
                memos_venue_api::VenueError::Api(format!(
                    "geçersiz venue order id: {venue_order_id}"
                ))
            })?;
        self.exec()?.cancel_order(symbol, id).await?;
        Ok(())
    }

    async fn close_position(&self, symbol: &str) -> Result<OrderReceipt> {
        Self::ensure_execution_symbol(symbol)?;
        Ok(OrderReceipt::from_binance(
            self.exec()?.close_position(symbol).await?,
        ))
    }

    // ─── 🛰️ OTONOM EMİR SORGULAMA KANCASI YERLEŞTİRMESİ ──────────────────────────────────
    // GEREKÇE: İki argümanlı mutlak Result şemasıyla nesne güvenliği mühürlenmiştir.
    // BinanceFuturesExecutor somut yapısındaki açık emir REST çağrılarını otonom JSON array'e dönüştürür.
    async fn fetch_open_orders(&self) -> core::result::Result<Vec<serde_json::Value>, String> {
        let exec = match self.exec() {
            Ok(e) => e,
            Err(err) => return Err(err.to_string()),
        };
        // Not: Executor modülünüzdeki açık emir sorgulama fonksiyonuna delege edilir
        match exec.get_open_orders("").await {
            Ok(raw_orders) => return Ok(raw_orders),
            Err(e) => return Err(format!("Binance fetch_open_orders hatası: {:?}", e)),
        }
    }
    async fn fetch_account_positions(
        &self,
    ) -> core::result::Result<Vec<serde_json::Value>, String> {
        let exec = match self.exec() {
            Ok(e) => e,
            Err(err) => return Err(err.to_string()),
        };
        // Not: Executor modülünüzdeki hesap pozisyonları sorgulama fonksiyonuna delege edilir
        match exec.get_account_positions().await {
            Ok(raw_positions) => return Ok(raw_positions),
            Err(e) => return Err(format!("Binance fetch_account_positions hatası: {:?}", e)),
        }
    }
}

impl VenueAdapter for BinanceVenue {
    /// Kripto-perp: 7/24 likit, yüksek vol toleransı → 4 kademe, ATR%5 tavan.
    fn trading_profile(&self) -> TradingProfile {
        return TradingProfile {
            max_pyramid_tranches: 4,
            max_atr_pct: 5.0,
        };
    }

    fn exchange(&self) -> Exchange {
        return Exchange::Binance;
    }
    fn market(&self) -> Market {
        return self.market;
    }

    fn is_delisting_error(&self, error_msg: &str) -> bool {
        return crate::fetcher::fetch_error_is_delisting(error_msg);
    }

    fn execution_engine(&self) -> Option<Arc<dyn TradeExecutor>> {
        return self.executor.as_ref().map(|exec| {
            let wrapped_executor = BinanceTradeExecutor::new(Arc::clone(exec));
            return Arc::new(wrapped_executor) as Arc<dyn TradeExecutor>;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_venue_api::AssetClass;

    fn authed(market: Market) -> BinanceVenue {
        let exec = Arc::new(BinanceFuturesExecutor::new_for_market(
            String::new(),
            String::new(),
            true, // paper/testnet — ağ çağrısı yapılmaz (kimlik/asset_class testi)
            market.as_str(),
        ));
        BinanceVenue::with_executor(market, exec)
    }

    #[test]
    fn identity_is_binance_crypto() {
        let v = authed(Market::Futures);
        assert_eq!(v.exchange(), Exchange::Binance);
        assert_eq!(v.market(), Market::Futures);
        assert_eq!(v.asset_class(), AssetClass::Crypto);
        assert!(v.has_live_feed());
        assert_eq!(v.name(), "binance:futures");
    }

    #[test]
    fn market_axis_is_carried() {
        assert_eq!(authed(Market::Spot).market(), Market::Spot);
        assert_eq!(authed(Market::Coinm).market(), Market::Coinm);
    }

    #[tokio::test]
    async fn data_only_rejects_execution_but_keeps_identity() {
        let v = BinanceVenue::data_only(Market::Futures);
        assert_eq!(v.exchange(), Exchange::Binance);
        assert_eq!(v.market(), Market::Futures);
        // Yürütme/book/filtre → açık hata (ağ çağrısı yapılmaz, executor None).
        assert!(v.balance().await.is_err());
        assert!(v.book_ticker("BTCUSDT").await.is_err());
        assert!(v.set_leverage("BTCUSDT", 5).await.is_err());
    }
}
