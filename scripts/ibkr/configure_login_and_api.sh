#!/usr/bin/env bash
set -euo pipefail

# IB Gateway login + API ayarlarini terminalden yapilandirir.
# UI acmadan, calisan ibcstart process'inin config.ini dosyasini gunceller.
#
# Kullanim:
#   bash scripts/ibkr/configure_login_and_api.sh
#   IB_USER=... IB_PASS=... bash scripts/ibkr/configure_login_and_api.sh
#
# Opsiyonel env:
#   API_PORT=4002
#   TRADING_MODE=paper|live
#   READ_ONLY_API=yes|no
#   INCOMING_ACTION=accept|reject|manual

API_PORT="${API_PORT:-4002}"
TRADING_MODE="${TRADING_MODE:-paper}"
READ_ONLY_API="${READ_ONLY_API:-no}"
INCOMING_ACTION="${INCOMING_ACTION:-accept}"

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

find_ibc_pid() {
  pgrep -f 'ibcstart.sh .*--ibc-ini=' | head -n1
}

extract_ini_from_cmdline() {
  local pid="$1"
  tr '\0' ' ' < "/proc/${pid}/cmdline" | sed -nE 's/.*--ibc-ini=([^ ]+).*/\1/p'
}

set_kv() {
  local file="$1" key="$2" val="$3"
  if grep -qE "^${key}=" "$file"; then
    sed -i -E "s|^${key}=.*|${key}=${val}|" "$file"
  else
    printf '\n%s=%s\n' "$key" "$val" >> "$file"
  fi
}

PID="$(find_ibc_pid || true)"
if [[ -z "$PID" ]]; then
  echo "ibcstart process bulunamadi" >&2
  exit 1
fi

INI_IN_CONTAINER="$(extract_ini_from_cmdline "$PID")"
if [[ -z "$INI_IN_CONTAINER" ]]; then
  echo "--ibc-ini yolu cmdline'dan okunamadi" >&2
  exit 1
fi

CFG="/proc/${PID}/root${INI_IN_CONTAINER}"
if [[ ! -f "$CFG" ]]; then
  echo "config.ini dosyasi bulunamadi: $CFG" >&2
  exit 1
fi

IB_USER="${IB_USER:-${IBKR_LOGIN:-}}"
IB_PASS="${IB_PASS:-${IBKR_PASSWORD:-}}"

if [[ -z "$IB_USER" ]]; then
  printf 'IB login user: '
  read -r IB_USER
fi
if [[ -z "$IB_USER" ]]; then
  echo "Kullanici adi bos olamaz" >&2
  exit 1
fi

if [[ -z "$IB_PASS" ]]; then
  printf 'IB password (gorunmez): '
  read -rs IB_PASS
  echo
fi
if [[ -z "$IB_PASS" ]]; then
  echo "Sifre bos olamaz" >&2
  exit 1
fi

TS="$(date +%Y%m%d_%H%M%S)"
BACKUP="/tmp/ibc_config.ini.bak_${TS}"
cp "$CFG" "$BACKUP"

set_kv "$CFG" "IbLoginId" "$IB_USER"
set_kv "$CFG" "IbPassword" "$IB_PASS"
set_kv "$CFG" "TradingMode" "$TRADING_MODE"
set_kv "$CFG" "OverrideTwsApiPort" "$API_PORT"
set_kv "$CFG" "ReadOnlyApi" "$READ_ONLY_API"
set_kv "$CFG" "AcceptIncomingConnectionAction" "$INCOMING_ACTION"
set_kv "$CFG" "AcceptNonBrokerageAccountWarning" "yes"

unset IB_PASS

cat <<EOF

Ayarlar yazildi.
Config: $CFG
Yedek : $BACKUP

Guncel anahtarlar:
EOF

grep -E '^(IbLoginId|TradingMode|OverrideTwsApiPort|ReadOnlyApi|AcceptIncomingConnectionAction|AcceptNonBrokerageAccountWarning)=' "$CFG" || true

echo
cat <<EOF
Sonraki adimlar:
1) IB Gateway'i restart et (kendi calistirma yonteminle)
2) IBKR_API_PORT=${API_PORT} bash scripts/doctor.sh paper
3) Handshake OK olursa bridge baslat:
   IBKR_API_PORT=${API_PORT} bash scripts/run_bridge.sh
EOF
