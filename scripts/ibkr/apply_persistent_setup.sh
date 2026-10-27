#!/usr/bin/env bash
set -euo pipefail

# IBKR icin tek-adim kalici uygulama akisi.
# Ilk calistirmada kullanici adi ve sifreyi sorar, yerel config'e kaydeder,
# sonra config -> restart -> doctor sirayla uygular.
#
# Kullanim:
#   bash scripts/ibkr/apply_persistent_setup.sh
#
# Opsiyonel env:
#   API_PORT=4002
#   TRADING_MODE=paper|live
#   READ_ONLY_API=yes|no
#   INCOMING_ACTION=accept|reject|manual
#   START_BRIDGE=1

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
STATE_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/memos_trading"
STATE_FILE="${STATE_DIR}/ibkr.env"

have_cmd() {
  command -v "$1" >/dev/null 2>&1
}

API_PORT="${API_PORT:-4002}"
TRADING_MODE="${TRADING_MODE:-paper}"
READ_ONLY_API="${READ_ONLY_API:-no}"
INCOMING_ACTION="${INCOMING_ACTION:-accept}"
START_BRIDGE="${START_BRIDGE:-0}"

mkdir -p "$STATE_DIR"
chmod 700 "$STATE_DIR" 2>/dev/null || true

if [[ -f "$STATE_FILE" ]]; then
  # shellcheck disable=SC1090
  . "$STATE_FILE"
fi

IB_USER="${IB_USER:-${IBKR_LOGIN:-}}"
IB_PASS="${IB_PASS:-${IBKR_PASSWORD:-}}"

if [[ -z "${IB_USER}" ]]; then
  printf 'IB login user: '
  read -r IB_USER
fi
if [[ -z "${IB_USER}" ]]; then
  echo "Kullanici adi bos olamaz" >&2
  exit 1
fi

if [[ -z "${IB_PASS}" ]]; then
  printf 'IB password (gorunmez): '
  read -rs IB_PASS
  echo
fi
if [[ -z "${IB_PASS}" ]]; then
  echo "Sifre bos olamaz" >&2
  exit 1
fi

if [[ ! "$TRADING_MODE" =~ ^(paper|live)$ ]]; then
  echo "TRADING_MODE paper veya live olmali" >&2
  exit 2
fi
if [[ ! "$READ_ONLY_API" =~ ^(yes|no)$ ]]; then
  echo "READ_ONLY_API yes veya no olmali" >&2
  exit 2
fi
if [[ ! "$INCOMING_ACTION" =~ ^(accept|reject|manual)$ ]]; then
  echo "INCOMING_ACTION accept/reject/manual olmali" >&2
  exit 2
fi

cat > "$STATE_FILE" <<EOF
IB_USER=${IB_USER}
IB_PASS=${IB_PASS}
API_PORT=${API_PORT}
TRADING_MODE=${TRADING_MODE}
READ_ONLY_API=${READ_ONLY_API}
INCOMING_ACTION=${INCOMING_ACTION}
START_BRIDGE=${START_BRIDGE}
EOF
chmod 600 "$STATE_FILE" 2>/dev/null || true

echo "Yerel IBKR ayarlari kaydedildi: $STATE_FILE"

IB_USER="$IB_USER" \
IB_PASS="$IB_PASS" \
API_PORT="$API_PORT" \
TRADING_MODE="$TRADING_MODE" \
READ_ONLY_API="$READ_ONLY_API" \
INCOMING_ACTION="$INCOMING_ACTION" \
bash "$REPO_DIR/scripts/ibkr/configure_login_and_api.sh"

PID="$(pgrep -f 'ibcstart.sh .*--ibc-ini=' | head -n1 || true)"
if [[ -z "$PID" ]]; then
  echo "ibcstart process bulunamadi; restart atlandi" >&2
  exit 1
fi

CONTAINER_ID="$(tr '\0' ' ' < "/proc/${PID}/cmdline" | grep -aoE '[0-9a-f]{64}' | head -n1 || true)"
if [[ -z "$CONTAINER_ID" ]]; then
  CONTAINER_ID="$(grep -aoE '[0-9a-f]{64}' "/proc/${PID}/cgroup" | head -n1 || true)"
fi

if [[ -n "$CONTAINER_ID" ]]; then
  echo "Container restart ediliyor: $CONTAINER_ID"
  sudo docker restart "$CONTAINER_ID"
else
  echo "WARN: container id tespit edilemedi; restart manual gerekli" >&2
fi

echo
IBKR_API_PORT="$API_PORT" bash "$REPO_DIR/scripts/ibkr/doctor.sh" "$TRADING_MODE"

if [[ "$START_BRIDGE" == "1" ]]; then
  if IBKR_API_PORT="$API_PORT" bash "$REPO_DIR/scripts/ibkr/doctor.sh" "$TRADING_MODE" | grep -q 'OK  - IB API handshake basarili'; then
    if ! have_cmd ss || ! ss -ltn 2>/dev/null | grep -q ':9101 '; then
      echo "Bridge baslatiliyor..."
      nohup bash "$REPO_DIR/scripts/ibkr/run_bridge.sh" >/tmp/ibkr_bridge.log 2>&1 &
      disown || true
      echo "Bridge log: /tmp/ibkr_bridge.log"
    else
      echo "Bridge zaten calisiyor gibi gorunuyor"
    fi
  else
    echo "Bridge atlandi; API handshake hazir degil"
  fi
fi

echo
echo "Tekrar calistirmak icin: bash scripts/ibkr/apply_persistent_setup.sh"