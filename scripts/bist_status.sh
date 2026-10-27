#!/usr/bin/env bash
# BIST indirme ilerlemesini snapshot dosyasindan raporlar.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SNAPSHOT_PATH="${BIST_SNAPSHOT_PATH:-$REPO_DIR/logs/soak/bist/snapshot.json}"
PID_PATH="${BIST_PID_PATH:-$REPO_DIR/logs/soak/bist/engine.pid}"
HEARTBEAT_PATH="${BIST_HEARTBEAT_PATH:-$REPO_DIR/logs/soak/bist/heartbeat.jsonl}"
REFRESH_STATUS_PATH="${BIST_REFRESH_STATUS_PATH:-$REPO_DIR/data/bist_refresh_status.json}"
TREND_STATUS_PATH="${BIST_TREND_STATUS_PATH:-$REPO_DIR/data/bist_trend_status.json}"
PAPER_STATE_PATH="${BIST_PAPER_STATE_PATH:-$REPO_DIR/data/bist_paper_state.json}"
INTERVAL="${BIST_STATUS_INTERVAL:-10}"
WATCH_MODE=false

usage() {
  cat <<'EOF'
Kullanim:
  scripts/bist_status.sh             Tek seferlik durum
  scripts/bist_status.sh --watch     Her 10 saniyede canlı durum
  scripts/bist_status.sh --watch 5   Her 5 saniyede canlı durum

Ortam degiskenleri:
  BIST_SNAPSHOT_PATH  Snapshot dosyasi yolu
  BIST_STATUS_INTERVAL Varsayilan canli izleme araligi (saniye)
EOF
}

if ! command -v jq >/dev/null 2>&1; then
  echo "Hata: jq kurulu degil." >&2
  exit 1
fi

if [[ ! -f "$SNAPSHOT_PATH" ]]; then
  echo "Hata: snapshot bulunamadi: $SNAPSHOT_PATH" >&2
  exit 1
fi

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

if [[ "${1:-}" == "--watch" ]]; then
  WATCH_MODE=true
  if [[ -n "${2:-}" ]]; then
    INTERVAL="$2"
  fi
fi

if ! [[ "$INTERVAL" =~ ^[1-9][0-9]*$ ]]; then
  echo "Hata: izleme araligi pozitif bir tam sayi olmali: $INTERVAL" >&2
  exit 1
fi

process_state() {
  local pid=""
  if [[ -f "$PID_PATH" ]]; then
    pid="$(cat "$PID_PATH" 2>/dev/null || true)"
  fi
  if [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null; then
    echo "RUNNING (pid=$pid)"
  elif [[ -n "$pid" ]]; then
    echo "STOPPED (stale_pid=$pid)"
  else
    echo "STOPPED"
  fi
}

heartbeat_age() {
  if [[ ! -f "$HEARTBEAT_PATH" ]]; then
    echo "missing"
    return
  fi
  echo "$(( $(date +%s) - $(stat -c %Y "$HEARTBEAT_PATH" 2>/dev/null || echo 0) ))s"
}

print_status() {
  if [[ -f "$REFRESH_STATUS_PATH" ]]; then
    jq -r '"Refresh worker: \(.status // "UNKNOWN") (\(.checked_at // "-"))",
      "Refresh hatasi: \(if (.error // "") == "" then "yok" else .error end)"
    ' "$REFRESH_STATUS_PATH"
  else
    printf 'Refresh worker: UNKNOWN (durum artefakti yok)\n'
  fi
  if [[ -f "$TREND_STATUS_PATH" ]]; then
    jq -r '
      "Trend scan: \(.status // "UNKNOWN")",
      "Trend veri tazeliği: \(.data_freshness // "-")",
      "Trend sembolleri: \(.successful_download_count // 0)/\(.expected_symbol_count // 0)",
      "Trend son candle: \(.latest_candle_timestamp // "-")",
      "Trend son çalışma: \(.run_finished // "-")"
    ' "$TREND_STATUS_PATH"
  else
    printf 'Trend scan: UNKNOWN (durum artefakti yok)\n'
  fi
  if [[ -f "$PAPER_STATE_PATH" ]]; then
    jq -r '
      "Paper simulation: PROCESSED",
      "Paper as-of: \(.last_as_of // "-")",
      "Paper açık pozisyon: \((.positions // {}) | length)",
      "Paper realized PnL: \(.realized_pnl // 0)",
      "Paper fees: \(.fees // 0)"
    ' "$PAPER_STATE_PATH"
  else
    printf 'Paper simulation: UNKNOWN (durum artefakti yok)\n'
  fi
  printf '\n--- Legacy soak snapshot ---\n'
  printf 'Legacy BIST engine: %s\n' "$(process_state)"
  printf 'Legacy heartbeat yasi: %s\n' "$(heartbeat_age)"
  jq -r '
    . as $snapshot
    | $snapshot.download_progress
    |     "Legacy veri durumu: \(
        if .active then "DEVAM EDIYOR"
        elif .total_symbols == 0
          then "BEKLIYOR"
        elif (.total_symbols > 0 and .completed_symbols == .total_symbols and .failed_symbols == 0 and .last_error == "")
          then "TAMAMLANDI"
        elif (.failed_symbols > 0 or .last_error != "")
          then "HATA"
        else "TAMAMLANMADI"
        end
      )",
      "Legacy ilerleme: \(.completed_symbols)/\(.total_symbols)",
      "Legacy basarisiz: \(.failed_symbols)",
      "Legacy sembol: \(.current_symbol // "-")",
      "Legacy faz: \(.current_phase // "-")",
      "Legacy son tamamlanan: \(.last_completed_symbol // "-")",
      "Legacy son hata: \(if .last_error == "" then "yok" else .last_error end)",
      "Legacy kaynak: \($snapshot.bist_data_status.data_source // "google_sheets")",
      "Legacy kaynak senkronizasyonu: \($snapshot.bist_data_status.source_sync_status // "WAITING")",
      "Legacy guncel semboller: \($snapshot.bist_data_status.fresh_symbols // 0)/\($snapshot.bist_data_status.expected_symbols // 0)",
      "Legacy reddedilen kaynak satiri: \($snapshot.bist_data_status.rejected_rows // 0)",
      "Legacy son candle: \($snapshot.bist_data_status.latest_candle // "-")",
      "Legacy candle yasi (sn): \($snapshot.bist_data_status.latest_age_secs // "-")",
      "Legacy piyasa durumu: \($snapshot.bist_data_status.market_state // "-")",
      "Legacy gunluk candle beklentisi: \($snapshot.bist_data_status.daily_candle_expectation // "-")",
      "Legacy soak sagligi: \($snapshot.bist_data_status.soak_health // "-")",
      "Legacy readiness: \($snapshot.bist_data_status.readiness_status // "SIGNAL_ONLY")",
      "Legacy execution: \($snapshot.bist_data_status.execution_status // "DISABLED")"
  ' "$SNAPSHOT_PATH"
}

if [[ "$WATCH_MODE" == true ]]; then
  while true; do
    clear
    printf '%s\n\n' "$(date '+%Y-%m-%d %H:%M:%S %Z')"
    if ! print_status; then
      echo "Hata: snapshot okunamadi: $SNAPSHOT_PATH" >&2
      exit 1
    fi
    sleep "$INTERVAL"
  done
else
  print_status
fi
