#!/usr/bin/env python3
"""fetch_yahoo_csv — dünya-piyasası günlük OHLC'yi yfinance ile CSV'ye döker (import_csv için).

[[project_world_markets]] Faz A: ham HTTP Yahoo'da 429/cookie kapısına takılıyor; yfinance
çerez/crumb dansını düzgün yapar → normal makineden çalışır. Her sembol için ÇIPLAK-adlı CSV
(THYAO.csv, EURUSD.csv, ...) yazar; sonra Rust tarafında:
    cargo run --release --example import_csv -- <market> 1d <out_dir>

Kurulum:  pip install yfinance
Kullanım: python3 scripts/fetch_yahoo_csv.py <bist|forex|commodity|usequity> <out_dir> SYM1,SYM2,...
Örnek:    python3 scripts/fetch_yahoo_csv.py bist ./csv_bist THYAO,GARAN,AKBNK,EREGL
          python3 scripts/fetch_yahoo_csv.py forex ./csv_fx EURUSD,GBPUSD,USDJPY
"""
import os
import sys

import yfinance as yf


def yahoo_ticker(asset_class: str, base: str) -> str:
    b = base.strip()
    if b.endswith((".IS", "=X", "=F")):
        return b
    ac = asset_class.lower()
    if ac in ("bist", "equity_tr"):
        return f"{b}.IS"
    if ac in ("forex", "fx"):
        return f"{b}=X"
    if ac in ("commodity", "comm", "gold"):
        return f"{b}=F"
    return b  # usequity / index / etf → çıplak


def main() -> int:
    if len(sys.argv) < 4:
        print("Kullanım: fetch_yahoo_csv.py <bist|forex|commodity|usequity> <out_dir> SYM1,SYM2,...")
        return 2
    asset_class, out_dir, symbols_csv = sys.argv[1], sys.argv[2], sys.argv[3]
    symbols = [s.strip().upper() for s in symbols_csv.split(",") if s.strip()]
    os.makedirs(out_dir, exist_ok=True)

    ok = 0
    for base in symbols:
        ticker = yahoo_ticker(asset_class, base)
        try:
            df = yf.download(ticker, period="max", interval="1d", auto_adjust=False, progress=False)
        except Exception as e:  # noqa: BLE001
            print(f"  ✗ {base:10} ({ticker}) hata: {e}")
            continue
        if df is None or df.empty:
            print(f"  ⚠ {base:10} ({ticker}) veri yok")
            continue
        # MultiIndex kolonları düzleştir; standart başlık (Date,Open,High,Low,Close,Adj Close,Volume).
        df = df.reset_index()
        if hasattr(df.columns, "get_level_values"):
            df.columns = [c[0] if isinstance(c, tuple) else c for c in df.columns]
        path = os.path.join(out_dir, f"{base}.csv")
        df.to_csv(path, index=False)
        n = len(df)
        first = str(df["Date"].iloc[0])[:10] if "Date" in df else "?"
        last = str(df["Date"].iloc[-1])[:10] if "Date" in df else "?"
        print(f"  ✅ {base:10} ({ticker}) {n} satır ({first} → {last}) → {path}")
        ok += 1

    print(f"\n→ {ok}/{len(symbols)} sembol yazıldı: {out_dir}")
    print(f"  Sonra: cargo run --release --example import_csv -- {asset_class} 1d {out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
