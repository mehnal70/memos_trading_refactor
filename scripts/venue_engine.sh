#!/usr/bin/env bash
# Venue başına izole headless engine + watchdog yöneticisi.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

# Profil ayarlarına ek olarak kök .env içindeki sırları miras al.
. "$REPO_DIR/scripts/lib_launchconf.sh"
load_launch_conf "$REPO_DIR/.env"

RUNTIME_DIR="${VENUE_RUNTIME_DIR:-$REPO_DIR/logs/.venue-runtime}"
PROFILE_DIR="$RUNTIME_DIR/profiles"
STATE_DIR="$RUNTIME_DIR/state"
mkdir -p "$PROFILE_DIR" "$STATE_DIR"

venue_profile() {
  case "$1" in
    mt5) echo "paper-mt5" ;;
    ibkr) echo "paper-ibkr" ;;
    binance) echo "paper-binance-testnet" ;;
    bist) echo "paper-bist-signal" ;;
    *) return 1 ;;
  esac
}

runtime_conf() {
  local venue="$1" profile
  profile="$(venue_profile "$venue")"
  local conf="$PROFILE_DIR/$venue.conf"
  {
    cat "$REPO_DIR/profiles/_base.conf" "$REPO_DIR/profiles/$profile.conf"
    echo "ENGINE_PID_PATH=$STATE_DIR/$venue.engine.pid"
    echo "ENGINE_STDOUT_PATH=$STATE_DIR/$venue.engine.stdout.log"
    echo "ENGINE_STDERR_PATH=$STATE_DIR/$venue.engine.stderr.log"
    if [ "$venue" = "ibkr" ]; then
      # Her headless engine kendi MT5 listener adresini kullansın; IBKR
      # profili MT5 venue'sini kullanmasa da core listener'ı açabilir.
      echo "MT5_BRIDGE_ADDR=127.0.0.1:9201"
      echo "METRICS_PORT=9200"
    elif [ "$venue" = "binance" ]; then
      # Core listener'ları diğer venue engine'lerinden ayır.
      echo "MT5_BRIDGE_ADDR=127.0.0.1:9301"
      echo "METRICS_PORT=9300"
    elif [ "$venue" = "bist" ]; then
      # BIST signal-only profili TCP bridge kullanmaz; yine de core
      # listener/metrics portları diğer headless engine'lerinden ayrıdır.
      echo "MT5_BRIDGE_ADDR=127.0.0.1:9401"
      echo "METRICS_PORT=9400"
    fi
  } > "$conf"
  echo "$conf"
}

engine_pid_file() { echo "$STATE_DIR/$1.engine.pid"; }
watchdog_pid_file() { echo "$STATE_DIR/$1.watchdog.pid"; }

pid_alive() {
  [ -n "${1:-}" ] && kill -0 "$1" 2>/dev/null
}

headless_mt5_owner_active() {
  [[ "$1" == "mt5" ]] || return 1
  systemctl --user is-active --quiet memos-watchdog.service 2>/dev/null &&
    pgrep -x rtc_headless >/dev/null 2>&1
}

start_watchdog() {
  local venue="$1" conf="$2" pid_file watchdog_file
  pid_file="$(engine_pid_file "$venue")"
  watchdog_file="$(watchdog_pid_file "$venue")"
  if [ -s "$watchdog_file" ] && pid_alive "$(cat "$watchdog_file")"; then
    echo "[$venue] watchdog zaten çalışıyor (PID $(cat "$watchdog_file"))"
    return 0
  fi
  RTC_LAUNCH_CONF="$conf" ENGINE_PID_PATH="$pid_file" \
    WATCHDOG_PAUSE_FILE="$STATE_DIR/$venue.watchdog.pause" \
    WATCHDOG_LOCK_DIR="$STATE_DIR/$venue.watchdog.lock" \
    WATCHDOG_LOG_PATH="$STATE_DIR/$venue.watchdog.log" \
    nohup "$REPO_DIR/scripts/watchdog.sh" --release \
    >"$STATE_DIR/$venue.watchdog.log" 2>&1 &
  echo "$!" > "$watchdog_file"
  echo "[$venue] watchdog başlatıldı (PID $!)"
}

start_venue() {
  local venue="$1" conf pid_file
  if headless_mt5_owner_active "$venue"; then
    echo "[mt5] headless supervisor aktif; bağımsız MT5 venue engine başlatılmadı" >&2
    return 0
  fi
  conf="$(runtime_conf "$venue")"
  pid_file="$(engine_pid_file "$venue")"
  if [ -s "$pid_file" ] && pid_alive "$(cat "$pid_file")"; then
    echo "[$venue] engine zaten çalışıyor (PID $(cat "$pid_file")); yeniden başlatılmadı"
    start_watchdog "$venue" "$conf"
    return 0
  fi
  RTC_LAUNCH_CONF="$conf" ENGINE_PID_PATH="$pid_file" \
    "$REPO_DIR/scripts/engine.sh" start --release
  start_watchdog "$venue" "$conf"
}

stop_venue() {
  local venue="$1" conf pid_file watchdog_file watchdog_pid
  conf="$(runtime_conf "$venue")"
  pid_file="$(engine_pid_file "$venue")"
  watchdog_file="$(watchdog_pid_file "$venue")"
  if [ -s "$watchdog_file" ]; then
    watchdog_pid="$(cat "$watchdog_file")"
    pid_alive "$watchdog_pid" && kill "$watchdog_pid" || true
    rm -f "$watchdog_file"
  fi
  RTC_LAUNCH_CONF="$conf" ENGINE_PID_PATH="$pid_file" \
    "$REPO_DIR/scripts/engine.sh" stop || true
}

status_venue() {
  local venue="$1" conf pid_file watchdog_file pid
  conf="$(runtime_conf "$venue")"
  pid_file="$(engine_pid_file "$venue")"
  watchdog_file="$(watchdog_pid_file "$venue")"
  pid="$(cat "$pid_file" 2>/dev/null || true)"
  if pid_alive "$pid"; then
    echo "$venue engine=up pid=$pid watchdog=$(if [ -s "$watchdog_file" ] && pid_alive "$(cat "$watchdog_file")"; then echo up; else echo down; fi)"
  else
    echo "$venue engine=down watchdog=down"
  fi
}

run_venue() {
  local venue="$1" conf pid_file
  conf="$(runtime_conf "$venue")"
  pid_file="$(engine_pid_file "$venue")"
  printf '%s\n' "$$" > "$pid_file"
  RTC_LAUNCH_CONF="$conf" ENGINE_PID_PATH="$pid_file" \
    exec "$REPO_DIR/scripts/_engine_exec.sh" "$REPO_DIR/target/release/rtc_headless"
}

command="${1:-status}"
target="${2:-all}"
case "$command" in
  start|stop|status|run)
    if [ "$target" = "all" ]; then
      targets=(mt5 ibkr binance bist)
    else
      targets=("$target")
    fi
    for venue in "${targets[@]}"; do
      case "$command" in
        start) start_venue "$venue" ;;
        stop) stop_venue "$venue" ;;
        status) status_venue "$venue" ;;
        run)
          if [ "$target" = "all" ]; then
            echo "run komutu tek venue ister" >&2
            exit 2
          fi
          run_venue "$venue"
          ;;
      esac
    done
    ;;
  *)
    echo "Kullanim: $0 start|stop|status|run [mt5|ibkr|binance|bist|all]" >&2
    exit 2
    ;;
esac
