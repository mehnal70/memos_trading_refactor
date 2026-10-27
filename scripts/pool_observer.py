#!/usr/bin/env python3
"""paper-blend/MT5 gozlem raporu.

Tek komutta su sorulari cevaplar:
- MT5 veri yolu saglikli mi? (snapshot loglarindan)
- Islem yapilabilirlik sinyali var mi? (heartbeat + son trade)
- Strateji/havuz secim zincirinde neler oldu? (snapshot timeline)

Opsiyonel canli izleme ile degisimleri zaman icinde yazdirir.
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple

REPO_ROOT = Path(__file__).resolve().parent.parent
ACTIVE_PROFILE_FILE = REPO_ROOT / "scripts" / ".active"
LAUNCH_CONF = REPO_ROOT / "scripts" / ".launch.conf"
MT5_SCRIPT = REPO_ROOT / "scripts" / "mt5" / "setup_and_run.sh"


@dataclass
class RuntimePaths:
    profile: str
    snapshot: Path
    heartbeat: Path
    trades: Path
    db_path: Path


@dataclass
class SnapshotView:
    phase: str
    logs: List[Dict[str, Any]]
    fleet: List[Dict[str, Any]]
    pipeline_steps: List[Dict[str, Any]]
    opportunities: List[Dict[str, Any]]
    positions: List[Dict[str, Any]]


WATCH_PATTERNS = (
    "MT5 veri ozeti",
    "MT5 veri ozeti",
    "AUX MT5",
    "Backtest",
    "Walk-Forward",
    "WF",
    "aday taramasi",
    "symbol_strategy",
    "edge seed",
    "Scheduler",
    "Tetik",
    "Carry Mod",
    "Blend Mod",
    "Supertrend",
    "XS",
)


def read_active_profile() -> str:
    if ACTIVE_PROFILE_FILE.exists():
        v = ACTIVE_PROFILE_FILE.read_text(encoding="utf-8").strip()
        if v:
            return v
    return "paper-blend"


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


def resolve_runtime_paths(profile: str) -> RuntimePaths:
    cfg = parse_kv_file(LAUNCH_CONF)
    snap = cfg.get("SNAPSHOT_PATH", f"data/profiles/{profile}-snapshot.json")
    hb = cfg.get("HEARTBEAT_PATH", f"logs/{profile}/heartbeat.jsonl")
    tr = cfg.get("TRADES_LOG_PATH", f"logs/{profile}/trades.jsonl")
    db = cfg.get("DB_PATH", "data/trader.db")
    return RuntimePaths(
        profile=profile,
        snapshot=REPO_ROOT / snap,
        heartbeat=REPO_ROOT / hb,
        trades=REPO_ROOT / tr,
        db_path=REPO_ROOT / db,
    )


def load_snapshot(path: Path) -> SnapshotView:
    if not path.exists():
        return SnapshotView("unknown", [], [], [], [], [])
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return SnapshotView("unknown", [], [], [], [], [])

    def as_list(name: str) -> List[Dict[str, Any]]:
        v = doc.get(name, [])
        if not isinstance(v, list):
            return []
        return [x for x in v if isinstance(x, dict)]

    phase = str(doc.get("phase", "unknown"))
    return SnapshotView(
        phase=phase,
        logs=as_list("logs"),
        fleet=as_list("fleet"),
        pipeline_steps=as_list("pipeline_steps"),
        opportunities=as_list("opportunity_snapshots"),
        positions=as_list("positions"),
    )


def read_last_jsonl(path: Path) -> Optional[Dict[str, Any]]:
    if not path.exists() or path.stat().st_size == 0:
        return None
    last = ""
    with path.open("r", encoding="utf-8", errors="replace") as f:
        for line in f:
            if line.strip():
                last = line.strip()
    if not last:
        return None
    try:
        parsed = json.loads(last)
    except Exception:
        return None
    return parsed if isinstance(parsed, dict) else None


def read_tail_jsonl(path: Path, max_lines: int) -> List[Dict[str, Any]]:
    if not path.exists() or path.stat().st_size == 0:
        return []
    rows: List[Dict[str, Any]] = []
    with path.open("r", encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except Exception:
                continue
            if isinstance(obj, dict):
                rows.append(obj)
    if max_lines <= 0:
        return rows
    return rows[-max_lines:]


def find_latest_log(logs: Iterable[Dict[str, Any]], token: str) -> Optional[Dict[str, Any]]:
    token_lower = token.lower()
    for item in logs:
        msg = str(item.get("message", ""))
        if token_lower in msg.lower():
            return item
    return None


def extract_timeline(logs: Iterable[Dict[str, Any]], limit: int) -> List[Dict[str, Any]]:
    out: List[Dict[str, Any]] = []
    for row in logs:
        msg = str(row.get("message", ""))
        if any(tok.lower() in msg.lower() for tok in WATCH_PATTERNS):
            out.append(row)
    return out[:limit]


def parse_hb_time(hb: Dict[str, Any]) -> Optional[datetime]:
    raw = str(hb.get("timestamp", "")).strip()
    if not raw:
        return None
    try:
        # fromisoformat accepts +00:00, but not Z in old versions.
        if raw.endswith("Z"):
            raw = raw[:-1] + "+00:00"
        # Some logs keep nanosecond precision; trim to microseconds for stdlib parser.
        if "." in raw:
            head, tail = raw.split(".", 1)
            tz_pos = max(tail.rfind("+"), tail.rfind("-"))
            if tz_pos > 0:
                frac = tail[:tz_pos]
                tz = tail[tz_pos:]
            else:
                frac = tail
                tz = ""
            frac = (frac + "000000")[:6]
            raw = f"{head}.{frac}{tz}"
        return datetime.fromisoformat(raw)
    except Exception:
        return None


def freshness_label(hb: Optional[Dict[str, Any]], max_age_secs: int = 180) -> str:
    if not hb:
        return "BILINMIYOR"
    dt = parse_hb_time(hb)
    if not dt:
        return "BILINMIYOR"
    now = datetime.now(timezone.utc)
    age = (now - dt.astimezone(timezone.utc)).total_seconds()
    if age <= max_age_secs:
        return f"TAZE ({int(age)}s)"
    return f"BAYAT ({int(age)}s)"


def db_symbol_status_summary(db_path: Path) -> List[Tuple[str, int]]:
    if not db_path.exists():
        return []
    try:
        conn = sqlite3.connect(str(db_path))
        try:
            cur = conn.cursor()
            cur.execute("SELECT status, COUNT(*) FROM symbol_status GROUP BY status ORDER BY COUNT(*) DESC")
            rows = cur.fetchall()
            return [(str(s), int(c)) for s, c in rows]
        finally:
            conn.close()
    except Exception:
        return []


def short(s: Any, max_len: int = 180) -> str:
    txt = str(s)
    if len(txt) <= max_len:
        return txt
    return txt[: max_len - 3] + "..."


def run_probe(mode: str) -> Tuple[int, str]:
    if not MT5_SCRIPT.exists():
        return (127, "setup_and_run.sh bulunamadi")
    cmd = [str(MT5_SCRIPT), mode]
    try:
        cp = subprocess.run(
            cmd,
            cwd=str(REPO_ROOT),
            capture_output=True,
            text=True,
            timeout=120,
            check=False,
        )
    except Exception as exc:
        return (1, f"probe calismadi: {exc}")
    merged = (cp.stdout or "") + ("\n" + cp.stderr if cp.stderr else "")
    lines = [ln for ln in merged.splitlines() if ln.strip()]
    tail = "\n".join(lines[-8:])
    return (cp.returncode, tail)


def print_header(title: str) -> None:
    print()
    print(f"=== {title} ===")


def render_report(paths: RuntimePaths, timeline_limit: int, run_live_probe: bool) -> Dict[str, Any]:
    snap = load_snapshot(paths.snapshot)
    hb = read_last_jsonl(paths.heartbeat)
    trades = read_tail_jsonl(paths.trades, max_lines=5)
    timeline = extract_timeline(snap.logs, timeline_limit)

    state: Dict[str, Any] = {
        "phase": snap.phase,
        "strategy": hb.get("strategy") if hb else None,
        "open_positions": hb.get("open_positions") if hb else None,
        "closed_trades": hb.get("closed_trades") if hb else None,
        "anomalies": hb.get("anomalies") if hb else None,
        "last_trade": trades[-1] if trades else None,
        "timeline_keys": {(str(x.get("timestamp", "")), str(x.get("message", ""))) for x in timeline},
    }

    print(f"profil={paths.profile}")
    print(f"snapshot={paths.snapshot}")
    print(f"heartbeat={paths.heartbeat}")
    print(f"trades={paths.trades}")

    print_header("MT5 Veri/Islem Durumu")
    mt5_open = find_latest_log(snap.logs, "Pazar saatleri [mt5")
    mt5_fresh = find_latest_log(snap.logs, "MT5 veri ozeti") or find_latest_log(snap.logs, "MT5 veri özeti")
    aux_mt5 = find_latest_log(snap.logs, "AUX MT5")
    if mt5_open:
        print(f"seans: {short(mt5_open.get('message', ''))}")
    else:
        print("seans: snapshot logunda bulunamadi")
    if mt5_fresh:
        print(f"veri:  {short(mt5_fresh.get('message', ''))}")
    else:
        print("veri:  MT5 veri ozeti bulunamadi")
    if aux_mt5:
        print(f"aux:   {short(aux_mt5.get('message', ''))}")

    if run_live_probe:
        code_d, out_d = run_probe("probe")
        code_t, out_t = run_probe("probe-trade")
        print(f"probe(data): {'OK' if code_d == 0 else 'FAIL'} (exit={code_d})")
        if out_d:
            print(short(out_d, 420))
        print(f"probe(trade): {'OK' if code_t == 0 else 'FAIL'} (exit={code_t})")
        if out_t:
            print(short(out_t, 420))

    print_header("Islem Yapilabilirlik")
    if hb:
        print(
            "heartbeat: "
            f"tick={hb.get('tick')} phase={hb.get('phase')} strategy={hb.get('strategy')} "
            f"open={hb.get('open_positions')} closed={hb.get('closed_trades')} "
            f"anom={hb.get('anomalies')} gbt={hb.get('gbt_ready')} freshness={freshness_label(hb)}"
        )
    else:
        print("heartbeat: yok")

    if trades:
        last = trades[-1]
        print(
            "son_trade: "
            f"{last.get('event_type')} {last.get('symbol')} sig={last.get('signal')} "
            f"strat={last.get('strategy', '')} px={last.get('price')}"
        )
    else:
        print("son_trade: yok")

    print_header("Havuz/Strateji Katilim Durumu")
    if snap.fleet:
        top = sorted(snap.fleet, key=lambda x: float(x.get("score", 0.0) or 0.0), reverse=True)[:6]
        as_text = ", ".join(f"{i.get('symbol')}@{i.get('market')} score={float(i.get('score', 0.0) or 0.0):.2f}" for i in top)
        print(f"fleet_top: {as_text}")
    else:
        print("fleet_top: yok")

    if snap.opportunities:
        top_opp = sorted(
            snap.opportunities,
            key=lambda x: float(x.get("opportunity_score", 0.0) or 0.0),
            reverse=True,
        )[:6]
        text = ", ".join(
            f"{o.get('symbol')} score={float(o.get('opportunity_score', 0.0) or 0.0):.2f} bias={o.get('trend_bias', '-') }"
            for o in top_opp
        )
        print(f"opp_top:   {text}")
    else:
        print("opp_top:   yok")

    if snap.pipeline_steps:
        parts = []
        for st in snap.pipeline_steps[:10]:
            parts.append(
                f"{st.get('label')}={st.get('status')}"
                + (f" age={st.get('last_run_age_secs')}s" if st.get("last_run_age_secs") is not None else "")
            )
        print("pipeline:  " + " | ".join(parts))
    else:
        print("pipeline:  yok")

    if snap.positions:
        pos_text = ", ".join(
            f"{p.get('symbol')} qty={p.get('qty')} lev={p.get('leverage')} mkt={p.get('market')}"
            for p in snap.positions[:6]
        )
        print(f"positions: {pos_text}")
    else:
        print("positions: yok")

    db_summary = db_symbol_status_summary(paths.db_path)
    if db_summary:
        text = ", ".join(f"{s}:{c}" for s, c in db_summary)
        print(f"symbol_status: {text}")

    print_header("Zaman Cizgisi (Son Olaylar)")
    if timeline:
        for row in timeline:
            print(f"{row.get('timestamp', '?')} [{row.get('level', '?')}] {short(row.get('message', ''))}")
    else:
        print("Filtreye uyan olay bulunamadi.")

    return state


def watch_loop(paths: RuntimePaths, args: argparse.Namespace) -> None:
    print(f"canli izleme basladi: interval={args.interval}s, timeline_limit={args.timeline_limit}")
    print("cikis: Ctrl+C")
    prev: Optional[Dict[str, Any]] = None
    while True:
        current = render_report(paths, args.timeline_limit, run_live_probe=args.probe)
        if prev is not None:
            changed: List[str] = []
            for key in ("phase", "strategy", "open_positions", "closed_trades", "anomalies"):
                if prev.get(key) != current.get(key):
                    changed.append(f"{key}:{prev.get(key)}->{current.get(key)}")
            if changed:
                print()
                print("degisim: " + ", ".join(changed))

            new_timeline = current.get("timeline_keys", set()) - prev.get("timeline_keys", set())
            if new_timeline:
                print(f"yeni_timeline_olayi={len(new_timeline)}")

        prev = current
        print()
        print("-" * 72)
        time.sleep(max(2, int(args.interval)))


def main() -> int:
    parser = argparse.ArgumentParser(description="MT5 + strategy pool gozlem raporu")
    parser.add_argument("--profile", default=None, help="profil adi (varsayilan: scripts/.active)")
    parser.add_argument("--timeline-limit", type=int, default=14, help="zaman cizgisi olay limiti")
    parser.add_argument("--probe", action="store_true", help="MT5 probe + probe-trade komutlarini calistir")
    parser.add_argument("--watch", action="store_true", help="canli izleme dongusu")
    parser.add_argument("--interval", type=int, default=60, help="watch yenileme saniyesi")
    args = parser.parse_args()

    profile = args.profile or read_active_profile()
    paths = resolve_runtime_paths(profile)

    if args.watch:
        try:
            watch_loop(paths, args)
        except KeyboardInterrupt:
            print("\nizleme sonlandirildi")
            return 0
        return 0

    render_report(paths, args.timeline_limit, run_live_probe=args.probe)
    return 0


if __name__ == "__main__":
    sys.exit(main())
