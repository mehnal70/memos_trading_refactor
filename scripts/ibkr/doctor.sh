#!/usr/bin/env bash
set -euo pipefail

# IBKR Doctor: TWS/IB Gateway + Bridge hazır mı hızlı teşhis.
# Kullanim: scripts/ibkr/doctor.sh [paper|live]

MODE="${1:-paper}"
case "${MODE}" in
  # Bu hosttaki paper Gateway 4002 kullanıyor; standart TWS 7497 ise otomatik keşif
  # aşağıdaki detect_api_port() ile devreye girer.
  paper) API_PORT_DEFAULT=4002 ;;
  live)  API_PORT_DEFAULT=7496 ;;
  *)
    echo "Kullanim: $0 [paper|live]"
    exit 2
    ;;
esac

API_HOST="${IBKR_API_HOST:-127.0.0.1}"
API_PORT="${IBKR_API_PORT:-$API_PORT_DEFAULT}"
BRIDGE_ADDR="${IBKR_BRIDGE_ADDR:-127.0.0.1:9101}"
BRIDGE_HOST="${BRIDGE_ADDR%%:*}"
BRIDGE_PORT="${BRIDGE_ADDR##*:}"

# shellcheck source=scripts/ibkr/lib_single_owner.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib_single_owner.sh"

detect_api_port() {
  local host="$1"
  local p
  for p in 7497 7496 4002 4001; do
    if check_tcp "$host" "$p"; then
      echo "$p"
      return 0
    fi
  done
  return 1
}

have_cmd() { command -v "$1" >/dev/null 2>&1; }

check_tcp() {
  local host="$1" port="$2"
  if have_cmd nc; then
    nc -z -w 2 "$host" "$port" >/dev/null 2>&1
    return $?
  fi
  if have_cmd bash; then
    timeout 2 bash -lc "</dev/tcp/${host}/${port}" >/dev/null 2>&1
    return $?
  fi
  return 3
}

check_ib_handshake() {
  local host="$1" port="$2"
  timeout "${IBKR_HANDSHAKE_TIMEOUT_SECS:-6}" \
    python3 - "$host" "$port" <<'PY' >/dev/null 2>&1
import sys
try:
    from ib_insync import IB
except Exception:
    sys.exit(10)

host = sys.argv[1]
port = int(sys.argv[2])
ib = IB()
try:
    ib.connect(host, port, clientId=987, timeout=4, readonly=True)
    ib.disconnect()
    sys.exit(0)
except Exception:
    sys.exit(1)
PY
  return $?
}

line() { printf '%s\n' "------------------------------------------------------------"; }

echo "IBKR Doctor"
line
echo "Mod             : $MODE"
echo "API hedefi      : ${API_HOST}:${API_PORT}"
echo "Bridge hedefi   : ${BRIDGE_HOST}:${BRIDGE_PORT}"
line

echo "[0/6] Single-owner kontrolu"
if ibkr_single_owner_violation; then
  echo "WARN- Ayni anda IBC stack + local GUI acik (competing-session riski yuksek)"
  ibkr_single_owner_report | sed 's/^/      /'
  echo "      Cozum: scripts/ibkr/enforce_single_owner.sh"
else
  echo "OK  - Gateway ownership tekil"
fi

echo "[1/6] Process kontrolu"
if ps -ef | grep -Ei 'ibgateway|trader workstation|tws' | grep -v grep >/dev/null 2>&1; then
  echo "OK  - IBKR sureci gorunuyor"
else
  echo "FAIL- IB Gateway/TWS sureci gorunmuyor"
fi

echo "[2/6] API port kontrolu (${API_HOST}:${API_PORT})"
if check_tcp "$API_HOST" "$API_PORT"; then
  echo "OK  - TCP port acik"
  if check_ib_handshake "$API_HOST" "$API_PORT"; then
    echo "OK  - IB API handshake basarili"
  else
    hs_code=$?
    if [[ "$hs_code" == "10" ]]; then
      echo "WARN- ib_insync yok; API handshake dogrulanamadi"
    else
      echo "FAIL- IB API handshake basarisiz (gateway ayari/oturum onayi gerekli)"
      echo "      Kontrol: API Enabled, Read-Only gecici acik, localhost/trusted IP izinli"
    fi
  fi
else
  echo "FAIL- API port kapali"
  if alt_port="$(detect_api_port "$API_HOST")"; then
    echo "      Not: API ${API_HOST}:${alt_port} portunda dinliyor gibi gorunuyor."
    echo "      Cozum: export IBKR_API_PORT=${alt_port} ve tekrar dene."
  else
    echo "      TWS/Gateway -> API ayarlari: socket clients acik, port dogru, localhost izinli"
  fi
fi

echo "[3/6] Bridge port kontrolu (${BRIDGE_HOST}:${BRIDGE_PORT})"
if check_tcp "$BRIDGE_HOST" "$BRIDGE_PORT"; then
  echo "OK  - Bridge port dinliyor"
else
  echo "WARN- Bridge port kapali (core calismiyor ya da bridge servisi yok)"
fi

echo "[4/6] Launch conf kontrolu"
CONF="scripts/.launch.conf"
if [[ -f "$CONF" ]]; then
  if grep -q '^IBKR_ENABLED=1' "$CONF"; then
    echo "OK  - IBKR_ENABLED=1"
  else
    echo "WARN- IBKR_ENABLED=0 (IBKR route pasif)"
  fi
  if grep -q '^IBKR_BRIDGE_ADDR=' "$CONF"; then
    echo "OK  - IBKR_BRIDGE_ADDR tanimli"
  else
    echo "WARN- IBKR_BRIDGE_ADDR tanimsiz"
  fi
  if grep -q 'VENUES=.*ibkr' "$CONF"; then
    echo "OK  - VENUES icinde ibkr var"
  else
    echo "WARN- VENUES icinde ibkr yok (symbol@ibkr explicit route gerekir)"
  fi
else
  echo "WARN- scripts/.launch.conf bulunamadi"
fi

echo "[5/6] Ozet"
if check_tcp "$API_HOST" "$API_PORT" && check_ib_handshake "$API_HOST" "$API_PORT"; then
  echo "Hazirlik: API ulasilabilir. Sonraki adim bridge'i ayaga kaldirmak ve ping testi."
else
  if alt_port="$(detect_api_port "$API_HOST")"; then
    if check_ib_handshake "$API_HOST" "$alt_port"; then
      echo "Oncelik: Varsayilan port yerine ${alt_port} kullan."
      echo "Ornek: IBKR_API_PORT=${alt_port} bash scripts/ibkr/doctor.sh paper"
    else
      echo "Oncelik: API handshake'i duzelt (port acik ama API oturumu hazir degil)."
      echo "Kontrol listesi: Gateway'de API Enabled + Read-Only gecici acik + localhost izinli"
    fi
  else
    echo "Oncelik: TWS/IB Gateway API portunu ac (bu olmadan bridge calismaz)."
  fi
fi

echo "[6/6] Kalici stabilite notu"
if ibkr_single_owner_violation; then
  echo "Single-owner ihlali suruyor: handshake timeout/competition hatalari devam edebilir."
else
  echo "Single-owner temiz: competing-session riski minimize edildi."
fi
