#!/usr/bin/env bash
set -euo pipefail

# IBKR bridge baslatir.
# Gerekenler:
# - IB Gateway/TWS acik
# - API port ulasilabilir (bu ortamda genelde 4002)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# shellcheck source=scripts/ibkr/lib_single_owner.sh
. "${SCRIPT_DIR}/lib_single_owner.sh"

export IBKR_API_HOST="${IBKR_API_HOST:-127.0.0.1}"
export IBKR_API_PORT="${IBKR_API_PORT:-4002}"
export IBKR_BRIDGE_ADDR="${IBKR_BRIDGE_ADDR:-127.0.0.1:9101}"
export IBKR_CLIENT_ID="${IBKR_CLIENT_ID:-42}"
export IBKR_CLIENT_ID_FALLBACK="${IBKR_CLIENT_ID_FALLBACK:-1}"
export IBKR_SINGLE_OWNER_AUTO_PRUNE="${IBKR_SINGLE_OWNER_AUTO_PRUNE:-1}"

if ! ibkr_single_owner_enforce; then
  echo "IBKR bridge baslatilmadi: single-owner guard ihlali." >&2
  echo "Cozum: scripts/ibkr/enforce_single_owner.sh && tekrar scripts/ibkr/run_bridge.sh" >&2
  exit 3
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 bulunamadi"
  exit 1
fi

python3 - <<'PY'
import importlib.util, sys
if importlib.util.find_spec('ib_insync') is None:
    print('ib_insync eksik. Kurulum: python3 -m pip install --user ib_insync')
    sys.exit(2)
print('ib_insync bulundu')
PY

exec python3 "${REPO_DIR}/scripts/ibkr/bridge_ibkr.py"
