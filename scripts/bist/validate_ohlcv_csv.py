#!/usr/bin/env python3
"""OHLCV CSV dosyalarını DB importundan önce biçim ve tutarlılık açısından denetler."""

from __future__ import annotations

import argparse
import csv
import math
from datetime import date
from pathlib import Path

REQUIRED = ("date", "open", "high", "low", "close")
OPTIONAL = ("volume", "adj close")


def normalize_header(value: str) -> str:
    return " ".join(value.strip().strip('"').lower().split())


def parse_float(raw: str, *, field: str, line: int) -> float:
    value = raw.strip().strip('"')
    if not value:
        raise ValueError(f"satır {line}: {field} boş")
    number = float(value)
    if not math.isfinite(number):
        raise ValueError(f"satır {line}: {field} sonlu sayı değil")
    return number


def validate_file(path: Path) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    warnings: list[str] = []
    with path.open("r", encoding="utf-8-sig", newline="") as handle:
        reader = csv.DictReader(handle)
        if not reader.fieldnames:
            return ["başlık satırı yok"], []
        columns = {normalize_header(name): name for name in reader.fieldnames}
        missing = [name for name in REQUIRED if name not in columns]
        if missing:
            return [f"eksik kolon: {', '.join(missing)}"], []

        dates: list[date] = []
        row_count = 0
        for line, row in enumerate(reader, start=2):
            row_count += 1
            try:
                current_date = date.fromisoformat(row[columns["date"]].strip()[:10])
                values = {
                    field: parse_float(row[columns[field]], field=field, line=line)
                    for field in ("open", "high", "low", "close")
                }
                volume = (
                    parse_float(row[columns["volume"]], field="volume", line=line)
                    if "volume" in columns
                    else 0.0
                )
            except (KeyError, ValueError) as exc:
                errors.append(f"satır {line}: {exc}")
                continue

            if min(values.values()) <= 0:
                warnings.append(f"satır {line}: fiyatlar pozitif olmalı")
            if values["high"] < max(values["open"], values["low"], values["close"]):
                warnings.append(f"satır {line}: high OHLC aralığını kapsamıyor")
            if values["low"] > min(values["open"], values["high"], values["close"]):
                warnings.append(f"satır {line}: low OHLC aralığını kapsamıyor")
            if volume < 0:
                warnings.append(f"satır {line}: volume negatif")
            dates.append(current_date)

        if row_count == 0:
            errors.append("veri satırı yok")
        if len(dates) != len(set(dates)):
            errors.append("duplicate tarih var")
        if dates != sorted(dates):
            errors.append("tarih sırası artan değil")
        if len(dates) > 1:
            largest_gap = max(
                (later - earlier).days for earlier, later in zip(dates, dates[1:])
            )
            if largest_gap > 10:
                warnings.append(f"en büyük tarih boşluğu {largest_gap} gün")

    return errors, warnings


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("path", help="CSV dosyası veya CSV klasörü")
    args = parser.parse_args()
    source = Path(args.path)
    files = sorted(source.glob("*.csv")) if source.is_dir() else [source]
    if not files:
        print(f"✗ CSV bulunamadı: {source}")
        return 2

    failed = 0
    for path in files:
        errors, warnings = validate_file(path)
        if errors:
            failed += 1
            print(f"✗ {path}:")
            for error in errors[:20]:
                print(f"  - {error}")
        else:
            print(f"✅ {path}")
            for warning in warnings[:10]:
                print(f"  ⚠ {warning}")
            if len(warnings) > 10:
                print(f"  ⚠ ... {len(warnings) - 10} ek uyarı")
    print(f"\n→ {len(files) - failed}/{len(files)} CSV geçerli")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
