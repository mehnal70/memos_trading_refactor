//! MT5 köprü protokolü — satır-sınırlı JSON (NDJSON) istek/yanıt çiftleri.
//!
//! Rust = TCP **server**, MT5 EA = native MQL5 **client** (MQL5'te server socket yok).
//! EA bağlanır → döngü: bir istek satırı OKU → işle (CopyRates/SymbolInfoTick/OrderSend) →
//! bir yanıt satırı YAZ. Tek persistan bağlantıda istek↔yanıt seri (bkz. [`super::bridge`]).
//!
//! Bu modül SAF'tır: istek satırı kurar, yanıt gövdesini parse eder — ağ yok, böylece tüm
//! eşleme/sınır-durumları soketsiz test edilir (BybitVenue::parse_klines deseni). Adaptörün
//! kirli kenarı (`Value/JSON → Candle/OrderReceipt`) burada normalleşir; çağrı yerleri saf
//! kalır. ASLA sahte başarı dönmez — ya gerçek alan ya açık `Err`.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::core::model::SymbolFilters;
use crate::core::types::Candle;
use crate::robot::venue::types::{OrderKind, OrderReceipt, OrderRequest, OrderStatus};
use crate::Result;

// ───────────────────────── İstek satırı kurucuları (Rust → EA) ─────────────────────────
//
// Her istek tek satırlık bir JSON nesnesidir; `id` istek-yanıt eşlemesi/teşhis içindir
// (seri protokolde zorunlu değil ama loglama + olası out-of-order tespiti için taşınır).

/// `candles` isteği: son `limit` mum, EN YENİ SONDA beklenir (EA artan sırada döndürmeli;
/// adaptör yine de sıralar). `tf` borsa-doğal TF string'i ("1m"/"1h"/"1d").
pub fn req_candles(id: u64, symbol: &str, tf: &str, limit: usize) -> String {
    json!({"id": id, "cmd": "candles", "symbol": symbol, "tf": tf, "limit": limit}).to_string()
}

/// `tick` isteği: en iyi (bid, ask).
pub fn req_tick(id: u64, symbol: &str) -> String {
    json!({"id": id, "cmd": "tick", "symbol": symbol}).to_string()
}

/// `filters` isteği: lot adımı / min lot / tick / min-notional.
pub fn req_filters(id: u64, symbol: &str) -> String {
    json!({"id": id, "cmd": "filters", "symbol": symbol}).to_string()
}

