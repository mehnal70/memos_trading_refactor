#!/usr/bin/env bash
# scripts/autonomy.sh - Memos icin olabildigince tam otonom isletim yardimcisi.
#
# Amac:
#   - Runtime'i kendi kendine toparlamak (watchdog/motor/panel/bot)
#   - MT5 mum senkronunu port cakismasiz yonetmek (gerekirse gecici down/up)
#   - systemd --user timer ile periyodik otomatik dongu calistirmak

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

AUTO_ENV_FILE="$REPO_DIR/scripts/.autonomy.env"

AUTO_SERVICE="memos-autonomy"
AUTO_UNIT="$HOME/.config/systemd/user/${AUTO_SERVICE}.service"
AUTO_TIMER_UNIT="$HOME/.config/systemd/user/${AUTO_SERVICE}.timer"
AUTO_ONCALENDAR="${AUTO_ONCALENDAR:-*:0/30}"

# load runtime/env values if present
# shellcheck source=scripts/lib_launchconf.sh
. "$REPO_DIR/scripts/lib_launchconf.sh" 2>/dev/null || true
# Kalici varsayimlar once yuklenir; aktif profil (.launch.conf) bunlari gerekirse ezer.
load_launch_conf "$AUTO_ENV_FILE" 2>/dev/null || true
load_launch_conf "$REPO_DIR/scripts/.launch.conf" 2>/dev/null || true
load_launch_conf "$REPO_DIR/.env" 2>/dev/null || true

AUTO_MT5_MARKET="${AUTO_MT5_MARKET:-mt5}"
AUTO_MT5_INTERVAL="${AUTO_MT5_INTERVAL:-1h}"
AUTO_MT5_COUNT="${AUTO_MT5_COUNT:-200}"
AUTO_MT5_BOOTSTRAP="${AUTO_MT5_BOOTSTRAP:-0}"
AUTO_MT5_BOOTSTRAP_COUNT="${AUTO_MT5_BOOTSTRAP_COUNT:-1000}"
AUTO_MT5_BOOTSTRAP_STATE_FILE="${AUTO_MT5_BOOTSTRAP_STATE_FILE:-logs/.autonomy-mt5-bootstrap.done}"
AUTO_MT5_SYNC_STATE_FILE="${AUTO_MT5_SYNC_STATE_FILE:-logs/.autonomy-mt5-sync.state}"
AUTO_MT5_ESTAB_PROBE_ON_MISS="${AUTO_MT5_ESTAB_PROBE_ON_MISS:-1}"
AUTO_MT5_ESTAB_PROBE_MIN_SECS="${AUTO_MT5_ESTAB_PROBE_MIN_SECS:-300}"
AUTO_MT5_ESTAB_PROBE_STATE_FILE="${AUTO_MT5_ESTAB_PROBE_STATE_FILE:-logs/.autonomy-mt5-estab-probe.state}"
AUTO_COVERAGE_AUDIT_ENABLED="${AUTO_COVERAGE_AUDIT_ENABLED:-1}"
AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS="${AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS:-1}"
AUTO_COVERAGE_AUDIT_AUTOFIX="${AUTO_COVERAGE_AUDIT_AUTOFIX:-1}"
AUTO_COVERAGE_AUDIT_MARKET="${AUTO_COVERAGE_AUDIT_MARKET:-futures}"
AUTO_COVERAGE_AUDIT_INTERVALS="${AUTO_COVERAGE_AUDIT_INTERVALS:-1d,4h}"
AUTO_COVERAGE_AUDIT_MIN_ROWS="${AUTO_COVERAGE_AUDIT_MIN_ROWS:-300}"
AUTO_COVERAGE_AUDIT_STATE_FILE="${AUTO_COVERAGE_AUDIT_STATE_FILE:-logs/.autonomy-coverage-audit-date}"
AUTO_SYNC_MT5="${AUTO_SYNC_MT5:-1}"
AUTO_SESSION_HANDOFF="${AUTO_SESSION_HANDOFF:-0}"
AUTO_SESSION_HANDOFF_IMMEDIATE_RESTART="${AUTO_SESSION_HANDOFF_IMMEDIATE_RESTART:-0}"
AUTO_SESSION_HANDOFF_PENDING_FILE="${AUTO_SESSION_HANDOFF_PENDING_FILE:-logs/.autonomy-handoff.pending}"
AUTO_SESSION_HANDOFF_PENDING_APPLY="${AUTO_SESSION_HANDOFF_PENDING_APPLY:-1}"
AUTO_SESSION_HANDOFF_PENDING_MIN_AGE_SECS="${AUTO_SESSION_HANDOFF_PENDING_MIN_AGE_SECS:-120}"
AUTO_SESSION_HANDOFF_PENDING_REQUIRE_FLAT="${AUTO_SESSION_HANDOFF_PENDING_REQUIRE_FLAT:-1}"
AUTO_OPPORTUNITY_MAX="${AUTO_OPPORTUNITY_MAX:-0}"
AUTO_PINNED_EXPAND_ENABLED="${AUTO_PINNED_EXPAND_ENABLED:-0}"
AUTO_PINNED_MIN_COUNT="${AUTO_PINNED_MIN_COUNT:-0}"
AUTO_PINNED_MIN_ASIA="${AUTO_PINNED_MIN_ASIA:-}"
AUTO_PINNED_MIN_ASIA_EU="${AUTO_PINNED_MIN_ASIA_EU:-}"
AUTO_PINNED_MIN_EU="${AUTO_PINNED_MIN_EU:-}"
AUTO_PINNED_MIN_EU_US="${AUTO_PINNED_MIN_EU_US:-}"
AUTO_PINNED_MIN_US="${AUTO_PINNED_MIN_US:-}"
AUTO_PINNED_MIN_OFFHOURS="${AUTO_PINNED_MIN_OFFHOURS:-}"
AUTO_PINNED_REVIEW_ENABLED="${AUTO_PINNED_REVIEW_ENABLED:-1}"
AUTO_PINNED_REVIEW_INTERVAL_SECS="${AUTO_PINNED_REVIEW_INTERVAL_SECS:-600}"
AUTO_PINNED_REVIEW_AUTOFIX="${AUTO_PINNED_REVIEW_AUTOFIX:-1}"
AUTO_PINNED_REVIEW_TRIM_EXTRAS="${AUTO_PINNED_REVIEW_TRIM_EXTRAS:-0}"
AUTO_PINNED_REVIEW_MAX_ADD="${AUTO_PINNED_REVIEW_MAX_ADD:-4}"
AUTO_PINNED_REVIEW_RESTART_ON_CHANGE="${AUTO_PINNED_REVIEW_RESTART_ON_CHANGE:-0}"
AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD="${AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD:-6}"
AUTO_PINNED_REVIEW_STATE_FILE="${AUTO_PINNED_REVIEW_STATE_FILE:-logs/.autonomy-pinned-review.state}"
AUTO_CYCLE_FAST_EU_US_ONLY="${AUTO_CYCLE_FAST_EU_US_ONLY:-0}"
AUTO_CYCLE_BASE_MIN_SECS="${AUTO_CYCLE_BASE_MIN_SECS:-0}"
AUTO_CYCLE_EU_US_MIN_SECS="${AUTO_CYCLE_EU_US_MIN_SECS:-0}"
AUTO_CYCLE_STATE_FILE="${AUTO_CYCLE_STATE_FILE:-logs/.autonomy-cycle-last.state}"
AUTO_REQUIRE_MT5_ESTAB="${AUTO_REQUIRE_MT5_ESTAB:-0}"
AUTO_DAILY_ENABLED="${AUTO_DAILY_ENABLED:-0}"
AUTO_MT5_CHECK_ENABLED="${AUTO_MT5_CHECK_ENABLED:-1}"
AUTO_VENUE_HEALTHCHECK_ENABLED="${AUTO_VENUE_HEALTHCHECK_ENABLED:-1}"
AUTO_VENUE_HEALTHCHECK_STRICT="${AUTO_VENUE_HEALTHCHECK_STRICT:-0}"
AUTO_IBKR_CHECK_ENABLED="${AUTO_IBKR_CHECK_ENABLED:-1}"
AUTO_IBKR_CHECK_INTERVAL_SECS="${AUTO_IBKR_CHECK_INTERVAL_SECS:-300}"
AUTO_IBKR_CHECK_STATE_FILE="${AUTO_IBKR_CHECK_STATE_FILE:-logs/.autonomy-ibkr-check.state}"
AUTO_IBKR_API_PORT="${AUTO_IBKR_API_PORT:-${IBKR_API_PORT:-4002}}"
AUTO_IBKR_BRIDGE_ADDR="${AUTO_IBKR_BRIDGE_ADDR:-${IBKR_BRIDGE_ADDR:-127.0.0.1:9101}}"
AUTO_IBKR_BRIDGE_AUTOSTART="${AUTO_IBKR_BRIDGE_AUTOSTART:-1}"
AUTO_IBKR_BRIDGE_AUTOSTART_COOLDOWN_SECS="${AUTO_IBKR_BRIDGE_AUTOSTART_COOLDOWN_SECS:-600}"
AUTO_IBKR_BRIDGE_STATE_FILE="${AUTO_IBKR_BRIDGE_STATE_FILE:-logs/.autonomy-ibkr-bridge.state}"
AUTO_NOTIFY_CRITICAL="${AUTO_NOTIFY_CRITICAL:-0}"
AUTO_DAILY_SUMMARY="${AUTO_DAILY_SUMMARY:-0}"
AUTO_SUMMARY_STATE_FILE="${AUTO_SUMMARY_STATE_FILE:-logs/.autonomy-summary-date}"
AUTO_EVENT_LOG_PATH="${AUTO_EVENT_LOG_PATH:-${TRADING_LOG_PATH:-logs/robotic_trading.log}}"
AUTO_OPS_SNAPSHOT_ENABLED="${AUTO_OPS_SNAPSHOT_ENABLED:-0}"
AUTO_OPS_SNAPSHOT_INTERVAL_SECS="${AUTO_OPS_SNAPSHOT_INTERVAL_SECS:-3600}"
AUTO_OPS_SNAPSHOT_TIMELINE="${AUTO_OPS_SNAPSHOT_TIMELINE:-8}"
AUTO_OPS_SNAPSHOT_PROBE="${AUTO_OPS_SNAPSHOT_PROBE:-0}"
AUTO_OPS_SNAPSHOT_STATE_FILE="${AUTO_OPS_SNAPSHOT_STATE_FILE:-logs/.autonomy-ops-snapshot.state}"
AUTO_AUTONOMY_POLICY="${AUTO_AUTONOMY_POLICY:-core}"

log() { echo "[autonomy $(date '+%F %T')] $*"; }
warn() { echo "[autonomy $(date '+%F %T')] WARN: $*"; }

log_event() {
  local msg="${1:-}"
  [ -z "$msg" ] && return 0
  local ts
  ts="$(date -u +"%Y-%m-%dT%H:%M:%S.%N+00:00")"
  mkdir -p "$(dirname "$AUTO_EVENT_LOG_PATH")"
  printf '[%s] %-10s %-8s %-5s - %s\n' "$ts" "AUTONOMY" "SYSTEM" "INFO" "$msg" >> "$AUTO_EVENT_LOG_PATH" 2>/dev/null || true
}

is_true() {
  case "${1:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

trim() {
  local s="${1:-}"
  s="${s#${s%%[![:space:]]*}}"
  s="${s%${s##*[![:space:]]}}"
  echo "$s"
}

dedupe_csv() {
  local raw="${1:-}"
  local out=""
  local item
  declare -A seen=()
  IFS=',' read -r -a arr <<< "$raw"
  for item in "${arr[@]}"; do
    item="$(trim "$item")"
    [ -z "$item" ] && continue
    if [ -z "${seen[$item]:-}" ]; then
      seen[$item]=1
      out+="$item,"
    fi
  done
  echo "${out%,}"
}

csv_count() {
  local raw="${1:-}"
  local n=0
  local item
  IFS=',' read -r -a arr <<< "$raw"
  for item in "${arr[@]}"; do
    item="$(trim "$item")"
    [ -z "$item" ] && continue
    n=$((n + 1))
  done
  echo "$n"
}

csv_diff() {
  local left="$(dedupe_csv "${1:-}")"
  local right="$(dedupe_csv "${2:-}")"
  local out=""
  local item
  IFS=',' read -r -a arr <<< "$left"
  for item in "${arr[@]}"; do
    item="$(trim "$item")"
    [ -z "$item" ] && continue
    case ",$right," in
      *",$item,"*) ;;
      *) out="${out:+$out,}$item" ;;
    esac
  done
  echo "$out"
}

csv_take() {
  local raw="$(dedupe_csv "${1:-}")"
  local max="${2:-0}"
  local out=""
  local n=0
  local item
  case "$max" in ''|*[!0-9]*) max=0 ;; esac
  [ "$max" -le 0 ] && { echo "$raw"; return 0; }

  IFS=',' read -r -a arr <<< "$raw"
  for item in "${arr[@]}"; do
    item="$(trim "$item")"
    [ -z "$item" ] && continue
    [ "$n" -ge "$max" ] && break
    out="${out:+$out,}$item"
    n=$((n + 1))
  done
  echo "$out"
}

candidate_symbol_pool() {
  local all="${BLEND_LIVE_SYMBOLS:-},${XS_LIVE_SYMBOLS:-},${CARRY_LIVE_SYMBOLS:-},${AUTO_MT5_SYMBOLS:-},${AUTO_IBKR_SYMBOLS:-},${SESSION_CORE_SYMBOLS:-},${SESSION_ASIA_SYMBOLS:-},${SESSION_ASIA_EU_OVERLAP_SYMBOLS:-},${SESSION_EU_SYMBOLS:-},${SESSION_EU_US_OVERLAP_SYMBOLS:-},${SESSION_US_SYMBOLS:-},${SESSION_OFFHOURS_SYMBOLS:-}"
  dedupe_csv "$all"
}

