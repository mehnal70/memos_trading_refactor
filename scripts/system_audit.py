#!/usr/bin/env python3
"""Read-only multi-venue operational audit and action report."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_DIR = Path(__file__).resolve().parents[1]


def read_json(path: Path) -> tuple[dict[str, Any] | None, str | None]:
    try:
        with path.open(encoding="utf-8") as handle:
            value = json.load(handle)
        if not isinstance(value, dict):
            return None, "JSON object bekleniyordu"
        return value, None
    except FileNotFoundError:
        return None, "dosya yok"
    except (OSError, json.JSONDecodeError) as exc:
        return None, str(exc)


def load_config_values() -> dict[str, str]:
    values: dict[str, str] = {}
    for path in (REPO_DIR / ".env", REPO_DIR / "scripts/.launch.conf"):
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except OSError:
            continue
        for line in lines:
            if not line or line.lstrip().startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            if key and key.replace("_", "").isalnum():
                values[key] = value.strip().strip('"').strip("'")
    return values


def run_readiness() -> tuple[dict[str, Any], str | None]:
    command = [sys.executable, str(REPO_DIR / "scripts/venue_readiness.py"), "all"]
    try:
        result = subprocess.run(command, cwd=REPO_DIR, capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {}, f"readiness çalıştırılamadı: {exc}"
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        return {}, f"readiness JSON okunamadı: {exc}"
    if not isinstance(value, dict):
        return {}, "readiness JSON object değil"
    return value, None


def active_venues() -> set[str]:
    values: set[str] = set()
    conf = REPO_DIR / "scripts/.launch.conf"
    try:
        for line in conf.read_text(encoding="utf-8").splitlines():
            if line.startswith("VENUES="):
                values = {
                    item.split(":", 1)[0].strip()
                    for item in line.split("=", 1)[1].split(",")
                    if ":" in item
                }
                break
    except OSError:
        pass
    return values


def service_state(venue: str) -> str:
    try:
        result = subprocess.run(
            ["systemctl", "--user", "is-active", f"memos-venue-engine@{venue}.service"],
            cwd=REPO_DIR,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return f"unknown ({exc})"
    return result.stdout.strip() or "inactive"


def add_issue(
    issues: list[dict[str, str]],
    severity: str,
    area: str,
    finding: str,
    action: str,
) -> None:
    issues.append(
        {
            "severity": severity,
            "area": area,
            "finding": finding,
            "action": action,
        }
    )


def audit_once() -> tuple[dict[str, Any], int]:
    issues: list[dict[str, str]] = []
    readiness, readiness_error = run_readiness()
    configured = active_venues()
    venue_report: dict[str, Any] = {}

    if readiness_error:
        add_issue(issues, "CRITICAL", "readiness", readiness_error, "venue_readiness.py çıktısını kontrol edin")
    runtime_venues = {
        venue
        for venue, item in readiness.items()
        if isinstance(item, dict) and item.get("overall") != "NOT_ACTIVE"
    }
    for venue in sorted(configured | runtime_venues):
        item = readiness.get(venue)
        state = service_state(venue)
        venue_report[venue] = {
            "overall": item.get("overall", "UNKNOWN") if item else "MISSING",
            "runtime": item.get("runtime", "UNKNOWN") if item else "UNKNOWN",
            "engine": item.get("engine", {}).get("up", False) if item else False,
            "watchdog": item.get("watchdog", {}).get("up", False) if item else False,
            "heartbeat": item.get("heartbeat", {}) if item else {},
            "account": item.get("account", {}) if item else {},
            "service": state,
        }
        if item is None:
            add_issue(issues, "CRITICAL", venue, "readiness kaydı yok", "venue_readiness.py çalıştırın")
            continue
        if item.get("overall") not in {"READY", "SIGNAL_READY", "MARKET_CLOSED"}:
            blockers = ",".join(item.get("blockers", [])) or "belirsiz"
            add_issue(issues, "CRITICAL", venue, f"readiness={item.get('overall')} blockers={blockers}", f"{venue} loglarını ve recovery durumunu kontrol edin")
        if state != "active":
            add_issue(issues, "CRITICAL", venue, f"engine service={state}", f"systemctl --user status memos-venue-engine@{venue}.service")

    refresh, refresh_error = read_json(REPO_DIR / "data/bist_refresh_status.json")
    trend, trend_error = read_json(REPO_DIR / "data/bist_trend_status.json")
    paper, paper_error = read_json(REPO_DIR / "data/bist_paper_state.json")
    bist_report = {
        "refresh": refresh or {},
        "trend": trend or {},
        "paper": paper or {},
    }
    for name, error, action in (
        ("refresh", refresh_error, "BIST refresh worker durumunu kontrol edin"),
        ("trend", trend_error, "BIST trend pipeline loglarını kontrol edin"),
        ("paper", paper_error, "BIST paper simulation state dosyasını kontrol edin"),
    ):
        if error:
            add_issue(issues, "CRITICAL", f"bist.{name}", error, action)
    if refresh and refresh.get("status") not in {"READY", "COMPLETED"}:
        add_issue(issues, "CRITICAL", "bist.refresh", f"status={refresh.get('status')}", "Google Sheets refresh worker'ı kontrol edin")
    if trend and trend.get("status") != "HEALTHY":
        add_issue(issues, "CRITICAL", "bist.trend", f"status={trend.get('status')}", "BIST trend scan loglarını kontrol edin")
    if trend and trend.get("data_freshness") not in {"FRESH", "MARKET_CLOSED"}:
        add_issue(issues, "WARN", "bist.freshness", f"freshness={trend.get('data_freshness')}", "Yeni günlük BIST batch'ini bekleyin veya refresh'i inceleyin")

    execution = {}
    launch_conf = REPO_DIR / "scripts/.launch.conf"
    try:
        for line in launch_conf.read_text(encoding="utf-8").splitlines():
            if "=" in line and line.split("=", 1)[0] in {
                "LIVE_EXECUTION_ARMED",
                "LIVE_GO_NO_GO",
                "LIVE_DRY_RUN",
                "IBKR_READ_ONLY",
                "IBKR_EXECUTION_ENABLED",
            }:
                key, value = line.split("=", 1)
                execution[key] = value
    except OSError as exc:
        add_issue(issues, "CRITICAL", "execution_gate", str(exc), "scripts/.launch.conf dosyasını kontrol edin")
    if execution.get("LIVE_EXECUTION_ARMED") != "0" or execution.get("LIVE_GO_NO_GO") != "NO_GO":
        add_issue(issues, "CRITICAL", "execution_gate", f"gate={execution}", "Gerçek execution kapılarını kapatın")
    ibkr_conf = REPO_DIR / "logs/.venue-runtime/profiles/ibkr.conf"
    if not ibkr_conf.exists():
        ibkr_conf = launch_conf
    ibkr_execution = {}
    try:
        for line in ibkr_conf.read_text(encoding="utf-8").splitlines():
            if "=" in line and line.split("=", 1)[0] in {"IBKR_READ_ONLY", "IBKR_EXECUTION_ENABLED"}:
                key, value = line.split("=", 1)
                ibkr_execution[key] = value
    except OSError as exc:
        add_issue(issues, "CRITICAL", "ibkr.execution", str(exc), "IBKR runtime profilini kontrol edin")
    if "ibkr" in runtime_venues and (
        ibkr_execution.get("IBKR_READ_ONLY") != "1"
        or ibkr_execution.get("IBKR_EXECUTION_ENABLED") != "0"
    ):
        add_issue(issues, "CRITICAL", "ibkr.execution", f"policy={ibkr_execution}", "IBKR read-only politikasını koruyun")

    critical = sum(issue["severity"] == "CRITICAL" for issue in issues)
    warnings = sum(issue["severity"] == "WARN" for issue in issues)
    report = {
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "profile": (REPO_DIR / "scripts/.active").read_text(encoding="utf-8").strip()
        if (REPO_DIR / "scripts/.active").exists()
        else "unknown",
        "configured_venues": sorted(configured),
        "venues": venue_report,
        "bist": bist_report,
        "execution": execution,
        "ibkr_execution": ibkr_execution,
        "summary": {"critical": critical, "warnings": warnings, "status": "FAIL" if critical else "WARN" if warnings else "READY"},
        "issues": issues,
    }
    return report, 1 if critical else 2 if warnings else 0


def print_report(report: dict[str, Any]) -> None:
    summary = report["summary"]
    print(f"Memos System Audit | {report['checked_at']} | profile={report['profile']}")
    print(f"SUMMARY status={summary['status']} critical={summary['critical']} warnings={summary['warnings']}")
    print("\nVENUES")
    for venue, item in report["venues"].items():
        heartbeat = item["heartbeat"]
        phase = heartbeat.get("phase", "-")
        age = heartbeat.get("age_seconds", "-")
        print(f"  {venue:<8} {item['overall']:<12} engine={item['engine']} watchdog={item['watchdog']} phase={phase} heartbeat_age={age}s")
    trend = report["bist"]["trend"]
    refresh = report["bist"]["refresh"]
    print("\nBIST")
    print(f"  refresh={refresh.get('status', 'UNKNOWN')} trend={trend.get('status', 'UNKNOWN')} freshness={trend.get('data_freshness', '-')}")
    print(f"  symbols={trend.get('successful_download_count', 0)}/{trend.get('expected_symbol_count', 0)} latest={trend.get('latest_candle_timestamp', '-')}")
    print("\nEXECUTION")
    for key, value in report["execution"].items():
        print(f"  {key}={value}")
    if report.get("ibkr_execution"):
        print("  IBKR_RUNTIME_POLICY=" + json.dumps(report["ibkr_execution"], ensure_ascii=False))
    print("\nFINDINGS")
    if not report["issues"]:
        print("  PASS  anomalı tespit edilmedi")
    else:
        for index, issue in enumerate(report["issues"], 1):
            print(f"  {index}. {issue['severity']:<8} {issue['area']}: {issue['finding']}")
            print(f"     ACTION   {issue['action']}")


def notify_if_needed(report: dict[str, Any], state_path: Path) -> None:
    config = load_config_values()
    token = config.get("TELEGRAM_BOT_TOKEN", "")
    chat_id = config.get("TELEGRAM_CHAT_ID", "")
    if not token or not chat_id:
        return
    summary = report["summary"]
    issues = report["issues"]
    signature = json.dumps(
        [(item["severity"], item["area"], item["finding"]) for item in issues],
        ensure_ascii=False,
        sort_keys=True,
    )
    previous, _ = read_json(state_path)
    previous_signature = (previous or {}).get("signature")
    previous_status = (previous or {}).get("status")
    current_status = summary["status"]
    if previous is None:
        state_path.parent.mkdir(parents=True, exist_ok=True)
        state_path.write_text(
            json.dumps(
                {"checked_at": report["checked_at"], "signature": signature, "status": current_status},
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        return
    changed = signature != previous_signature or current_status != previous_status
    if not changed:
        return
    if current_status == "READY":
        text = "[MEMOS RECOVERY] Sistem audit tekrar READY oldu."
    else:
        lines = [
            f"[MEMOS {current_status}] critical={summary['critical']} warnings={summary['warnings']}",
        ]
        for item in issues[:5]:
            lines.append(f"- {item['severity']} {item['area']}: {item['finding']}")
            lines.append(f"  Aksiyon: {item['action']}")
        text = "\n".join(lines)
    payload = urllib.parse.urlencode({"chat_id": chat_id, "text": text}).encode()
    request = urllib.request.Request(
        f"https://api.telegram.org/bot{token}/sendMessage",
        data=payload,
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            if response.status < 200 or response.status >= 300:
                raise RuntimeError(f"Telegram HTTP {response.status}")
    except (OSError, RuntimeError) as exc:
        print(f"Telegram bildirimi gönderilemedi: {exc}", file=sys.stderr)
        return
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.write_text(
        json.dumps(
            {"checked_at": report["checked_at"], "signature": signature, "status": current_status},
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


def safe_action(issue: dict[str, str]) -> tuple[str, list[str]] | None:
    area = issue["area"]
    finding = issue["finding"]
    if area in {"binance", "mt5", "ibkr"}:
        if "service=" in finding:
            return f"{area} engine servisini başlat", [
                "systemctl",
                "--user",
                "start",
                f"memos-venue-engine@{area}.service",
            ]
        return f"{area} venue recovery çalıştır", ["./memos", "recover", area]
    if area == "bist.refresh":
        return "BIST refresh worker'ı başlat", [
            "systemctl",
            "--user",
            "start",
            "memos-bist-refresh.service",
        ]
    return None


def run_menu() -> int:
    while True:
        report, exit_code = audit_once()
        print_report(report)
        findings = [
            (index, issue, safe_action(issue))
            for index, issue in enumerate(report["issues"], 1)
        ]
        print("\nAKSIYON MENUSU")
        print("  [r] Raporu yenile")
        for index, issue, action in findings:
            label = action[0] if action else "manuel inceleme gerekli"
            print(f"  [{index}] {label} ({issue['severity']})")
        print("  [q] Çıkış")
        choice = input("Seçim: ").strip().lower()
        if choice == "q":
            return exit_code
        if choice == "r":
            print()
            continue
        if not choice.isdigit():
            print("Geçersiz seçim.\n")
            continue
        selected = next((item for item in findings if item[0] == int(choice)), None)
        if not selected:
            print("Bu bulgu için otomatik güvenli aksiyon yok.\n")
            continue
        _, issue, action = selected
        command = action[1]
        print(f"Çalıştırılacak: {' '.join(command)}")
        confirm = input("Onaylıyor musunuz? [y/N] ").strip().lower()
        if confirm not in {"y", "yes", "e", "evet"}:
            print("İşlem iptal edildi.\n")
            continue
        try:
            result = subprocess.run(command, cwd=REPO_DIR, check=False, timeout=120)
        except (OSError, subprocess.TimeoutExpired) as exc:
            print(f"İşlem çalıştırılamadı: {exc}\n")
            continue
        print(f"İşlem çıkış kodu: {result.returncode}\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="JSON raporu yazdır")
    parser.add_argument("--watch", type=int, metavar="SECONDS", help="raporu periyodik yenile")
    parser.add_argument("--notify", action="store_true", help="durum değişimlerinde Telegram bildir")
    parser.add_argument("--output", type=Path, help="JSON rapor artefaktı yolu")
    parser.add_argument("--menu", action="store_true", help="önerilen güvenli aksiyonları menüle")
    args = parser.parse_args()
    if args.menu:
        return run_menu()
    interval = args.watch
    if interval is not None and interval < 1:
        parser.error("--watch pozitif saniye olmalı")
    while True:
        report, exit_code = audit_once()
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        if args.notify:
            notify_if_needed(report, REPO_DIR / "logs/system-audit-notify.json")
        if args.json:
            print(json.dumps(report, ensure_ascii=False, indent=2))
        else:
            print_report(report)
        if interval is None:
            return exit_code
        time.sleep(interval)
        print("\n" + "=" * 72 + "\n")


if __name__ == "__main__":
    raise SystemExit(main())
