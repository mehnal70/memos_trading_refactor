#!/usr/bin/env bash
# Memos headless çalışma tanısı.
#
# Kullanım:
#   scripts/health_diagnostic.sh              # rapor
#   scripts/health_diagnostic.sh --fix        # güvenli kalıntı düzeltmeleri
#
# --fix engine'i öldürmez, restart etmez, execution gate'ini değiştirmez ve
# logs/.watchdog.pause dosyasını kaldırmaz. Yalnızca sahipsiz PID/lock
# kalıntılarını temizler ve watchdog durmuşsa mevcut user service'i başlatır.

set -u

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR" || exit 1

FIX=0
for arg in "$@"; do
  case "$arg" in
    --fix) FIX=1 ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *) echo "Kullanım: $0 [--fix]" >&2; exit 2 ;;
  esac
done

# shellcheck source=scripts/lib_launchconf.sh
. "$REPO_DIR/scripts/lib_launchconf.sh" 2>/dev/null || true
load_launch_conf "$REPO_DIR/scripts/.launch.conf" 2>/dev/null || true

PROFILE="$(cat scripts/.active 2>/dev/null || echo unknown)"
HEARTBEAT="${HEARTBEAT_PATH:-logs/heartbeat.jsonl}"
ENGINE_PID_FILE="${ENGINE_PID_PATH:-logs/.engine.pid}"
WATCHDOG_LOCK="${WATCHDOG_LOCK_DIR:-logs/.watchdog.lock}"
FAILURES=0
WARNINGS=0
FIXES=()

emit() {
  local level="$1" key="$2" value="$3"
  printf '%-7s %-28s %s\n' "$level" "$key" "$value"
}

fail() { FAILURES=$((FAILURES + 1)); emit "CRITICAL" "$1" "$2"; }
warn() { WARNINGS=$((WARNINGS + 1)); emit "WARN" "$1" "$2"; }
pass() { emit "PASS" "$1" "$2"; }
fix_note() { FIXES+=("$1"); emit "FIXED" "repair" "$1"; }

pid_is_engine() {
  local pid="$1"
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  [[ "$(ps -o comm= -p "$pid" 2>/dev/null | tr -d ' ')" == "rtc_headless" ]]
}

venue_runtime_active() {
  local venue
  for venue in binance mt5 ibkr bist; do
    if systemctl --user is-active --quiet "memos-venue-engine@${venue}.service" 2>/dev/null; then
      return 0
    fi
  done
  return 1
}

if venue_runtime_active; then
  echo "Venue-local runtime readiness:"
  python3 scripts/venue_readiness.py all |
    python3 -c '
import json
import sys

data = json.load(sys.stdin)
failed = False
for venue, item in data.items():
    overall = item.get("overall")
    if overall == "NOT_ACTIVE":
        continue
    engine = "UP" if item.get("engine", {}).get("up") else "DOWN"
    watchdog = "UP" if item.get("watchdog", {}).get("up") else "DOWN"
    print(f"{venue}: {overall} engine={engine} watchdog={watchdog}")
    if overall not in {"READY", "SIGNAL_READY"}:
        failed = True
raise SystemExit(1 if failed else 0)
'
  exit $?
fi

if [[ -f "$ENGINE_PID_FILE" ]]; then
  ENGINE_PID="$(tr -d '[:space:]' < "$ENGINE_PID_FILE")"
else
  ENGINE_PID=""
fi

if pid_is_engine "$ENGINE_PID"; then
  pass "engine" "rtc_headless PID=$ENGINE_PID"
else
  if [[ -n "$ENGINE_PID" && "$FIX" -eq 1 ]]; then
    rm -f "$ENGINE_PID_FILE"
    fix_note "sahipsiz engine PID dosyası temizlendi"
  fi
  fail "engine" "engine PID dosyası canlı rtc_headless göstermiyor"
fi

ENGINE_COUNT="$(pgrep -x rtc_headless 2>/dev/null | wc -l | tr -d ' ')"
if [[ "$ENGINE_COUNT" == "1" ]]; then
  pass "engine_count" "tek rtc_headless"
elif [[ "$ENGINE_COUNT" == "0" ]]; then
  fail "engine_count" "rtc_headless bulunamadı"
else
  fail "engine_count" "$ENGINE_COUNT rtc_headless bulundu; port/process yarışı riski"
fi

if systemctl --user is-active --quiet memos-watchdog.service 2>/dev/null; then
  pass "watchdog" "memos-watchdog.service active"
else
  if (( FIX )); then
    warn "watchdog" "legacy memos-watchdog.service etkin değil; venue-local runtime kullanılıyor"
  else
    fail "watchdog" "memos-watchdog.service active değil"
  fi
fi

if [[ -f "$WATCHDOG_LOCK/pid" ]]; then
  WATCHDOG_PID="$(tr -d '[:space:]' < "$WATCHDOG_LOCK/pid")"
  if [[ "$WATCHDOG_PID" =~ ^[0-9]+$ ]] && kill -0 "$WATCHDOG_PID" 2>/dev/null; then
    pass "watchdog_lock" "sahibi canlı PID=$WATCHDOG_PID"
  elif (( FIX )); then
    rm -rf "$WATCHDOG_LOCK"
    fix_note "sahipsiz watchdog lock temizlendi"
  else
    fail "watchdog_lock" "sahipsiz watchdog lock var"
  fi
