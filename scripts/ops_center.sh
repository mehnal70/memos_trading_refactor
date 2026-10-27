#!/usr/bin/env bash
# scripts/ops_center.sh - Otonom operasyon kontrol/takip/rapor merkezi

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

# shellcheck source=scripts/lib_launchconf.sh
. "$REPO_DIR/scripts/lib_launchconf.sh" 2>/dev/null || true
load_launch_conf "$REPO_DIR/scripts/.launch.conf" 2>/dev/null || true

TRADING_LOG="${TRADING_LOG_PATH:-logs/robotic_trading.log}"
HEARTBEAT_LOG="${HEARTBEAT_PATH:-logs/heartbeat.jsonl}"
TIMELINE_DEFAULT="${OPS_TIMELINE_LIMIT:-10}"
EVENT_TAIL_DEFAULT="${OPS_EVENT_TAIL:-80}"
SNAPSHOT_DIR_DEFAULT="${OPS_SNAPSHOT_DIR:-data/reports/ops}"

trim() {
  local s="${1:-}"
  s="${s#${s%%[![:space:]]*}}"
  s="${s%${s##*[![:space:]]}}"
  echo "$s"
}

csv_join_nonempty() {
  local out=""
  local part
  for part in "$@"; do
    part="$(trim "$part")"
    [ -z "$part" ] && continue
    out="${out:+$out,}${part}"
  done
  echo "$out"
}

normalize_symbol_item() {
  local item="$(trim "${1:-}")"
  local sym venue
  [ -z "$item" ] && { echo ""; return 0; }
  case "$item" in
    *@*)
      sym="${item%@*}"
      venue="${item##*@}"
      if [ "$venue" = "binance" ]; then
        echo "$sym"
      else
        echo "$sym@$venue"
      fi
      ;;
    *)
      echo "$item"
      ;;
  esac
}

dedupe_csv() {
  local raw="${1:-}"
  local out=""
  local item norm
  IFS=',' read -r -a arr <<< "$raw"
  for item in "${arr[@]}"; do
    norm="$(normalize_symbol_item "$item")"
    [ -z "$norm" ] && continue
    case ",$out," in
      *",$norm,"*) ;;
      *) out="${out:+$out,}$norm" ;;
    esac
  done
  echo "$out"
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

csv_count() {
  local raw="$(dedupe_csv "${1:-}")"
  [ -z "$raw" ] && { echo 0; return 0; }
  awk -F',' '{print NF}' <<< "$raw"
}

candidate_symbol_pool() {
  local all
  all="$(csv_join_nonempty \
    "${BLEND_LIVE_SYMBOLS:-}" \
    "${AUTO_MT5_SYMBOLS:-}" \
    "${SESSION_CORE_SYMBOLS:-}" \
    "${SESSION_ASIA_SYMBOLS:-}" \
    "${SESSION_ASIA_EU_OVERLAP_SYMBOLS:-}" \
    "${SESSION_EU_SYMBOLS:-}" \
    "${SESSION_EU_US_OVERLAP_SYMBOLS:-}" \
    "${SESSION_US_SYMBOLS:-}" \
    "${SESSION_OFFHOURS_SYMBOLS:-}")"
  dedupe_csv "$all"
}

print_header() {
  echo
  echo "=== $1 ==="
}

cmd_report() {
  local probe_flag="${1:-}"
  local timeline="${2:-$TIMELINE_DEFAULT}"

  print_header "Sistem Durumu"
  "$REPO_DIR/scripts/autonomy.sh" status || true

  print_header "MT5 + Havuz Ozeti"
  if [ "$probe_flag" = "--probe" ]; then
    python3 "$REPO_DIR/scripts/pool_observer.py" --probe --timeline-limit "$timeline"
  else
    python3 "$REPO_DIR/scripts/pool_observer.py" --timeline-limit "$timeline"
  fi

  print_header "Autonomy Olay Gunlugu"
  if [ -f "$TRADING_LOG" ]; then
    grep -Ei "AUTONOMY|cycle_throttle|handoff_|opportunity_max_activated" "$TRADING_LOG" | tail -n "$EVENT_TAIL_DEFAULT" || echo "(Autonomy event bulunamadi)"
  else
    echo "(Trading log bulunamadi: $TRADING_LOG)"
  fi

  print_header "Sembol Bosluk Analizi"
  cmd_symbol_gap "--short"
}

cmd_symbol_gap() {
  local short="${1:-}"
  local pinned pool missing extra

  pinned="$(dedupe_csv "${PINNED_SYMBOLS:-}")"
  pool="$(candidate_symbol_pool)"
  missing="$(csv_diff "$pool" "$pinned")"
  extra="$(csv_diff "$pinned" "$pool")"

  if [ "$short" = "--short" ]; then
    echo "pool_count=$(csv_count "$pool") pinned_count=$(csv_count "$pinned") missing_count=$(csv_count "$missing") extra_count=$(csv_count "$extra")"
    return 0
  fi

  print_header "Sembol Bosluk Ozeti"
  echo "pool_count=$(csv_count "$pool")"
  echo "pinned_count=$(csv_count "$pinned")"
  echo "missing_count=$(csv_count "$missing")"
  echo "extra_count=$(csv_count "$extra")"

  print_header "Eksik Potansiyel Semboller (Pool - Pinned)"
  if [ -n "$missing" ]; then
    echo "$missing" | tr ',' '\n'
  else
    echo "(eksik yok)"
  fi

  print_header "Pool Disi Pinli Semboller (Pinned - Pool)"
  if [ -n "$extra" ]; then
    echo "$extra" | tr ',' '\n'
  else
    echo "(pool disi pinned yok)"
  fi
}

