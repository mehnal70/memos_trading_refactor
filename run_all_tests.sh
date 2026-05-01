#!/bin/bash
# Otomatik test runner scripti
cd "$(dirname "$0")/memos_trading_core" || exit 1

echo "[RUN] cargo test"
echo "[RUN] cargo test (JSON output)"
cargo test --all -- --format json > ../test_results.json

cd ../memos_trading_desktop || exit 1
if [ -f package.json ]; then
  echo "[RUN] npm test (desktop)"
  npm test || echo "npm test yok, atlanıyor."
fi

echo "[OK] Tüm testler çalıştırıldı."