remove_mt5_symbols() {
  local raw="${1:-}"
  local out=""
  local item venue
  IFS=',' read -r -a arr <<< "$raw"
  for item in "${arr[@]}"; do
    item="$(trim "$item")"
    [ -z "$item" ] && continue
    case "$item" in
      *@*)
        venue="${item##*@}"
        [ "$venue" = "mt5" ] && continue
        ;;
    esac
    out="${out:+$out,}$item"
  done
  echo "$out"
}

conf_get() {
  local key="$1"
  local file="${2:-$REPO_DIR/scripts/.launch.conf}"
  [ -f "$file" ] || { echo ""; return 0; }
  grep -m1 "^${key}=" "$file" | cut -d= -f2-
}

conf_upsert() {
  local key="$1"
  local val="$2"
  local file="${3:-$REPO_DIR/scripts/.launch.conf}"
  mkdir -p "$(dirname "$file")"
  touch "$file"
  if grep -q "^${key}=" "$file"; then
    sed -i "s|^${key}=.*|${key}=${val}|" "$file"
  else
    echo "${key}=${val}" >> "$file"
  fi
}

interval_to_secs() {
  local interval="${1:-}"
  case "$interval" in
    *m) echo $(( ${interval%m} * 60 )) ;;
    *h) echo $(( ${interval%h} * 3600 )) ;;
    *d) echo $(( ${interval%d} * 86400 )) ;;
    *) echo 0 ;;
  esac
}

current_interval_bucket_start() {
  local interval="${1:-}"
  local secs now
  secs="$(interval_to_secs "$interval")"
  if [ -z "$secs" ] || [ "$secs" -le 0 ]; then
    return 1
  fi
  now="$(date +%s)"
  echo $(( now - (now % secs) ))
}

mt5_sync_bucket_key() {
  local market="$1"
  local interval="$2"
  local symbols="$3"
  local bucket
  bucket="$(current_interval_bucket_start "$interval")" || return 1
  symbols="${symbols// /}"
  echo "${market}|${interval}|${symbols}|${bucket}"
}

mt5_sync_already_done_for_bucket() {
  local market="$1"
  local interval="$2"
  local symbols="$3"
  local want have
  want="$(mt5_sync_bucket_key "$market" "$interval" "$symbols")" || return 1
  [ -f "$AUTO_MT5_SYNC_STATE_FILE" ] || return 1
  have="$(cat "$AUTO_MT5_SYNC_STATE_FILE" 2>/dev/null || true)"
  [ "$want" = "$have" ]
}

mark_mt5_sync_done_for_bucket() {
  local market="$1"
  local interval="$2"
  local symbols="$3"
  local key
  key="$(mt5_sync_bucket_key "$market" "$interval" "$symbols")" || return 0
  mkdir -p "$(dirname "$AUTO_MT5_SYNC_STATE_FILE")"
  echo "$key" > "$AUTO_MT5_SYNC_STATE_FILE"
}

coverage_audit_due() {
  local today last session
  today="$(date +%F)"
  last=""
  [ -f "$AUTO_COVERAGE_AUDIT_STATE_FILE" ] && last="$(cat "$AUTO_COVERAGE_AUDIT_STATE_FILE" 2>/dev/null || true)"
  [ "$last" = "$today" ] && return 1
  if is_true "$AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS"; then
    session="$(resolve_session_name)"
    [ "$session" = "offhours" ] || return 1
  fi
  return 0
}

mark_coverage_audit_done() {
  mkdir -p "$(dirname "$AUTO_COVERAGE_AUDIT_STATE_FILE")"
  date +%F > "$AUTO_COVERAGE_AUDIT_STATE_FILE"
}

utc_hour() { date -u +%H; }

resolve_session_name() {
  local forced="${AUTO_SESSION_FORCE:-auto}"
  local h h_dec
  if [ "$forced" != "auto" ]; then
    echo "$forced"
    return 0
  fi
  h="$(utc_hour)"
  h_dec=$((10#$h))
  if (( h_dec >= 0 && h_dec <= 6 )); then
    echo "asia"
  elif (( h_dec >= 7 && h_dec <= 8 )); then
    echo "asia-eu"
  elif (( h_dec >= 9 && h_dec <= 12 )); then
    echo "eu"
  elif (( h_dec >= 13 && h_dec <= 16 )); then
    echo "eu-us"
  elif (( h_dec >= 17 && h_dec <= 21 )); then
    echo "us"
  else
    echo "offhours"
  fi
}

cycle_min_secs_for_session() {
  local session="${1:-offhours}"
  local base="${AUTO_CYCLE_BASE_MIN_SECS:-0}"
  local euus="${AUTO_CYCLE_EU_US_MIN_SECS:-0}"

  case "$base" in ''|*[!0-9]*) base=0 ;; esac
  case "$euus" in ''|*[!0-9]*) euus=0 ;; esac

  if is_true "$AUTO_CYCLE_FAST_EU_US_ONLY"; then
    if [ "$session" = "eu-us" ]; then
      echo "$euus"
    else
      echo "$base"
    fi
    return 0
  fi

  if [ "$euus" -gt 0 ]; then
    echo "$euus"
  else
    echo "$base"
  fi
}

cycle_throttle_allows_run() {
  local session="${1:-offhours}"
  local min_secs now last_ts last_session
  min_secs="$(cycle_min_secs_for_session "$session")"
  case "$min_secs" in ''|*[!0-9]*) min_secs=0 ;; esac
  [ "$min_secs" -le 0 ] && return 0

  now="$(date +%s)"
  last_ts=0
  last_session="-"
  if [ -f "$AUTO_CYCLE_STATE_FILE" ]; then
    IFS='|' read -r last_ts last_session < "$AUTO_CYCLE_STATE_FILE" || true
    last_ts="${last_ts:-0}"
  fi
  case "$last_ts" in ''|*[!0-9]*) last_ts=0 ;; esac

  if [ "$last_ts" -gt 0 ] && [ $((now - last_ts)) -lt "$min_secs" ]; then
    log "cycle throttle: seans=$session min=${min_secs}s kalan=$((min_secs - (now - last_ts)))s"
    log_event "cycle_throttle seans=$session min=${min_secs}s kalan=$((min_secs - (now - last_ts)))s"
    return 1
  fi

  mkdir -p "$(dirname "$AUTO_CYCLE_STATE_FILE")"
  echo "${now}|${session}" > "$AUTO_CYCLE_STATE_FILE"
  return 0
}

extract_mt5_from_csv() {
  local raw="${1:-}"
  local out=""
  local item sym venue
  IFS=',' read -r -a arr <<< "$raw"
  for item in "${arr[@]}"; do
    item="$(trim "$item")"
    case "$item" in
      *@*)
        sym="${item%@*}"
        venue="${item##*@}"
        if [ "$venue" = "mt5" ] && mt5_symbol_allowed "${sym}@mt5"; then
          out+="${sym}@mt5,"
        fi
        ;;
    esac
  done
  echo "${out%,}"
}

mt5_only_profile() {
  [ "${VENUES:-}" = "mt5:cfd" ] || [ "${AUTO_AUTONOMY_POLICY:-}" = "mt5-only" ]
}

normalize_pinned_symbols() {
  local raw="${1:-}"
  local out=""
  local item sym venue
  IFS=',' read -r -a arr <<< "$raw"
  for item in "${arr[@]}"; do
    item="$(trim "$item")"
    [ -z "$item" ] && continue
    case "$item" in
      *@*)
        sym="${item%@*}"
        venue="${item##*@}"
        if mt5_only_profile && [ "$venue" != "mt5" ]; then
          continue
        fi
        if [ "$venue" = "mt5" ] && ! mt5_symbol_allowed "$item"; then
          continue
        fi
        # Binance sembolleri download pipeline'ında çıplak olmalı.
        if [ "$venue" = "binance" ]; then
          out+="${sym},"
        else
          out+="${item},"
        fi
        ;;
      *)
        if mt5_only_profile && ! mt5_symbol_allowed "${item}@mt5"; then
          continue
        fi
        out+="${item},"
        ;;
    esac
  done
  dedupe_csv "${out%,}"
}

mt5_symbol_allowed() {
  local symbol="${1:-}"
  local allowlist="${MT5_SYMBOL_ALLOWLIST:-}"
  [ -z "$allowlist" ] && return 0
  local item
  IFS=',' read -r -a allowed_items <<< "$allowlist"
  for item in "${allowed_items[@]}"; do
    item="$(trim "$item")"
    [ -z "$item" ] && continue
    [ "${item^^}" = "${symbol^^}" ] && return 0
  done
  return 1
}

expand_pinned_symbols() {
  local raw="${1:-}"
  local mt5_ready="${2:-1}"
  local min_override="${3:-}"

  if ! is_true "$AUTO_PINNED_EXPAND_ENABLED"; then
    echo "$(dedupe_csv "$raw")"
    return 0
  fi

  local min="${AUTO_PINNED_MIN_COUNT:-0}"
  [ -n "$min_override" ] && min="$min_override"
  case "$min" in
    ''|*[!0-9]*) min=0 ;;
  esac
  if [ "$min" -le 0 ]; then
    echo "$(dedupe_csv "$raw")"
    return 0
  fi

  local expanded current
  expanded="$(dedupe_csv "$raw")"
  current="$(csv_count "$expanded")"
  if [ "$current" -ge "$min" ]; then
    echo "$expanded"
    return 0
  fi

  local candidates=""
  local blend="${BLEND_LIVE_SYMBOLS:-}"
  local xs="${XS_LIVE_SYMBOLS:-}"
  local carry="${CARRY_LIVE_SYMBOLS:-}"
  local all_sessions="${SESSION_CORE_SYMBOLS:-},${SESSION_ASIA_SYMBOLS:-},${SESSION_ASIA_EU_OVERLAP_SYMBOLS:-},${SESSION_EU_SYMBOLS:-},${SESSION_EU_US_OVERLAP_SYMBOLS:-},${SESSION_US_SYMBOLS:-},${SESSION_OFFHOURS_SYMBOLS:-}"
  candidates="${blend},${xs},${carry},${AUTO_MT5_SYMBOLS:-},${AUTO_IBKR_SYMBOLS:-},${all_sessions}"
  candidates="$(dedupe_csv "$candidates")"

  local item venue sym
  IFS=',' read -r -a arr <<< "$candidates"
  for item in "${arr[@]}"; do
    [ "$current" -ge "$min" ] && break
    item="$(trim "$item")"
    [ -z "$item" ] && continue
    case "$item" in
      *@*)
        sym="${item%@*}"
        venue="${item##*@}"
        if [ "$venue" = "mt5" ] && [ "$mt5_ready" != "1" ]; then
          continue
        fi
        if [ "$venue" = "binance" ]; then
          item="$sym"
        fi
        ;;
    esac

    case ",${expanded}," in
      *",${item},"*)
        continue
        ;;
      *)
        expanded="${expanded:+$expanded,}${item}"
        current=$((current + 1))
        ;;
    esac
  done

  echo "$expanded"
}

resolve_pinned_min_for_session() {
  local session="${1:-offhours}"
  local min="${AUTO_PINNED_MIN_COUNT:-0}"

  case "$session" in
    asia)      [ -n "${AUTO_PINNED_MIN_ASIA:-}" ] && min="$AUTO_PINNED_MIN_ASIA" ;;
    asia-eu)   [ -n "${AUTO_PINNED_MIN_ASIA_EU:-}" ] && min="$AUTO_PINNED_MIN_ASIA_EU" ;;
    eu)        [ -n "${AUTO_PINNED_MIN_EU:-}" ] && min="$AUTO_PINNED_MIN_EU" ;;
    eu-us)     [ -n "${AUTO_PINNED_MIN_EU_US:-}" ] && min="$AUTO_PINNED_MIN_EU_US" ;;
    us)        [ -n "${AUTO_PINNED_MIN_US:-}" ] && min="$AUTO_PINNED_MIN_US" ;;
    offhours)  [ -n "${AUTO_PINNED_MIN_OFFHOURS:-}" ] && min="$AUTO_PINNED_MIN_OFFHOURS" ;;
  esac

  case "$min" in
    ''|*[!0-9]*) min=0 ;;
  esac
  echo "$min"
}

