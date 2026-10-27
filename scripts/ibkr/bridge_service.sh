#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_DIR"

. "$REPO_DIR/scripts/lib_launchconf.sh"
load_launch_conf "$REPO_DIR/scripts/.launch.conf"

while ! IBKR_API_PORT="${IBKR_API_PORT:-4002}" bash "$REPO_DIR/scripts/ibkr/doctor.sh" paper 2>/dev/null \
	| grep 'OK  - IB API handshake basarili' >/dev/null; do
	echo "IBKR Gateway API bekleniyor (${IBKR_API_HOST:-127.0.0.1}:${IBKR_API_PORT:-4002})"
	sleep 15
done

exec "$REPO_DIR/scripts/ibkr/run_bridge.sh"