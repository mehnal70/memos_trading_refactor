#!/usr/bin/env python3
"""Günlük BIST kapanış sinyallerini gerçek emir göndermeden yürütür."""

from __future__ import annotations

import json
import os
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
REPORT_PATH = Path(os.getenv("BIST_TREND_REPORT", ROOT / "data/bist_trend_daily.json"))
STATE_PATH = Path(os.getenv("BIST_PAPER_STATE", ROOT / "data/bist_paper_state.json"))
INITIAL_CAPITAL = float(os.getenv("BIST_PAPER_INITIAL_CAPITAL", "100000"))
COMMISSION_RATE = float(os.getenv("BIST_PAPER_COMMISSION_RATE", "0.001"))
SLIPPAGE_BPS = float(os.getenv("BIST_PAPER_SLIPPAGE_BPS", "5"))
MAX_POSITIONS = int(os.getenv("BIST_PAPER_MAX_POSITIONS", "5"))
ALLOCATION_RATE = float(os.getenv("BIST_PAPER_ALLOCATION_RATE", "0.2"))


def load_state() -> dict[str, Any]:
    if not STATE_PATH.exists():
        return {
            "version": 1,
            "last_as_of": None,
            "cash": INITIAL_CAPITAL,
            "realized_pnl": 0.0,
            "fees": 0.0,
            "positions": {},
            "events": [],
        }
    with STATE_PATH.open(encoding="utf-8") as handle:
        state = json.load(handle)
    if state.get("version") != 1:
        raise ValueError(f"Desteklenmeyen BIST paper state sürümü: {state.get('version')}")
    return state


def save_state(state: dict[str, Any]) -> None:
    STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".bist-paper-", suffix=".json", dir=STATE_PATH.parent)
    try:
        with open(fd, "w", encoding="utf-8") as handle:
            json.dump(state, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        os.replace(temporary, STATE_PATH)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def event(state: dict[str, Any], payload: dict[str, Any]) -> None:
    state["events"].append(
        {"timestamp": datetime.now(timezone.utc).isoformat(), **payload}
    )
    state["events"] = state["events"][-500:]


def main() -> None:
    with REPORT_PATH.open(encoding="utf-8") as handle:
        report = json.load(handle)
    if not isinstance(report, list):
        raise ValueError("BIST trend raporu liste olmalı")
    rows = {str(row["symbol"]).upper(): row for row in report}
    as_of_values = {str(row.get("as_of", "")) for row in report}
    if len(as_of_values) != 1 or not next(iter(as_of_values)):
        raise ValueError("BIST raporu tek ve geçerli bir as_of tarihi içermeli")
    as_of = next(iter(as_of_values))
    state = load_state()
    if state["last_as_of"] == as_of:
        print(json.dumps({"status": "SKIPPED", "as_of": as_of, "reason": "already_processed"}))
        return

    positions: dict[str, dict[str, Any]] = state["positions"]
    for symbol, position in list(positions.items()):
        row = rows.get(symbol)
        if row is None:
            continue
        action = str(row.get("action", "")).upper()
        if action not in {"EXIT", "EXIT_WARNING"}:
            continue
        close = float(row["close"])
        qty = int(position["qty"])
        exit_price = close * (1 - SLIPPAGE_BPS / 10000)
        gross = (exit_price - float(position["entry_price"])) * qty
        fee = exit_price * qty * COMMISSION_RATE
        state["cash"] += exit_price * qty - fee
        state["realized_pnl"] += gross - fee - float(position["entry_fee"])
        state["fees"] += fee
        event(state, {"type": "SELL", "symbol": symbol, "qty": qty, "price": exit_price, "reason": action})
        del positions[symbol]

    for symbol, row in rows.items():
        action = str(row.get("action", "")).upper()
        if action not in {"BUY", "ENTER", "ENTRY_CANDIDATE"} or symbol in positions or len(positions) >= MAX_POSITIONS:
            continue
        close = float(row["close"])
        entry_price = close * (1 + SLIPPAGE_BPS / 10000)
        allocation = min(state["cash"], INITIAL_CAPITAL * ALLOCATION_RATE)
        qty = int(allocation // entry_price)
        if qty <= 0:
            continue
        fee = entry_price * qty * COMMISSION_RATE
        state["cash"] -= entry_price * qty + fee
        state["fees"] += fee
        positions[symbol] = {"qty": qty, "entry_price": entry_price, "entry_fee": fee, "as_of": as_of}
        event(state, {"type": "BUY", "symbol": symbol, "qty": qty, "price": entry_price, "reason": action})

    state["last_as_of"] = as_of
    save_state(state)
    market_value = sum(int(p["qty"]) * float(rows[s]["close"]) for s, p in positions.items() if s in rows)
    print(json.dumps({
        "status": "PROCESSED",
        "as_of": as_of,
        "cash": round(state["cash"], 2),
        "market_value": round(market_value, 2),
        "equity": round(state["cash"] + market_value, 2),
        "realized_pnl": round(state["realized_pnl"], 2),
        "fees": round(state["fees"], 2),
        "open_positions": len(positions),
    }))


if __name__ == "__main__":
    main()
