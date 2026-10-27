#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONF="${RTC_LAUNCH_CONF:-${REPO_DIR}/scripts/matriks-ziraat-paper.conf}"
ADDR_OVERRIDE="${MATRIKS_ORDER_BRIDGE_ADDR:-}"

if [[ -f "$CONF" ]]; then
  # shellcheck source=scripts/lib_launchconf.sh
  . "${REPO_DIR}/scripts/lib_launchconf.sh"
  load_launch_conf "$CONF"
fi

ADDR="${ADDR_OVERRIDE:-${MATRIKS_ORDER_BRIDGE_ADDR:-127.0.0.1:9000}}"
HOST="${ADDR%:*}"
PORT="${ADDR##*:}"

if ! [[ "$PORT" =~ ^[0-9]+$ ]] || (( PORT < 1 || PORT > 65535 )); then
  echo "Invalid MATRIKS_ORDER_BRIDGE_ADDR: $ADDR" >&2
  exit 2
fi

if ! command -v nc >/dev/null 2>&1; then
  echo "nc is required for a transport-only TCP probe" >&2
  exit 2
fi

if nc -z -w 3 "$HOST" "$PORT"; then
  echo "TCP-READY $HOST:$PORT"
else
  echo "TCP-UNAVAILABLE $HOST:$PORT" >&2
  exit 1
fi