cmd_session_handoff() {
  if ! is_true "$AUTO_SESSION_HANDOFF"; then
    log "AUTO_SESSION_HANDOFF=0, seans-handoff atlandi."
    return 0
  fi

  local core="${SESSION_CORE_SYMBOLS:-BTCUSDT@binance,ETHUSDT@binance}"
  local asia="${SESSION_ASIA_SYMBOLS:-USDJPY@mt5,XAUUSD@mt5}"
  local asia_eu="${SESSION_ASIA_EU_OVERLAP_SYMBOLS:-USDJPY@mt5,EURUSD@mt5,XAUUSD@mt5}"
  local eu="${SESSION_EU_SYMBOLS:-EURUSD@mt5,GBPUSD@mt5,XAUUSD@mt5}"
  local eu_us="${SESSION_EU_US_OVERLAP_SYMBOLS:-EURUSD@mt5,GBPUSD@mt5,USDCAD@mt5,XAUUSD@mt5}"
  local us="${SESSION_US_SYMBOLS:-USDCAD@mt5,EURUSD@mt5,XAUUSD@mt5}"
  local offhours="${SESSION_OFFHOURS_SYMBOLS:-BTCUSDT@binance,ETHUSDT@binance,SOLUSDT@binance,USDJPY@mt5}"
  local session session_symbols interval sync_mins
  session="$(resolve_session_name)"

  case "$session" in
    asia)      session_symbols="$asia";     interval="1h";  sync_mins="10" ;;
    asia-eu)   session_symbols="$asia_eu";  interval="15m"; sync_mins="5"  ;;
    eu)        session_symbols="$eu";       interval="30m"; sync_mins="10" ;;
    eu-us)     session_symbols="$eu_us";    interval="15m"; sync_mins="5"  ;;
    us)        session_symbols="$us";       interval="30m"; sync_mins="10" ;;
    offhours)  session_symbols="$offhours"; interval="1h";  sync_mins="15" ;;
    *)
      warn "gecersiz seans adi: $session"
      return 0
      ;;
  esac

  if is_true "$AUTO_OPPORTUNITY_MAX"; then
    case "$session" in
      asia)      interval="30m"; sync_mins="5" ;;
      asia-eu)   interval="15m"; sync_mins="3" ;;
      eu)        interval="15m"; sync_mins="5" ;;
      eu-us)     interval="15m"; sync_mins="3" ;;
      us)        interval="15m"; sync_mins="5" ;;
      offhours)  interval="30m"; sync_mins="5" ;;
    esac
  fi

  local mt5_ready=1
  # EA zorunluysa ve bağlantı yoksa bu tur MT5 sembollerini handoff setine yazma.
  if is_true "$AUTO_REQUIRE_MT5_ESTAB" && ! mt5_ea_connected; then
    mt5_ready=0
    local filtered=""
    local item venue
    IFS=',' read -r -a arr <<< "$session_symbols"
    for item in "${arr[@]}"; do
      item="$(trim "$item")"
      [ -z "$item" ] && continue
      case "$item" in
        *@*)
          venue="${item##*@}"
          [ "$venue" = "mt5" ] && continue
          ;;
      esac
      filtered+="${item},"
    done
    session_symbols="${filtered%,}"
    log "EA ESTAB yok ve AUTO_REQUIRE_MT5_ESTAB=1: bu tur handoff MT5 sembolleri skip edildi."
    log_event "handoff_mt5_skip reason=ea_not_estab seans=$session"
  fi

  local pinned_min
  pinned_min="$(resolve_pinned_min_for_session "$session")"

  local pinned mt5_symbols changed=0
  pinned="$(dedupe_csv "${core},${session_symbols}")"
  pinned="$(expand_pinned_symbols "$pinned" "$mt5_ready" "$pinned_min")"
  pinned="$(normalize_pinned_symbols "$pinned")"
  local pinned_count
  pinned_count="$(csv_count "$pinned")"
  mt5_symbols="$(extract_mt5_from_csv "$pinned")"

  local old_pinned old_interval old_sync
  old_pinned="$(conf_get PINNED_SYMBOLS)"
  old_interval="$(conf_get TRADE_INTERVAL)"
  old_sync="$(conf_get SCHEDULER_SYNC_EXCHANGE_EVERY_MINS)"

  if [ "$old_pinned" != "$pinned" ]; then
    conf_upsert PINNED_SYMBOLS "$pinned"
    changed=1
  fi
  if [ "$old_interval" != "$interval" ]; then
    conf_upsert TRADE_INTERVAL "$interval"
    changed=1
  fi
  if [ "$old_sync" != "$sync_mins" ]; then
    conf_upsert SCHEDULER_SYNC_EXCHANGE_EVERY_MINS "$sync_mins"
    changed=1
  fi
  conf_upsert SCHEDULER_SYNC_EXCHANGE_ENABLED "1"
  conf_upsert SCHEDULER_SYNC_EXCHANGE_WARMUP "1"
  conf_upsert ELITE_SYMBOL_HARVEST_ENABLED "0"
  [ -n "$mt5_symbols" ] && conf_upsert AUTO_MT5_SYMBOLS "$mt5_symbols"

  if [ "$changed" -eq 1 ]; then
    log "seans-handoff uygulandi: seans=$session interval=$interval sync=${sync_mins}m"
    log "PINNED_SYMBOLS (${pinned_count}, hedef_min=${pinned_min}) -> $pinned"
    log_event "handoff_applied seans=$session interval=$interval sync=${sync_mins}m pinned_count=${pinned_count} hedef_min=${pinned_min}"
    if runtime_running; then
      if is_true "$AUTO_SESSION_HANDOFF_IMMEDIATE_RESTART"; then
        log "runtime yeniden baslatiliyor (yeni env/simge seti icin)."
        ./memos restart || warn "memos restart basarisiz"
      else
        mkdir -p "$(dirname "$AUTO_SESSION_HANDOFF_PENDING_FILE")"
        printf '%s\n' "$(date -u +"%Y-%m-%dT%H:%M:%SZ") seans=$session interval=$interval pinned_count=${pinned_count}" > "$AUTO_SESSION_HANDOFF_PENDING_FILE"
        log "handoff degisikligi yazildi; restart ertelendi (pending: $AUTO_SESSION_HANDOFF_PENDING_FILE)."
        log_event "handoff_restart_deferred pending=$AUTO_SESSION_HANDOFF_PENDING_FILE seans=$session interval=$interval pinned_count=${pinned_count}"
      fi
    fi
  else
    log "seans-handoff: degisiklik yok (seans=$session)."
    log_event "handoff_nochange seans=$session"
  fi
}

cmd_review_pinned_symbols() {
  if ! is_true "$AUTO_PINNED_REVIEW_ENABLED"; then
    return 0
  fi

  local now last_ts interval
  now="$(date +%s)"
  last_ts=0
  interval="${AUTO_PINNED_REVIEW_INTERVAL_SECS:-600}"
  case "$interval" in ''|*[!0-9]*) interval=600 ;; esac

  if [ -f "$AUTO_PINNED_REVIEW_STATE_FILE" ]; then
    last_ts="$(cat "$AUTO_PINNED_REVIEW_STATE_FILE" 2>/dev/null || echo 0)"
  fi
  case "$last_ts" in ''|*[!0-9]*) last_ts=0 ;; esac
  if [ "$last_ts" -gt 0 ] && [ $((now - last_ts)) -lt "$interval" ]; then
    return 0
  fi

  mkdir -p "$(dirname "$AUTO_PINNED_REVIEW_STATE_FILE")"
  echo "$now" > "$AUTO_PINNED_REVIEW_STATE_FILE"

  local session mt5_ready pinned_min old_pinned pool missing extra
  local missing_count extra_count max_add notify_threshold target additions changed
  session="$(resolve_session_name)"
  mt5_ready=1
  if is_true "$AUTO_REQUIRE_MT5_ESTAB" && ! mt5_ea_connected; then
    mt5_ready=0
  fi
  pinned_min="$(resolve_pinned_min_for_session "$session")"

  old_pinned="$(normalize_pinned_symbols "$(conf_get PINNED_SYMBOLS)")"
  pool="$(dedupe_csv "$(candidate_symbol_pool)")"
  if [ "$mt5_ready" != "1" ]; then
    pool="$(remove_mt5_symbols "$pool")"
  fi

  missing="$(csv_diff "$pool" "$old_pinned")"
  extra="$(csv_diff "$old_pinned" "$pool")"
  missing_count="$(csv_count "$missing")"
  extra_count="$(csv_count "$extra")"

  changed=0
  target="$old_pinned"
  if is_true "$AUTO_PINNED_REVIEW_AUTOFIX" && [ "$missing_count" -gt 0 ]; then
    max_add="${AUTO_PINNED_REVIEW_MAX_ADD:-4}"
    additions="$(csv_take "$missing" "$max_add")"
    if [ -n "$additions" ]; then
      target="$(dedupe_csv "${target},${additions}")"
    fi
  fi

  if is_true "$AUTO_PINNED_REVIEW_TRIM_EXTRAS" && [ "$extra_count" -gt 0 ]; then
    target="$(csv_diff "$target" "$extra")"
  fi

  target="$(expand_pinned_symbols "$target" "$mt5_ready" "$pinned_min")"
  target="$(normalize_pinned_symbols "$target")"

  if [ "$target" != "$old_pinned" ]; then
    conf_upsert PINNED_SYMBOLS "$target"
    local mt5_symbols
    mt5_symbols="$(extract_mt5_from_csv "$target")"
    [ -n "$mt5_symbols" ] && conf_upsert AUTO_MT5_SYMBOLS "$mt5_symbols"
    changed=1
  fi

  log "pinned review: seans=$session pool=$(csv_count "$pool") pinned=$(csv_count "$old_pinned") missing=$missing_count extra=$extra_count changed=$changed"
  log_event "pinned_review seans=$session pool=$(csv_count "$pool") pinned=$(csv_count "$old_pinned") missing=$missing_count extra=$extra_count changed=$changed"

  notify_threshold="${AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD:-6}"
  case "$notify_threshold" in ''|*[!0-9]*) notify_threshold=6 ;; esac
  if [ "$missing_count" -ge "$notify_threshold" ]; then
    notify_critical "pinned review: missing_count=$missing_count threshold=$notify_threshold"
  fi

  if [ "$changed" -eq 1 ] && is_true "$AUTO_PINNED_REVIEW_RESTART_ON_CHANGE"; then
    if runtime_running; then
      log "pinned review degisikligi uygulandi; runtime yeniden baslatiliyor."
      ./memos restart || warn "memos restart basarisiz"
    fi
  fi
}

telegram_configured() {
  [ -n "${TELEGRAM_BOT_TOKEN:-}" ] && [ -n "${TELEGRAM_CHAT_ID:-}" ]
}

send_telegram_text() {
  local text="${1:-}"
  [ -z "$text" ] && return 0
  telegram_configured || { log "Telegram ayari yok, mesaj atlanacak."; return 0; }

  if command -v curl >/dev/null 2>&1; then
    curl -fsS -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendMessage" \
      --data-urlencode "chat_id=${TELEGRAM_CHAT_ID}" \
      --data-urlencode "text=${text}" >/dev/null || warn "Telegram gonderimi basarisiz oldu."
  else
    warn "curl bulunamadi, Telegram mesaji gonderilemedi."
  fi
}

notify_critical() {
  local msg="${1:-}"
  is_true "$AUTO_NOTIFY_CRITICAL" || return 0
  send_telegram_text "[MEMOS CRITICAL] ${msg}"
}

maybe_snapshot_ops_report() {
  if ! is_true "$AUTO_OPS_SNAPSHOT_ENABLED"; then
    return 0
  fi

  local now last_ts interval timeline probe out
  now="$(date +%s)"
  last_ts=0
  interval="${AUTO_OPS_SNAPSHOT_INTERVAL_SECS:-3600}"
  timeline="${AUTO_OPS_SNAPSHOT_TIMELINE:-8}"

  case "$interval" in ''|*[!0-9]*) interval=3600 ;; esac
  case "$timeline" in ''|*[!0-9]*) timeline=8 ;; esac

  if [ -f "$AUTO_OPS_SNAPSHOT_STATE_FILE" ]; then
    last_ts="$(cat "$AUTO_OPS_SNAPSHOT_STATE_FILE" 2>/dev/null || echo 0)"
  fi
  case "$last_ts" in ''|*[!0-9]*) last_ts=0 ;; esac

  if [ "$last_ts" -gt 0 ] && [ $((now - last_ts)) -lt "$interval" ]; then
    return 0
  fi

  probe=""
  if is_true "$AUTO_OPS_SNAPSHOT_PROBE"; then
    probe="--probe"
  fi

  if [ -n "$probe" ]; then
    out="$("$REPO_DIR/scripts/ops_center.sh" snapshot "$probe" "$timeline" 2>/dev/null || true)"
  else
    out="$("$REPO_DIR/scripts/ops_center.sh" snapshot "$timeline" 2>/dev/null || true)"
  fi

  if [ -n "$out" ]; then
    mkdir -p "$(dirname "$AUTO_OPS_SNAPSHOT_STATE_FILE")"
    echo "$now" > "$AUTO_OPS_SNAPSHOT_STATE_FILE"
    log "ops snapshot yazildi: $out"
    log_event "ops_snapshot_written file=$out"
  else
    warn "ops snapshot yazilamadi"
    log_event "ops_snapshot_failed"
  fi
}

maybe_send_daily_summary() {
  is_true "$AUTO_DAILY_SUMMARY" || return 0

  local today
  local last=""
  local runtime="stopped"
  local system_status="down"
  local metrics=""
  local pnl="n/a"
  local open_positions="n/a"

  mkdir -p "$(dirname "$AUTO_SUMMARY_STATE_FILE")"
  today="$(date +%F)"
  [ -f "$AUTO_SUMMARY_STATE_FILE" ] && last="$(cat "$AUTO_SUMMARY_STATE_FILE" 2>/dev/null || true)"
  [ "$last" = "$today" ] && return 0

  if runtime_running; then
    runtime="running"
    system_status="up"
  fi

  metrics="$(summary_metrics)"
  pnl="${metrics%%|*}"
  open_positions="${metrics##*|}"

  send_telegram_text "[MEMOS DAILY] date=${today} system=${system_status} pnl=${pnl} open_positions=${open_positions}"
  echo "$today" > "$AUTO_SUMMARY_STATE_FILE"
}

