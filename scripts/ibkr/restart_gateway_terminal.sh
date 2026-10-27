#!/usr/bin/env bash
set -euo pipefail

# IB Gateway restart yardimcisi (terminal-only).
# Once IBC command server ile yumuşak RESTART dener; başarısızsa yalnız bilgi verir.
#
# Kullanim:
#   bash scripts/ibkr/restart_gateway_terminal.sh

find_ibc_pid() {
  pgrep -f 'ibcstart.sh .*--ibc-ini=' | head -n1
}

PID="$(find_ibc_pid || true)"
if [[ -z "$PID" ]]; then
  echo "ibcstart process bulunamadi" >&2
  exit 1
fi

ROOT="/proc/${PID}/root"
IBC_DIR="${ROOT}/home/ibgateway/ibc"
CMD_SEND="${IBC_DIR}/commandsend.sh"

echo "ibc pid: ${PID}"
echo "ibc dir: ${IBC_DIR}"

if [[ -x "$CMD_SEND" ]]; then
  echo "IBC command server uzerinden RESTART deneniyor..."
  (
    cd "$IBC_DIR"
    ./commandsend.sh RESTART
  ) || true
else
  echo "commandsend.sh bulunamadi/yurutulemedi"
fi

echo "Durum kontrolu..."
sleep 3
if pgrep -f 'ibcstart.sh .*--ibc-ini=' >/dev/null 2>&1; then
  echo "OK: ibcstart process gorunuyor (restart emri iletilmis olabilir)."
else
  echo "WARN: ibcstart process gorunmuyor. Gateway'i baslattigin komutla tekrar acman gerekebilir."
fi

echo
cat <<EOF
Sonraki adim:
1) IBKR_API_PORT=4002 bash scripts/doctor.sh paper
2) Handshake OK ise: IBKR_API_PORT=4002 bash scripts/run_bridge.sh
EOF
