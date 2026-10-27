use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::core::model::SymbolFilters;
use crate::core::types::Candle;
use crate::robot::venue::types::{OrderKind, OrderReceipt, OrderRequest, OrderStatus};
use memos_venue_api::Result;

pub fn req_ping(id: u64) -> String {
    json!({"id": id, "cmd": "ping"}).to_string()
}

pub fn req_candles(id: u64, symbol: &str, tf: &str, limit: usize) -> String {
    json!({"id": id, "cmd": "candles", "symbol": symbol, "tf": tf, "limit": limit}).to_string()
}

pub fn req_tick(id: u64, symbol: &str) -> String {
    json!({"id": id, "cmd": "tick", "symbol": symbol}).to_string()
}

pub fn req_filters(id: u64, symbol: &str) -> String {
    json!({"id": id, "cmd": "filters", "symbol": symbol}).to_string()
}

pub fn req_order(id: u64, o: &OrderRequest) -> String {
    let side = match o.side {
        crate::robot::venue::types::OrderSide::Buy => "buy",
        crate::robot::venue::types::OrderSide::Sell => "sell",
    };
    let mut obj = json!({
        "id": id,
        "cmd": "order",
        "symbol": o.symbol,
        "side": side,
        "qty": o.qty,
        "reduce_only": o.reduce_only,
    });
    match o.kind {
        OrderKind::Market => obj["kind"] = json!("market"),
        OrderKind::PostOnlyLimit { price } => {
            obj["kind"] = json!("limit");
            obj["price"] = json!(price);
        }
    }
    obj.to_string()
}

pub fn req_cancel_all(id: u64, symbol: &str) -> String {
    json!({"id": id, "cmd": "cancel_all", "symbol": symbol}).to_string()
}

pub fn req_set_leverage(id: u64, symbol: &str, leverage: u32) -> String {
    json!({"id": id, "cmd": "set_leverage", "symbol": symbol, "leverage": leverage}).to_string()
}

pub fn req_balance(id: u64) -> String {
    json!({"id": id, "cmd": "balance"}).to_string()
}

fn ok_value(scope: &str, body: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(body.trim())
        .map_err(|e| format!("IBKR JSON parse [{scope}]: {e}"))?;
    if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
        Ok(v)
    } else {
        let msg = v.get("error").and_then(|m| m.as_str()).unwrap_or("bilinmeyen IBKR hatası");
        Err(format!("IBKR köprü hatası [{scope}]: {msg}").into())
    }
}

pub fn parse_candles(symbol: &str, interval: &str, body: &str) -> Result<Vec<Candle>> {
    let v = ok_value(symbol, body)?;
    let list = v.get("candles").and_then(|c| c.as_array())
        .ok_or_else(|| format!("IBKR yanıtında candles dizisi yok [{symbol}]"))?;

    let mut candles = Vec::with_capacity(list.len());
    for k in list {
        let arr = match k.as_array() {
            Some(a) if a.len() >= 6 => a,
            _ => continue,
        };
        let nf = |i: usize| arr.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0);
        let ts_ms = match arr.first().and_then(|x| x.as_i64()) {
            Some(t) if t > 0 => t,
            _ => continue,
        };
        let (open, high, low, close) = (nf(1), nf(2), nf(3), nf(4));
        let volume = nf(5).max(0.0);
        if crate::robot::data_fetcher::validate_ohlcv(open, high, low, close, volume).is_err() {
            continue;
        }
        if let Some(dt) = DateTime::from_timestamp_millis(ts_ms) {
            candles.push(Candle {
                timestamp: dt.with_timezone(&Utc),
                open,
                high,
                low,
                close,
                volume,
                symbol: symbol.to_string(),
                interval: interval.to_string(),
            });
        }
    }
    candles.sort_by_key(|c| c.timestamp);
    Ok(candles)
}

pub fn parse_tick(symbol: &str, body: &str) -> Result<(f64, f64)> {
    let v = ok_value(symbol, body)?;
    let f = |k: &str| v.get(k).and_then(|x| x.as_f64());
    match (f("bid"), f("ask")) {
        (Some(b), Some(a)) if b > 0.0 && a > 0.0 => Ok((b, a)),
        _ => Err(format!("IBKR bid/ask alınamadı [{symbol}]").into()),
    }
}

pub fn parse_filters(symbol: &str, body: &str) -> Result<SymbolFilters> {
    let v = ok_value(symbol, body)?;
    let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    Ok(SymbolFilters {
        step_size: f("lot_step"),
        min_qty: f("min_lot"),
        tick_size: f("tick_size"),
        min_notional: f("min_notional"),
    })
}

pub fn parse_order(symbol: &str, body: &str) -> Result<OrderReceipt> {
    let v = ok_value(symbol, body)?;
    let status = match v.get("status").and_then(|s| s.as_str()).unwrap_or("") {
        "filled" => OrderStatus::Filled,
        "partial" | "partially_filled" => OrderStatus::PartiallyFilled,
        "new" | "placed" | "submitted" => OrderStatus::New,
        "canceled" | "cancelled" => OrderStatus::Canceled,
        "rejected" => OrderStatus::Rejected,
        _ => OrderStatus::Unknown,
    };
    let nf = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    let venue_order_id = v.get("order_id")
        .map(|x| x.as_str().map(|s| s.to_owned()).unwrap_or_else(|| x.to_string()));
    Ok(OrderReceipt {
        venue_order_id,
        status,
        filled_qty: nf("filled_qty"),
        avg_price: nf("avg_price"),
        raw: Some(v),
    })
}

pub fn parse_balance(body: &str) -> Result<f64> {
    let v = ok_value("ibkr-balance", body)?;
    v.get("balance")
        .and_then(|x| x.as_f64())
        .ok_or_else(|| "IBKR balance alanı yok".into())
}

pub fn parse_ack(scope: &str, body: &str) -> Result<()> {
    ok_value(scope, body)?;
    Ok(())
}