summary_metrics() {
  local snap="${SNAPSHOT_PATH:-data/snapshot.json}"

  [ -f "$snap" ] || { echo "n/a|n/a"; return 0; }
  command -v python3 >/dev/null 2>&1 || { echo "n/a|n/a"; return 0; }

  python3 - "$snap" <<'PY'
import json
import sys

path = sys.argv[1]
try:
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    finance = data.get("finance", {}) if isinstance(data, dict) else {}
    realize = float(finance.get("realize_pnl", 0) or 0)
    open_pnl = float(finance.get("open_pnl", 0) or 0)
    total_pnl = realize + open_pnl
    positions = data.get("positions", []) if isinstance(data, dict) else []
    open_count = len(positions) if isinstance(positions, list) else 0
    print(f"{total_pnl:+.2f}|{open_count}")
except Exception:
    print("n/a|n/a")
PY
}

snapshot_open_positions_count() {
  local snap="${SNAPSHOT_PATH:-data/snapshot.json}"

  [ -f "$snap" ] || { echo -1; return 0; }
  command -v python3 >/dev/null 2>&1 || { echo -1; return 0; }

  python3 - "$snap" <<'PY'
import json
import sys

path = sys.argv[1]
try:
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    positions = data.get("positions", []) if isinstance(data, dict) else []
    print(len(positions) if isinstance(positions, list) else -1)
except Exception:
    print(-1)
PY
}

pending_handoff_age_secs() {
  local file="$1"
  local ts epoch now
  [ -f "$file" ] || { echo -1; return 0; }
  ts="$(awk 'NR==1 {print $1}' "$file" 2>/dev/null || true)"
  [ -n "$ts" ] || { echo -1; return 0; }
  epoch="$(date -u -d "$ts" +%s 2>/dev/null || true)"
  [[ "$epoch" =~ ^[0-9]+$ ]] || { echo -1; return 0; }
  now="$(date +%s)"
  echo $((now - epoch))
}

cmd_apply_pending_handoff() {
  if ! is_true "$AUTO_SESSION_HANDOFF_PENDING_APPLY"; then
    return 0
  fi

  if [ ! -f "$AUTO_SESSION_HANDOFF_PENDING_FILE" ]; then
    return 0
  fi

  local min_age age open_positions
  min_age="${AUTO_SESSION_HANDOFF_PENDING_MIN_AGE_SECS:-120}"
  case "$min_age" in ''|*[!0-9]*) min_age=120 ;; esac
  age="$(pending_handoff_age_secs "$AUTO_SESSION_HANDOFF_PENDING_FILE")"

  if [[ "$age" =~ ^-?[0-9]+$ ]] && [ "$age" -ge 0 ] && [ "$age" -lt "$min_age" ]; then
    log "handoff pending beklemede: yas=${age}s, min_age=${min_age}s"
    return 0
  fi

  if ! runtime_running; then
    log "handoff pending temizlendi: runtime zaten kapali, sonraki up'ta yeni config uygulanacak."
    log_event "handoff_pending_cleared reason=runtime_stopped pending=$AUTO_SESSION_HANDOFF_PENDING_FILE"
    rm -f "$AUTO_SESSION_HANDOFF_PENDING_FILE"
    return 0
  fi

  if headless_supervisor_active; then
    log "handoff pending ertelendi: headless supervisor aktif; otomatik down/up yasak."
    log_event "handoff_pending_deferred reason=headless_supervisor"
    return 0
  fi

  if is_true "$AUTO_SESSION_HANDOFF_PENDING_REQUIRE_FLAT"; then
    open_positions="$(snapshot_open_positions_count)"
    if [[ "$open_positions" =~ ^[0-9]+$ ]] && [ "$open_positions" -gt 0 ]; then
      log "handoff pending ertelendi: acik pozisyon var (count=${open_positions})."
      log_event "handoff_pending_deferred reason=open_positions count=${open_positions}"
      return 0
    fi
  fi

  log "handoff pending uygulaniyor: kontrollu runtime restart."
  if ./memos restart; then
    rm -f "$AUTO_SESSION_HANDOFF_PENDING_FILE"
    log_event "handoff_pending_applied action=restart"
    return 0
  fi

  warn "handoff pending restart basarisiz; pending korunuyor."
  log_event "handoff_pending_failed action=restart"
  return 1
}

mt5_ea_connected() {
  if ss -tn 2>/dev/null | awk '$4 ~ /127\.0\.0\.1:9001$/ && $1 == "ESTAB" { found=1 } END { exit(found ? 0 : 1) }'; then
    return 0
  fi

  if ! is_true "$AUTO_MT5_ESTAB_PROBE_ON_MISS"; then
    return 1
  fi

  # Canli TUI 9001'i zaten tutuyorsa probe-trade ikinci listener acamaz.
  if ss -ltnp 2>/dev/null | grep -Eq ':9001.*rtc_tui|rtc_tui.*:9001'; then
    return 1
  fi

  local now last_ts last_ok tmp_log rc
  now="$(date +%s)"
  last_ts=0
  last_ok=0

  if [ -f "$AUTO_MT5_ESTAB_PROBE_STATE_FILE" ]; then
    IFS='|' read -r last_ts last_ok < "$AUTO_MT5_ESTAB_PROBE_STATE_FILE" || true
    last_ts="${last_ts:-0}"
    last_ok="${last_ok:-0}"
  fi

  if [ "$last_ok" = "1" ] && [ $((now - last_ts)) -le "$AUTO_MT5_ESTAB_PROBE_MIN_SECS" ]; then
    return 0
  fi
  if [ "$last_ok" != "1" ] && [ $((now - last_ts)) -le 60 ]; then
    return 1
  fi

  tmp_log="$(mktemp)"
  if "$REPO_DIR/scripts/mt5/setup_and_run.sh" probe-trade >"$tmp_log" 2>&1; then
    rc=0
    echo "${now}|1" > "$AUTO_MT5_ESTAB_PROBE_STATE_FILE"
    log "MT5 ESTAB fallback probe: OK (probe-trade)"
  else
    rc=1
    echo "${now}|0" > "$AUTO_MT5_ESTAB_PROBE_STATE_FILE"
    warn "MT5 ESTAB fallback probe: FAIL (probe-trade)"
  fi
  rm -f "$tmp_log"
  return "$rc"
}

ibkr_enabled() {
  case "${IBKR_ENABLED:-0}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

ibkr_bridge_port() {
  local addr="${AUTO_IBKR_BRIDGE_ADDR:-127.0.0.1:9101}"
  echo "${addr##*:}"
}

port_listening_local() {
  local port="$1"
  ss -tnl 2>/dev/null | grep -q ":${port}"
}

ibkr_check_due() {
  local now last_ts interval
  now="$(date +%s)"
  last_ts=0
  interval="${AUTO_IBKR_CHECK_INTERVAL_SECS:-300}"
  case "$interval" in ''|*[!0-9]*) interval=300 ;; esac

  if [ -f "$AUTO_IBKR_CHECK_STATE_FILE" ]; then
    last_ts="$(cat "$AUTO_IBKR_CHECK_STATE_FILE" 2>/dev/null || echo 0)"
  fi
  case "$last_ts" in ''|*[!0-9]*) last_ts=0 ;; esac

  [ "$last_ts" -eq 0 ] && return 0
  [ $((now - last_ts)) -ge "$interval" ]
}

mark_ibkr_check_now() {
  mkdir -p "$(dirname "$AUTO_IBKR_CHECK_STATE_FILE")"
  date +%s > "$AUTO_IBKR_CHECK_STATE_FILE"
}

ibkr_bridge_autostart_allowed() {
  local now last_ts cooldown
  now="$(date +%s)"
  last_ts=0
  cooldown="${AUTO_IBKR_BRIDGE_AUTOSTART_COOLDOWN_SECS:-600}"
  case "$cooldown" in ''|*[!0-9]*) cooldown=600 ;; esac

  if [ -f "$AUTO_IBKR_BRIDGE_STATE_FILE" ]; then
    last_ts="$(cat "$AUTO_IBKR_BRIDGE_STATE_FILE" 2>/dev/null || echo 0)"
  fi
  case "$last_ts" in ''|*[!0-9]*) last_ts=0 ;; esac

  [ "$last_ts" -eq 0 ] && return 0
  [ $((now - last_ts)) -ge "$cooldown" ]
}

mark_ibkr_bridge_autostart_now() {
  mkdir -p "$(dirname "$AUTO_IBKR_BRIDGE_STATE_FILE")"
  date +%s > "$AUTO_IBKR_BRIDGE_STATE_FILE"
}

cmd_check_ibkr_readiness() {
  is_true "$AUTO_IBKR_CHECK_ENABLED" || return 0
  ibkr_enabled || return 0
  ibkr_check_due || return 0

  mark_ibkr_check_now

  local tmp out_ok handshake_ok bridge_ok api_port bridge_port
  api_port="${AUTO_IBKR_API_PORT:-4002}"
  bridge_port="$(ibkr_bridge_port)"
  tmp="$(mktemp)"

  if IBKR_API_PORT="$api_port" bash "$REPO_DIR/scripts/ibkr/doctor.sh" paper >"$tmp" 2>&1; then
    out_ok=1
  else
    out_ok=0
  fi

  if grep -q "OK  - IB API handshake basarili" "$tmp"; then
    handshake_ok=1
  else
    handshake_ok=0
  fi

  if port_listening_local "$bridge_port"; then
    bridge_ok=1
  else
    bridge_ok=0
  fi

  if [ "$handshake_ok" -eq 1 ] && [ "$bridge_ok" -eq 0 ] && is_true "$AUTO_IBKR_BRIDGE_AUTOSTART"; then
    if ibkr_bridge_autostart_allowed; then
      log "IBKR bridge kapali ama handshake hazir: bridge autostart deneniyor."
      nohup bash "$REPO_DIR/scripts/ibkr/run_bridge.sh" > "$REPO_DIR/logs/ibkr-bridge-autostart.log" 2>&1 &
      mark_ibkr_bridge_autostart_now
      sleep 1
      if port_listening_local "$bridge_port"; then
        bridge_ok=1
        log_event "ibkr_bridge_autostart_ok port=${bridge_port}"
      else
        log_event "ibkr_bridge_autostart_fail port=${bridge_port}"
      fi
    fi
  fi

  if [ "$handshake_ok" -eq 1 ] && [ "$bridge_ok" -eq 1 ]; then
    log "IBKR readiness: OK (api=${api_port}, bridge=${bridge_port})."
    log_event "ibkr_ready api_port=${api_port} bridge_port=${bridge_port}"
    rm -f "$tmp"
    return 0
  fi

  warn "IBKR readiness degraded: handshake=${handshake_ok} bridge=${bridge_ok} (api=${api_port}, bridge=${bridge_port})"
  log "IBKR hizli aksiyon: scripts/autonomy.sh operator-checklist ibkr"
  log_event "ibkr_degraded handshake=${handshake_ok} bridge=${bridge_ok} api_port=${api_port} bridge_port=${bridge_port}"
  notify_critical "ibkr readiness degraded: handshake=${handshake_ok} bridge=${bridge_ok}"
  rm -f "$tmp"

  if is_true "$AUTO_VENUE_HEALTHCHECK_STRICT"; then
    return 1
  fi
  [ "$out_ok" -eq 1 ] && return 0
  return 0
}

cmd_check_mt5_readiness() {
  is_true "$AUTO_MT5_CHECK_ENABLED" || return 0
  case ",${VENUES:-}," in
    *,mt5:*) ;;
    *) return 0 ;;
  esac

  if mt5_ea_connected; then
    log_event "mt5_ready estab=1"
    return 0
  fi

  warn "MT5 readiness degraded: EA baglantisi/exec kanali hazir degil"
  log "MT5 hizli aksiyon: scripts/autonomy.sh operator-checklist mt5"
  log_event "mt5_degraded estab=0"
  notify_critical "mt5 readiness degraded: EA baglantisi veya exec kanali hazir degil"

  if is_true "$AUTO_VENUE_HEALTHCHECK_STRICT"; then
    return 1
  fi
  return 0
}

cmd_check_venue_readiness() {
  is_true "$AUTO_VENUE_HEALTHCHECK_ENABLED" || return 0

  local ok=1
  if ! cmd_check_mt5_readiness; then
    ok=0
  fi
  if ! cmd_check_ibkr_readiness; then
    ok=0
  fi
  [ "$ok" -eq 1 ]
}

cmd_operator_checklist() {
  local target="${1:-all}"
  echo "-- operator checklist (${target}) --"

  case "$target" in
    mt5|all)
      echo "[MT5] 1) terminal64.exe acik + EA grafikte yüklü"
      echo "[MT5] 2) AutoTrading acik, Tools->Options->Expert Advisors izinli"
      echo "[MT5] 3) localhost izni var (127.0.0.1)"
      echo "[MT5] 4) hizli test: scripts/mt5/setup_and_run.sh probe-trade"
      ;;
  esac

  case "$target" in
    ibkr|all)
      echo "[IBKR] 1) Gateway/TWS acik, API Enabled, localhost trusted"
      echo "[IBKR] 2) port kontrol: IBKR_API_PORT=4002 bash scripts/ibkr/doctor.sh paper"
      echo "[IBKR] 3) bridge baslat: bash scripts/run_bridge.sh"
      echo "[IBKR] 4) tekrar doctor: IBKR_API_PORT=4002 bash scripts/ibkr/doctor.sh paper"
      ;;
  esac

  echo "[AUTO] dogrulama: scripts/autonomy.sh check-venues"
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

runtime_running() {
  if venue_runtime_active; then
    return 0
  fi
  if systemctl --user is-active --quiet memos-watchdog 2>/dev/null; then
    return 0
  fi
  if [ -f "logs/.watchdog.pid" ] && kill -0 "$(cat logs/.watchdog.pid 2>/dev/null)" 2>/dev/null; then
    return 0
  fi
  if [ -f "logs/.engine.pid" ] && kill -0 "$(cat logs/.engine.pid 2>/dev/null)" 2>/dev/null; then
    return 0
  fi
  if command -v tmux >/dev/null 2>&1 && tmux has-session -t "${MEMOS_TUI_SESSION:-memos}" 2>/dev/null; then
    return 0
  fi
  return 1
}

