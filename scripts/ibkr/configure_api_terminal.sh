#!/usr/bin/env bash
set -euo pipefail

# IB Gateway API ayarlarini UI acmadan terminalden yapilandirir.
# Calisan ibcstart process'inden config.ini yolunu otomatik bulur.
#
# Kullanim:
#   bash scripts/ibkr/configure_api_terminal.sh
#   API_PORT=4002 READ_ONLY_API=no INCOMING_ACTION=accept bash scripts/ibkr/configure_api_terminal.sh

API_PORT="${API_PORT:-4002}"
READ_ONLY_API="${READ_ONLY_API:-no}"           # yes/no/blank
INCOMING_ACTION="${INCOMING_ACTION:-accept}"    # accept/reject/manual/blank
TRADING_MODE="${TRADING_MODE:-paper}"           # paper/live/blank

if [[ ! "$READ_ONLY_API" =~ ^(yes|no|)$ ]]; then
  echo "READ_ONLY_API yes/no/blank olmali" >&2
  exit 2
fi
if [[ ! "$INCOMING_ACTION" =~ ^(accept|reject|manual|)$ ]]; then
  echo "INCOMING_ACTION accept/reject/manual/blank olmali" >&2
  exit 2
fi
if [[ ! "$TRADING_MODE" =~ ^(paper|live|)$ ]]; then
  echo "TRADING_MODE paper/live/blank olmali" >&2
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

INI_HOST_PATH="/proc/${PID}/root${INI_IN_CONTAINER}"
if [[ ! -f "$INI_HOST_PATH" ]]; then
  echo "config.ini dosyasi bulunamadi: $INI_HOST_PATH" >&2
  exit 1
fi

TS="$(date +%Y%m%d_%H%M%S)"
BACKUP="/tmp/ibc_config.ini.bak_${TS}"
cp "$INI_HOST_PATH" "$BACKUP"

echo "PID:          $PID"
echo "Config yolu:  $INI_HOST_PATH"
echo "Yedek:        $BACKUP"

echo "Ayarlar uygulaniyor..."
set_kv "$INI_HOST_PATH" "OverrideTwsApiPort" "$API_PORT"
set_kv "$INI_HOST_PATH" "ReadOnlyApi" "$READ_ONLY_API"
set_kv "$INI_HOST_PATH" "AcceptIncomingConnectionAction" "$INCOMING_ACTION"
if [[ -n "$TRADING_MODE" ]]; then
  set_kv "$INI_HOST_PATH" "TradingMode" "$TRADING_MODE"
fi

# Paper popup'ini otomatik kabul et (API handshake'i bloklamasin)
set_kv "$INI_HOST_PATH" "AcceptNonBrokerageAccountWarning" "yes"

echo
echo "Guncel anahtarlar:"
grep -E '^(TradingMode|OverrideTwsApiPort|ReadOnlyApi|AcceptIncomingConnectionAction|AcceptNonBrokerageAccountWarning)=' "$INI_HOST_PATH" || true

echo
echo "Not: Ayarlarin etkili olmasi icin IB Gateway yeniden baslamali."
echo "Oneri: mevcut ibgateway supervisor komutlarinizla restart edin."
