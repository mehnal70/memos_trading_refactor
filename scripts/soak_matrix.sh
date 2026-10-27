#!/usr/bin/env bash
# Kontrollu multi-venue paper/dry-run soak matrisi.
#
# Varsayilan profiller: IBKR paper, Binance testnet dry-run ve MT5 paper.
# Her profil kendi DB, PID, heartbeat ve log dizininde calisir.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOAK_ROOT="${SOAK_ROOT:-$REPO_DIR/logs/soak}"
STATE_ROOT="${STATE_ROOT:-$REPO_DIR/data/profiles/soak}"
BIN="${SOAK_BINARY:-$REPO_DIR/target/release/rtc_headless}"

declare -A PROFILE_FILES=(
  [ibkr]="$REPO_DIR/profiles/paper-ibkr.conf"
  [binance]="$REPO_DIR/profiles/paper-binance-testnet.conf"
  [mt5]="$REPO_DIR/profiles/paper-mt5.conf"
  [bist]="$REPO_DIR/profiles/paper-bist-signal.conf"
)
declare -A METRICS_PORTS=(
  [ibkr]=9111
  [binance]=9112
  [mt5]=9113
  [bist]=9114
)

die() { echo "soak_matrix: $*" >&2; exit 1; }

profile_pid_file() { echo "$SOAK_ROOT/$1/engine.pid"; }