headless_supervisor_active() {
  if systemctl --user is-active --quiet memos-watchdog.service 2>/dev/null &&
    pgrep -x rtc_headless >/dev/null 2>&1; then
    return 0
  fi
  venue_runtime_active
}

extract_mt5_symbols() {
  local raw="${AUTO_MT5_SYMBOLS:-${PINNED_SYMBOLS:-}}"
  local out=""
  local item sym venue

  if [ -n "$raw" ]; then
    IFS=',' read -r -a arr <<<"$raw"
    for item in "${arr[@]}"; do
      item="${item// /}"
      case "$item" in
        *@*)
          sym="${item%@*}"
          venue="${item##*@}"
          if [ "$venue" = "mt5" ]; then
            out+="${sym},"
          fi
          ;;
      esac
    done
  fi

  out="${out%,}"
  if [ -z "$out" ]; then
    out="EURUSD,GBPUSD,XAUUSD"
  fi
  echo "$out"
}

cmd_status() {
  local symbols
  symbols="$(extract_mt5_symbols)"
  echo "-- autonomy status --"
  echo "service: $AUTO_SERVICE"
  if [ -f "$AUTO_UNIT" ]; then
    echo "unit: installed"
  else
    echo "unit: not installed"
  fi
  if [ -f "$AUTO_TIMER_UNIT" ]; then
    echo "timer: installed"
    systemctl --user is-active "$AUTO_SERVICE.timer" 2>/dev/null | sed 's/^/timer-active: /'
    systemctl --user is-enabled "$AUTO_SERVICE.timer" 2>/dev/null | sed 's/^/timer-enabled: /'
  else
    echo "timer: not installed"
  fi
  if runtime_running; then echo "runtime: running"; else echo "runtime: stopped"; fi
  echo "mt5 market: $AUTO_MT5_MARKET"
  echo "mt5 interval: $AUTO_MT5_INTERVAL"
  echo "mt5 count: $AUTO_MT5_COUNT"
  echo "mt5 bootstrap: $AUTO_MT5_BOOTSTRAP"
  echo "mt5 bootstrap count: $AUTO_MT5_BOOTSTRAP_COUNT"
  if [ -f "$AUTO_MT5_BOOTSTRAP_STATE_FILE" ]; then
    echo "mt5 bootstrap state: done"
  else
    echo "mt5 bootstrap state: pending"
  fi
  echo "mt5 symbols: $symbols"
  echo "daily enabled: $AUTO_DAILY_ENABLED"
  echo "mt5 check enabled: $AUTO_MT5_CHECK_ENABLED"
  echo "venue healthcheck enabled: $AUTO_VENUE_HEALTHCHECK_ENABLED"
  echo "venue healthcheck strict: $AUTO_VENUE_HEALTHCHECK_STRICT"
  echo "ibkr check enabled: $AUTO_IBKR_CHECK_ENABLED"
  echo "ibkr check interval secs: $AUTO_IBKR_CHECK_INTERVAL_SECS"
  echo "ibkr api port: $AUTO_IBKR_API_PORT"
  echo "ibkr bridge addr: $AUTO_IBKR_BRIDGE_ADDR"
  echo "ibkr bridge autostart: $AUTO_IBKR_BRIDGE_AUTOSTART"
  echo "ibkr bridge autostart cooldown secs: $AUTO_IBKR_BRIDGE_AUTOSTART_COOLDOWN_SECS"
  echo "mt5 sync enabled: $AUTO_SYNC_MT5"
  echo "session handoff: $AUTO_SESSION_HANDOFF"
  echo "session handoff immediate restart: $AUTO_SESSION_HANDOFF_IMMEDIATE_RESTART"
  echo "session handoff pending file: $AUTO_SESSION_HANDOFF_PENDING_FILE"
  echo "session handoff pending apply: $AUTO_SESSION_HANDOFF_PENDING_APPLY"
  echo "session handoff pending min age secs: $AUTO_SESSION_HANDOFF_PENDING_MIN_AGE_SECS"
  echo "session handoff pending require flat: $AUTO_SESSION_HANDOFF_PENDING_REQUIRE_FLAT"
  if [ -f "$AUTO_SESSION_HANDOFF_PENDING_FILE" ]; then
    echo "session handoff pending: yes"
  else
    echo "session handoff pending: no"
  fi
  echo "pinned expand enabled: $AUTO_PINNED_EXPAND_ENABLED"
  echo "pinned min count: $AUTO_PINNED_MIN_COUNT"
  echo "pinned min asia: ${AUTO_PINNED_MIN_ASIA:-inherit}"
  echo "pinned min asia-eu: ${AUTO_PINNED_MIN_ASIA_EU:-inherit}"
  echo "pinned min eu: ${AUTO_PINNED_MIN_EU:-inherit}"
  echo "pinned min eu-us: ${AUTO_PINNED_MIN_EU_US:-inherit}"
  echo "pinned min us: ${AUTO_PINNED_MIN_US:-inherit}"
  echo "pinned min offhours: ${AUTO_PINNED_MIN_OFFHOURS:-inherit}"
  echo "pinned review enabled: $AUTO_PINNED_REVIEW_ENABLED"
  echo "pinned review interval secs: $AUTO_PINNED_REVIEW_INTERVAL_SECS"
  echo "pinned review autofix: $AUTO_PINNED_REVIEW_AUTOFIX"
  echo "pinned review trim extras: $AUTO_PINNED_REVIEW_TRIM_EXTRAS"
  echo "pinned review max add: $AUTO_PINNED_REVIEW_MAX_ADD"
  echo "pinned review restart on change: $AUTO_PINNED_REVIEW_RESTART_ON_CHANGE"
  echo "pinned review notify threshold: $AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD"
  echo "cycle fast eu-us only: $AUTO_CYCLE_FAST_EU_US_ONLY"
  echo "cycle base min secs: $AUTO_CYCLE_BASE_MIN_SECS"
  echo "cycle eu-us min secs: $AUTO_CYCLE_EU_US_MIN_SECS"
  echo "require ea estab: $AUTO_REQUIRE_MT5_ESTAB"
  echo "coverage audit: $AUTO_COVERAGE_AUDIT_ENABLED"
  echo "coverage audit offhours only: $AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS"
  echo "coverage audit autofix: $AUTO_COVERAGE_AUDIT_AUTOFIX"
  echo "coverage audit market: $AUTO_COVERAGE_AUDIT_MARKET"
  echo "coverage audit intervals: $AUTO_COVERAGE_AUDIT_INTERVALS"
  echo "notify critical: $AUTO_NOTIFY_CRITICAL"
  echo "daily summary: $AUTO_DAILY_SUMMARY"
  echo "ops snapshot enabled: $AUTO_OPS_SNAPSHOT_ENABLED"
  echo "ops snapshot interval secs: $AUTO_OPS_SNAPSHOT_INTERVAL_SECS"
  echo "ops snapshot timeline: $AUTO_OPS_SNAPSHOT_TIMELINE"
  echo "ops snapshot probe: $AUTO_OPS_SNAPSHOT_PROBE"
  echo "autonomy policy: $AUTO_AUTONOMY_POLICY"
  echo "trend strict mode: ${TREND_STRICT_MODE:-0}"
  echo "trend strict pool: ${TREND_STRICT_STRATEGY_POOL:-SUPERTREND,EMA_CROSSOVER,DONCHIAN,MA_CROSSOVER}"
}

cmd_ensure() {
  if runtime_running; then
    log "runtime zaten aktif."
  else
    log "runtime kapali. memos up cagriliyor."
    ./memos up
  fi

  if is_true "$AUTO_DAILY_ENABLED"; then
    # daily metrics dosyasi yoksa sessizce atla; varsa hata verse de akisi bozma.
    if [ -f "scripts/daily_metrics.py" ]; then
      if ! ./memos daily; then
        warn "daily metrics adimi basarisiz oldu (akisa devam)."
      fi
    else
      log "scripts/daily_metrics.py yok; daily metrics adimi atlandi."
    fi
  fi

  if is_true "$AUTO_MT5_CHECK_ENABLED"; then
    if ! scripts/mt5/setup_and_run.sh check; then
      warn "mt5 check adimi basarisiz oldu (akisa devam)."
    fi
  fi
}

run_download_mt5() {
  local market="${1:-$AUTO_MT5_MARKET}"
  local interval="${2:-$AUTO_MT5_INTERVAL}"
  local symbols="${3:-$(extract_mt5_symbols)}"
  local count="${4:-$AUTO_MT5_COUNT}"

  log "download_mt5 basliyor: market=$market interval=$interval symbols=$symbols count=$count"
  MT5_BRIDGE_ADDR="${MT5_BRIDGE_ADDR:-127.0.0.1:9001}" \
    cargo run --release --example download_mt5 -- "$market" "$interval" "$symbols" "$count"
}

cmd_bootstrap_mt5() {
  local market="${1:-$AUTO_MT5_MARKET}"
  local interval="${2:-$AUTO_MT5_INTERVAL}"
  local symbols="${3:-$(extract_mt5_symbols)}"
  local count="${4:-$AUTO_MT5_BOOTSTRAP_COUNT}"
  local was_running=0

  if [ -f "$AUTO_MT5_BOOTSTRAP_STATE_FILE" ]; then
    log "MT5 bootstrap daha once tamamlanmis; atlandi."
    return 0
  fi

  if headless_supervisor_active; then
    log "headless supervisor aktif; MT5 bootstrap icin runtime down/up atlandi."
    return 0
  fi

  if runtime_running; then
    was_running=1
    log "bootstrap icin runtime gecici down yapiliyor."
    ./memos down
  fi

  if run_download_mt5 "$market" "$interval" "$symbols" "$count"; then
    mkdir -p "$(dirname "$AUTO_MT5_BOOTSTRAP_STATE_FILE")"
    date +%FT%T > "$AUTO_MT5_BOOTSTRAP_STATE_FILE"
    log "MT5 bootstrap tamamlandi ve isaretlendi: $AUTO_MT5_BOOTSTRAP_STATE_FILE"
  else
    warn "MT5 bootstrap basarisiz oldu; sonraki cycle'da tekrar denenecek."
    if [ "$was_running" -eq 1 ]; then
      log "runtime geri kaldiriliyor."
      ./memos up
    fi
    return 1
  fi

  if [ "$was_running" -eq 1 ]; then
    log "runtime geri kaldiriliyor."
    ./memos up
  fi
}

cmd_sync_mt5() {
  local market="${1:-$AUTO_MT5_MARKET}"
  local interval="${2:-$AUTO_MT5_INTERVAL}"
  local symbols="${3:-$(extract_mt5_symbols)}"
  local count="${4:-$AUTO_MT5_COUNT}"
  local was_running=0

  if is_true "$AUTO_REQUIRE_MT5_ESTAB" && ! mt5_ea_connected; then
    log "EA bagli degil (ESTAB yok); mt5 sync bu tur atlandi."

    return 0
  fi

  if mt5_sync_already_done_for_bucket "$market" "$interval" "$symbols"; then
    log "MT5 sync atlandi: ${interval} icin bu bar dilimi zaten indirildi."
    return 0
  fi

  if headless_supervisor_active; then
    log "headless supervisor aktif; MT5 sync icin runtime down/up atlandi."
    return 0
  fi

  if runtime_running; then
    was_running=1
    log "runtime aktif; port cakismasini onlemek icin gecici down yapiliyor."
    ./memos down
  fi

  if run_download_mt5 "$market" "$interval" "$symbols" "$count"; then
    mark_mt5_sync_done_for_bucket "$market" "$interval" "$symbols"
    log "download_mt5 tamamlandi."
  else
    warn "download_mt5 basarisiz oldu."
    if [ "$was_running" -eq 1 ]; then
      log "runtime geri kaldiriliyor."
      ./memos up
    fi
    return 1
  fi

  if [ "$was_running" -eq 1 ]; then
    log "runtime geri kaldiriliyor."
    ./memos up
  fi
}

cmd_coverage_audit() {
  local was_running=0
  local tmp_plan tmp_report

  if ! is_true "$AUTO_COVERAGE_AUDIT_ENABLED"; then
    log "coverage-audit kapali; atlandi."
    return 0
  fi

  if ! coverage_audit_due; then
    log "coverage-audit bugün zaten koştu veya uygun zaman değil; atlandi."
    return 0
  fi

  if runtime_running && ! headless_supervisor_active; then
    was_running=1
    log "coverage-audit icin runtime gecici down yapiliyor."
    ./memos down
  fi

  tmp_plan="$(mktemp)"
  tmp_report="$(mktemp)"
  if cargo run --release -p memos_trading_core --example db_coverage -- "$AUTO_COVERAGE_AUDIT_MARKET" "$AUTO_COVERAGE_AUDIT_INTERVALS" "$AUTO_COVERAGE_AUDIT_MIN_ROWS" --plan >"$tmp_plan" 2>"$tmp_report"; then
    if grep -q "Sağlıksız seri yok — plan boş" "$tmp_plan"; then
      log "coverage-audit: temiz (backfill gerekmiyor)."
    else
      log "coverage-audit: backfill planı bulundu."
      if is_true "$AUTO_COVERAGE_AUDIT_AUTOFIX"; then
        bash "$tmp_plan"
        log "coverage-audit: backfill tamamlandı."
      else
        log "coverage-audit: autofix kapalı; plan üretildi ama çalıştırılmadı."
      fi
    fi
    mark_coverage_audit_done
  else
    warn "coverage-audit başarısız oldu; rapor çıktısı aşağıda."
    cat "$tmp_report" >&2 || true
    rm -f "$tmp_plan" "$tmp_report"
    if [ "$was_running" -eq 1 ]; then
      log "runtime geri kaldiriliyor."
      ./memos up
    fi
    return 1
  fi

  rm -f "$tmp_plan" "$tmp_report"
  if [ "$was_running" -eq 1 ]; then
    log "runtime geri kaldiriliyor."
    ./memos up
  fi
}

