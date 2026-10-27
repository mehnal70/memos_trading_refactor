#!/usr/bin/env python3
"""BIST candle namespace audit; read-only and non-destructive."""

from __future__ import annotations

import argparse
import json
import sqlite3
from collections import defaultdict
from pathlib import Path


def normalize_symbol(symbol: str) -> str:
    return symbol.strip().upper().removesuffix("@BIST").removesuffix(".IS")


def audit(db_path: Path) -> dict[str, object]:
    connection = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    try:
        rows = connection.execute(
            """
            SELECT profile, exchange, market, symbol, interval, timestamp,
                   open, high, low, close, volume
            FROM candles
            WHERE LOWER(exchange) = 'bist'
            ORDER BY profile, market, symbol, interval, timestamp
            """
        ).fetchall()
    finally:
        connection.close()

    groups: dict[tuple[str, str, str, str], dict[str, list[sqlite3.Row]]] = defaultdict(
        lambda: defaultdict(list)
    )
    for row in rows:
        key = (
            row["profile"],
            row["market"],
            normalize_symbol(row["symbol"]),
            row["interval"],
        )
        groups[key][row["symbol"]].append(row)

    comparisons: list[dict[str, object]] = []
    for (profile, market, normalized, interval), symbols in groups.items():
        if len(symbols) < 2:
            continue
        by_symbol = {
            symbol: {row["timestamp"]: row for row in symbol_rows}
            for symbol, symbol_rows in symbols.items()
        }
        timestamps = set().union(*(values.keys() for values in by_symbol.values()))
        overlapping = 0
        value_mismatches = 0
        for timestamp in timestamps:
            candidates = [
                values[timestamp]
                for values in by_symbol.values()
                if timestamp in values
            ]
            if len(candidates) < 2:
                continue
            overlapping += 1
            first = candidates[0]
            if any(
                any(first[field] != candidate[field] for field in ("open", "high", "low", "close", "volume"))
                for candidate in candidates[1:]
            ):
                value_mismatches += 1
        comparisons.append(
            {
                "profile": profile,
                "market": market,
                "normalized_symbol": normalized,
                "interval": interval,
                "symbols": sorted(by_symbol),
                "rows_by_symbol": {symbol: len(values) for symbol, values in by_symbol.items()},
                "overlapping_timestamps": overlapping,
                "value_mismatches": value_mismatches,
            }
        )

    return {
        "database": str(db_path),
        "scope": "exchange=bist",
        "total_rows": len(rows),
        "normalized_groups": len(groups),
        "groups_with_multiple_namespaces": len(comparisons),
        "comparisons": comparisons,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--db",
        type=Path,
        default=Path("data/profiles/soak-bist.db"),
        help="SQLite database path",
    )
    parser.add_argument("--json", action="store_true", help="Print JSON only")
    args = parser.parse_args()

    if not args.db.is_file():
        parser.error(f"database not found: {args.db}")

    report = audit(args.db)
    if args.json:
        print(json.dumps(report, ensure_ascii=True, indent=2))
        return 0

    print(f"Database: {report['database']}")
    print(f"BIST rows: {report['total_rows']}")
    print(f"Normalized groups: {report['normalized_groups']}")
    print(f"Groups with multiple namespaces: {report['groups_with_multiple_namespaces']}")
    for comparison in report["comparisons"]:
        print(
            f"- {comparison['normalized_symbol']} [{comparison['interval']}] "
            f"namespaces={','.join(comparison['symbols'])} "
            f"overlap={comparison['overlapping_timestamps']} "
            f"value_mismatches={comparison['value_mismatches']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
