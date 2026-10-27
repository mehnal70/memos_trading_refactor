#!/usr/bin/env bash

# IBKR gateway stack teklesmesi icin ortak yardimci.
# Amaç: Ayni anda birden fazla gateway instance'inin (IBC container + local GUI)
# acik kalmasini engellemek ve handshake conflict riskini dusurmek.

have_cmd() { command -v "$1" >/dev/null 2>&1; }

ibkr_single_owner_inventory() {
  local ibc_pids gui_pids
  ibc_pids="$(pgrep -f 'ibcstart\.sh .*--ibc-ini=' | tr '\n' ' ' | sed 's/[[:space:]]*$//')"
  gui_pids="$(pgrep -f 'install4j\.ibgateway\.GWClient|/Jts/ibgateway/.*/jre/bin/java' | tr '\n' ' ' | sed 's/[[:space:]]*$//')"

  # IBC tarafindaki JVM pid'lerini GUI listesinden cikar (yanlis pozitifleri azalt).
  local cleaned_gui=""
  local pid
  for pid in $gui_pids; do
    if ps -p "$pid" -o args= 2>/dev/null | grep -q '/home/ibgateway/Jts/ibgateway/'; then
      continue
    fi
    cleaned_gui+="${cleaned_gui:+ }$pid"
  done

  echo "ibc:${ibc_pids:-}" 
  echo "gui:${cleaned_gui:-}"
}

ibkr_single_owner_report() {
  local inv ibc gui
  inv="$(ibkr_single_owner_inventory)"
  ibc="$(printf '%s\n' "$inv" | sed -n 's/^ibc://p')"
  gui="$(printf '%s\n' "$inv" | sed -n 's/^gui://p')"

  echo "IBC stack pidleri : ${ibc:-yok}"
  echo "Local GUI pidleri : ${gui:-yok}"
}

ibkr_single_owner_violation() {
  local inv ibc gui
  inv="$(ibkr_single_owner_inventory)"
  ibc="$(printf '%s\n' "$inv" | sed -n 's/^ibc://p')"
  gui="$(printf '%s\n' "$inv" | sed -n 's/^gui://p')"

  [ -n "$ibc" ] && [ -n "$gui" ]
}

ibkr_single_owner_enforce() {
  local auto_prune="${IBKR_SINGLE_OWNER_AUTO_PRUNE:-1}"

  if ! ibkr_single_owner_violation; then
    return 0
  fi

  echo "WARN: IBKR single-owner ihlali: IBC + local GUI birlikte calisiyor." >&2
  ibkr_single_owner_report >&2

  case "$auto_prune" in
    1|true|TRUE|yes|YES|on|ON)
      local inv gui pid
      inv="$(ibkr_single_owner_inventory)"
      gui="$(printf '%s\n' "$inv" | sed -n 's/^gui://p')"
      if [ -n "$gui" ]; then
        echo "INFO: Local GUI instance'lari sonlandiriliyor: $gui" >&2
        # GUI processleri sadece bu kullanicida oldugu varsayimiyla guvenli prune.
        kill $gui 2>/dev/null || true
        sleep 1
      fi
      # Hala yasayan GUI instance varsa zorla kapat.
      inv="$(ibkr_single_owner_inventory)"
      gui="$(printf '%s\n' "$inv" | sed -n 's/^gui://p')"
      if [ -n "$gui" ]; then
        echo "INFO: Kalan GUI instance'lar zorla sonlandiriliyor: $gui" >&2
        kill -9 $gui 2>/dev/null || true
        sleep 1
      fi

      if ibkr_single_owner_violation; then
        echo "FAIL: Single-owner ihlali devam ediyor; manuel temizleme gerekli." >&2
        return 3
      fi

      echo "OK: IBKR single-owner teklesmesi saglandi." >&2
      return 0
      ;;
    *)
      echo "FAIL: Single-owner ihlali var. Otomatik prune kapali (IBKR_SINGLE_OWNER_AUTO_PRUNE=$auto_prune)." >&2
      echo "Cozum: IBKR_SINGLE_OWNER_AUTO_PRUNE=1 ile tekrar dene veya local GUI'yi kapat." >&2
      return 2
      ;;
  esac
}