cmd_cycle() {
  local cycle_ok=1
  local cycle_session
  cycle_session="$(resolve_session_name)"

  if ! cycle_throttle_allows_run "$cycle_session"; then
    maybe_snapshot_ops_report
    maybe_send_daily_summary
    return 0
  fi

  if ! cmd_session_handoff; then
    cycle_ok=0
    notify_critical "session handoff adimi basarisiz"
  fi

  if ! cmd_review_pinned_symbols; then
    cycle_ok=0
    notify_critical "pinned review adimi basarisiz"
  fi

  if ! cmd_ensure; then
    cycle_ok=0
    notify_critical "cmd_ensure basarisiz"
  fi

  if ! cmd_check_venue_readiness; then
    cycle_ok=0
    notify_critical "venue readiness adimi basarisiz"
  fi

  if ! runtime_running; then
    cycle_ok=0
    notify_critical "runtime ensure sonrasi hala kapali"
  fi

  if is_true "$AUTO_MT5_BOOTSTRAP"; then
    if ! cmd_bootstrap_mt5 "$@"; then
      cycle_ok=0
      notify_critical "mt5 bootstrap basarisiz"
    fi
  fi

  if is_true "$AUTO_SYNC_MT5"; then
    if ! cmd_sync_mt5 "$@"; then
      cycle_ok=0
      notify_critical "mt5 sync basarisiz"
    fi
  else
    log "AUTO_SYNC_MT5=0 oldugu icin mt5 sync adimi atlandi."
  fi

  if ! cmd_coverage_audit; then
    cycle_ok=0
    notify_critical "coverage audit basarisiz"
  fi

  if ! cmd_ensure; then
    cycle_ok=0
    notify_critical "cycle sonu cmd_ensure basarisiz"
  fi

  if ! cmd_apply_pending_handoff; then
    cycle_ok=0
    notify_critical "pending handoff uygulamasi basarisiz"
  fi

  maybe_snapshot_ops_report
  maybe_send_daily_summary

  [ "$cycle_ok" -eq 1 ]
}

apply_opportunity_profile_conf() {
  # Firsat kacirma riskini azaltmak icin scheduler ve seans-kapsamini sikilastir.
  conf_upsert AUTO_SESSION_HANDOFF "1"
  conf_upsert AUTO_PINNED_EXPAND_ENABLED "1"
  conf_upsert AUTO_PINNED_MIN_COUNT "10"
  conf_upsert AUTO_PINNED_MIN_ASIA "9"
  conf_upsert AUTO_PINNED_MIN_ASIA_EU "11"
  conf_upsert AUTO_PINNED_MIN_EU "10"
  conf_upsert AUTO_PINNED_MIN_EU_US "14"
  conf_upsert AUTO_PINNED_MIN_US "10"
  conf_upsert AUTO_PINNED_MIN_OFFHOURS "8"
  conf_upsert SCHEDULER_SYNC_EXCHANGE_ENABLED "1"
  conf_upsert SCHEDULER_SYNC_EXCHANGE_WARMUP "1"
  conf_upsert SCHEDULER_SYNC_EXCHANGE_EVERY_MINS "5"
  conf_upsert SCHEDULER_ML_ENABLED "1"
  conf_upsert SCHEDULER_ML_EVERY_MINS "60"
  conf_upsert SCHEDULER_ML_WARMUP_SKIP "0"
  conf_upsert SCHEDULER_BACKTEST_EVERY_MINS "120"
  conf_upsert SCHEDULER_BACKTEST_WARMUP "1"
  conf_upsert ANOMALY_ML_TRIGGER_COOLDOWN_SECS "21600"

  conf_upsert BLEND_LIVE_TOP_K "5"
  conf_upsert BLEND_LIVE_REBALANCE_BARS "7"

  conf_upsert SESSION_CORE_SYMBOLS "BTCUSDT@binance,ETHUSDT@binance,SOLUSDT@binance,BNBUSDT@binance,XRPUSDT@binance,ADAUSDT@binance"
  conf_upsert SESSION_ASIA_SYMBOLS "BTCUSDT@binance,SOLUSDT@binance,BNBUSDT@binance,USDJPY@mt5,AUDUSD@mt5,XAUUSD@mt5"
  conf_upsert SESSION_ASIA_EU_OVERLAP_SYMBOLS "ETHUSDT@binance,BNBUSDT@binance,XRPUSDT@binance,USDJPY@mt5,EURUSD@mt5,GBPUSD@mt5,XAUUSD@mt5"
  conf_upsert SESSION_EU_SYMBOLS "ETHUSDT@binance,BTCUSDT@binance,XRPUSDT@binance,ADAUSDT@binance,EURUSD@mt5,GBPUSD@mt5,USDCHF@mt5,XAUUSD@mt5"
  conf_upsert SESSION_EU_US_OVERLAP_SYMBOLS "BTCUSDT@binance,ETHUSDT@binance,SOLUSDT@binance,BNBUSDT@binance,EURUSD@mt5,GBPUSD@mt5,USDCAD@mt5,USDCHF@mt5,XAUUSD@mt5"
  conf_upsert SESSION_US_SYMBOLS "BTCUSDT@binance,SOLUSDT@binance,AVAXUSDT@binance,LINKUSDT@binance,USDCAD@mt5,EURUSD@mt5,USDCHF@mt5,XAUUSD@mt5"
  conf_upsert SESSION_OFFHOURS_SYMBOLS "BTCUSDT@binance,ETHUSDT@binance,SOLUSDT@binance,BNBUSDT@binance,XRPUSDT@binance,USDJPY@mt5,XAUUSD@mt5"

  conf_upsert AUTO_MT5_SYMBOLS "EURUSD@mt5,GBPUSD@mt5,USDJPY@mt5,AUDUSD@mt5,USDCAD@mt5,USDCHF@mt5,XAUUSD@mt5,XAGUSD@mt5"
  conf_upsert AUTO_MT5_COUNT "1000"
  conf_upsert AUTO_MT5_BOOTSTRAP "1"
  conf_upsert AUTO_MT5_BOOTSTRAP_COUNT "1000"
  conf_upsert AUTO_REQUIRE_MT5_ESTAB "1"
}

save_auto_env() {
  cat > "$AUTO_ENV_FILE" <<EOF
AUTO_AUTONOMY_POLICY=$AUTO_AUTONOMY_POLICY
AUTO_SYNC_MT5=$AUTO_SYNC_MT5
AUTO_SESSION_HANDOFF=$AUTO_SESSION_HANDOFF
AUTO_SESSION_HANDOFF_IMMEDIATE_RESTART=$AUTO_SESSION_HANDOFF_IMMEDIATE_RESTART
AUTO_SESSION_HANDOFF_PENDING_FILE=$AUTO_SESSION_HANDOFF_PENDING_FILE
AUTO_SESSION_HANDOFF_PENDING_APPLY=$AUTO_SESSION_HANDOFF_PENDING_APPLY
AUTO_SESSION_HANDOFF_PENDING_MIN_AGE_SECS=$AUTO_SESSION_HANDOFF_PENDING_MIN_AGE_SECS
AUTO_SESSION_HANDOFF_PENDING_REQUIRE_FLAT=$AUTO_SESSION_HANDOFF_PENDING_REQUIRE_FLAT
AUTO_OPPORTUNITY_MAX=$AUTO_OPPORTUNITY_MAX
AUTO_PINNED_EXPAND_ENABLED=$AUTO_PINNED_EXPAND_ENABLED
AUTO_PINNED_MIN_COUNT=$AUTO_PINNED_MIN_COUNT
AUTO_PINNED_MIN_ASIA=$AUTO_PINNED_MIN_ASIA
AUTO_PINNED_MIN_ASIA_EU=$AUTO_PINNED_MIN_ASIA_EU
AUTO_PINNED_MIN_EU=$AUTO_PINNED_MIN_EU
AUTO_PINNED_MIN_EU_US=$AUTO_PINNED_MIN_EU_US
AUTO_PINNED_MIN_US=$AUTO_PINNED_MIN_US
AUTO_PINNED_MIN_OFFHOURS=$AUTO_PINNED_MIN_OFFHOURS
AUTO_PINNED_REVIEW_ENABLED=$AUTO_PINNED_REVIEW_ENABLED
AUTO_PINNED_REVIEW_INTERVAL_SECS=$AUTO_PINNED_REVIEW_INTERVAL_SECS
AUTO_PINNED_REVIEW_AUTOFIX=$AUTO_PINNED_REVIEW_AUTOFIX
AUTO_PINNED_REVIEW_TRIM_EXTRAS=$AUTO_PINNED_REVIEW_TRIM_EXTRAS
AUTO_PINNED_REVIEW_MAX_ADD=$AUTO_PINNED_REVIEW_MAX_ADD
AUTO_PINNED_REVIEW_RESTART_ON_CHANGE=$AUTO_PINNED_REVIEW_RESTART_ON_CHANGE
AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD=$AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD
AUTO_PINNED_REVIEW_STATE_FILE=$AUTO_PINNED_REVIEW_STATE_FILE
AUTO_CYCLE_FAST_EU_US_ONLY=$AUTO_CYCLE_FAST_EU_US_ONLY
AUTO_CYCLE_BASE_MIN_SECS=$AUTO_CYCLE_BASE_MIN_SECS
AUTO_CYCLE_EU_US_MIN_SECS=$AUTO_CYCLE_EU_US_MIN_SECS
AUTO_CYCLE_STATE_FILE=$AUTO_CYCLE_STATE_FILE
AUTO_REQUIRE_MT5_ESTAB=$AUTO_REQUIRE_MT5_ESTAB
AUTO_DAILY_ENABLED=$AUTO_DAILY_ENABLED
AUTO_MT5_CHECK_ENABLED=$AUTO_MT5_CHECK_ENABLED
AUTO_VENUE_HEALTHCHECK_ENABLED=$AUTO_VENUE_HEALTHCHECK_ENABLED
AUTO_VENUE_HEALTHCHECK_STRICT=$AUTO_VENUE_HEALTHCHECK_STRICT
AUTO_IBKR_CHECK_ENABLED=$AUTO_IBKR_CHECK_ENABLED
AUTO_IBKR_CHECK_INTERVAL_SECS=$AUTO_IBKR_CHECK_INTERVAL_SECS
AUTO_IBKR_CHECK_STATE_FILE=$AUTO_IBKR_CHECK_STATE_FILE
AUTO_IBKR_API_PORT=$AUTO_IBKR_API_PORT
AUTO_IBKR_BRIDGE_ADDR=$AUTO_IBKR_BRIDGE_ADDR
AUTO_IBKR_BRIDGE_AUTOSTART=$AUTO_IBKR_BRIDGE_AUTOSTART
AUTO_IBKR_BRIDGE_AUTOSTART_COOLDOWN_SECS=$AUTO_IBKR_BRIDGE_AUTOSTART_COOLDOWN_SECS
AUTO_IBKR_BRIDGE_STATE_FILE=$AUTO_IBKR_BRIDGE_STATE_FILE
AUTO_NOTIFY_CRITICAL=$AUTO_NOTIFY_CRITICAL
AUTO_DAILY_SUMMARY=$AUTO_DAILY_SUMMARY
AUTO_OPS_SNAPSHOT_ENABLED=$AUTO_OPS_SNAPSHOT_ENABLED
AUTO_OPS_SNAPSHOT_INTERVAL_SECS=$AUTO_OPS_SNAPSHOT_INTERVAL_SECS
AUTO_OPS_SNAPSHOT_TIMELINE=$AUTO_OPS_SNAPSHOT_TIMELINE
AUTO_OPS_SNAPSHOT_PROBE=$AUTO_OPS_SNAPSHOT_PROBE
AUTO_OPS_SNAPSHOT_STATE_FILE=$AUTO_OPS_SNAPSHOT_STATE_FILE
AUTO_MT5_MARKET=$AUTO_MT5_MARKET
AUTO_MT5_INTERVAL=$AUTO_MT5_INTERVAL
AUTO_MT5_COUNT=$AUTO_MT5_COUNT
AUTO_MT5_BOOTSTRAP=$AUTO_MT5_BOOTSTRAP
AUTO_MT5_BOOTSTRAP_COUNT=$AUTO_MT5_BOOTSTRAP_COUNT
AUTO_MT5_ESTAB_PROBE_ON_MISS=$AUTO_MT5_ESTAB_PROBE_ON_MISS
AUTO_MT5_ESTAB_PROBE_MIN_SECS=$AUTO_MT5_ESTAB_PROBE_MIN_SECS
AUTO_MT5_ESTAB_PROBE_STATE_FILE=$AUTO_MT5_ESTAB_PROBE_STATE_FILE
AUTO_COVERAGE_AUDIT_ENABLED=$AUTO_COVERAGE_AUDIT_ENABLED
AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS=$AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS
AUTO_COVERAGE_AUDIT_AUTOFIX=$AUTO_COVERAGE_AUDIT_AUTOFIX
AUTO_COVERAGE_AUDIT_MARKET=$AUTO_COVERAGE_AUDIT_MARKET
AUTO_COVERAGE_AUDIT_INTERVALS=$AUTO_COVERAGE_AUDIT_INTERVALS
AUTO_COVERAGE_AUDIT_MIN_ROWS=$AUTO_COVERAGE_AUDIT_MIN_ROWS
AUTO_COVERAGE_AUDIT_STATE_FILE=$AUTO_COVERAGE_AUDIT_STATE_FILE
AUTO_ONCALENDAR=$AUTO_ONCALENDAR
EOF
}

