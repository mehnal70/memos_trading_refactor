use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use memos_venue_api::{
    Candle, OrderKind, OrderReceipt, OrderRequest, OrderSide, OrderStatus, Result, SymbolFilters,
};

pub fn req_ping(id: u64) -> String {
    json!({"id": id, "cmd": "ping"}).to_string()
}

pub fn req_health(id: u64) -> String {
    json!({"id": id, "cmd": "health"}).to_string()
}

pub fn req_candles(id: u64, symbol: &str, tf: &str, limit: usize) -> String {
    json!({"id": id, "cmd": "candles", "symbol": symbol, "tf": tf, "limit": limit}).to_string()
}

pub fn req_candles_until(
    id: u64,
    symbol: &str,
    tf: &str,
    limit: usize,
    end_timestamp_ms: i64,
) -> String {
    json!({
        "id": id,
        "cmd": "candles",
        "symbol": symbol,
        "tf": tf,
        "limit": limit,
        "end_timestamp_ms": end_timestamp_ms
    })
    .to_string()
}

pub fn req_tick(id: u64, symbol: &str) -> String {
    json!({"id": id, "cmd": "tick", "symbol": symbol}).to_string()
}

pub fn req_filters(id: u64, symbol: &str) -> String {
    json!({"id": id, "cmd": "filters", "symbol": symbol}).to_string()
}

