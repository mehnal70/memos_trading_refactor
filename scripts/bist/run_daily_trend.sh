#!/usr/bin/env bash
set -euo pipefail

# Günlük BIST karar destek akışını manuel olarak tetikler.
# Asıl otonom pipeline Rust daemon'dır; aynı taramanın ikinci bir kopyasını başlatmaz.

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
DB_PATH="${DB_PATH:-$ROOT_DIR/data/trader.db}"
export DB_PATH

cd "$ROOT_DIR"

if ! cargo run --release \
  --manifest-path "$ROOT_DIR/Cargo.toml" \
  -p memos_trading_core \
  --example bist_trend_scan; then
  echo "BIST ana veri kaynağı kullanılamadı; yerel CSV önbelleğine geçiliyor." >&2
  python3 "$ROOT_DIR/scripts/bist/local_daily_scan.py"
fi

exec python3 "$ROOT_DIR/scripts/bist/daily_paper_sim.py"