write_units() {
  mkdir -p "$(dirname "$AUTO_UNIT")"

  cat > "$AUTO_UNIT" <<UNIT
[Unit]
Description=Memos autonomy cycle service
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
WorkingDirectory=$REPO_DIR
Environment=AUTO_SYNC_MT5=$AUTO_SYNC_MT5
Environment=AUTO_SESSION_HANDOFF=$AUTO_SESSION_HANDOFF
Environment=AUTO_SESSION_HANDOFF_IMMEDIATE_RESTART=$AUTO_SESSION_HANDOFF_IMMEDIATE_RESTART
Environment=AUTO_SESSION_HANDOFF_PENDING_FILE=$AUTO_SESSION_HANDOFF_PENDING_FILE
Environment=AUTO_SESSION_HANDOFF_PENDING_APPLY=$AUTO_SESSION_HANDOFF_PENDING_APPLY
Environment=AUTO_SESSION_HANDOFF_PENDING_MIN_AGE_SECS=$AUTO_SESSION_HANDOFF_PENDING_MIN_AGE_SECS
Environment=AUTO_SESSION_HANDOFF_PENDING_REQUIRE_FLAT=$AUTO_SESSION_HANDOFF_PENDING_REQUIRE_FLAT
Environment=AUTO_OPPORTUNITY_MAX=$AUTO_OPPORTUNITY_MAX
Environment=AUTO_PINNED_EXPAND_ENABLED=$AUTO_PINNED_EXPAND_ENABLED
Environment=AUTO_PINNED_MIN_COUNT=$AUTO_PINNED_MIN_COUNT
Environment=AUTO_PINNED_MIN_ASIA=$AUTO_PINNED_MIN_ASIA
Environment=AUTO_PINNED_MIN_ASIA_EU=$AUTO_PINNED_MIN_ASIA_EU
Environment=AUTO_PINNED_MIN_EU=$AUTO_PINNED_MIN_EU
Environment=AUTO_PINNED_MIN_EU_US=$AUTO_PINNED_MIN_EU_US
Environment=AUTO_PINNED_MIN_US=$AUTO_PINNED_MIN_US
Environment=AUTO_PINNED_MIN_OFFHOURS=$AUTO_PINNED_MIN_OFFHOURS
Environment=AUTO_PINNED_REVIEW_ENABLED=$AUTO_PINNED_REVIEW_ENABLED
Environment=AUTO_PINNED_REVIEW_INTERVAL_SECS=$AUTO_PINNED_REVIEW_INTERVAL_SECS
Environment=AUTO_PINNED_REVIEW_AUTOFIX=$AUTO_PINNED_REVIEW_AUTOFIX
Environment=AUTO_PINNED_REVIEW_TRIM_EXTRAS=$AUTO_PINNED_REVIEW_TRIM_EXTRAS
Environment=AUTO_PINNED_REVIEW_MAX_ADD=$AUTO_PINNED_REVIEW_MAX_ADD
Environment=AUTO_PINNED_REVIEW_RESTART_ON_CHANGE=$AUTO_PINNED_REVIEW_RESTART_ON_CHANGE
Environment=AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD=$AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD
Environment=AUTO_PINNED_REVIEW_STATE_FILE=$AUTO_PINNED_REVIEW_STATE_FILE
Environment=AUTO_CYCLE_FAST_EU_US_ONLY=$AUTO_CYCLE_FAST_EU_US_ONLY
Environment=AUTO_CYCLE_BASE_MIN_SECS=$AUTO_CYCLE_BASE_MIN_SECS
Environment=AUTO_CYCLE_EU_US_MIN_SECS=$AUTO_CYCLE_EU_US_MIN_SECS
Environment=AUTO_CYCLE_STATE_FILE=$AUTO_CYCLE_STATE_FILE
Environment=AUTO_REQUIRE_MT5_ESTAB=$AUTO_REQUIRE_MT5_ESTAB
Environment=AUTO_DAILY_ENABLED=$AUTO_DAILY_ENABLED
Environment=AUTO_MT5_CHECK_ENABLED=$AUTO_MT5_CHECK_ENABLED
Environment=AUTO_VENUE_HEALTHCHECK_ENABLED=$AUTO_VENUE_HEALTHCHECK_ENABLED
Environment=AUTO_VENUE_HEALTHCHECK_STRICT=$AUTO_VENUE_HEALTHCHECK_STRICT
Environment=AUTO_IBKR_CHECK_ENABLED=$AUTO_IBKR_CHECK_ENABLED
Environment=AUTO_IBKR_CHECK_INTERVAL_SECS=$AUTO_IBKR_CHECK_INTERVAL_SECS
Environment=AUTO_IBKR_CHECK_STATE_FILE=$AUTO_IBKR_CHECK_STATE_FILE
Environment=AUTO_IBKR_API_PORT=$AUTO_IBKR_API_PORT
Environment=AUTO_IBKR_BRIDGE_ADDR=$AUTO_IBKR_BRIDGE_ADDR
Environment=AUTO_IBKR_BRIDGE_AUTOSTART=$AUTO_IBKR_BRIDGE_AUTOSTART
Environment=AUTO_IBKR_BRIDGE_AUTOSTART_COOLDOWN_SECS=$AUTO_IBKR_BRIDGE_AUTOSTART_COOLDOWN_SECS
Environment=AUTO_IBKR_BRIDGE_STATE_FILE=$AUTO_IBKR_BRIDGE_STATE_FILE
Environment=AUTO_NOTIFY_CRITICAL=$AUTO_NOTIFY_CRITICAL
Environment=AUTO_DAILY_SUMMARY=$AUTO_DAILY_SUMMARY
Environment=AUTO_OPS_SNAPSHOT_ENABLED=$AUTO_OPS_SNAPSHOT_ENABLED
Environment=AUTO_OPS_SNAPSHOT_INTERVAL_SECS=$AUTO_OPS_SNAPSHOT_INTERVAL_SECS
Environment=AUTO_OPS_SNAPSHOT_TIMELINE=$AUTO_OPS_SNAPSHOT_TIMELINE
Environment=AUTO_OPS_SNAPSHOT_PROBE=$AUTO_OPS_SNAPSHOT_PROBE
Environment=AUTO_OPS_SNAPSHOT_STATE_FILE=$AUTO_OPS_SNAPSHOT_STATE_FILE
Environment=AUTO_AUTONOMY_POLICY=$AUTO_AUTONOMY_POLICY
Environment=AUTO_MT5_MARKET=$AUTO_MT5_MARKET
Environment=AUTO_MT5_INTERVAL=$AUTO_MT5_INTERVAL
Environment=AUTO_MT5_COUNT=$AUTO_MT5_COUNT
Environment=AUTO_MT5_BOOTSTRAP=$AUTO_MT5_BOOTSTRAP
Environment=AUTO_MT5_BOOTSTRAP_COUNT=$AUTO_MT5_BOOTSTRAP_COUNT
Environment=AUTO_MT5_ESTAB_PROBE_ON_MISS=$AUTO_MT5_ESTAB_PROBE_ON_MISS
Environment=AUTO_MT5_ESTAB_PROBE_MIN_SECS=$AUTO_MT5_ESTAB_PROBE_MIN_SECS
Environment=AUTO_MT5_ESTAB_PROBE_STATE_FILE=$AUTO_MT5_ESTAB_PROBE_STATE_FILE
Environment=AUTO_COVERAGE_AUDIT_ENABLED=$AUTO_COVERAGE_AUDIT_ENABLED
Environment=AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS=$AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS
Environment=AUTO_COVERAGE_AUDIT_AUTOFIX=$AUTO_COVERAGE_AUDIT_AUTOFIX
Environment=AUTO_COVERAGE_AUDIT_MARKET=$AUTO_COVERAGE_AUDIT_MARKET
Environment=AUTO_COVERAGE_AUDIT_INTERVALS=$AUTO_COVERAGE_AUDIT_INTERVALS
Environment=AUTO_COVERAGE_AUDIT_MIN_ROWS=$AUTO_COVERAGE_AUDIT_MIN_ROWS
Environment=AUTO_COVERAGE_AUDIT_STATE_FILE=$AUTO_COVERAGE_AUDIT_STATE_FILE
ExecStart=$REPO_DIR/scripts/autonomy.sh cycle
UNIT

  cat > "$AUTO_TIMER_UNIT" <<UNIT
[Unit]
Description=Run Memos autonomy cycle periodically

[Timer]
OnCalendar=$AUTO_ONCALENDAR
Persistent=true
RandomizedDelaySec=60
Unit=$AUTO_SERVICE.service

[Install]
WantedBy=timers.target
UNIT
}

cmd_install_timer() {
  save_auto_env
  write_units

  systemctl --user daemon-reload
  systemctl --user enable --now "$AUTO_SERVICE.timer"
  log "autonomy timer kuruldu: $AUTO_ONCALENDAR"
}

apply_autonomy_core_conf() {
  AUTO_SYNC_MT5="1"
  AUTO_REQUIRE_MT5_ESTAB="1"
  AUTO_DAILY_ENABLED="0"
  AUTO_MT5_CHECK_ENABLED="1"
  AUTO_NOTIFY_CRITICAL="1"
  AUTO_DAILY_SUMMARY="1"
  AUTO_MT5_BOOTSTRAP="1"
  AUTO_MT5_ESTAB_PROBE_ON_MISS="1"
  AUTO_COVERAGE_AUDIT_ENABLED="1"
  AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS="1"
  AUTO_COVERAGE_AUDIT_AUTOFIX="1"
  AUTO_COVERAGE_AUDIT_MARKET="futures"
  AUTO_COVERAGE_AUDIT_INTERVALS="1d,4h"
  AUTO_COVERAGE_AUDIT_MIN_ROWS="300"
}

apply_autonomy_mt5_only_conf() {
  AUTO_AUTONOMY_POLICY="mt5-only"
  AUTO_SESSION_HANDOFF="1"
  AUTO_OPPORTUNITY_MAX="0"
  AUTO_PINNED_EXPAND_ENABLED="1"
  AUTO_PINNED_MIN_COUNT="6"
  AUTO_PINNED_MIN_ASIA="6"
  AUTO_PINNED_MIN_ASIA_EU="7"
  AUTO_PINNED_MIN_EU="6"
  AUTO_PINNED_MIN_EU_US="7"
  AUTO_PINNED_MIN_US="6"
  AUTO_PINNED_MIN_OFFHOURS="5"
  AUTO_PINNED_REVIEW_ENABLED="1"
  AUTO_PINNED_REVIEW_INTERVAL_SECS="900"
  AUTO_PINNED_REVIEW_AUTOFIX="1"
  AUTO_PINNED_REVIEW_TRIM_EXTRAS="0"
  AUTO_PINNED_REVIEW_MAX_ADD="3"
  AUTO_PINNED_REVIEW_RESTART_ON_CHANGE="0"
  AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD="5"
  AUTO_CYCLE_FAST_EU_US_ONLY="0"
  AUTO_CYCLE_BASE_MIN_SECS="0"
  AUTO_CYCLE_EU_US_MIN_SECS="0"
  AUTO_OPS_SNAPSHOT_ENABLED="0"
  AUTO_OPS_SNAPSHOT_INTERVAL_SECS="3600"
  AUTO_OPS_SNAPSHOT_TIMELINE="8"
  AUTO_OPS_SNAPSHOT_PROBE="0"
  AUTO_MT5_COUNT="${AUTO_MT5_COUNT:-200}"
  AUTO_MT5_BOOTSTRAP_COUNT="${AUTO_MT5_BOOTSTRAP_COUNT:-1000}"
  AUTO_MT5_ESTAB_PROBE_MIN_SECS="300"
}

apply_autonomy_hybrid_conf() {
  AUTO_AUTONOMY_POLICY="hybrid"
  AUTO_SESSION_HANDOFF="1"
  AUTO_OPPORTUNITY_MAX="1"
  AUTO_PINNED_EXPAND_ENABLED="1"
  AUTO_PINNED_MIN_COUNT="10"
  AUTO_PINNED_MIN_ASIA="9"
  AUTO_PINNED_MIN_ASIA_EU="11"
  AUTO_PINNED_MIN_EU="10"
  AUTO_PINNED_MIN_EU_US="14"
  AUTO_PINNED_MIN_US="10"
  AUTO_PINNED_MIN_OFFHOURS="8"
  AUTO_PINNED_REVIEW_ENABLED="1"
  AUTO_PINNED_REVIEW_INTERVAL_SECS="300"
  AUTO_PINNED_REVIEW_AUTOFIX="1"
  AUTO_PINNED_REVIEW_TRIM_EXTRAS="1"
  AUTO_PINNED_REVIEW_MAX_ADD="4"
  AUTO_PINNED_REVIEW_RESTART_ON_CHANGE="0"
  AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD="6"
  AUTO_CYCLE_FAST_EU_US_ONLY="1"
  AUTO_CYCLE_BASE_MIN_SECS="300"
  AUTO_CYCLE_EU_US_MIN_SECS="120"
  AUTO_OPS_SNAPSHOT_ENABLED="1"
  AUTO_OPS_SNAPSHOT_INTERVAL_SECS="3600"
  AUTO_OPS_SNAPSHOT_TIMELINE="8"
  AUTO_OPS_SNAPSHOT_PROBE="0"
  AUTO_MT5_COUNT="${AUTO_MT5_COUNT:-1000}"
  AUTO_MT5_BOOTSTRAP_COUNT="${AUTO_MT5_BOOTSTRAP_COUNT:-1000}"
  AUTO_MT5_ESTAB_PROBE_MIN_SECS="180"
}

