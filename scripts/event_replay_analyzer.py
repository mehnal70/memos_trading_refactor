#!/usr/bin/env python3
"""Canonical event replay analyzer.

Reads EventBus replay JSONL and prints operational summaries:
- event counts by domain, type, venue
- pipeline step status breakdown
- pipeline anomaly severity/kind breakdown

Optional JSON export can be used by dashboards or post-run reports.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from dataclasses import asdict, dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple


REPO_ROOT = Path(__file__).resolve().parent.parent
LAUNCH_CONF = REPO_ROOT / "scripts" / ".launch.conf"


def parse_kv_file(path: Path) -> Dict[str, str]:
    out: Dict[str, str] = {}
    if not path.exists():
        return out
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        out[k.strip()] = v.strip()
    return out


def resolve_default_replay_path() -> Path:
    cfg = parse_kv_file(LAUNCH_CONF)
    raw = cfg.get("EVENT_REPLAY_LOG_PATH", "logs/events_replay.jsonl")
    return (REPO_ROOT / raw).resolve()


def parse_ts(ts: str) -> Optional[datetime]:
    if not ts:
        return None
    txt = ts.strip()
    if txt.endswith("Z"):
        txt = txt[:-1] + "+00:00"
    try:
        dt = datetime.fromisoformat(txt)
    except Exception:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(timezone.utc)


def iter_jsonl(path: Path) -> Iterable[Dict[str, Any]]:
    with path.open("r", encoding="utf-8", errors="replace") as f:
        for line in f:
            s = line.strip()
            if not s:
                continue
            try:
                obj = json.loads(s)
            except Exception:
                continue
            if isinstance(obj, dict):
                yield obj


@dataclass
class ReplaySummary:
    source_path: str
    rows_scanned: int
    rows_kept: int
    first_ts: Optional[str]
    last_ts: Optional[str]
    domain_counts: Dict[str, int]
    event_type_counts: Dict[str, int]
    venue_counts: Dict[str, int]
    pipeline_step_status_counts: Dict[str, int]
    pipeline_anomaly_severity_counts: Dict[str, int]
    pipeline_anomaly_kind_counts: Dict[str, int]
    pipeline_step_fail_ratio: float


def sorted_counter(counter: Counter) -> Dict[str, int]:
    return {k: counter[k] for k in sorted(counter, key=lambda x: (-counter[x], x))}


def analyze(rows: List[Dict[str, Any]]) -> ReplaySummary:
    domain_counts: Counter = Counter()
    event_type_counts: Counter = Counter()
    venue_counts: Counter = Counter()
    step_status_counts: Counter = Counter()
    anomaly_sev_counts: Counter = Counter()
    anomaly_kind_counts: Counter = Counter()

    first_dt: Optional[datetime] = None
    last_dt: Optional[datetime] = None

    for row in rows:
        domain = str(row.get("domain", "unknown") or "unknown")
        event_type = str(row.get("event_type", "unknown") or "unknown")
        venue = str(row.get("venue", "unknown") or "unknown")

        domain_counts[domain] += 1
        event_type_counts[event_type] += 1
        venue_counts[venue] += 1

        dt = parse_ts(str(row.get("timestamp", "")))
        if dt is not None:
            if first_dt is None or dt < first_dt:
                first_dt = dt
            if last_dt is None or dt > last_dt:
                last_dt = dt

        payload = row.get("payload", {})
        if not isinstance(payload, dict):
            payload = {}

        if event_type == "PIPELINE_STEP":
            status = str(payload.get("status", "UNKNOWN") or "UNKNOWN")
            step_status_counts[status] += 1

        if event_type == "PIPELINE_ANOMALY":
            sev = str(payload.get("severity", "UNKNOWN") or "UNKNOWN")
            kind = str(payload.get("kind", "UNKNOWN") or "UNKNOWN")
            anomaly_sev_counts[sev] += 1
            anomaly_kind_counts[kind] += 1

    step_total = sum(step_status_counts.values())
    step_failed = step_status_counts.get("Failed", 0)
    fail_ratio = (step_failed / step_total) if step_total > 0 else 0.0

    return ReplaySummary(
        source_path="",
        rows_scanned=0,
        rows_kept=len(rows),
        first_ts=first_dt.isoformat() if first_dt else None,
        last_ts=last_dt.isoformat() if last_dt else None,
        domain_counts=sorted_counter(domain_counts),
        event_type_counts=sorted_counter(event_type_counts),
        venue_counts=sorted_counter(venue_counts),
        pipeline_step_status_counts=sorted_counter(step_status_counts),
        pipeline_anomaly_severity_counts=sorted_counter(anomaly_sev_counts),
        pipeline_anomaly_kind_counts=sorted_counter(anomaly_kind_counts),
        pipeline_step_fail_ratio=round(fail_ratio, 6),
    )


def print_top(counter_map: Dict[str, int], title: str, limit: int) -> None:
    print(title)
    if not counter_map:
        print("  - none")
        return
    n = 0
    for key, val in counter_map.items():
        print(f"  - {key}: {val}")
        n += 1
        if n >= limit:
            break


def select_rows(path: Path, tail: int, since: Optional[datetime]) -> Tuple[List[Dict[str, Any]], int]:
    scanned = 0
    rows: List[Dict[str, Any]] = []
    for row in iter_jsonl(path):
        scanned += 1
        if since is not None:
            dt = parse_ts(str(row.get("timestamp", "")))
            if dt is None or dt < since:
                continue
        rows.append(row)

    if tail > 0 and len(rows) > tail:
        rows = rows[-tail:]

    return rows, scanned


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Analyze canonical replay JSONL from EventBus")
    p.add_argument(
        "--input",
        default="",
        help="Replay JSONL path (default: EVENT_REPLAY_LOG_PATH from scripts/.launch.conf, or logs/events_replay.jsonl)",
    )
    p.add_argument("--tail", type=int, default=0, help="Analyze only last N events after filtering")
    p.add_argument("--since-minutes", type=int, default=0, help="Analyze only events from last N minutes")
    p.add_argument("--top", type=int, default=15, help="Top item count to print per section")
    p.add_argument("--out", default="", help="Optional output JSON path")
    return p


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    src = Path(args.input).expanduser() if args.input else resolve_default_replay_path()
    if not src.is_absolute():
        src = (REPO_ROOT / src).resolve()

    if not src.exists():
        print(f"error: replay file not found: {src}")
        return 2

    since_dt: Optional[datetime] = None
    if args.since_minutes and args.since_minutes > 0:
        since_dt = datetime.now(timezone.utc) - timedelta(minutes=args.since_minutes)

    rows, scanned = select_rows(src, max(0, args.tail), since_dt)
    summary = analyze(rows)
    summary.source_path = str(src)
    summary.rows_scanned = scanned

    print("Event Replay Summary")
    print(f"source: {summary.source_path}")
    print(f"rows scanned: {summary.rows_scanned}")
    print(f"rows kept: {summary.rows_kept}")
    print(f"time window: {summary.first_ts or '-'} .. {summary.last_ts or '-'}")
    print(f"pipeline step fail ratio: {summary.pipeline_step_fail_ratio:.4f}")

    print_top(summary.domain_counts, "\nBy domain:", args.top)
    print_top(summary.event_type_counts, "\nBy event type:", args.top)
    print_top(summary.venue_counts, "\nBy venue:", args.top)
    print_top(summary.pipeline_step_status_counts, "\nPipeline step status:", args.top)
    print_top(summary.pipeline_anomaly_severity_counts, "\nPipeline anomaly severity:", args.top)
    print_top(summary.pipeline_anomaly_kind_counts, "\nPipeline anomaly kind:", args.top)

    if args.out:
        out_path = Path(args.out).expanduser()
        if not out_path.is_absolute():
            out_path = (REPO_ROOT / out_path).resolve()
        out_path.parent.mkdir(parents=True, exist_ok=True)
        with out_path.open("w", encoding="utf-8") as f:
            json.dump(asdict(summary), f, ensure_ascii=False, indent=2)
        print(f"\nwritten: {out_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