cmd_events() {
  local n="${1:-$EVENT_TAIL_DEFAULT}"
  print_header "Autonomy Olaylari (son $n)"
  if [ -f "$TRADING_LOG" ]; then
    grep -Ei "AUTONOMY|cycle_throttle|handoff_|opportunity_max_activated" "$TRADING_LOG" | tail -n "$n" || echo "(Autonomy event bulunamadi)"
  else
    echo "(Trading log bulunamadi: $TRADING_LOG)"
  fi
}

cmd_follow() {
  print_header "Canli Otonomi Takibi (Ctrl-C ile cik)"
  echo "trading=$TRADING_LOG"
  echo "heartbeat=$HEARTBEAT_LOG"
  tail -n 200 -F "$TRADING_LOG" "$HEARTBEAT_LOG" 2>/dev/null | \
    grep --line-buffered -Ei "AUTONOMY|cycle_throttle|handoff_|opportunity_max_activated|Scheduler|Backtest|MT5 veri ozeti|MT5 veri özeti|AUX MT5|Tetik|trigger|pipeline"
}

cmd_snapshot() {
  local probe_flag="${1:-}"
  local timeline="${2:-$TIMELINE_DEFAULT}"
  local out_dir="${3:-$SNAPSHOT_DIR_DEFAULT}"
  local ts out_file latest_file profile_name

  profile_name="${ACTIVE_PROFILE:-}"
  if [ -z "$profile_name" ] && [ -f "$REPO_DIR/scripts/.active" ]; then
    profile_name="$(cat "$REPO_DIR/scripts/.active" 2>/dev/null || true)"
  fi
  [ -z "$profile_name" ] && profile_name="unknown"

  mkdir -p "$out_dir"
  ts="$(date +%Y%m%d_%H%M%S)"
  out_file="$out_dir/ops_report_${ts}.txt"
  latest_file="$out_dir/ops_report_latest.txt"

  {
    echo "# memos ops snapshot"
    echo "generated_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "profile=$profile_name"
    if [ "$probe_flag" = "--probe" ]; then
      cmd_report "--probe" "$timeline"
    else
      cmd_report "" "$timeline"
    fi
  } > "$out_file"

  cp "$out_file" "$latest_file"
  echo "$out_file"
}

cmd_setup() {
  local action="${1:-status}" target="${2:-all}"
  case "$action" in
    status|verify|start|start-gui|engine|daemon)
      "$REPO_DIR/scripts/runtime_supervisor.sh" "$action" "$target"
      ;;
    *)
      echo "setup action geçersiz: $action" >&2
      return 2
      ;;
  esac
}

usage() {
  cat <<'EOF'
Kullanim:
  scripts/ops_center.sh report [--probe] [timeline_limit]
  scripts/ops_center.sh events [n]
  scripts/ops_center.sh follow
  scripts/ops_center.sh snapshot [--probe] [timeline_limit] [out_dir]
  scripts/ops_center.sh symbol-gap
  scripts/ops_center.sh setup [status|verify|start|start-gui|engine|daemon] [mt5|ibkr|binance|bist|all]

Amac:
  report -> tek komutta status + observe + autonomy olaylari
  events -> sadece autonomy olay satirlari
  follow -> canli otonomi/pipeline olay takibi
  snapshot -> tek raporu dosyaya yazar (timestamp + latest)
  symbol-gap -> aday havuzuna gore eksik/pool-disi sembolleri listeler
  setup -> hazır bağımlılıkları yeniden başlatmadan dış bağımlılıkları ve venue engine'lerini yönetir
EOF
}

case "${1:-report}" in
  report)
    shift
    if [ "${1:-}" = "--probe" ]; then
      shift
      cmd_report "--probe" "${1:-$TIMELINE_DEFAULT}"
    else
      cmd_report "" "${1:-$TIMELINE_DEFAULT}"
    fi
    ;;
  events)
    shift
    cmd_events "${1:-$EVENT_TAIL_DEFAULT}"
    ;;
  follow)
    cmd_follow
    ;;
  snapshot)
    shift
    if [ "${1:-}" = "--probe" ]; then
      shift
      cmd_snapshot "--probe" "${1:-$TIMELINE_DEFAULT}" "${2:-$SNAPSHOT_DIR_DEFAULT}"
    else
      cmd_snapshot "" "${1:-$TIMELINE_DEFAULT}" "${2:-$SNAPSHOT_DIR_DEFAULT}"
    fi
    ;;
  symbol-gap)
    cmd_symbol_gap
    ;;
  setup)
    shift
    cmd_setup "${1:-status}" "${2:-all}"
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