apply_autonomy_trend_strict_conf() {
  AUTO_AUTONOMY_POLICY="trend-strict"
  AUTO_SESSION_HANDOFF="1"
  AUTO_OPPORTUNITY_MAX="1"
  AUTO_PINNED_EXPAND_ENABLED="1"
  AUTO_PINNED_MIN_COUNT="12"
  AUTO_PINNED_MIN_ASIA="10"
  AUTO_PINNED_MIN_ASIA_EU="12"
  AUTO_PINNED_MIN_EU="12"
  AUTO_PINNED_MIN_EU_US="14"
  AUTO_PINNED_MIN_US="12"
  AUTO_PINNED_MIN_OFFHOURS="10"
  AUTO_PINNED_REVIEW_ENABLED="1"
  AUTO_PINNED_REVIEW_INTERVAL_SECS="300"
  AUTO_PINNED_REVIEW_AUTOFIX="1"
  AUTO_PINNED_REVIEW_TRIM_EXTRAS="1"
  AUTO_PINNED_REVIEW_MAX_ADD="4"
  AUTO_PINNED_REVIEW_RESTART_ON_CHANGE="0"
  AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD="6"
  AUTO_CYCLE_FAST_EU_US_ONLY="1"
  AUTO_CYCLE_BASE_MIN_SECS="300"
  AUTO_CYCLE_EU_US_MIN_SECS="120"
  AUTO_OPS_SNAPSHOT_ENABLED="1"
  AUTO_OPS_SNAPSHOT_INTERVAL_SECS="3600"
  AUTO_OPS_SNAPSHOT_TIMELINE="8"
  AUTO_OPS_SNAPSHOT_PROBE="0"
  AUTO_MT5_COUNT="${AUTO_MT5_COUNT:-1000}"
  AUTO_MT5_BOOTSTRAP_COUNT="${AUTO_MT5_BOOTSTRAP_COUNT:-1000}"
  AUTO_MT5_ESTAB_PROBE_MIN_SECS="180"
}

apply_trend_strict_launch_conf() {
  conf_upsert TREND_STRICT_MODE "1"
  conf_upsert TREND_STRICT_STRATEGY_POOL "SUPERTREND,EMA_CROSSOVER,DONCHIAN,MA_CROSSOVER"
  conf_upsert STRATEGY_SELECT_EVAL "1"
  conf_upsert LET_WINNERS_RUN "1"
  conf_upsert REGIME_DIRECTIONAL "1"
  conf_upsert BLEND_LIVE_REBALANCE_BARS "14"
  conf_upsert BLEND_LIVE_TOP_K "3"
  conf_upsert SCALP_SWING_ENABLE "0"
  conf_upsert SCALP_SWING_MEM15M_ENABLE "0"
  conf_upsert BACKTEST_ELIMINATION_ENABLED "1"
}

cmd_no_touch_install() {
  AUTO_ONCALENDAR="${1:-*:0/20}"
  apply_autonomy_core_conf
  apply_autonomy_mt5_only_conf
  AUTO_MT5_MARKET="${AUTO_MT5_MARKET:-mt5}"
  AUTO_MT5_INTERVAL="${AUTO_MT5_INTERVAL:-1h}"
  AUTO_MT5_COUNT="${AUTO_MT5_COUNT:-200}"

  save_auto_env

  cmd_install_timer
  cmd_ensure
  log "no-touch modu aktif: periyot=$AUTO_ONCALENDAR, only-self-heal+mt5-sync"
}

cmd_hybrid_install() {
  AUTO_ONCALENDAR="${1:-*:0/3}"
  apply_autonomy_core_conf
  apply_autonomy_hybrid_conf
  AUTO_MT5_MARKET="${AUTO_MT5_MARKET:-mt5}"
  AUTO_MT5_INTERVAL="${AUTO_MT5_INTERVAL:-1h}"
  AUTO_MT5_COUNT="${AUTO_MT5_COUNT:-1000}"

  apply_opportunity_profile_conf
  save_auto_env

  cmd_install_timer
  cmd_session_handoff || warn "session-handoff adimi basarisiz"
  cmd_ensure
  log "hybrid modu aktif: periyot=$AUTO_ONCALENDAR, opportunity-max + MT5 sync"
  log "profilde blend + MT5 policy birlesimi uygulandi"
  log_event "hybrid_autonomy_activated oncalendar=$AUTO_ONCALENDAR euus_min=${AUTO_CYCLE_EU_US_MIN_SECS}s base_min=${AUTO_CYCLE_BASE_MIN_SECS}s"
}

cmd_opportunity_max_install() {
  AUTO_ONCALENDAR="${1:-*:0/3}"
  apply_autonomy_core_conf
  apply_autonomy_hybrid_conf
  AUTO_MT5_MARKET="${AUTO_MT5_MARKET:-mt5}"
  AUTO_MT5_INTERVAL="${AUTO_MT5_INTERVAL:-1h}"
  AUTO_MT5_COUNT="${AUTO_MT5_COUNT:-1000}"

  apply_opportunity_profile_conf
  save_auto_env

  cmd_install_timer
  cmd_session_handoff || warn "session-handoff adimi basarisiz"
  cmd_ensure
  log "opportunity-max modu aktif: periyot=$AUTO_ONCALENDAR"
  log "profilde agresif scheduler + genisletilmis seans sembol seti uygulandi"
  log_event "opportunity_max_activated oncalendar=$AUTO_ONCALENDAR euus_min=${AUTO_CYCLE_EU_US_MIN_SECS}s base_min=${AUTO_CYCLE_BASE_MIN_SECS}s"
}

cmd_trend_strict_install() {
  AUTO_ONCALENDAR="${1:-*:0/3}"
  apply_autonomy_core_conf
  apply_autonomy_trend_strict_conf
  AUTO_MT5_MARKET="${AUTO_MT5_MARKET:-mt5}"
  AUTO_MT5_INTERVAL="${AUTO_MT5_INTERVAL:-1h}"
  AUTO_MT5_COUNT="${AUTO_MT5_COUNT:-1000}"

  apply_opportunity_profile_conf
  apply_trend_strict_launch_conf
  save_auto_env

  cmd_install_timer
  cmd_session_handoff || warn "session-handoff adimi basarisiz"
  cmd_review_pinned_symbols || warn "pinned-review adimi basarisiz"
  cmd_ensure
  log "trend-strict modu aktif: periyot=$AUTO_ONCALENDAR"
  log "yalniz trend strateji havuzu + winners-run + otonom pinned/policy aktif"
  log_event "trend_strict_activated oncalendar=$AUTO_ONCALENDAR"
}

cmd_uninstall_timer() {
  systemctl --user disable --now "$AUTO_SERVICE.timer" 2>/dev/null || true
  rm -f "$AUTO_UNIT" "$AUTO_TIMER_UNIT"
  systemctl --user daemon-reload
  log "autonomy timer kaldirildi."
}

usage() {
  cat <<'EOF'
Kullanim:
  scripts/autonomy.sh status
  scripts/autonomy.sh ensure
  scripts/autonomy.sh check-venues
  scripts/autonomy.sh operator-checklist [mt5|ibkr|all]
  scripts/autonomy.sh sync-mt5 [market interval symbols_csv count]
  scripts/autonomy.sh cycle [market interval symbols_csv count]
  scripts/autonomy.sh apply-pending-handoff
  scripts/autonomy.sh review-pinned
  scripts/autonomy.sh install-timer
  scripts/autonomy.sh hybrid-install [OnCalendar]
  scripts/autonomy.sh trend-strict-install [OnCalendar]
  scripts/autonomy.sh no-touch-install [OnCalendar]
  scripts/autonomy.sh opportunity-max-install [OnCalendar]
  scripts/autonomy.sh uninstall-timer

Varsayilanlar:
  AUTO_MT5_MARKET   (default: mt5)
  AUTO_MT5_INTERVAL (default: 1h)
  AUTO_MT5_COUNT    (default: 200)
  AUTO_MT5_BOOTSTRAP (default: 0)
  AUTO_MT5_BOOTSTRAP_COUNT (default: 1000)
  AUTO_MT5_ESTAB_PROBE_ON_MISS (default: 1)
  AUTO_MT5_ESTAB_PROBE_MIN_SECS (default: 300)
  AUTO_MT5_SYMBOLS  (default: PINNED_SYMBOLS icindeki @mt5 sembolleri)
  AUTO_ONCALENDAR   (default: *:0/30)
  AUTO_SYNC_MT5     (default: 1)
  AUTO_SESSION_HANDOFF (default: 0)
  AUTO_SESSION_HANDOFF_IMMEDIATE_RESTART (default: 0)
  AUTO_SESSION_HANDOFF_PENDING_FILE (default: logs/.autonomy-handoff.pending)
  AUTO_SESSION_HANDOFF_PENDING_APPLY (default: 1)
  AUTO_SESSION_HANDOFF_PENDING_MIN_AGE_SECS (default: 120)
  AUTO_SESSION_HANDOFF_PENDING_REQUIRE_FLAT (default: 1)
  AUTO_OPPORTUNITY_MAX (default: 0)
  AUTO_PINNED_EXPAND_ENABLED (default: 0)
  AUTO_PINNED_MIN_COUNT (default: 0)
  AUTO_PINNED_MIN_ASIA / _ASIA_EU / _EU / _EU_US / _US / _OFFHOURS (default: inherit)
  AUTO_PINNED_REVIEW_ENABLED (default: 1)
  AUTO_PINNED_REVIEW_INTERVAL_SECS (default: 600)
  AUTO_PINNED_REVIEW_AUTOFIX (default: 1)
  AUTO_PINNED_REVIEW_TRIM_EXTRAS (default: 0)
  AUTO_PINNED_REVIEW_MAX_ADD (default: 4)
  AUTO_PINNED_REVIEW_RESTART_ON_CHANGE (default: 0)
  AUTO_PINNED_REVIEW_NOTIFY_THRESHOLD (default: 6)
  AUTO_CYCLE_FAST_EU_US_ONLY (default: 0)
  AUTO_CYCLE_BASE_MIN_SECS (default: 0)
  AUTO_CYCLE_EU_US_MIN_SECS (default: 0)
  AUTO_REQUIRE_MT5_ESTAB (default: 0)
  AUTO_COVERAGE_AUDIT_ENABLED (default: 1)
  AUTO_COVERAGE_AUDIT_ONLY_OFFHOURS (default: 1)
  AUTO_COVERAGE_AUDIT_AUTOFIX (default: 1)
  AUTO_COVERAGE_AUDIT_MARKET (default: futures)
  AUTO_COVERAGE_AUDIT_INTERVALS (default: 1d,4h)
  AUTO_DAILY_ENABLED (default: 0)
  AUTO_MT5_CHECK_ENABLED (default: 1)
  AUTO_VENUE_HEALTHCHECK_ENABLED (default: 1)
  AUTO_VENUE_HEALTHCHECK_STRICT (default: 0)
  AUTO_IBKR_CHECK_ENABLED (default: 1)
  AUTO_IBKR_CHECK_INTERVAL_SECS (default: 300)
  AUTO_IBKR_CHECK_STATE_FILE (default: logs/.autonomy-ibkr-check.state)
  AUTO_IBKR_API_PORT (default: 4002)
  AUTO_IBKR_BRIDGE_ADDR (default: 127.0.0.1:9101)
  AUTO_IBKR_BRIDGE_AUTOSTART (default: 1)
  AUTO_IBKR_BRIDGE_AUTOSTART_COOLDOWN_SECS (default: 600)
  AUTO_IBKR_BRIDGE_STATE_FILE (default: logs/.autonomy-ibkr-bridge.state)
  AUTO_NOTIFY_CRITICAL (default: 0)
  AUTO_DAILY_SUMMARY (default: 0)
  AUTO_OPS_SNAPSHOT_ENABLED (default: 0)
  AUTO_OPS_SNAPSHOT_INTERVAL_SECS (default: 3600)
  AUTO_OPS_SNAPSHOT_TIMELINE (default: 8)
  AUTO_OPS_SNAPSHOT_PROBE (default: 0)
  AUTO_AUTONOMY_POLICY (default: core)
  TREND_STRICT_MODE (default: 0)
  TREND_STRICT_STRATEGY_POOL (default: SUPERTREND,EMA_CROSSOVER,DONCHIAN,MA_CROSSOVER)
EOF
}

case "${1:-status}" in
  status)
    cmd_status
    ;;
  ensure)
    cmd_ensure
    ;;
  check-venues)
    cmd_check_venue_readiness
    ;;
  operator-checklist)
    shift
    cmd_operator_checklist "${1:-all}"
    ;;
  sync-mt5)
    shift
    cmd_sync_mt5 "$@"
    ;;
  cycle)
    shift
    cmd_cycle "$@"
    ;;
  apply-pending-handoff)
    cmd_apply_pending_handoff
    ;;
  review-pinned)
    cmd_review_pinned_symbols
    ;;
  install-timer)
    cmd_install_timer
    ;;
  hybrid-install)
    shift
    cmd_hybrid_install "${1:-}"
    ;;
  trend-strict-install)
    shift
    cmd_trend_strict_install "${1:-}"
    ;;
  no-touch-install)
    shift
    cmd_no_touch_install "${1:-}"
    ;;
  opportunity-max-install)
    shift
    cmd_opportunity_max_install "${1:-}"
    ;;
  uninstall-timer)
    cmd_uninstall_timer
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    echo "Bilinmeyen komut: $1"
    usage
    exit 1
    ;;
esac
