#!/usr/bin/env bash
# scripts/soak_check.sh - Hybrid soak ara kontrol raporu uretir.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

# shellcheck source=scripts/lib_launchconf.sh
. "$REPO_DIR/scripts/lib_launchconf.sh" 2>/dev/null || true
load_launch_conf "$REPO_DIR/scripts/.launch.conf" 2>/dev/null || true

ACTIVE_PROFILE="$(cat "$REPO_DIR/scripts/.active" 2>/dev/null || echo unknown)"
SNAP_PATH="${SNAPSHOT_PATH:-data/snapshot.json}"
TS="$(date +%Y%m%d_%H%M%S)"
OUT="reports/hybrid_soak_check_${ACTIVE_PROFILE}_${TS}.txt"

mkdir -p reports

{
  echo "# Hybrid Soak Check ${TS}"
  echo ""
  echo "profile=${ACTIVE_PROFILE}"
  echo "snapshot=${SNAP_PATH}"
  echo ""

  echo "## memos status"
  ./memos status
  echo ""

  echo "## memos auto status"
  ./memos auto status
  echo ""

  echo "## snapshot key events"
  python3 - <<'PY'
import json
import os

snap = os.environ.get("SNAP_PATH", "data/snapshot.json")
keys = (
    "Download başladı:",
    "Download ✓ tamamlandı:",
    "CopyRates başarısız",
    "Screener",
    "ML Retrain",
    "Anomali onarımı",
)

try:
    with open(snap, encoding="utf-8") as f:
        data = json.load(f)
    logs = data.get("logs", [])
    for item in logs[:120]:
        if not isinstance(item, dict):
            continue
        msg = item.get("message", "")
        if any(k in msg for k in keys):
            ts = item.get("timestamp", "??")
            print(f"{ts} {msg}")
except Exception as e:
    print(f"snapshot parse error: {e}")
PY
} > "$OUT"

echo "$OUT"
