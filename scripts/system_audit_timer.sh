#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
SERVICE="memos-system-audit.service"
TIMER="memos-system-audit.timer"

case "${1:-}" in
  install)
    mkdir -p "$UNIT_DIR"
    install -m 0644 "$REPO_DIR/scripts/$SERVICE" "$UNIT_DIR/$SERVICE"
    install -m 0644 "$REPO_DIR/scripts/$TIMER" "$UNIT_DIR/$TIMER"
    systemctl --user daemon-reload
    systemctl --user enable --now "$TIMER"
    systemctl --user start "$SERVICE"
    echo "Audit timer aktif: $TIMER"
    ;;
  uninstall)
    systemctl --user disable --now "$TIMER" 2>/dev/null || true
    rm -f "$UNIT_DIR/$TIMER" "$UNIT_DIR/$SERVICE"
    systemctl --user daemon-reload
    echo "Audit timer kaldırıldı."
    ;;
  status)
    systemctl --user status --no-pager "$TIMER" "$SERVICE"
    ;;
  *)
    printf 'Kullanim: %s {install|uninstall|status}\n' "$0" >&2
    exit 2
    ;;
esac