pid_alive() {
  local pid="${1:-}"
  [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null
}

tcp_listening() {
  local host="$1" port="$2"
  if command -v nc >/dev/null 2>&1; then
    nc -z -w 2 "$host" "$port" >/dev/null 2>&1
    return $?
  fi
  timeout 2 bash -lc "</dev/tcp/${host}/${port}" >/dev/null 2>&1
}

ibkr_stack_state() {
  local profile="$REPO_DIR/profiles/paper-ibkr.conf"
  local api_port bridge_addr bridge_port
  api_port="$(grep -E '^IBKR_API_PORT=' "$profile" | tail -1 | cut -d= -f2- || true)"
  bridge_addr="$(grep -E '^IBKR_BRIDGE_ADDR=' "$profile" | tail -1 | cut -d= -f2- || true)"
  api_port="${api_port:-4002}"
  bridge_addr="${bridge_addr:-127.0.0.1:9101}"
  bridge_port="${bridge_addr##*:}"

  local gateway="DOWN" api="DOWN" bridge="DOWN"
  ps -ef | grep -Ei 'ibgateway|trader workstation|tws' | grep -v grep >/dev/null 2>&1 &&
    gateway="UP"
  tcp_listening "127.0.0.1" "$api_port" && api="READY"
  tcp_listening "127.0.0.1" "$bridge_port" && bridge="READY"
  printf 'gateway=%s api=%s bridge=%s' "$gateway" "$api" "$bridge"
}

assert_safe_profile() {
  local name="$1" file="$2"
  [[ -f "$file" ]] || die "profil bulunamadi: $file"
  local trading_mode live_dry_run ibkr_exec ibkr_read_only
  trading_mode="$(grep -E '^TRADING_MODE=' "$file" | tail -1 | cut -d= -f2- || true)"
  live_dry_run="$(grep -E '^LIVE_DRY_RUN=' "$file" | tail -1 | cut -d= -f2- || true)"
  ibkr_exec="$(grep -E '^IBKR_EXECUTION_ENABLED=' "$file" | tail -1 | cut -d= -f2- || true)"
  ibkr_read_only="$(grep -E '^IBKR_READ_ONLY=' "$file" | tail -1 | cut -d= -f2- || true)"
  [[ "${live_dry_run,,}" == "true" || "$live_dry_run" == "1" ]] ||
    die "$name profili LIVE_DRY_RUN=true olmadan soak'a alinamaz"
  [[ "${ibkr_exec,,}" != "1" && "${ibkr_exec,,}" != "true" ]] ||
    die "$name profili IBKR execution acik"
  [[ "${ibkr_read_only,,}" != "0" && "${ibkr_read_only,,}" != "false" ]] ||
    die "$name profili IBKR_READ_ONLY kapali"
  [[ "${trading_mode,,}" == "paper" || "${live_dry_run,,}" == "true" || "$live_dry_run" == "1" ]] ||
    die "$name profili paper/dry-run degil"
}

start_one() {
  local name="$1" file="${PROFILE_FILES[$1]}"
  assert_safe_profile "$name" "$file"
  local dir="$SOAK_ROOT/$name" state_db="$STATE_ROOT-$name.db"
  local pid_file="$dir/engine.pid"
  mkdir -p "$dir" "$STATE_ROOT"
  if [[ -f "$pid_file" ]] && pid_alive "$(cat "$pid_file")"; then
    echo "$name zaten calisiyor (pid=$(cat "$pid_file"))"
    return 0
  fi
  [[ -x "$BIN" ]] || die "binary yok: $BIN (cargo build --release -p rtc_headless)"
  (
    set -a
    # Profil dosyalari repository kontrollu key=value konfigurasyonudur.
    source "$file"
    set +a
    export LIVE_DRY_RUN=true
    export ENGINE_PID_PATH="$pid_file"
    export STATE_DB_PATH="$state_db"
    export DB_PATH="$state_db"
    export SNAPSHOT_PATH="$dir/snapshot.json"
    export HEARTBEAT_PATH="$dir/heartbeat.jsonl"
    export TRADING_LOG_PATH="$dir/robotic_trading.log"
    export TRADES_LOG_PATH="$dir/trades.jsonl"
    export ENGINE_STDOUT_PATH="$dir/stdout.log"
    export ENGINE_STDERR_PATH="$dir/stderr.log"
    export METRICS_PORT="${METRICS_PORTS[$name]}"
    exec setsid "$BIN"
  ) &
  local launcher_pid=$!
  echo "$launcher_pid" > "$pid_file"
  echo "$name baslatildi (pid=$launcher_pid, profile=$file)"
}

stop_one() {
  local name="$1" pid_file
  pid_file="$(profile_pid_file "$name")"
  [[ -f "$pid_file" ]] || { echo "$name calismiyor"; return 0; }
  local pid
  pid="$(cat "$pid_file")"
  if pid_alive "$pid"; then
    kill "$pid"
    for _ in {1..50}; do
      pid_alive "$pid" || break
      sleep 0.1
    done
    if pid_alive "$pid"; then
      echo "$name graceful kapanmadi (pid=$pid); kill -9 kullanilmadi" >&2
      return 1
    fi
  fi
  rm -f "$pid_file"
  echo "$name durduruldu"
}

status_one() {
  local name="$1" dir="$SOAK_ROOT/$1" pid_file
  pid_file="$(profile_pid_file "$name")"
  local pid="-" state="STOPPED"
  if [[ -f "$pid_file" ]]; then
    pid="$(cat "$pid_file")"
    pid_alive "$pid" && state="RUNNING" || state="STALE_PID"
  fi
  if [[ "$name" == "ibkr" && "$state" == "STALE_PID" ]]; then
    state="STALE_HEADLESS_OWNER ($(ibkr_stack_state))"
  fi
  local heartbeat="$dir/heartbeat.jsonl"
  local hb_age="missing"
  if [[ -f "$heartbeat" ]]; then
    hb_age="$(( $(date +%s) - $(stat -c %Y "$heartbeat") ))s"
  fi
  printf '%-8s %-10s pid=%-8s heartbeat_age=%s\n' "$name" "$state" "$pid" "$hb_age"
}

usage() {
  cat <<'EOF'
Kullanim:
  scripts/soak_matrix.sh preflight
  scripts/soak_matrix.sh start [all|ibkr|binance|mt5|bist]
  scripts/soak_matrix.sh stop  [all|ibkr|binance|mt5|bist]
  scripts/soak_matrix.sh status
EOF
}

command="${1:-status}"
target="${2:-all}"
case "$command" in
  preflight)
    [[ -x "$BIN" ]] || die "binary yok: $BIN"
    for name in ibkr binance mt5 bist; do
      assert_safe_profile "$name" "${PROFILE_FILES[$name]}"
      echo "$name guvenli profil ve dosya kontrolleri: OK"
    done
    echo "soak binary: $BIN"
    ;;
  start|stop)
    [[ "$target" == "all" ]] && targets=(ibkr binance mt5 bist) || targets=("$target")
    for name in "${targets[@]}"; do
      [[ -v "PROFILE_FILES[$name]" ]] || die "bilinmeyen venue: $name"
      [[ "$command" == "start" ]] && start_one "$name" || stop_one "$name"
    done
    ;;
  status)
    for name in ibkr binance mt5 bist; do status_one "$name"; done
    ;;
  *) usage; exit 2 ;;
esac
