//! Venue katmanı — borsa-bağımsız emir/sonuç tipleri.
//!
//! `BinanceFuturesExecutor` ham `serde_json::Value` döndürür; bu kirli REST-detayı
//! venue adaptörünün kenarında `OrderReceipt`'e normalleşir → çağrı yerleri borsa-bağımsız
//! kalır. Yeni borsa eklerken yalnız adaptör `Value/JSON → OrderReceipt` çevrimini yapar.

use serde::{Deserialize, Serialize};

/// Emir yönü — borsa-bağımsız. `as_binance()` REST `side` string'ine çevirir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    /// Binance REST `side` parametresi ("BUY"/"SELL").
    pub fn as_binance(&self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }

    /// `is_long` → yön (long açılış/short kapanış = Buy).
    pub fn from_long(is_long: bool) -> Self {
        if is_long { Self::Buy } else { Self::Sell }
    }
}

/// Emir tipi — market ya da maker-only limit. Yeni tip (stop, trailing) gerektiğinde
/// buraya bir kol eklenir; adaptör eşlemesi tek-nokta kalır.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OrderKind {
    /// Anında taker dolum.
    Market,
    /// POST_ONLY maker limit (Binance futures GTX / spot LIMIT_MAKER) verilen fiyatta.
    PostOnlyLimit { price: f64 },
}

/// Borsa-bağımsız emir isteği. Sizing/yön kararı motorda alınır; adaptör yalnız iletir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub symbol: String,
    pub side: OrderSide,
    pub qty: f64,
    pub kind: OrderKind,
    /// Yalnız pozisyon azaltır (futures reduce-only). Spot'ta yok sayılır.
    pub reduce_only: bool,
}

impl OrderRequest {
    /// Market emri kısayolu.
    pub fn market(symbol: impl Into<String>, side: OrderSide, qty: f64) -> Self {
        Self { symbol: symbol.into(), side, qty, kind: OrderKind::Market, reduce_only: false }
    }
}

/// Borsa-bağımsız emir durumu (REST status string'lerinin normalleşmiş hali).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Canceled,
    Rejected,
    Unknown,
}

impl OrderStatus {
    /// Binance `status` string'inden normalleştir (bilinmeyen → Unknown).
    pub fn from_binance(s: &str) -> Self {
        match s {
            "NEW" => Self::New,
            "PARTIALLY_FILLED" => Self::PartiallyFilled,
            "FILLED" => Self::Filled,
            "CANCELED" | "EXPIRED" => Self::Canceled,
            "REJECTED" => Self::Rejected,
            _ => Self::Unknown,
        }
    }
}

/// Borsa-bağımsız emir sonucu. `raw` debug/audit için ham yanıtı saklar (opsiyonel).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderReceipt {
    pub venue_order_id: Option<String>,
    pub status: OrderStatus,
    pub filled_qty: f64,
    /// Ortalama dolum fiyatı (0.0 = bilinmiyor/dolmadı).
    pub avg_price: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

impl OrderReceipt {
    /// Binance market/limit emir yanıtından (futures + spot) normalleştir.
    /// Futures: `avgPrice` doğrudan; spot: `cummulativeQuoteQty / executedQty` türetilir.
    pub fn from_binance(raw: serde_json::Value) -> Self {
        let num = |k: &str| -> f64 {
            raw.get(k)
                .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64()))
                .unwrap_or(0.0)
        };
        let filled_qty = num("executedQty");
        let avg_price = {
            let ap = num("avgPrice");
            if ap > 0.0 {
                ap
            } else {
                // spot: cummulativeQuoteQty / executedQty (futures: cumQuote / executedQty)
                let quote = {
                    let cq = num("cummulativeQuoteQty");
                    if cq > 0.0 { cq } else { num("cumQuote") }
                };
                if filled_qty > 0.0 && quote > 0.0 { quote / filled_qty } else { 0.0 }
            }
        };
        let status = raw
            .get("status")
            .and_then(|v| v.as_str())
            .map(OrderStatus::from_binance)
            .unwrap_or(OrderStatus::Unknown);
        let venue_order_id = raw
            .get("orderId")
            .map(|v| v.as_str().map(|s| s.to_owned()).unwrap_or_else(|| v.to_string()));
        Self { venue_order_id, status, filled_qty, avg_price, raw: Some(raw) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn order_side_binance_mapping() {
        assert_eq!(OrderSide::Buy.as_binance(), "BUY");
        assert_eq!(OrderSide::Sell.as_binance(), "SELL");
        assert_eq!(OrderSide::from_long(true), OrderSide::Buy);
        assert_eq!(OrderSide::from_long(false), OrderSide::Sell);
    }

    #[test]
    fn receipt_from_futures_response() {
        let v = json!({"orderId": 123, "status": "FILLED", "executedQty": "0.500", "avgPrice": "100.0"});
        let r = OrderReceipt::from_binance(v);
        assert_eq!(r.status, OrderStatus::Filled);
        assert_eq!(r.filled_qty, 0.5);
        assert_eq!(r.avg_price, 100.0);
        assert_eq!(r.venue_order_id.as_deref(), Some("123"));
    }

    #[test]
    fn receipt_from_spot_response_derives_avg_price() {
        // spot: avgPrice yok → cummulativeQuoteQty / executedQty
        let v = json!({"orderId": "A1", "status": "FILLED", "executedQty": "2.0", "cummulativeQuoteQty": "50.0"});
        let r = OrderReceipt::from_binance(v);
        assert_eq!(r.filled_qty, 2.0);
        assert_eq!(r.avg_price, 25.0);
        assert_eq!(r.venue_order_id.as_deref(), Some("A1"));
    }

    #[test]
    fn receipt_unknown_status_is_safe() {
        let r = OrderReceipt::from_binance(json!({"status": "WEIRD"}));
        assert_eq!(r.status, OrderStatus::Unknown);
        assert_eq!(r.filled_qty, 0.0);
        assert_eq!(r.avg_price, 0.0);
    }
}