else
  warn "watchdog_lock" "lock sahibi okunamadı"
fi

PORT_OWNER="$(ss -ltnp '( sport = :9001 )' 2>/dev/null | sed -n 's/.*pid=\([0-9]\+\).*/\1/p' | head -1)"
if [[ -n "$PORT_OWNER" ]] && pid_is_engine "$PORT_OWNER"; then
  pass "mt5_bridge" "127.0.0.1:9001 rtc_headless PID=$PORT_OWNER"
elif [[ -n "$PORT_OWNER" ]]; then
  fail "mt5_bridge" "9001 sahibi rtc_headless değil: PID=$PORT_OWNER"
else
  fail "mt5_bridge" "127.0.0.1:9001 dinlenmiyor"
fi

if [[ -f "$HEARTBEAT" ]]; then
  HB_MTIME="$(stat -c %Y "$HEARTBEAT" 2>/dev/null || echo 0)"
  HB_AGE="$(( $(date +%s) - HB_MTIME ))"
  HB_AGE="${HB_AGE:-999999}"
  if (( HB_AGE <= 180 )); then
    pass "heartbeat" "fresh age=${HB_AGE}s path=$HEARTBEAT"
  else
    fail "heartbeat" "stale age=${HB_AGE}s path=$HEARTBEAT"
  fi
  LAST="$(tail -n 1 "$HEARTBEAT" 2>/dev/null)"
  if [[ -n "$LAST" ]]; then
    read -r HB_PHASE HB_ANOM HB_RUNTIME HB_SCHED <<<"$(python3 -c '
import json,sys
try:
    x=json.loads(sys.argv[1]); v=next(iter((x.get("venue_runtime") or {}).values()), {})
    lock=(x.get("runtime_lock_diagnostic") or {}).get("phase") or "idle"
    scheduler=v.get("scheduler_phase") or "NotStarted"
    print(x.get("phase") or "?", x.get("anomalies", "?"), lock, scheduler)
except (ValueError, TypeError, IndexError):
    print("? ? ? ?")
' "$LAST" 2>/dev/null || echo "? ? ? ?")"
    case "$HB_PHASE" in
      Scanning) pass "heartbeat_phase" "Scanning anomalies=$HB_ANOM scheduler=$HB_SCHED" ;;
      Recovering) warn "heartbeat_phase" "Recovering anomalies=$HB_ANOM scheduler=$HB_SCHED" ;;
      LockContended|AccountError) fail "heartbeat_phase" "$HB_PHASE anomalies=$HB_ANOM scheduler=$HB_SCHED" ;;
      *) warn "heartbeat_phase" "$HB_PHASE anomalies=$HB_ANOM scheduler=$HB_SCHED" ;;
    esac
    [[ "$HB_RUNTIME" == "idle" || -z "$HB_RUNTIME" ]] || warn "runtime_lock" "$HB_RUNTIME"
  fi
else
  fail "heartbeat" "dosya yok: $HEARTBEAT"
fi

symbol_policy_bad=0
for symbol_list in "${PINNED_SYMBOLS:-}" "${AUTO_MT5_SYMBOLS:-}"; do
  IFS=',' read -r -a symbol_items <<< "$symbol_list"
  for symbol_item in "${symbol_items[@]}"; do
    symbol_item="$(echo "$symbol_item" | tr -d '[:space:]')"
    [[ -z "$symbol_item" ]] && continue
    if [[ "${VENUES:-}" == "mt5:cfd" && "$symbol_item" != "EURUSD@mt5" ]]; then
      symbol_policy_bad=1
    elif [[ "$symbol_item" == *@mt5 ]] &&
         [[ ",${MT5_SYMBOL_ALLOWLIST:-}," != *",${symbol_item},"* ]]; then
      symbol_policy_bad=1
    fi
  done
done
if (( symbol_policy_bad == 0 )) && [[ -n "${MT5_SYMBOL_ALLOWLIST:-}" ]]; then
  pass "symbol_policy" "MT5 sembolleri allowlist içinde"
else
  warn "symbol_policy" "allowlist dışında MT5 sembol veya allowlist eksik"
fi

if [[ "${LIVE_EXECUTION_ARMED:-0}" == "0" && "${LIVE_GO_NO_GO:-NO_GO}" == "NO_GO" ]]; then
  pass "execution_gate" "kapalı (armed=0, go_no_go=NO_GO)"
else
  warn "execution_gate" "açık veya beklenmeyen değer: armed=${LIVE_EXECUTION_ARMED:-unset} go_no_go=${LIVE_GO_NO_GO:-unset}"
fi

echo
echo "profile=$PROFILE failures=$FAILURES warnings=$WARNINGS fixes=${#FIXES[@]}"

(( FAILURES == 0 )) && exit 0
exit 1
