#!/usr/bin/env python3
"""BIST signal-only sembol evrenini veri kalitesine göre günceller."""

from __future__ import annotations

import json
import os
import sqlite3
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DB_PATH = Path(os.environ.get("DB_PATH", ROOT / "data" / "trader.db"))
PROFILE_PATH = Path(os.environ.get("BIST_PROFILE_PATH", ROOT / "profiles" / "paper-bist-signal.conf"))
STATE_PATH = Path(os.environ.get("BIST_AUTO_PIN_STATE", ROOT / "logs/.venue-runtime/state/bist-auto-pin.json"))
MIN_COUNT = max(1, int(os.environ.get("BIST_AUTO_PIN_MIN", "8")))
MAX_COUNT = max(MIN_COUNT, int(os.environ.get("BIST_AUTO_PIN_MAX", "20")))


def atomic_write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as handle:
        handle.write(content)
        temporary = Path(handle.name)
    temporary.replace(path)


def candidates() -> list[dict]:
    with sqlite3.connect(DB_PATH, timeout=5) as db:
        rows = db.execute(
            """
            SELECT symbol, COUNT(*) AS bars,
                   MAX(timestamp) AS latest,
                   AVG(volume) AS avg_volume,
                   (MAX(close) - MIN(close)) AS range_proxy
            FROM candles
            WHERE lower(exchange) = 'bist' AND interval = '1d'
            GROUP BY symbol
            HAVING COUNT(*) >= 60
            """
        ).fetchall()
    scored = []
    for symbol, bars, latest, avg_volume, range_proxy in rows:
        score = min(float(bars) / 1000.0, 1.0) * 0.35
        score += min(float(avg_volume or 0) / 50_000_000.0, 1.0) * 0.35
        score += min(abs(float(range_proxy or 0)) / 100.0, 1.0) * 0.30
        scored.append({"symbol": symbol, "bars": bars, "latest": latest, "score": round(score, 6)})
    scored.sort(key=lambda item: (-item["score"], item["symbol"]))
    quota = min(MAX_COUNT, max(MIN_COUNT, len(scored)))
    return scored[:quota]


def update_profile(symbols: list[str]) -> bool:
    if not PROFILE_PATH.exists():
        raise FileNotFoundError(PROFILE_PATH)
    content = PROFILE_PATH.read_text(encoding="utf-8")
    values = ",".join(f"{symbol}@bist" for symbol in symbols)
    lines = content.splitlines()
    replacements = {
        "PINNED_SYMBOLS": values,
        "AUTO_BIST_SYMBOLS": values,
    }
    changed = False
    for index, line in enumerate(lines):
        key = line.split("=", 1)[0]
        if key in replacements:
            replacement = f"{key}={replacements[key]}"
            changed |= line != replacement
            lines[index] = replacement
    if changed:
        atomic_write(PROFILE_PATH, "\n".join(lines) + "\n")
    return changed


def main() -> int:
    selected = candidates()
    symbols = [item["symbol"] for item in selected]
    changed = update_profile(symbols)
    atomic_write(
        STATE_PATH,
        json.dumps(
            {
                "venue": "bist:spot",
                "signal_only": True,
                "min_quota": MIN_COUNT,
                "max_quota": MAX_COUNT,
                "selected_count": len(symbols),
                "symbols": [f"{symbol}@bist" for symbol in symbols],
                "candidates": selected,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
    )
    print(f"BIST auto-pin: {len(symbols)} sembol ({MIN_COUNT}-{MAX_COUNT}), changed={int(changed)}")
    return 10 if changed else 0


if __name__ == "__main__":
    raise SystemExit(main())
