#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONF="${RTC_LAUNCH_CONF:-${REPO_DIR}/scripts/matriks-ziraat-paper.conf}"
BRIDGE_ADDR_OVERRIDE="${MATRIKS_BRIDGE_ADDR:-}"

if [[ -f "$CONF" ]]; then
  # shellcheck source=scripts/lib_launchconf.sh
  . "${REPO_DIR}/scripts/lib_launchconf.sh"
  load_launch_conf "$CONF"
fi

export MATRIKS_BRIDGE_ADDR="${BRIDGE_ADDR_OVERRIDE:-${MATRIKS_BRIDGE_ADDR:-127.0.0.1:9010}}"
exec python3 "${REPO_DIR}/scripts/matriks/probe_bridge.py" \
  --addr "$MATRIKS_BRIDGE_ADDR" "$@"
