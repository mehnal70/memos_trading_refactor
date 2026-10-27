#!/usr/bin/env bash
# Memos headless operasyon runbook menusu.

set -u

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR" || exit 1

pause() {
  read -r -p "Devam etmek icin Enter..." _
}

confirm() {
  local prompt="$1" answer
  read -r -p "$prompt [e/h]: " answer
  case "${answer,,}" in
    e|evet|y|yes) return 0 ;;
    *) echo "Islem iptal edildi."; return 1 ;;
  esac
}

run_health() {
  ./scripts/health_diagnostic.sh
}

run_readiness() {
  python3 scripts/venue_readiness.py
}

show_config() {
  echo "--- aktif profil ---"
  cat scripts/.active 2>/dev/null || true
  echo "--- gate ve MT5 policy ---"
  grep -E '^(TRADING_MODE|LIVE_DRY_RUN|VENUES|TRADE_SYMBOL|MT5_SYMBOL_ALLOWLIST|PINNED_SYMBOLS|AUTO_MT5_SYMBOLS|LIVE_EXECUTION_ARMED|LIVE_GO_NO_GO|LIVE_MAX_NOTIONAL_USD|VENUE_MAX_CONCURRENT_OPEN|VENUE_DAILY_LOSS_PCT)=' \
    scripts/.launch.conf 2>/dev/null || true
}

run_autonomy() {
  systemctl --user start memos-autonomy.service
}

run_trade_probe() {
  echo "Bu islem headless runtime'i gecici olarak durdurur."
  confirm "MT5 trade probe icin kontrollu down/probe/up akisi calistirilsin mi?" || return 0
  ./memos down || return 1
  local rc=0
  scripts/mt5/setup_and_run.sh probe-trade || rc=$?
  ./memos up || rc=1
  echo "Runtime geri getiriliyor; 60 saniye sonra yeniden kontrol edin."
  printf 'trade_probe_rc=%s\n' "$rc"
  return "$rc"
}

arm_demo() {
  run_health || return 1
  echo
  echo "Gate acilacak: EURUSD demo execution, max 1 pozisyon, max notional 10 USD."
  confirm "Demo canary gate acilsin ve runtime restart edilsin mi?" || return 0
  printf '3\ne\n0\n' | ./memos demo-execution || return 1
  ./memos restart || return 1
  echo "Startup sonrasi bir heartbeat cevrimi bekleyin: sleep 60"
}

disarm_demo() {
  printf '4\n0\n' | ./memos demo-execution
  run_health
}

safe_shutdown() {
  local open_positions
  open_positions="$(
    tail -n 1 logs/demo-mt5-execution/heartbeat.jsonl 2>/dev/null |
      python3 -c 'import json,sys; print(json.load(sys.stdin).get("open_positions", "unknown"))' 2>/dev/null ||
      echo unknown
  )"
  if [[ "$open_positions" != "0" ]]; then
    echo "Kapatma reddedildi: open_positions=$open_positions. Once pozisyonu kontrol edin."
    return 1
  fi
  confirm "Gate kapatilsin ve runtime durdurulsun mu?" || return 0
  disarm_demo || return 1
  ./memos down
}

show_logs() {
  echo "--- watchdog ---"
  journalctl --user -u memos-watchdog.service -n 40 --no-pager
  echo "--- autonomy ---"
  journalctl --user -u memos-autonomy.service -n 40 --no-pager
  echo "--- heartbeat ---"
  tail -n 5 logs/demo-mt5-execution/heartbeat.jsonl
}

while true; do
  clear
  echo "=============================================="
  echo " Memos Trading - Headless Operator Runbook"
  echo "=============================================="
  echo "1) Saglik diagnostigi"
  echo "2) Venue readiness"
  echo "3) Profil / gate / risk ayarlari"
  echo "4) MT5 veri probe (pozisyon acmaz)"
  echo "5) MT5 trade probe (down/probe/up)"
  echo "6) Autonomy dongusunu manuel calistir"
  echo "7) Demo canary gate ac + restart"
  echo "8) Demo canary gate kapat"
  echo "9) Loglari goster"
  echo "10) Guvenli shutdown (pozisyon=0 zorunlu)"
  echo "11) Runtime'i baslat"
  echo "0) Cikis"
  echo
  read -r -p "Secim: " choice
  echo
  case "$choice" in
    1) run_health; pause ;;
    2) run_readiness; pause ;;
    3) show_config; pause ;;
    4) scripts/mt5/setup_and_run.sh probe; pause ;;
    5) run_trade_probe; pause ;;
    6) run_autonomy; pause ;;
    7) arm_demo; pause ;;
    8) disarm_demo; pause ;;
    9) show_logs; pause ;;
    10) safe_shutdown; pause ;;
    11) ./memos up; pause ;;
    0) exit 0 ;;
    *) echo "Gecersiz secim."; sleep 1 ;;
  esac
done