pub fn req_order(id: u64, order: &OrderRequest) -> String {
    let side = match order.side {
        OrderSide::Buy => "buy",
        OrderSide::Sell => "sell",
    };
    let mut value = json!({
        "id": id,
        "cmd": "order",
        "symbol": order.symbol,
        "side": side,
        "qty": order.qty,
        "reduce_only": order.reduce_only,
    });
    match order.kind {
        OrderKind::Market => value["kind"] = json!("market"),
        OrderKind::PostOnlyLimit { price } => {
            value["kind"] = json!("limit");
            value["price"] = json!(price);
        }
    }
    value.to_string()
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

pub fn req_protection(
    id: u64,
    symbol: &str,
    is_long: bool,
    qty: f64,
    stop_loss: f64,
    take_profit: f64,
) -> String {
    json!({
        "id": id,
        "cmd": "protection",
        "symbol": symbol,
        "is_long": is_long,
        "qty": qty,
        "stop_loss": stop_loss,
        "take_profit": take_profit
    })
    .to_string()
}

fn ok_value(scope: &str, body: &str) -> Result<Value> {
    let value: Value =
        serde_json::from_str(body.trim()).map_err(|e| format!("IBKR JSON parse [{scope}]: {e}"))?;
    if value.get("ok").and_then(|ok| ok.as_bool()) == Some(true) {
        Ok(value)
    } else {
        let message = value
            .get("error")
            .and_then(|error| error.as_str())
            .unwrap_or("bilinmeyen IBKR hatası");
        Err(format!("IBKR köprü hatası [{scope}]: {message}").into())
    }
}

fn valid_ohlcv(open: f64, high: f64, low: f64, close: f64, volume: f64) -> bool {
    open.is_finite()
        && high.is_finite()
        && low.is_finite()
        && close.is_finite()
        && volume.is_finite()
        && open > 0.0
        && high >= low
        && high >= open
        && high >= close
        && low <= open
        && low <= close
        && volume >= 0.0
}

pub fn parse_candles(symbol: &str, interval: &str, body: &str) -> Result<Vec<Candle>> {
    let value = ok_value(symbol, body)?;
    let list = value
        .get("candles")
        .and_then(|candles| candles.as_array())
        .ok_or_else(|| format!("IBKR yanıtında candles dizisi yok [{symbol}]"))?;

    let mut candles = Vec::with_capacity(list.len());
    for item in list {
        let array = match item.as_array() {
            Some(array) if array.len() >= 6 => array,
            _ => continue,
        };
        let number = |index: usize| {
            array
                .get(index)
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0)
        };
        let timestamp = match array.first().and_then(|value| value.as_i64()) {
            Some(timestamp) if timestamp > 0 => timestamp,
            _ => continue,
        };
        let (open, high, low, close) = (number(1), number(2), number(3), number(4));
        let volume = number(5).max(0.0);
        if !valid_ohlcv(open, high, low, close, volume) {
            continue;
        }
        if let Some(timestamp) = DateTime::from_timestamp_millis(timestamp) {
            candles.push(Candle {
                timestamp: timestamp.with_timezone(&Utc),
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
    candles.sort_by_key(|candle| candle.timestamp);
    validate_cadence(symbol, interval, &candles)?;
    Ok(candles)
}

fn interval_ms(interval: &str) -> Option<i64> {
    match interval {
        "1m" => Some(60_000),
        "5m" => Some(300_000),
        "15m" => Some(900_000),
        "30m" => Some(1_800_000),
        "1h" => Some(3_600_000),
        "4h" => Some(14_400_000),
        "1d" => Some(86_400_000),
        _ => None,
    }
}

fn validate_cadence(symbol: &str, interval: &str, candles: &[Candle]) -> Result<()> {
    let Some(expected) = interval_ms(interval) else {
        return Ok(());
    };
    let mut counts = std::collections::BTreeMap::<i64, usize>::new();
    for pair in candles.windows(2) {
        let delta = (pair[1].timestamp - pair[0].timestamp).num_milliseconds();
        if delta > 0 {
            *counts.entry(delta).or_insert(0) += 1;
        }
    }
    let Some((observed, _)) = counts.into_iter().max_by_key(|(_, count)| *count) else {
        return Ok(());
    };
    if observed != expected {
        return Err(format!(
            "IBKR cadence uyuşmazlığı [{symbol} {interval}]: beklenen={}ms gözlenen={}ms",
            expected, observed
        )
        .into());
    }
    Ok(())
}

pub fn parse_tick(symbol: &str, body: &str) -> Result<(f64, f64)> {
    let value = ok_value(symbol, body)?;
    let number = |key: &str| value.get(key).and_then(|value| value.as_f64());
    match (number("bid"), number("ask")) {
        (Some(bid), Some(ask)) if bid > 0.0 && ask > 0.0 => Ok((bid, ask)),
        _ => Err(format!("IBKR bid/ask alınamadı [{symbol}]").into()),
    }
}

pub fn parse_filters(symbol: &str, body: &str) -> Result<SymbolFilters> {
    let value = ok_value(symbol, body)?;
    let number = |key: &str| {
        value
            .get(key)
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0)
    };
    Ok(SymbolFilters {
        step_size: number("lot_step"),
        min_qty: number("min_lot"),
        tick_size: number("tick_size"),
        min_notional: number("min_notional"),
    })
}

pub fn parse_order(symbol: &str, body: &str) -> Result<OrderReceipt> {
    let value = ok_value(symbol, body)?;
    let status = match value
        .get("status")
        .and_then(|status| status.as_str())
        .unwrap_or("")
    {
        "filled" => OrderStatus::Filled,
        "partial" | "partially_filled" => OrderStatus::PartiallyFilled,
        "new" | "placed" | "submitted" => OrderStatus::New,
        "canceled" | "cancelled" => OrderStatus::Canceled,
        "rejected" => OrderStatus::Rejected,
        _ => OrderStatus::Unknown,
    };
    let number = |key: &str| {
        value
            .get(key)
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0)
    };
    let venue_order_id = value.get("order_id").map(|id| {
        id.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| id.to_string())
    });
    Ok(OrderReceipt {
        venue_order_id,
        status,
        filled_qty: number("filled_qty"),
        avg_price: number("avg_price"),
        raw: Some(value),
    })
}

pub fn parse_balance(body: &str) -> Result<f64> {
    let value = ok_value("ibkr-balance", body)?;
    value
        .get("balance")
        .and_then(|balance| balance.as_f64())
        .ok_or_else(|| "IBKR balance alanı yok".into())
}

pub fn parse_ack(scope: &str, body: &str) -> Result<()> {
    ok_value(scope, body)?;
    Ok(())
}
