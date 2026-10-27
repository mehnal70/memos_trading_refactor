#!/usr/bin/env python3
"""Symbolic gating advisor.

Creates a baseline from current snapshot counters, then evaluates deltas after an
observation window and writes recommendation reports without mutating config.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, Tuple


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def read_json(path: Path) -> Dict:
    with path.open("r", encoding="utf-8") as f:
        data = json.load(f)
    if not isinstance(data, dict):
        raise ValueError(f"invalid json root in {path}")
    return data


def parse_kv_file(path: Path) -> Dict[str, str]:
    out: Dict[str, str] = {}
    if not path.exists():
        return out
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        out[k.strip()] = v.strip()
    return out


def env_bool(v: str, default: bool = False) -> bool:
    if v is None:
        return default
    return v.strip().lower() in {"1", "true", "yes", "on"}


def env_float(v: str, default: float) -> float:
    try:
        return float(v)
    except Exception:
        return default


def active_profile(repo: Path) -> str:
    p = repo / "scripts" / ".active"
    if not p.exists():
        return "unknown"
    return p.read_text(encoding="utf-8").strip() or "unknown"


def snapshot_path(repo: Path) -> Path:
    launch = parse_kv_file(repo / "scripts" / ".launch.conf")
    raw = launch.get("SNAPSHOT_PATH", "data/snapshot.json")
    return (repo / raw).resolve()


@dataclass
class Delta:
    scan: int
    detected: int
    aligned_buy: int
    aligned_sell: int
    conflict: int
    would_block: int
    gated: int
    out_of_scope: int
    adaptive_applied: int
    adaptive_tightened: int
    adaptive_relaxed: int

    @property
    def aligned(self) -> int:
        return self.aligned_buy + self.aligned_sell

    @property
    def directional(self) -> int:
        return self.aligned + self.conflict


def get_counter(c: Dict[str, int], key: str) -> int:
    v = c.get(key, 0)
    try:
        return int(v)
    except Exception:
        return 0


def build_delta(now_c: Dict[str, int], base_c: Dict[str, int]) -> Delta:
    def d(key: str) -> int:
        return max(0, get_counter(now_c, key) - get_counter(base_c, key))

    return Delta(
        scan=d("symbolic.scan"),
        detected=d("symbolic.detected"),
        aligned_buy=d("symbolic.signal.aligned_buy"),
        aligned_sell=d("symbolic.signal.aligned_sell"),
        conflict=d("symbolic.signal.conflict"),
        would_block=d("symbolic.compare.would_block"),
        gated=d("symbolic.gated"),
        out_of_scope=d("symbolic.compare.out_of_scope"),
        adaptive_applied=d("symbolic.adaptive.applied"),
        adaptive_tightened=d("symbolic.adaptive.tightened"),
        adaptive_relaxed=d("symbolic.adaptive.relaxed"),
    )


def pct(n: float, den: float) -> float:
    if den <= 0:
        return 0.0
    return (n / den) * 100.0


def clamp(x: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, x))


def recommend(launch: Dict[str, str], d: Delta) -> Tuple[Dict, str]:
    base_min_conf = env_float(launch.get("SYMBOLIC_GATING_MIN_CONF"), 0.75)
    adaptive_on = env_bool(launch.get("SYMBOLIC_GATING_ADAPTIVE", "0"), False)
    min_floor = env_float(launch.get("SYMBOLIC_GATING_ADAPTIVE_MIN_CONF"), 0.65)
    max_ceil = env_float(launch.get("SYMBOLIC_GATING_ADAPTIVE_MAX_CONF"), 0.90)
    target_alignment = env_float(launch.get("SYMBOLIC_GATING_TARGET_ALIGNMENT_PCT"), 55.0)

    directional = float(d.directional)
    alignment_pct = pct(d.aligned, directional)
    conflict_pct = pct(d.conflict, directional)
    would_block_pct = pct(d.would_block, directional)
    gated_pct = pct(d.gated, directional)
    out_scope_pct = pct(d.out_of_scope, directional)

    rec_min_conf = base_min_conf
    reasons = []

    if directional < 20:
        reasons.append("insufficient_directional_events")
    else:
        if conflict_pct > 52.0 or gated_pct > 32.0:
            rec_min_conf += 0.03
            reasons.append("high_conflict_or_gate_pressure")
        elif alignment_pct > max(target_alignment + 12.0, 70.0) and gated_pct < 12.0:
            rec_min_conf -= 0.02
            reasons.append("high_alignment_low_gate_pressure")

        if out_scope_pct > 25.0:
            reasons.append("scope_may_be_too_narrow")

    lo = min(min_floor, max_ceil)
    hi = max(min_floor, max_ceil)
    rec_min_conf = clamp(rec_min_conf, lo, hi)

    scope_hint = "keep"
    if directional >= 20:
        if out_scope_pct > 35.0 and conflict_pct < 45.0:
            scope_hint = "expand_one_step"
        elif gated_pct > 40.0:
            scope_hint = "hold_or_narrow"

    adaptive_hint = "keep"
    if not adaptive_on and directional >= 30 and abs(alignment_pct - target_alignment) > 10.0:
        adaptive_hint = "consider_enable"

    summary = {
        "window": {
            "directional_events": d.directional,
            "scan": d.scan,
            "detected": d.detected,
        },
        "metrics": {
            "alignment_pct": round(alignment_pct, 2),
            "conflict_pct": round(conflict_pct, 2),
            "would_block_pct": round(would_block_pct, 2),
            "gated_pct": round(gated_pct, 2),
            "out_of_scope_pct": round(out_scope_pct, 2),
        },
        "current": {
            "symbolic_gating_min_conf": round(base_min_conf, 4),
            "symbolic_gating_adaptive": adaptive_on,
            "symbolic_gating_adaptive_min_conf": round(min_floor, 4),
            "symbolic_gating_adaptive_max_conf": round(max_ceil, 4),
            "symbolic_gating_target_alignment_pct": round(target_alignment, 2),
        },
        "recommendation": {
            "symbolic_gating_min_conf": round(rec_min_conf, 4),
            "scope_hint": scope_hint,
            "adaptive_hint": adaptive_hint,
            "reasons": reasons,
        },
    }

    short = (
        f"directional={d.directional}, align={alignment_pct:.1f}%, conflict={conflict_pct:.1f}%, "
        f"gated={gated_pct:.1f}% -> min_conf {base_min_conf:.2f} -> {rec_min_conf:.2f}"
    )
    return summary, short


def baseline_mode(repo: Path, snap_file: Path, state_file: Path) -> int:
    snap = read_json(snap_file)
    counters = snap.get("autonomy_counters", {})
    if not isinstance(counters, dict):
        counters = {}

    payload = {
        "created_at": now_iso(),
        "profile": active_profile(repo),
        "snapshot_path": str(snap_file),
        "autonomy_counters": counters,
    }
    state_file.parent.mkdir(parents=True, exist_ok=True)
    state_file.write_text(json.dumps(payload, ensure_ascii=True, indent=2), encoding="utf-8")
    print(f"baseline_saved={state_file}")
    return 0


def report_mode(repo: Path, snap_file: Path, state_file: Path, out_dir: Path) -> int:
    if not state_file.exists():
        print(f"baseline_missing={state_file}")
        print("run: python3 scripts/symbolic_gate_advisor.py start")
        return 2

    baseline = read_json(state_file)
    b_counters = baseline.get("autonomy_counters", {})
    if not isinstance(b_counters, dict):
        b_counters = {}

    snap = read_json(snap_file)
    now_counters = snap.get("autonomy_counters", {})
    if not isinstance(now_counters, dict):
        now_counters = {}

    d = build_delta(now_counters, b_counters)
    launch = parse_kv_file(repo / "scripts" / ".launch.conf")
    rec, short = recommend(launch, d)

    ts = datetime.now().strftime("%Y%m%d_%H%M%S")
    prof = active_profile(repo)
    out_dir.mkdir(parents=True, exist_ok=True)
    out_json = out_dir / f"symbolic_gating_advice_{prof}_{ts}.json"
    out_txt = out_dir / f"symbolic_gating_advice_{prof}_{ts}.txt"

    payload = {
        "generated_at": now_iso(),
        "profile": prof,
        "snapshot_path": str(snap_file),
        "baseline": {
            "file": str(state_file),
            "created_at": baseline.get("created_at", ""),
        },
        "delta": {
            "scan": d.scan,
            "detected": d.detected,
            "aligned_buy": d.aligned_buy,
            "aligned_sell": d.aligned_sell,
            "conflict": d.conflict,
            "would_block": d.would_block,
            "gated": d.gated,
            "out_of_scope": d.out_of_scope,
            "adaptive_applied": d.adaptive_applied,
            "adaptive_tightened": d.adaptive_tightened,
            "adaptive_relaxed": d.adaptive_relaxed,
        },
        "analysis": rec,
    }

    out_json.write_text(json.dumps(payload, ensure_ascii=True, indent=2), encoding="utf-8")

    lines = [
        f"# Symbolic Gating Advice ({ts})",
        "",
        f"profile={prof}",
        f"snapshot={snap_file}",
        f"baseline={state_file}",
        "",
        "## Summary",
        short,
        "",
        "## Recommendation",
        json.dumps(rec["recommendation"], ensure_ascii=True, indent=2),
        "",
        "## Metrics",
        json.dumps(rec["metrics"], ensure_ascii=True, indent=2),
    ]
    out_txt.write_text("\n".join(lines) + "\n", encoding="utf-8")

    print(f"report_json={out_json}")
    print(f"report_txt={out_txt}")
    print(short)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Symbolic gating advisor")
    parser.add_argument("mode", choices=["start", "report"], help="start baseline or produce advice report")
    parser.add_argument("--repo", default=".", help="repo root")
    parser.add_argument("--snapshot", default="", help="override snapshot path")
    parser.add_argument("--state", default="", help="override baseline state file")
    parser.add_argument("--out-dir", default="reports", help="report output directory")
    args = parser.parse_args()

    repo = Path(args.repo).resolve()
    snap_file = Path(args.snapshot).resolve() if args.snapshot else snapshot_path(repo)
    prof = active_profile(repo)
    default_state = repo / "reports" / f"symbolic_gating_baseline_{prof}.json"
    state_file = Path(args.state).resolve() if args.state else default_state
    out_dir = Path(args.out_dir).resolve() if Path(args.out_dir).is_absolute() else (repo / args.out_dir)

    if not snap_file.exists():
        print(f"snapshot_missing={snap_file}")
        return 2

    if args.mode == "start":
        return baseline_mode(repo, snap_file, state_file)
    return report_mode(repo, snap_file, state_file, out_dir)


if __name__ == "__main__":
    raise SystemExit(main())
