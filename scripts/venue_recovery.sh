#!/usr/bin/env bash
# Ortak readiness sözleşmesine göre venue başına güvenli recovery.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

TARGET="${1:-all}"
COOLDOWN_SECS="${VENUE_RECOVERY_COOLDOWN_SECS:-300}"
STATE_DIR="$REPO_DIR/logs/.venue-runtime/state"
LOCK_DIR="$STATE_DIR/recovery.lock.d"
LOCK_OWNER="$LOCK_DIR/owner.pid"
LOG_FILE="$REPO_DIR/logs/venue-recovery.log"
mkdir -p "$STATE_DIR" "$(dirname "$LOG_FILE")"

log() {
  printf '[venue-recovery %s] %s\n' "$(date -Is)" "$*" | tee -a "$LOG_FILE"
}

readiness() {
  python3 scripts/venue_readiness.py "$1"
}

overall() {
  readiness "$1" | python3 -c 'import json,sys; print(json.load(sys.stdin)["'"$1"'"]["overall"])'
}

cooldown_active() {
  local marker="$STATE_DIR/$1.recovery"
  [[ -f "$marker" ]] || return 1
  (( $(date +%s) - $(stat -c %Y "$marker") < COOLDOWN_SECS ))
}

mt5_transport_backlog_saturated() {
  command -v ss >/dev/null 2>&1 || return 1
  ss -ltnH '( sport = :9001 )' 2>/dev/null |
    awk '$2 >= $3 { saturated = 1 } END { exit !saturated }'
}

restart_engine() {
  local venue="$1"
  local unit="memos-venue-engine@${venue}.service"
  if systemctl --user cat "$unit" >/dev/null 2>&1; then
    log "$venue engine systemd unit üzerinden yeniden başlatılıyor"
    systemctl --user restart "$unit"
  else
    scripts/venue_engine.sh start "$venue"
  fi
}

recover_one() {
  local venue="$1" state
  if [[ "$venue" == "mt5" ]] &&
    systemctl --user is-active --quiet memos-watchdog.service 2>/dev/null; then
    log "mt5 headless supervisor tarafından yönetiliyor; venue recovery atlandı"
    return 0
  fi
  state="$(overall "$venue")"
  if [[ "$state" == "NOT_ACTIVE" ]]; then
    log "$venue etkin değil; recovery atlandı"
    return 0
  fi
  if [[ "$state" == "READY" || "$state" == "SIGNAL_READY" ]]; then
    log "$venue hazır; recovery atlandı"
    return 0
  fi
  if cooldown_active "$venue"; then
    log "$venue degraded; cooldown aktif, recovery atlandı"
    return 1
  fi
  touch "$STATE_DIR/$venue.recovery"
  log "$venue recovery başlıyor (state=$state)"
  case "$venue" in
    mt5)
      if mt5_transport_backlog_saturated; then
        log "mt5 9001 listener backlog dolu; engine kontrollü yeniden başlatılacak"
        scripts/venue_engine.sh stop mt5
      fi
      MT5_SELF_HEAL_POLICY=smart bash scripts/mt5/setup_and_run.sh self-heal || true
      restart_engine mt5
      ;;
    ibkr)
      ./memos recover ibkr || true
      restart_engine ibkr
      ;;
    binance|bist)
      restart_engine "$venue"
      ;;
    *)
      log "geçersiz venue: $venue"
      return 2
      ;;
  esac
  state="$(overall "$venue")"
  log "$venue recovery sonucu: $state"
  [[ "$state" == "READY" || "$state" == "SIGNAL_READY" ]]
}

case "$TARGET" in
  all) targets=(mt5 ibkr binance bist) ;;
  mt5|ibkr|binance|bist) targets=("$TARGET") ;;
  *) echo "Kullanım: $0 [all|mt5|ibkr|binance|bist]" >&2; exit 2 ;;
esac

acquire_recovery_lock() {
  if mkdir "$LOCK_DIR" 2>/dev/null; then
    printf '%s\n' "$$" >"$LOCK_OWNER"
    trap 'rm -rf "$LOCK_DIR"' EXIT
    return 0
  fi

  if [[ -r "$LOCK_OWNER" ]]; then
    local owner
    owner="$(cat "$LOCK_OWNER")"
    if [[ "$owner" =~ ^[0-9]+$ ]] && ! kill -0 "$owner" 2>/dev/null; then
      rm -rf "$LOCK_DIR"
      mkdir "$LOCK_DIR" 2>/dev/null || return 1
      printf '%s\n' "$$" >"$LOCK_OWNER"
      trap 'rm -rf "$LOCK_DIR"' EXIT
      return 0
    fi
  fi

  log "başka bir recovery çalışıyor; bu çağrı atlandı"
  return 1
}

acquire_recovery_lock || exit 1

failed=0
for venue in "${targets[@]}"; do
  recover_one "$venue" || failed=1
done
exit "$failed"
