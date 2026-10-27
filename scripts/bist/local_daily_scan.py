#!/usr/bin/env python3
"""Yerel Yahoo CSV önbelleğinden günlük BIST trend raporu üretir."""

from __future__ import annotations

import csv
import json
import os
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CSV_DIR = Path(os.getenv("BIST_LOCAL_CSV_DIR", ROOT / "data/bist_yahoo_csv"))
SYMBOLS = Path(os.getenv("BIST_SYMBOLS_FILE", ROOT / "config/bist100_symbols.txt"))
REPORT = Path(os.getenv("BIST_TREND_REPORT", ROOT / "data/bist_trend_daily.json"))


def ema(values: list[float], period: int) -> float:
    result = values[0]
    alpha = 2 / (period + 1)
    for value in values[1:]:
        result = alpha * value + (1 - alpha) * result
    return result


def scan(symbol: str) -> dict:
    with (CSV_DIR / f"{symbol}.csv").open(newline="", encoding="utf-8") as handle:
        rows = list(csv.DictReader(handle))[-150:]
    if len(rows) < 100:
        raise ValueError(f"{symbol}: en az 100 günlük mum gerekli")
    closes = [float(row["Close"]) for row in rows]
    highs = [float(row["High"]) for row in rows]
    lows = [float(row["Low"]) for row in rows]
    close = closes[-1]
    ema50 = ema(closes, 50)
    ema100 = ema(closes, 100)
    true_ranges = [
        max(highs[i] - lows[i], abs(highs[i] - closes[i - 1]), abs(lows[i] - closes[i - 1]))
        for i in range(1, len(rows))
    ]
    atr14 = sum(true_ranges[-14:]) / 14
    previous_high = max(highs[-21:-1])
    uptrend = close > ema50 and ema50 > ema100
    breakout = close > previous_high
    action = "ENTRY_CANDIDATE" if uptrend and breakout else "WATCH" if uptrend else "EXIT_WARNING" if close < ema50 else "HOLD"
    as_of = datetime.fromisoformat(rows[-1]["Date"]).replace(tzinfo=timezone.utc).isoformat().replace("+00:00", "Z")
    return {"symbol": symbol, "as_of": as_of, "close": close, "ema50": ema50, "ema100": ema100,
            "atr14": atr14, "donchian20_previous_high": previous_high,
            "trend": "UPTREND" if uptrend else "NO_UPTREND", "action": action,
            "reason": "yerel Yahoo CSV günlük kapanış taraması"}


def main() -> None:
    symbols = [line.strip().upper() for line in SYMBOLS.read_text().splitlines() if line.strip()]
    decisions = [scan(symbol) for symbol in symbols if (CSV_DIR / f"{symbol}.csv").exists()]
    if not decisions:
        raise RuntimeError(f"Yerel BIST CSV bulunamadı: {CSV_DIR}")
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(decisions, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Yerel BIST taraması: {len(decisions)} sembol → {REPORT}")


if __name__ == "__main__":
    main()
