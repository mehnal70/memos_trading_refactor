#!/usr/bin/env python3
"""
IBKR -> Memos NDJSON bridge.

Core expects line-delimited JSON requests on IBKR_BRIDGE_ADDR (default 127.0.0.1:9101).
This bridge connects to IB Gateway/TWS socket API and answers the protocol used by
memos_trading_core/src/robot/venue/ibkr/protocol.rs.

Supported commands:
- ping
- health
- candles
- tick
- filters
- order
- cancel_all
- set_leverage (ACK/no-op)
- balance
"""

from __future__ import annotations

import json
import math
import os
import asyncio
import socketserver
import sys
import threading
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Dict, List, Tuple

from ib_insync import IB, CFD, Forex, MarketOrder, LimitOrder, StopOrder, Stock


@dataclass
class BridgeConfig:
    api_host: str
    api_port: int
    client_id: int
    bind_host: str
    bind_port: int
    account_ccy: str
    read_only: bool
    allowed_account: str


def parse_bind(addr: str) -> Tuple[str, int]:
    host, port = addr.split(":", 1)
    return host.strip(), int(port.strip())


def load_config() -> BridgeConfig:
    bind = os.getenv("IBKR_BRIDGE_ADDR", "127.0.0.1:9101")
    bind_host, bind_port = parse_bind(bind)
    return BridgeConfig(
        api_host=os.getenv("IBKR_API_HOST", "127.0.0.1"),
        api_port=int(os.getenv("IBKR_API_PORT", "4002")),
        client_id=int(os.getenv("IBKR_CLIENT_ID", "42")),
        bind_host=bind_host,
        bind_port=bind_port,
        account_ccy=os.getenv("IBKR_ACCOUNT_CCY", "USD"),
        read_only=os.getenv("IBKR_READ_ONLY", "1").strip().lower() in {"1", "true", "yes", "on"},
        allowed_account=os.getenv("IBKR_ALLOWED_ACCOUNT", "").strip().upper(),
    )