/// `order` isteği — borsa-bağımsız `OrderRequest`'i köprü sözlüğüne çevirir.
pub fn req_order(id: u64, o: &OrderRequest) -> String {
    let side = match o.side {
        crate::robot::venue::types::OrderSide::Buy => "buy",
        crate::robot::venue::types::OrderSide::Sell => "sell",
    };
    let mut obj = json!({
        "id": id, "cmd": "order", "symbol": o.symbol, "side": side,
        "qty": o.qty, "reduce_only": o.reduce_only,
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

/// `cancel_all` isteği: sembolün tüm açık emirlerini iptal.
pub fn req_cancel_all(id: u64, symbol: &str) -> String {
    json!({"id": id, "cmd": "cancel_all", "symbol": symbol}).to_string()
}

/// `set_leverage` isteği (MT5'te hesap/sembol kaldıracı; çoğu kurulumda no-op olabilir).
pub fn req_set_leverage(id: u64, symbol: &str, leverage: u32) -> String {
    json!({"id": id, "cmd": "set_leverage", "symbol": symbol, "leverage": leverage}).to_string()
}

/// `balance` isteği: hesap teminat bakiyesi (quote varlık).
pub fn req_balance(id: u64) -> String {
    json!({"id": id, "cmd": "balance"}).to_string()
}

// ───────────────────────── Yanıt çözücüleri (EA → Rust, SAF) ─────────────────────────

/// Yanıt gövdesini JSON'a çevir ve `ok` bayrağını kontrol et. `ok:false` → `error` alanını
/// taşıyan açık `Err`. Tüm parse fonksiyonlarının ortak girişi (tek-nokta hata sınıflaması).
fn ok_value(symbol: &str, body: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(body.trim())
        .map_err(|e| format!("MT5 JSON parse [{symbol}]: {e}"))?;
    if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
        Ok(v)
    } else {
        let msg = v.get("error").and_then(|m| m.as_str()).unwrap_or("bilinmeyen MT5 hatası");
        Err(format!("MT5 köprü hatası [{symbol}]: {msg}").into())
    }
}

/// `candles` yanıtını `Candle`'a parse et. `candles` dizisi `[ts_ms, open, high, low, close,
/// volume]` öğelerinden oluşur. Artan'a (en yeni sonda) sıralanır — motor son mumu güncel sayar.
pub fn parse_candles(symbol: &str, interval: &str, body: &str) -> Result<Vec<Candle>> {
    let v = ok_value(symbol, body)?;
    let list = v
        .get("candles")
        .and_then(|c| c.as_array())
        .ok_or_else(|| format!("MT5 yanıtında candles dizisi yok [{symbol}]"))?;

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
        let (open, high, low, close, volume) = (nf(1), nf(2), nf(3), nf(4), nf(5));
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

/// `tick` yanıtından (bid, ask). İkisi de pozitif olmalı.
pub fn parse_tick(symbol: &str, body: &str) -> Result<(f64, f64)> {
    let v = ok_value(symbol, body)?;
    let f = |k: &str| v.get(k).and_then(|x| x.as_f64());
    match (f("bid"), f("ask")) {
        (Some(b), Some(a)) if b > 0.0 && a > 0.0 => Ok((b, a)),
        _ => Err(format!("MT5 bid/ask alınamadı [{symbol}]").into()),
    }
}

/// `filters` yanıtından `SymbolFilters` (eksik alan → 0.0, Default ile aynı güvenli davranış).
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

/// `order` yanıtından borsa-bağımsız `OrderReceipt`. `status` MT5 string'i normalleşir.
pub fn parse_order(symbol: &str, body: &str) -> Result<OrderReceipt> {
    let v = ok_value(symbol, body)?;
    let status = match v.get("status").and_then(|s| s.as_str()).unwrap_or("") {
        "filled" => OrderStatus::Filled,
        "partial" | "partially_filled" => OrderStatus::PartiallyFilled,
        "new" | "placed" => OrderStatus::New,
        "canceled" | "cancelled" => OrderStatus::Canceled,
        "rejected" => OrderStatus::Rejected,
        _ => OrderStatus::Unknown,
    };
    let nf = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    let venue_order_id = v
        .get("order_id")
        .map(|x| x.as_str().map(|s| s.to_owned()).unwrap_or_else(|| x.to_string()));
    Ok(OrderReceipt {
        venue_order_id,
        status,
        filled_qty: nf("filled_qty"),
        avg_price: nf("avg_price"),
        raw: Some(v),
    })
}

/// `balance` yanıtından hesap teminat bakiyesi.
pub fn parse_balance(body: &str) -> Result<f64> {
    let v = ok_value("-", body)?;
    v.get("balance")
        .and_then(|x| x.as_f64())
        .ok_or_else(|| "MT5 balance alanı yok".into())
}

/// Veri döndürmeyen komutların (`cancel_all`/`set_leverage`) onay çözücüsü — yalnız `ok`
/// bayrağını doğrular. `ok:false` → `error` alanını taşıyan açık `Err` (sahte başarı yok).
pub fn parse_ack(symbol: &str, body: &str) -> Result<()> {
    ok_value(symbol, body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::venue::types::OrderSide;

    #[test]
    fn request_lines_are_single_line_json() {
        let r = req_candles(1, "EURUSD", "1h", 200);
        assert!(!r.contains('\n'), "istek tek satır olmalı");
        let v: Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["cmd"], "candles");
        assert_eq!(v["symbol"], "EURUSD");
        assert_eq!(v["tf"], "1h");
        assert_eq!(v["limit"], 200);
    }

    #[test]
    fn order_request_maps_market_and_limit() {
        let m = OrderRequest::market("EURUSD", OrderSide::Buy, 0.1);
        let vm: Value = serde_json::from_str(&req_order(2, &m)).unwrap();
        assert_eq!(vm["kind"], "market");
        assert_eq!(vm["side"], "buy");
        assert_eq!(vm["qty"], 0.1);

        let l = OrderRequest {
            symbol: "XAUUSD".into(),
            side: OrderSide::Sell,
            qty: 0.5,
            kind: OrderKind::PostOnlyLimit { price: 2400.0 },
            reduce_only: true,
        };
        let vl: Value = serde_json::from_str(&req_order(3, &l)).unwrap();
        assert_eq!(vl["kind"], "limit");
        assert_eq!(vl["side"], "sell");
        assert_eq!(vl["price"], 2400.0);
        assert_eq!(vl["reduce_only"], true);
    }

    #[test]
    fn parse_candles_ascending_and_validated() {
        // EA azalan göndermiş olsa bile artan'a sıralanmalı; geçersiz OHLCV elenmeli.
        let body = r#"{"id":1,"ok":true,"candles":[
            [1718000600000, 1.085, 1.088, 1.084, 1.086, 100],
            [1718000000000, 1.080, 1.085, 1.079, 1.082, 120],
            [1718001200000, 0, 0, 0, 0, 0]
        ]}"#;
        let c = parse_candles("EURUSD", "1h", body).unwrap();
        assert_eq!(c.len(), 2, "geçersiz (0) mum elenmeli");
        assert!(c[0].timestamp < c[1].timestamp, "artan sıra");
        assert_eq!(c[1].close, 1.086);
        assert_eq!(c[0].symbol, "EURUSD");
        assert_eq!(c[0].interval, "1h");
    }

    #[test]
    fn parse_tick_requires_positive_pair() {
        assert_eq!(
            parse_tick("EURUSD", r#"{"ok":true,"bid":1.0819,"ask":1.0821}"#).unwrap(),
            (1.0819, 1.0821)
        );
        assert!(parse_tick("EURUSD", r#"{"ok":true,"bid":0,"ask":1.08}"#).is_err());
    }

    #[test]
    fn parse_filters_maps_fields() {
        let f = parse_filters(
            "EURUSD",
            r#"{"ok":true,"lot_step":0.01,"min_lot":0.01,"tick_size":0.00001,"min_notional":0}"#,
        )
        .unwrap();
        assert_eq!(f.step_size, 0.01);
        assert_eq!(f.min_qty, 0.01);
        assert_eq!(f.tick_size, 0.00001);
    }

    #[test]
    fn parse_order_normalizes_status() {
        let r = parse_order(
            "EURUSD",
            r#"{"ok":true,"order_id":"998","status":"filled","filled_qty":0.1,"avg_price":1.0821}"#,
        )
        .unwrap();
        assert_eq!(r.status, OrderStatus::Filled);
        assert_eq!(r.filled_qty, 0.1);
        assert_eq!(r.avg_price, 1.0821);
        assert_eq!(r.venue_order_id.as_deref(), Some("998"));
    }

    #[test]
    fn ok_false_surfaces_error_not_silent_success() {
        let e = parse_tick("FOO", r#"{"ok":false,"error":"symbol not found"}"#);
        assert!(e.is_err());
        assert!(format!("{}", e.unwrap_err()).contains("symbol not found"));
        // ok alanı yoksa da hata (sahte başarı yok).
        assert!(parse_balance(r#"{"balance":100}"#).is_err());
    }

    #[test]
    fn parse_balance_reads_field() {
        assert_eq!(parse_balance(r#"{"ok":true,"balance":10000.5}"#).unwrap(), 10000.5);
    }

    #[test]
    fn parse_ack_honors_ok_flag() {
        assert!(parse_ack("EURUSD", r#"{"ok":true,"canceled":2}"#).is_ok());
        let e = parse_ack("EURUSD", r#"{"ok":false,"error":"no pending orders"}"#);
        assert!(e.is_err());
        assert!(format!("{}", e.unwrap_err()).contains("no pending orders"));
    }
}
