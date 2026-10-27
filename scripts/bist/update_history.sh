#!/usr/bin/env bash
set -euo pipefail

# Günlük BIST OHLC geçmişini mevcut candles tablosuna upsert eder.
# Kaynak canlı tick değildir; sinyal/backtest verisi olarak kullanılmalıdır.

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
DB_PATH="${DB_PATH:-$ROOT_DIR/data/trader.db}"
BIST_INTERVAL="${BIST_INTERVAL:-1d}"
BIST_RANGE="${BIST_RANGE:-5y}"
BIST_SYMBOLS="${BIST_SYMBOLS:-THYAO,GARAN,AKBNK,EREGL,ASELS,BIMAS,SISE,TUPRS,XU100}"
LOCK_DIR="${BIST_TREND_LOCK:-$ROOT_DIR/data/bist_trend.lock}"
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  echo "BIST pipeline zaten çalışıyor: $LOCK_DIR" >&2
  exit 0
fi
trap 'rmdir "$LOCK_DIR" 2>/dev/null || true' EXIT
if [[ "${BIST_AUTO_REFRESH:-0}" == "1" ]]; then
  python3 "$ROOT_DIR/scripts/bist/refresh_bist100_symbols.py" \
    --output "$ROOT_DIR/config/bist100_symbols.txt"
  BIST_SYMBOLS="$ROOT_DIR/config/bist100_symbols.txt"
elif [[ -n "${BIST_SYMBOLS_FILE:-}" ]]; then
  BIST_SYMBOLS="$BIST_SYMBOLS_FILE"
fi

export DB_PATH
cd "$ROOT_DIR"

CSV_DIR="${BIST_CSV_DIR:-$ROOT_DIR/data/bist_yahoo_csv}"
mkdir -p "$CSV_DIR"

python3 "$ROOT_DIR/scripts/fetch_yahoo_csv.py" \
  bist "$CSV_DIR" "$BIST_SYMBOLS"

python3 "$ROOT_DIR/scripts/bist/validate_ohlcv_csv.py" "$CSV_DIR"

CSV_COUNT="$(find "$CSV_DIR" -maxdepth 1 -type f -name '*.csv' | wc -l | tr -d ' ')"
if [[ "$CSV_COUNT" -eq 0 ]]; then
  echo "✗ import için CSV bulunamadı: $CSV_DIR" >&2
  exit 1
fi
echo "📦 import hazırlanıyor: $CSV_COUNT CSV → $CSV_DIR"

exec cargo run --release \
  --manifest-path "$ROOT_DIR/Cargo.toml" \
  -p memos_trading_core \
  --example import_csv -- \
  bist "$BIST_INTERVAL" "$CSV_DIR"
