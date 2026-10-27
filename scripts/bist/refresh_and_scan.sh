#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
PROFILE="${BIST_PROFILE:-$ROOT_DIR/profiles/paper-bist-signal.conf}"
LOG_FILE="${BIST_REFRESH_LOG:-$ROOT_DIR/logs/bist-refresh.log}"
STATUS_FILE="${BIST_REFRESH_STATUS_FILE:-$ROOT_DIR/data/bist_refresh_status.json}"
TREND_STATUS_FILE="${BIST_TREND_STATUS_FILE:-$ROOT_DIR/data/bist_trend_status.json}"
PAPER_STATE_FILE="${BIST_PAPER_STATE_FILE:-$ROOT_DIR/data/bist_paper_state.json}"
export BIST_TREND_STATUS_PATH="$TREND_STATUS_FILE"
export BIST_PAPER_STATE_PATH="$PAPER_STATE_FILE"

cd "$ROOT_DIR"
mkdir -p "$(dirname "$LOG_FILE")"
mkdir -p "$(dirname "$STATUS_FILE")"
exec >>"$LOG_FILE" 2>&1

printf '[bist-refresh %s] başladı\n' "$(date -Is)"

if [[ ! -f "$PROFILE" ]]; then
  printf '[bist-refresh %s] profil bulunamadı: %s\n' "$(date -Is)" "$PROFILE" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
. "$PROFILE"
set +a

write_status() {
  local status="$1"
  local error_summary="${2:-}"
  STATUS_VALUE="$status" STATUS_ERROR="$error_summary" STATUS_PATH="$STATUS_FILE" \
    python3 - <<'PY'
import json
import os
from datetime import datetime, timezone
from pathlib import Path

payload = {
    "checked_at": datetime.now(timezone.utc).isoformat(),
    "status": os.environ["STATUS_VALUE"],
    "source": "google_sheets",
    "profile": os.environ.get("BIST_PROFILE", "paper-bist-signal"),
    "error": os.environ.get("STATUS_ERROR", ""),
    "pipeline_run_id": os.environ.get("BIST_SOURCE_COMPLETED_AT", ""),
    "source_completed_at": os.environ.get("BIST_SOURCE_COMPLETED_AT", ""),
}
trend_path = Path(os.environ["BIST_TREND_STATUS_PATH"])
if trend_path.exists():
    try:
        trend = json.loads(trend_path.read_text(encoding="utf-8"))
        payload["trend_run_finished"] = trend.get("run_finished", "")
        payload["trend_status"] = trend.get("status", "")
        payload["trend_latest_candle"] = trend.get("latest_candle_timestamp", "")
    except (OSError, json.JSONDecodeError):
        payload["trend_run_finished"] = ""
        payload["trend_status"] = ""
        payload["trend_latest_candle"] = ""
paper_path = Path(os.environ["BIST_PAPER_STATE_PATH"])
if paper_path.exists():
    try:
        paper = json.loads(paper_path.read_text(encoding="utf-8"))
        payload["paper_as_of"] = paper.get("last_as_of", "")
    except (OSError, json.JSONDecodeError):
        payload["paper_as_of"] = ""
path = Path(os.environ["STATUS_PATH"])
tmp = path.with_suffix(path.suffix + ".tmp")
tmp.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
tmp.replace(path)
PY
}

if [[ -z "${BIST_SHEETS_API_URL:-}" ]]; then
  printf '[bist-refresh %s] BIST_SHEETS_API_URL eksik\n' "$(date -Is)" >&2
  write_status "CONFIG_ERROR" "BIST_SHEETS_API_URL eksik"
  exit 1
fi

sync_status="$(
  BIST_SHEETS_STATUS_URL="$BIST_SHEETS_API_URL" \
  python3 - <<'PY'
import json
import os
import urllib.request
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

