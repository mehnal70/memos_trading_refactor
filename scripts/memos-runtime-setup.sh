#!/usr/bin/env bash
# Masaüstü üzerinden manuel venue runtime kontrolü.

set -euo pipefail
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

show_terminal() {
  if command -v x-terminal-emulator >/dev/null 2>&1; then
    x-terminal-emulator -e bash -lc "$1; printf '\\n\\nEnter ile kapatın...'; read -r"
  else
    exec "$REPO_DIR/memos" menu
  fi
}

if ! command -v zenity >/dev/null 2>&1; then
  exec "$REPO_DIR/memos" menu
fi

choice="$(zenity --list --radiolist \
  --title="Memos Runtime Setup" \
  --text="Venue runtime işlemini seçin:" \
  --column="Seç" --column="İşlem" \
  TRUE "Durum ve sağlık kontrolü" \
  FALSE "Manuel başlat (GUI → recovery → engine)" \
  FALSE "Venue engine'lerini durdur" \
  FALSE "IBKR bridge durumunu göster" \
  FALSE "IBKR bridge'i durdur" \
  FALSE "Terminal menüsünü aç" \
  --height=300 --width=560 2>/dev/null || true)"

case "$choice" in
  "Durum ve sağlık kontrolü")
    show_terminal "scripts/venue_engine.sh status all; echo; scripts/runtime_supervisor.sh status"
    ;;
  "Manuel başlat (GUI → recovery → engine)")
    show_terminal "scripts/runtime_supervisor.sh start-gui all || true; systemctl --user start memos-ibkr-bridge.service; scripts/venue_engine.sh start all"
    ;;
  "Venue engine'lerini durdur")
    show_terminal "scripts/venue_engine.sh stop all; systemctl --user stop memos-ibkr-bridge.service"
    ;;
  "IBKR bridge durumunu göster")
    show_terminal "systemctl --user status memos-ibkr-bridge.service --no-pager || true"
    ;;
  "IBKR bridge'i durdur")
    show_terminal "systemctl --user stop memos-ibkr-bridge.service"
    ;;
  "Terminal menüsünü aç")
    exec "$REPO_DIR/memos" menu
    ;;
esac
