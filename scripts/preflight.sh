#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

if [ -f scripts/lib_launchconf.sh ]; then
  # shellcheck disable=SC1091
  . scripts/lib_launchconf.sh
  load_launch_conf scripts/.launch.conf
fi

echo "── Memos Live öncesi preflight ──"
printf 'profil: '; cat scripts/.active 2>/dev/null || printf 'yok\n'
printf 'mod: %s · dry-run: %s · interval: %s\n' \
  "${TRADING_MODE:-unset}" \
  "${LIVE_DRY_RUN:-unset}" \
  "${TRADE_INTERVAL:-unset}"
printf 'DB: %s\n' "${DB_PATH:-unset}"
printf 'venues: %s\n' "${VENUES:-unset}"

if [ -z "${DB_PATH:-}" ] || [ -z "${TRADE_INTERVAL:-}" ] || [ -z "${VENUES:-}" ]; then
  echo "❌ Zorunlu runtime ayarı eksik."
  exit 1
fi

echo "── Workspace derlemesi ──"
cargo check --workspace

echo "── Kritik bridge testleri ──"
cargo test -p memos_trading_core core::bridge::tests

echo "── Emir göndermeyen health smoke ──"
HEALTHCHECK_DURATION_SECS="${HEALTHCHECK_DURATION_SECS:-60}" \
  HEALTHCHECK_SKIP_LIVE_DRY_RUN="${HEALTHCHECK_SKIP_LIVE_DRY_RUN:-0}" \
  cargo run -q -p rtc_healthcheck

echo "✅ Preflight tamamlandı: sistem Live için teknik olarak doğrulandı."
echo "⚠️  Bu sonuç venue hesap bakiyesi, gerçek emir yetkisi veya broker finansal koşullarını garanti etmez."