parts = urlsplit(os.environ["BIST_SHEETS_STATUS_URL"])
query = dict(parse_qsl(parts.query, keep_blank_values=True))
query["status"] = "1"
status_url = urlunsplit(
    (parts.scheme, parts.netloc, parts.path, urlencode(query), parts.fragment)
)
request = urllib.request.Request(
    status_url,
    headers={"User-Agent": "memos-bist-refresh/1.0"},
)
with urllib.request.urlopen(request, timeout=30) as response:
    payload = json.load(response)
if not payload.get("ok"):
    raise RuntimeError(payload.get("error") or "Google Sheets BIST API başarısız")
status = payload.get("sync_status") or {}
print(json.dumps(status, ensure_ascii=False))
if status.get("status") != "COMPLETED":
    raise RuntimeError(
        "Google Sheets batch tamamlanmadı: "
        f"{status.get('status', 'UNKNOWN')} "
        f"({status.get('symbols_with_session_date', 0)}/"
        f"{status.get('total_symbols', 0)})"
    )
if status.get("symbols_with_session_date") != status.get("total_symbols"):
    raise RuntimeError("Google Sheets sembol evreni aynı session tarihine ulaşmadı")
PY
)" || {
  printf '[bist-refresh %s] veri kapısı bekliyor: %s\n' "$(date -Is)" "$sync_status" >&2
  write_status "DATA_STALE" "$sync_status"
  exit 0
}

SOURCE_COMPLETED_AT="$(
  SOURCE_STATUS="$sync_status" python3 - <<'PY'
import json
import os

status = json.loads(os.environ["SOURCE_STATUS"])
print(status.get("completed_at", ""))
PY
)"
export BIST_SOURCE_COMPLETED_AT="$SOURCE_COMPLETED_AT"

printf '[bist-refresh %s] Google Sheets batch tamamlandı: %s\n' "$(date -Is)" "$sync_status"
if SOURCE_STATUS="$sync_status" TREND_STATUS_PATH="$TREND_STATUS_FILE" python3 - <<'PY'
import json
import os
from datetime import datetime
from pathlib import Path

def parse_timestamp(value):
    normalized = value.replace("Z", "+00:00")
    if "." in normalized:
        prefix, suffix = normalized.split(".", 1)
        timezone = ""
        if "+" in suffix:
            fraction, timezone = suffix.split("+", 1)
            timezone = "+" + timezone
        elif "-" in suffix:
            fraction, timezone = suffix.split("-", 1)
            timezone = "-" + timezone
        else:
            fraction = suffix
        normalized = f"{prefix}.{fraction[:6]}{timezone}"
    return datetime.fromisoformat(normalized)

source = json.loads(os.environ["SOURCE_STATUS"])
source_completed = source.get("completed_at")
trend_path = Path(os.environ["TREND_STATUS_PATH"])
if not source_completed or not trend_path.exists():
    raise SystemExit(1)

with trend_path.open(encoding="utf-8") as handle:
    trend = json.load(handle)
trend_finished = trend.get("run_finished")
if not trend_finished:
    raise SystemExit(1)

source_time = parse_timestamp(source_completed)
trend_time = parse_timestamp(trend_finished)
raise SystemExit(0 if source_time <= trend_time else 1)
PY
then
  write_status "READY"
  printf '[bist-refresh %s] kaynak batch değişmedi; trend ve paper adımı atlandı\n' "$(date -Is)"
  exit 0
fi

write_status "SYNCED"

if [[ -x "$ROOT_DIR/target/release/examples/bist_trend_scan" ]]; then
  "$ROOT_DIR/target/release/examples/bist_trend_scan"
else
  cargo run --release \
    --manifest-path "$ROOT_DIR/Cargo.toml" \
    -p memos_trading_core \
    --example bist_trend_scan
fi

BIST_TREND_REPORT="$ROOT_DIR/data/bist_trend.json" \
  python3 "$ROOT_DIR/scripts/bist/daily_paper_sim.py"
write_status "READY"
printf '[bist-refresh %s] tamamlandı\n' "$(date -Is)"