class IbkrService:
    def __init__(self, cfg: BridgeConfig) -> None:
        self.cfg = cfg
        self.ib = IB()
        self.loop = asyncio.get_event_loop()
        self.lock = threading.Lock()
        self.account_refresh_requested = threading.Event()
        self.account_timeout_sec = max(
            1.0, float(os.getenv("IBKR_ACCOUNT_TIMEOUT_SEC", "8"))
        )
        self.historical_timeout_sec = max(
            1.0, float(os.getenv("IBKR_HISTORICAL_TIMEOUT_SEC", "30"))
        )
        self.historical_failure_threshold = max(
            1, int(os.getenv("IBKR_HISTORICAL_FAILURE_THRESHOLD", "3"))
        )
        self.historical_cooldown_sec = max(
            1.0, float(os.getenv("IBKR_HISTORICAL_COOLDOWN_SEC", "60"))
        )
        self.historical_failures = 0
        self.historical_blocked_until = 0.0
        self.farm_health = {
            "market_data": True,
            "historical": True,
            "security_definition": True,
        }
        self.ib.errorEvent += self._on_ibkr_error

    @staticmethod
    def _log_event(event: str, **fields: Any) -> None:
        payload = {"event": event, **fields}
        print(json.dumps(payload, ensure_ascii=True, sort_keys=True), file=sys.stderr, flush=True)

    def _on_ibkr_error(
        self,
        req_id: int,
        error_code: int,
        error_string: str,
        contract: Any,
    ) -> None:
        farm_codes = {
            2103: ("market_data", False),
            2104: ("market_data", True),
            2105: ("historical", False),
            2106: ("historical", True),
            2157: ("security_definition", False),
            2156: ("security_definition", True),
            2155: ("security_definition", False),
        }
        if error_code in farm_codes:
            farm, healthy = farm_codes[error_code]
            self.farm_health[farm] = healthy
        self._log_event(
            "ibkr_error",
            req_id=req_id,
            error_code=error_code,
            error_string=error_string,
            symbol=getattr(contract, "symbol", None),
            sec_type=getattr(contract, "secType", None),
            exchange=getattr(contract, "exchange", None),
        )

    def ensure_allowed_account(self) -> None:
        if not self.cfg.allowed_account:
            raise RuntimeError("IBKR_ALLOWED_ACCOUNT tanimli degil; account allowlist olmadan islem kapali")
        accounts = {str(account).strip().upper() for account in self.ib.managedAccounts()}
        if self.cfg.allowed_account not in accounts:
            raise RuntimeError("IBKR aktif account allowlist ile eslesmiyor; islem kapali")

    def connect(self) -> None:
        candidates = [self.cfg.client_id]
        if os.getenv("IBKR_CLIENT_ID_FALLBACK", "1").strip().lower() in {"1", "true", "yes", "on"}:
            candidates.extend(self.cfg.client_id + offset for offset in range(1, 6))
        last_error: Exception | None = None
        for client_id in candidates:
            original_req_executions = self.ib.reqExecutionsAsync
            if self.cfg.read_only:
                async def skip_execution_sync() -> list[Any]:
                    return []

                self.ib.reqExecutionsAsync = skip_execution_sync  # type: ignore[method-assign]
            try:
                self.ib.connect(
                    self.cfg.api_host,
                    self.cfg.api_port,
                    clientId=client_id,
                    readonly=self.cfg.read_only,
                )
                if client_id != self.cfg.client_id:
                    print(f"IBKR clientId {self.cfg.client_id} kullanımda; fallback clientId={client_id} seçildi", flush=True)
                self.cfg.client_id = client_id
                self.ensure_allowed_account()
                return
            except Exception as exc:
                last_error = exc
                if "client id is already in use" not in str(exc).lower():
                    raise
                if self.ib.isConnected():
                    self.ib.disconnect()
            finally:
                self.ib.reqExecutionsAsync = original_req_executions  # type: ignore[method-assign]
        raise last_error or RuntimeError("IBKR bağlantısı kurulamadı")

    def ensure_connected(self) -> None:
        if self.ib.isConnected():
            return
        attempts = max(1, int(os.getenv("IBKR_RECONNECT_ATTEMPTS", "3")))
        delay = max(0.1, float(os.getenv("IBKR_RECONNECT_DELAY_SEC", "2")))
        last_error: Exception | None = None
        for attempt in range(1, attempts + 1):
            try:
                self.connect()
                return
            except Exception as exc:
                last_error = exc
                if attempt < attempts:
                    time.sleep(delay)
        raise last_error or RuntimeError("IBKR yeniden bağlantısı kurulamadı")

    def contract_for_symbol(self, symbol: str):
        sym = symbol.strip().upper()
        if sym in {"XAUUSD", "XAGUSD", "USOIL", "UKOIL"}:
            return CFD(sym, "SMART", self.cfg.account_ccy)
        if len(sym) == 6 and sym.isalpha():
            return Forex(sym)
        if sym.endswith("USD") and len(sym) > 6:
            # Example: XAUUSD as CFD instrument name.
            return CFD(sym, "SMART", self.cfg.account_ccy)
        return Stock(sym, "SMART", self.cfg.account_ccy)

    def cmd_ping(self, _: Dict[str, Any]) -> Dict[str, Any]:
        self.ensure_connected()
        return {"ok": True}

    def cmd_health(self, _: Dict[str, Any]) -> Dict[str, Any]:
        self.ensure_connected()
        return {
            "ok": True,
            "connected": self.ib.isConnected(),
            "farms": dict(self.farm_health),
        }

    def cmd_tick(self, req: Dict[str, Any]) -> Dict[str, Any]:
        symbol = str(req.get("symbol", "")).strip()
        if not symbol:
            return {"ok": False, "error": "symbol bos"}
        self.ensure_connected()
        c = self.contract_for_symbol(symbol)
        with self.lock:
            # Canlı abonelik yoksa delayed quote ile devam edebilmek için.
            self.ib.reqMarketDataType(3)
            ticker = self.ib.reqTickers(c)[0]

        def finite_pos(v: Any) -> float:
            try:
                x = float(v)
            except Exception:
                return 0.0
            return x if math.isfinite(x) and x > 0.0 else 0.0

        bid = finite_pos(ticker.bid)
        ask = finite_pos(ticker.ask)
        if bid <= 0 or ask <= 0:
            last = finite_pos(ticker.last)
            if last > 0:
                pad = max(last * 0.0001, 1e-6)
                bid = last - pad
                ask = last + pad
        if bid <= 0 or ask <= 0:
            return {"ok": False, "error": f"tick yok: {symbol}"}
        return {"ok": True, "bid": bid, "ask": ask}

    def _tf_to_bar_size(self, tf: str) -> str:
        tf = tf.lower().strip()
        mapping = {
            "1m": "1 min",
            "5m": "5 mins",
            "15m": "15 mins",
            "30m": "30 mins",
            "1h": "1 hour",
            "4h": "4 hours",
            "1d": "1 day",
        }
        return mapping.get(tf, "1 hour")

    def _duration_for_limit(self, tf: str, limit: int) -> str:
        tf = tf.lower().strip()
        unit_minutes = {
            "1m": 1,
            "5m": 5,
            "15m": 15,
            "30m": 30,
            "1h": 60,
            "4h": 240,
            "1d": 1440,
        }.get(tf, 60)
        total_minutes = max(1, limit) * unit_minutes
        days = max(1, math.ceil(total_minutes / (24 * 60)))
        if days > 365:
            years = math.ceil(days / 365)
            return f"{years} Y"
        return f"{days} D"

    @staticmethod
    def _parse_bar_datetime(value: Any) -> datetime | None:
        if isinstance(value, datetime):
            return value
        if not isinstance(value, str):
            return None
        raw = value.strip()
        for fmt in ("%Y%m%d  %H:%M:%S", "%Y%m%d %H:%M:%S", "%Y%m%d"):
            try:
                return datetime.strptime(raw, fmt)
            except ValueError:
                continue
        try:
            return datetime.fromisoformat(raw.replace(" ", "T"))
        except ValueError:
            return None

    def cmd_candles(self, req: Dict[str, Any]) -> Dict[str, Any]:
        symbol = str(req.get("symbol", "")).strip()
        tf = str(req.get("tf", "1h"))
        limit = int(req.get("limit", 200))
        if not symbol:
            return {"ok": False, "error": "symbol bos"}
        self.ensure_connected()
        c = self.contract_for_symbol(symbol)
        bar_size = self._tf_to_bar_size(tf)
        duration = self._duration_for_limit(tf, limit)
        end_timestamp_ms = req.get("end_timestamp_ms")
        end_datetime = ""
        if end_timestamp_ms is not None:
            try:
                end_dt = datetime.fromtimestamp(int(end_timestamp_ms) / 1000, tz=timezone.utc)
                end_datetime = end_dt.strftime("%Y%m%d %H:%M:%S UTC")
            except (TypeError, ValueError, OverflowError) as exc:
                return {"ok": False, "error": f"end_timestamp_ms geçersiz: {exc}"}
        started = time.monotonic()
        if self.account_refresh_requested.is_set():
            self._log_event(
                "historical_request_deferred",
                symbol=symbol,
                reason="account_refresh_priority",
            )
            return {
                "ok": False,
                "error": "IBKR account refresh önceliği nedeniyle historical istek ertelendi",
            }
        with self.lock:
            if self.account_refresh_requested.is_set():
                self._log_event(
                    "historical_request_deferred",
                    symbol=symbol,
                    reason="account_refresh_priority",
                )
                return {
                    "ok": False,
                    "error": "IBKR account refresh önceliği nedeniyle historical istek ertelendi",
                }
            now = time.monotonic()
            if now < self.historical_blocked_until:
                remaining = round(self.historical_blocked_until - now, 3)
                self._log_event(
                    "historical_request_blocked",
                    symbol=symbol,
                    remaining_seconds=remaining,
                    failure_streak=self.historical_failures,
                )
                return {
                    "ok": False,
                    "error": f"IBKR historical circuit breaker aktif ({remaining}s)",
                }
            if not self.farm_health["historical"]:
                return {
                    "ok": False,
                    "error": "IBKR historical farm sağlıksız",
                }
        self._log_event(
            "historical_request_start",
            symbol=symbol,
            tf=tf,
            limit=limit,
            end_datetime=end_datetime or "latest",
            duration=duration,
            bar_size=bar_size,
            what_to_show="MIDPOINT",
            use_rth=False,
        )
        try:
            with self.lock:
                bars = self.ib.reqHistoricalData(
                    c,
                    endDateTime=end_datetime,
                    durationStr=duration,
                    barSizeSetting=bar_size,
                    whatToShow="MIDPOINT",
                    useRTH=False,
                    formatDate=1,
                    keepUpToDate=False,
                    timeout=self.historical_timeout_sec,
                )
                self.historical_failures = 0
        except Exception as exc:
            with self.lock:
                self.historical_failures += 1
                if self.historical_failures >= self.historical_failure_threshold:
                    self.historical_blocked_until = (
                        time.monotonic() + self.historical_cooldown_sec
                    )
            self._log_event(
                "historical_request_failed",
                symbol=symbol,
                error=str(exc),
                failure_streak=self.historical_failures,
                circuit_breaker=(
                    self.historical_failures >= self.historical_failure_threshold
                ),
            )
            return {"ok": False, "error": f"IBKR historical isteği başarısız: {exc}"}
        self._log_event(
            "historical_request_end",
            symbol=symbol,
            tf=tf,
            returned_bars=len(bars),
            elapsed_seconds=round(time.monotonic() - started, 3),
        )
        out: List[List[float]] = []
        for b in bars[-limit:]:
            parsed = self._parse_bar_datetime(b.date)
            if parsed is None:
                continue
            if parsed.tzinfo is None:
                parsed = parsed.replace(tzinfo=timezone.utc)
            ts_ms = int(parsed.timestamp() * 1000)
            out.append([
                ts_ms,
                float(b.open),
                float(b.high),
                float(b.low),
                float(b.close),
                float(b.volume or 0.0),
            ])
        return {"ok": True, "candles": out}

    def cmd_filters(self, req: Dict[str, Any]) -> Dict[str, Any]:
        symbol = str(req.get("symbol", "")).strip()
        if not symbol:
            return {"ok": False, "error": "symbol bos"}
        # IB tarafinda lot step/min_notional her enstrumanda tek tip degil.
        # Core bu alanlari bilgilendirici olarak kullaniyor; no-op defaults guvenli.
        return {
            "ok": True,
            "lot_step": 0.0,
            "min_lot": 0.0,
            "tick_size": 0.0,
            "min_notional": 0.0,
        }

    def cmd_balance(self, _: Dict[str, Any]) -> Dict[str, Any]:
        self.ensure_connected()
        self.account_refresh_requested.set()
        try:
            cached_values = self.ib.accountValues()
            cached_balance = self._net_liquidation(cached_values)
            if cached_balance is not None:
                return {"ok": True, "balance": cached_balance, "source": "account_values_cache"}
            with self.lock:
                try:
                    vals = self.loop.run_until_complete(
                        asyncio.wait_for(
                            self.ib.accountSummaryAsync(),
                            timeout=self.account_timeout_sec,
                        )
                    )
                except asyncio.TimeoutError as exc:
                    self._log_event(
                        "account_summary_timeout",
                        timeout_seconds=self.account_timeout_sec,
                        connected=self.ib.isConnected(),
                    )
                    raise RuntimeError(
                        f"IBKR account summary timeout after {self.account_timeout_sec:.1f}s"
                    ) from exc
        finally:
            self.account_refresh_requested.clear()
        bal = self._net_liquidation(vals)
        if bal is None:
            raise RuntimeError("IBKR NetLiquidation değeri account summary içinde bulunamadı")
        return {"ok": True, "balance": bal, "source": "account_summary"}

    def _net_liquidation(self, values: Any) -> float | None:
        for v in values:
            if v.tag == "NetLiquidation" and (v.currency == self.cfg.account_ccy or v.currency == "BASE"):
                try:
                    return float(v.value)
                except Exception:
                    continue
        return None

    def cmd_order(self, req: Dict[str, Any]) -> Dict[str, Any]:
        if self.cfg.read_only:
            return {"ok": False, "error": "IBKR bridge read-only: order devre disi"}
        symbol = str(req.get("symbol", "")).strip()
        side = str(req.get("side", "buy")).lower().strip()
        qty = float(req.get("qty", 0.0))
        kind = str(req.get("kind", "market")).lower().strip()
        reduce_only = bool(req.get("reduce_only", False))
        if not symbol or qty <= 0:
            return {"ok": False, "error": "symbol/qty gecersiz"}
        if side not in ("buy", "sell"):
            return {"ok": False, "error": "side gecersiz"}

        self.ensure_connected()
        action = "BUY" if side == "buy" else "SELL"
        price = float(req.get("price", 0.0))
        if kind == "limit" and price <= 0:
            return {"ok": False, "error": "limit price gecersiz"}

        with self.lock:
            if reduce_only:
                positions = [
                    p
                    for p in self.ib.positions()
                    if p.contract.localSymbol.upper() == symbol.upper()
                    and (
                        (side == "sell" and float(p.position) > 0)
                        or (side == "buy" and float(p.position) < 0)
                    )
                ]
                available = sum(abs(float(p.position)) for p in positions)
                if not positions or qty > available + 1e-9:
                    return {
                        "ok": False,
                        "error": f"reduce_only pozisyon bulunamadı veya miktar fazla: {symbol} qty={qty} available={available}",
                    }
                position = positions[0]
                if len(positions) != 1 or abs(float(position.position)) + 1e-9 < qty:
                    return {
                        "ok": False,
                        "error": f"reduce_only çoklu/dağıtık pozisyon için güvenli tekil contract eşleşmesi yok: {symbol}",
                    }
                c = position.contract
                if c.secType == "CFD" and not c.exchange:
                    c.exchange = "SMART"
                elif c.secType == "STK" and c.exchange != "SMART":
                    c.exchange = "SMART"
                if kind == "limit":
                    order = LimitOrder(action, qty, price, account=position.account, tif="GTC")
                else:
                    order = MarketOrder(action, qty, account=position.account, tif="GTC")
            else:
                c = self.contract_for_symbol(symbol)
                if kind == "limit":
                    order = LimitOrder(action, qty, price, tif="GTC")
                else:
                    order = MarketOrder(action, qty, tif="GTC")
            trade = self.ib.placeOrder(c, order)

        deadline = time.time() + 8.0
        while time.time() < deadline:
            st = (trade.orderStatus.status or "").lower()
            if st in ("filled", "submitted", "presubmitted", "cancelled", "inactive", "rejected"):
                break
            time.sleep(0.2)

        st = (trade.orderStatus.status or "").lower()
        status = trade.orderStatus
        log_messages = [
            str(getattr(log, "message", "")).strip()
            for log in getattr(trade, "log", [])
            if str(getattr(log, "message", "")).strip()
        ]
        warning_text = str(getattr(status, "warningText", "") or "").strip()
        if warning_text:
            log_messages.append(warning_text)
        return {
            "ok": True,
            "order_id": str(trade.order.orderId),
            "status": st or "unknown",
            "filled_qty": float(trade.orderStatus.filled or 0.0),
            "avg_price": float(trade.orderStatus.avgFillPrice or 0.0),
            "remaining_qty": float(trade.orderStatus.remaining or 0.0),
            "reason": log_messages[-1] if log_messages else "",
            "messages": log_messages[-5:],
        }

    def cmd_cancel_all(self, _: Dict[str, Any]) -> Dict[str, Any]:
        if self.cfg.read_only:
            return {"ok": False, "error": "IBKR bridge read-only: cancel_all devre disi"}
        self.ensure_connected()
        with self.lock:
            self.ib.reqGlobalCancel()
            for trade in self.ib.trades():
                if trade.isActive():
                    self.ib.cancelOrder(trade.order)
            self.ib.sleep(0.5)
        return {"ok": True}

    def cmd_protection(self, req: Dict[str, Any]) -> Dict[str, Any]:
        if self.cfg.read_only:
            return {"ok": False, "error": "IBKR bridge read-only: protection devre disi"}
        symbol = str(req.get("symbol", "")).strip()
        qty = float(req.get("qty", 0.0))
        stop_loss = float(req.get("stop_loss", 0.0))
        take_profit = float(req.get("take_profit", 0.0))
        is_long = bool(req.get("is_long", True))
        if not symbol or qty <= 0 or stop_loss <= 0 or take_profit <= 0:
            return {"ok": False, "error": "protection sembol/miktar/seviye gecersiz"}

        self.ensure_connected()
        action = "SELL" if is_long else "BUY"
        with self.lock:
            positions = [
                p for p in self.ib.positions()
                if p.contract.localSymbol.upper() == symbol.upper()
                and ((is_long and float(p.position) > 0) or (not is_long and float(p.position) < 0))
            ]
            if len(positions) != 1 or abs(float(positions[0].position)) + 1e-9 < qty:
                return {"ok": False, "error": f"protection için tekil açık pozisyon bulunamadı: {symbol}"}
            position = positions[0]
            contract = position.contract
            if contract.secType == "CFD" and not contract.exchange:
                contract.exchange = "SMART"
            elif contract.secType == "STK" and contract.exchange != "SMART":
                contract.exchange = "SMART"
            group = f"memos_protection_{time.time_ns()}"
            stop = StopOrder(action, qty, stop_loss, account=position.account, tif="GTC")
            target = LimitOrder(action, qty, take_profit, account=position.account, tif="GTC")
            for order in (stop, target):
                order.ocaGroup = group
                order.ocaType = 1
            stop.transmit = False
            target.transmit = True
            stop_trade = self.ib.placeOrder(contract, stop)
            target_trade = self.ib.placeOrder(contract, target)
        return {
            "ok": True,
            "status": "submitted",
            "order_id": f"{stop_trade.order.orderId},{target_trade.order.orderId}",
            "filled_qty": 0.0,
            "avg_price": 0.0,
        }

    def cmd_set_leverage(self, _: Dict[str, Any]) -> Dict[str, Any]:
        # IB tarafinda kaldirac hesap/urun bazli; runtime set evrensel degil.
        return {"ok": True}

    def cmd_open_orders(self, _: Dict[str, Any]) -> Dict[str, Any]:
        self.ensure_connected()
        orders_list = []
        with self.lock:
            # IB-Insync kütüphanesinin aktif bekleyen emir (Trades) listesini çekiyoruz
            trades = self.ib.trades()
            for t in trades:
                if t.isActive():
                    orders_list.append({
                        "orderId": str(t.order.orderId),
                        "symbol": str(t.contract.localSymbol),
                        "secType": str(t.contract.secType),
                        "orderType": str(t.order.orderType),
                        "side": "buy" if t.order.action.lower() in ["buy", "bot"] else "sell",
                        "qty": float(t.order.totalQuantity or 0.0),
                        "price": float(t.order.lmtPrice or 0.0),
                        "stop_price": float(t.order.auxPrice or 0.0),
                        "status": str(t.orderStatus.status or "unknown").lower(),
                        "filled_qty": float(t.orderStatus.filled or 0.0),
                        "remaining_qty": float(t.orderStatus.remaining or 0.0),
                        "account": str(t.order.account or "")
                    })
        return {"ok": True, "open_orders": orders_list}

    def cmd_account_positions(self, _: Dict[str, Any]) -> Dict[str, Any]:
        self.ensure_connected()
        pos_list = []
        with self.lock:
            # Pozisyon isteğini event loop üzerinden yenileyip cache'den oku.
            # Doğrudan positions() çağrısı, boş/yeniden bağlanan oturumlarda
            # senkron beklemede kalıp bridge'in ardındaki tüm istekleri bloke edebilir.
            self.ib.reqPositions()
            self.ib.sleep(float(os.getenv("IBKR_POSITIONS_WAIT_SEC", "1.0")))
            positions = self.ib.positions()
            for p in positions:
                pos_list.append({
                    "symbol": str(p.contract.localSymbol),
                    "positionAmt": str(p.position),
                    "entryPrice": float(p.avgCost or 0.0)
                })
        return {"ok": True, "account_positions": pos_list}


    def handle(self, req: Dict[str, Any]) -> Dict[str, Any]:
        cmd = str(req.get("cmd", "")).strip().lower()
        # bridge_ibkr.py içindeki fn_map yönlendirme haritasına eklenecek kılcal kancalar:
        fn_map = {
            "ping": self.cmd_ping,
            "health": self.cmd_health,
            "tick": self.cmd_tick,
            "candles": self.cmd_candles,
            "filters": self.cmd_filters,
            "balance": self.cmd_balance,
            "order": self.cmd_order,
            "protection": self.cmd_protection,
            "cancel_all": self.cmd_cancel_all,
            "set_leverage": self.cmd_set_leverage,
            # ─── 🛰️ OTONOM MUTABAKAT KANCALARI ───────────────────────────────────────────────
            "open_orders": self.cmd_open_orders,
            "account_positions": self.cmd_account_positions,
        }
        fn = fn_map.get(cmd)
        if fn is None:
            return {"ok": False, "error": f"bilinmeyen cmd: {cmd}"}
        return fn(req)


class BridgeTCPServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


class RequestHandler(socketserver.StreamRequestHandler):
    def handle(self) -> None:
        service: IbkrService = self.server.service  # type: ignore[attr-defined]
        asyncio.set_event_loop(service.loop)
        while True:
            raw = self.rfile.readline()
            if not raw:
                break
            line = raw.decode("utf-8", errors="ignore").strip()
            if not line:
                continue
            req_id: Any = None
            try:
                req = json.loads(line)
                req_id = req.get("id")
                body = service.handle(req)
            except Exception as e:
                body = {"ok": False, "error": f"bridge exception: {e}"}

            if req_id is not None:
                body["id"] = req_id
            payload = (json.dumps(body, ensure_ascii=True) + "\n").encode("utf-8")
            self.wfile.write(payload)
            self.wfile.flush()


def main() -> None:
    cfg = load_config()
    svc = IbkrService(cfg)
    svc.connect()

    # ib_insync tek event-loop bağlamında daha stabil çalıştığı için istekleri
    # tek-thread TCP sunucusunda sıralı işleriz.
    with BridgeTCPServer((cfg.bind_host, cfg.bind_port), RequestHandler) as srv:
        srv.service = svc  # type: ignore[attr-defined]
        print(f"IBKR bridge listening on {cfg.bind_host}:{cfg.bind_port} -> {cfg.api_host}:{cfg.api_port}")
        srv.serve_forever()


if __name__ == "__main__":
    main()
