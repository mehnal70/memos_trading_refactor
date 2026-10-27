#!/usr/bin/env python3
"""Unified read-only readiness probe for isolated venue runtimes."""

from __future__ import annotations

import json
import os
import socket
import subprocess
import time
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[1]
RUNTIME_DIR = REPO / "logs" / ".venue-runtime" / "state"
READINESS_ARTIFACT = RUNTIME_DIR / "venue-readiness.json"

VENUES: dict[str, dict[str, Any]] = {
    "mt5": {
        "profile": "paper-mt5",
        "heartbeat": REPO / "logs" / "paper-mt5" / "heartbeat.jsonl",
        "snapshot": REPO / "data" / "profiles" / "paper-mt5-snapshot.json",
        "ports": [(("127.0.0.1", 9001), "transport")],
        "account_required": True,
        "execution": "paper",
    },
    "ibkr": {
        "profile": "paper-ibkr",
        "heartbeat": REPO / "logs" / "paper-ibkr" / "heartbeat.jsonl",
        "snapshot": REPO / "data" / "profiles" / "paper-ibkr-snapshot.json",
        "ports": [
            (("127.0.0.1", 4002), "transport"),
            (("127.0.0.1", 9101), "bridge"),
        ],
        "account_required": True,
        "execution": "read_only",
    },
    "binance": {
        "profile": "paper-binance-testnet",
        "heartbeat": REPO / "logs" / "paper-binance-testnet" / "heartbeat.jsonl",
        "snapshot": REPO / "data" / "profiles" / "paper-binance-testnet-snapshot.json",
        "ports": [],
        # Bu profil market-data-only paper-soak kullanır; canlı/testnet
        # account capability'si olmadan da hazır sayılmalıdır.
        "account_required": False,
        "execution": "dry_run",
    },
    "bist": {
        "profile": "paper-bist-signal",
        "heartbeat": REPO / "logs" / "soak" / "bist" / "heartbeat.jsonl",
        "snapshot": REPO / "logs" / "soak" / "bist" / "snapshot.json",
        "ports": [],
        "account_required": False,
        "execution": "signal_only",
    },
}


def _pid_alive(path: Path) -> bool:
    try:
        pid = int(path.read_text(encoding="ascii").strip())
        os.kill(pid, 0)
        return True
    except (OSError, ValueError):
        return False


def _named_process_alive(name: str) -> bool:
    try:
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                if (entry / "comm").read_text(encoding="ascii").strip() == name:
                    return True
            except OSError:
                continue
    except OSError:
        return False
    return False


