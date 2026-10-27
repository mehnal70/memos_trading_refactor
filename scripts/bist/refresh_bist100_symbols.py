#!/usr/bin/env python3
"""Borsa İstanbul resmi bileşen CSV'sinden BIST 100 sembol manifestosu üretir."""

from __future__ import annotations

import argparse
import csv
import io
import os
import re
import tempfile
from pathlib import Path
from urllib.request import Request, urlopen

DEFAULT_URL = "https://www.borsaistanbul.com/datum/hisse_endeks_ds.csv"
SYMBOL_RE = re.compile(r"^[A-Z0-9]+$")


def fetch_text(url: str) -> str:
    request = Request(url, headers={"User-Agent": "memos-trading-bist-universe/1.0"})
    with urlopen(request, timeout=30) as response:
        raw = response.read()
    for encoding in ("utf-8-sig", "cp1254", "latin-1"):
        try:
            return raw.decode(encoding)
        except UnicodeDecodeError:
            continue
    raise ValueError("Borsa İstanbul CSV karakter kodlaması çözülemedi")


def parse_symbols(content: str) -> list[str]:
    reader = csv.DictReader(io.StringIO(content), delimiter=";")
    if not reader.fieldnames:
        raise ValueError("Borsa İstanbul CSV başlığı bulunamadı")
    constituent_key = next(
        (key for key in ("CONSTITUENT CODE", "BILESEN KODU") if key in reader.fieldnames),
        None,
    )
    index_key = next(
        (key for key in ("INDEX CODE", "ENDEKS KODU") if key in reader.fieldnames),
        None,
    )
    if constituent_key is None or index_key is None:
        raise ValueError(f"beklenen kolonlar yok: {reader.fieldnames!r}")

    symbols: set[str] = set()
    for row in reader:
        if row.get(index_key, "").strip().upper() != "XU100":
            continue
        code = row.get(constituent_key, "").strip().upper()
        symbol = code.removesuffix(".E")
        if SYMBOL_RE.fullmatch(symbol):
            symbols.add(symbol)

    result = sorted(symbols)
    if not 90 <= len(result) <= 110:
        raise ValueError(f"XU100 bileşen sayısı beklenmeyen: {len(result)}")
    return result


def write_manifest(path: Path, symbols: list[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    content = (
        "# Borsa İstanbul resmi XU100 bileşenleri\n"
        "# Kaynak: https://www.borsaistanbul.com/datum/hisse_endeks_ds.csv\n"
        + "".join(f"{symbol}\n" for symbol in symbols)
    )
    fd, temp_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent, text=True)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(content)
        os.replace(temp_name, path)
    except Exception:
        os.unlink(temp_name)
        raise


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default="config/bist100_symbols.txt")
    parser.add_argument("--url", default=DEFAULT_URL)
    args = parser.parse_args()

    symbols = parse_symbols(fetch_text(args.url))
    output = Path(args.output)
    write_manifest(output, symbols)
    print(f"✅ XU100 manifestosu güncellendi: {len(symbols)} sembol → {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