def _user_service_active(name: str) -> bool:
    try:
        return subprocess.run(
            ["systemctl", "--user", "is-active", "--quiet", name],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode == 0
    except OSError:
        return False


def _venue_service_active(venue: str, kind: str) -> bool:
    return _user_service_active(f"memos-venue-{kind}@{venue}.service")


def _active_profile() -> str:
    try:
        return (REPO / "scripts" / ".active").read_text(encoding="utf-8").strip()
    except OSError:
        return ""


def _profile_value(profile: str, key: str) -> str:
    path = REPO / "profiles" / f"{profile}.conf"
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith(f"{key}="):
                return line.split("=", 1)[1].strip()
    except OSError:
        pass
    return ""


def _launch_value(key: str) -> str:
    path = REPO / "scripts" / ".launch.conf"
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith(f"{key}="):
                return line.split("=", 1)[1].strip()
    except OSError:
        pass
    return ""


def _active_venues() -> set[str]:
    active_profile = _active_profile()
    configured = {
        token.split(":", 1)[0].strip().lower()
        for token in _launch_value("VENUES").split(",")
        if token.strip()
    }
    active = {
        venue
        for venue, config in VENUES.items()
        if config["profile"] == active_profile
    }
    if active_profile == "demo-mt5-execution":
        active.add("mt5")
    active.update(
        venue
        for venue in VENUES
        if _venue_service_active(venue, "engine")
        or _venue_service_active(venue, "watchdog")
    )
    return active | configured.intersection(VENUES)


def _mt5_runtime_config() -> tuple[dict[str, Any], bool]:
    base = VENUES["mt5"]
    profile = _active_profile()
    if profile != "demo-mt5-execution":
        config = dict(base)
        config["account_required"] = False
        return config, False
    config = dict(base)
    config.update(
        {
            "profile": profile,
            "heartbeat": REPO / "logs" / profile / "heartbeat.jsonl",
            "snapshot": REPO / "data" / "profiles" / f"{profile}-snapshot.json",
            "execution": "demo_gated",
        }
    )
    return config, True


def _port_open(host: str, port: int) -> bool:
    try:
        with socket.create_connection((host, port), timeout=0.75):
            return True
    except OSError:
        return False


def _last_json(path: Path) -> dict[str, Any] | None:
    try:
        with path.open("rb") as handle:
            handle.seek(0, os.SEEK_END)
            handle.seek(max(0, handle.tell() - 65536))
            lines = handle.read().decode("utf-8", errors="replace").splitlines()
    except OSError:
        return None
    for line in reversed(lines):
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    return None


def _financial(path: Path, required: bool) -> dict[str, Any]:
    if not required:
        return {
            "status": "NotApplicable",
            "equity": None,
            "balance": None,
            "currency": None,
            "age_seconds": None,
            "ready": True,
        }
    try:
        snapshot = json.loads(path.read_text(encoding="utf-8"))
        finance = snapshot.get("finance") or {}
        status = str(finance.get("status") or "Unavailable")
        account = {}
        venue_accounts = finance.get("venue_accounts") or {}
        if isinstance(venue_accounts, dict):
            fresh_accounts = [
                value for value in venue_accounts.values()
                if isinstance(value, dict)
                and str(value.get("status") or "").lower() in {"fresh", "partial"}
            ]
            if fresh_accounts:
                account = fresh_accounts[0]
                status = str(account.get("status") or status)
        result = {
            "status": status,
            "equity": account.get("equity", finance.get("total_equity")),
            "balance": account.get("balance", finance.get("balance")),
            "currency": account.get("currency", finance.get("currency")),
            "age_seconds": max(0, int(time.time() - path.stat().st_mtime)),
        }
    except (OSError, ValueError, TypeError):
        result = {
            "status": "NotApplicable" if not required else "Unavailable",
            "equity": None,
            "balance": None,
            "currency": None,
            "age_seconds": None,
        }
    result["ready"] = (
        result["status"].lower() in {"fresh", "partial"}
        and result["equity"] is not None
        and bool(result["currency"])
    )
    return result


def _status(
    venue: str,
    overall: str,
    blockers: list[str],
    execution: str,
    market_closed: bool,
) -> dict[str, Any]:
    critical_blockers = {"engine", "watchdog", "transport", "account"}
    critical = bool(critical_blockers.intersection(blockers))
    if market_closed and venue in {"mt5", "ibkr"}:
        severity = "INFO"
        state = "MARKET_CLOSED"
        headline = "Piyasa kapalı · seans dışı"
        action = "Hafta içi seans öncesi runtime, bridge ve hesap senkronunu yeniden doğrulayın."
        critical = False
    elif critical:
        severity = "CRITICAL"
        state = "CRITICAL"
        headline = "Kritik müdahale gerekli"
        action = "İlgili runtime, bağlantı veya hesap katmanını kontrol edip recovery çalıştırın."
    elif blockers:
        severity = "WARNING"
        state = "WARNING"
        headline = "Takip gerekli"
        action = "Eksik veya bayat veri tamamlanana kadar yeni operasyon başlatmayın."
    elif execution == "signal_only":
        severity = "INFO"
        state = "SIGNAL_ONLY"
        headline = "Sinyal üretimi hazır"
        action = "Bu venue yalnızca sinyal üretir; emir ve hesap bağlantısı uygulanamaz."
    else:
        severity = "OK"
        state = "READY"
        headline = "Operasyona hazır"
        action = "Müdahale gerekmiyor."
    return {
        "schema_version": 1,
        "severity": severity,
        "state": state,
        "headline_tr": headline,
        "action_tr": action,
        "critical": critical,
        "stale": "market_data" in blockers,
        "market_closed": market_closed,
        "venue": venue,
        "overall": overall,
    }


def probe(venue: str) -> dict[str, Any]:
    config = VENUES[venue]
    if venue not in _active_venues():
        return {
            "venue": venue,
            "profile": config["profile"],
            "runtime": "NOT_ACTIVE",
            "transport": "NOT_ACTIVE",
            "account": {
                "status": "NotApplicable",
                "equity": None,
                "balance": None,
                "currency": None,
                "age_seconds": None,
                "ready": True,
            },
            "market_data": "NOT_ACTIVE",
            "market_session": {"open": None, "reason": "venue_not_active"},
            "execution": config["execution"],
            "overall": "NOT_ACTIVE",
            "blockers": [],
            "engine": {"up": False},
            "watchdog": {"up": False},
            "heartbeat": {"age_seconds": None, "fresh": False, "phase": None},
            "ports": {},
            "status": {
                "schema_version": 1,
                "severity": "INFO",
                "state": "NOT_ACTIVE",
                "headline_tr": "Venue etkin değil",
                "action_tr": "Bu venue aktif profile dahil değil; readiness kontrolü gerekmiyor.",
                "critical": False,
                "stale": False,
                "market_closed": False,
                "venue": venue,
                "overall": "NOT_ACTIVE",
            },
        }
    headless_mt5 = False
    if venue == "mt5":
        config, headless_mt5 = _mt5_runtime_config()
    headless_active = (
        headless_mt5
        or _active_profile() == config["profile"]
        or _venue_service_active(venue, "engine")
        or _venue_service_active(venue, "watchdog")
    )
    market_closed = venue in {"mt5", "ibkr"} and time.localtime().tm_wday >= 5
    venue_engine_active = _venue_service_active(venue, "engine") if headless_active else False
    venue_watchdog_active = _venue_service_active(venue, "watchdog") if headless_active else False
    if venue_engine_active or venue_watchdog_active:
        engine_up = venue_engine_active and _named_process_alive("rtc_headless")
        watchdog_up = venue_watchdog_active
    elif headless_active:
        engine_up = _named_process_alive("rtc_headless")
        watchdog_up = _user_service_active("memos-watchdog.service")
    else:
        engine_up = _pid_alive(RUNTIME_DIR / f"{venue}.engine.pid")
        watchdog_up = _pid_alive(RUNTIME_DIR / f"{venue}.watchdog.pid")
    ports = {
        label: {
            "host": endpoint[0],
            "port": endpoint[1],
            "open": _port_open(*endpoint),
        }
        for endpoint, label in config["ports"]
    }
    heartbeat = _last_json(config["heartbeat"])
    heartbeat_phase = str((heartbeat or {}).get("phase") or "")
    try:
        heartbeat_age = max(0, int(time.time() - config["heartbeat"].stat().st_mtime))
    except OSError:
        heartbeat_age = None
    heartbeat_ready = heartbeat_age is not None and heartbeat_age <= 180
    transport_ready = all(item["open"] for item in ports.values())
    financial = _financial(config["snapshot"], config["account_required"])
    blockers = []
    if not engine_up:
        blockers.append("engine")
    if not watchdog_up:
        blockers.append("watchdog")
    if not transport_ready:
        blockers.append("transport")
    if not heartbeat_ready and not market_closed:
        blockers.append("market_data")
    if heartbeat_phase == "LockContended" and not market_closed:
        blockers.append("engine")
    elif heartbeat_phase == "AccountError" and not market_closed:
        blockers.append("account")
    venue_runtime = (heartbeat or {}).get("venue_runtime") or {}
    runtime_states = [
        runtime
        for runtime in venue_runtime.values()
        if isinstance(runtime, dict)
    ] if isinstance(venue_runtime, dict) else []
    scheduler_ready = bool(runtime_states) and all(
        str(runtime.get("scheduler_phase") or "") in {"AccountReady", "Ready", "Scanning", "Running"}
        and int(runtime.get("consecutive_failures") or 0) == 0
        and not runtime.get("last_error")
        for runtime in runtime_states
    )
    if heartbeat_phase == "Recovering" and not market_closed and not scheduler_ready:
        blockers.append("recovery")
    if (
        isinstance(venue_runtime, dict)
        and any(
            isinstance(runtime, dict)
            and str(runtime.get("scheduler_phase") or "") == "NotStarted"
            for runtime in venue_runtime.values()
        )
        and not market_closed
    ):
        blockers.append("recovery")
    if not financial["ready"] and not market_closed:
        blockers.append("account")
    overall = "READY" if not blockers else "DEGRADED"
    if market_closed:
        overall = "MARKET_CLOSED"
    if venue == "bist":
        blockers = [item for item in blockers if item != "account"]
        overall = "SIGNAL_READY" if not blockers else "DEGRADED"
    result = {
        "venue": venue,
        "profile": config["profile"],
        "runtime": "UP" if engine_up and watchdog_up else "DOWN",
        "transport": "READY" if transport_ready else "DOWN",
        "account": financial,
        "market_data": "CLOSED" if market_closed else "READY" if heartbeat_ready else "STALE",
        "market_session": {
            "open": not market_closed,
            "reason": "weekend" if market_closed else None,
        },
        "execution": config["execution"],
        "overall": overall,
        "blockers": blockers,
        "engine": {"up": engine_up},
        "watchdog": {"up": watchdog_up},
        "heartbeat": {
            "age_seconds": heartbeat_age,
            "fresh": heartbeat_ready,
            "phase": heartbeat_phase or None,
        },
        "ports": ports,
    }
    result["status"] = _status(venue, overall, blockers, config["execution"], market_closed)
    return result


def main() -> int:
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("venue", nargs="?", choices=["all", *sorted(VENUES)], default="all")
    args = parser.parse_args()
    venues = VENUES if args.venue == "all" else {args.venue: VENUES[args.venue]}
    result = {name: probe(name) for name in venues}
    RUNTIME_DIR.mkdir(parents=True, exist_ok=True)
    temporary = READINESS_ARTIFACT.with_suffix(".tmp")
    temporary.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "generated_at": time.time(),
                "venues": result,
            },
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
        ),
        encoding="utf-8",
    )
    temporary.replace(READINESS_ARTIFACT)
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if all(
        item["overall"] in {"READY", "SIGNAL_READY", "MARKET_CLOSED", "NOT_ACTIVE"}
        for item in result.values()
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
