#!/usr/bin/env python3
# scripts/dashboard_server.py — Memos RTC mobil izleme paneli için minimal HTTP sunucu.
#
# Salt-okuma endpoint'leri sunar (data/ dizinini AÇMAZ → trader.db sızmaz):
#   GET /               → web/index.html (mobil panel)
#   GET /snapshot.json  → aktif snapshot + venue bazlı runtime finans overlay'i
#   GET /readiness.json → tüm izole venue'lerin ortak readiness sözleşmesi
#   GET /venue-symbols.json?venue=... → DB/katalog sembol listesi (salt-okuma)
#   GET /healthz        → panelin canlılık kontrolü
# Salt-okuma; hiçbir kontrol/emir yok. Güvenli varsayılan olarak loopback'e bağlanır.
# Uzak erişim için Tailscale Serve/reverse-proxy kullanın veya MEMOS_DASH_BIND'i açıkça ayarlayın.
#
# Kullanım: python3 scripts/dashboard_server.py [PORT]
# Env: MEMOS_DASH_BIND (default 127.0.0.1), MEMOS_DASH_PORT (default 8090)
import http.server
import os
import socketserver
import sys
import time
import json
import re
import urllib.error
import urllib.parse
import urllib.request
import sqlite3
from datetime import datetime, timezone

from venue_readiness import probe as _venue_readiness

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INDEX = os.path.join(REPO, "web", "index.html")
BIND = os.environ.get("MEMOS_DASH_BIND", "127.0.0.1")
PORT = int(os.environ.get("MEMOS_DASH_PORT", sys.argv[1] if len(sys.argv) > 1 else 8090))
DEFAULT_SNAPSHOT = os.path.join(REPO, "data", "snapshot.json")
DEFAULT_BIST_TREND_REPORT = os.path.join(REPO, "data", "bist_trend.json")
DEFAULT_BIST_TREND_STATUS = os.path.join(REPO, "data", "bist_trend_status.json")
DEFAULT_LOGS = {
    "engine": os.path.join(REPO, "logs", "robotic_trading.log"),
    "trades": os.path.join(REPO, "logs", "trades.jsonl"),
    "heartbeat": os.path.join(REPO, "logs", "heartbeat.jsonl"),
}
DEFAULT_METRICS_PORT = 9100
DEFAULT_DATABASE = os.path.join(REPO, "data", "trader.db")


def _active_profile_name() -> str:
    try:
        with open(os.path.join(REPO, "scripts", ".active"), encoding="utf-8") as handle:
            profile = handle.read().strip()
    except OSError:
        profile = ""
    return profile or "paper-mt5"


ACTIVE_MT5_PROFILE = _active_profile_name()

VENDOR_FILES = {
    "/vendor/bootstrap-5.3.3.min.css": (
        os.path.join(REPO, "web", "vendor", "bootstrap-5.3.3.min.css"),
        "text/css; charset=utf-8",
    ),
    "/vendor/bootstrap-icons-1.11.3.min.css": (
        os.path.join(REPO, "web", "vendor", "bootstrap-icons-1.11.3.min.css"),
        "text/css; charset=utf-8",
    ),
    "/vendor/fonts/bootstrap-icons.woff2": (
        os.path.join(REPO, "web", "vendor", "fonts", "bootstrap-icons.woff2"),
        "font/woff2",
    ),
    "/vendor/fonts/bootstrap-icons.woff": (
        os.path.join(REPO, "web", "vendor", "fonts", "bootstrap-icons.woff"),
        "font/woff",
    ),
}
VENUE_RUNTIME = {
    "mt5": {
        "label": "MT5",
        "profile": "paper-mt5",
        "heartbeat": "logs/paper-mt5/heartbeat.jsonl",
        "pid": "logs/.venue-runtime/state/mt5.engine.pid",
        "watchdog_pid": "logs/.venue-runtime/state/mt5.watchdog.pid",
        "snapshot": "data/profiles/paper-mt5-snapshot.json",
        "listener": ("127.0.0.1", 9001),
        "metrics": ("127.0.0.1", 9100),
        "policy": "Kağıt işlem · Bottles/EA",
    },
    "ibkr": {
        "label": "IBKR",
        "profile": "paper-ibkr",
        "heartbeat": "logs/paper-ibkr/heartbeat.jsonl",
        "pid": "logs/.venue-runtime/state/ibkr.engine.pid",
        "watchdog_pid": "logs/.venue-runtime/state/ibkr.watchdog.pid",
        "snapshot": "data/profiles/paper-ibkr-snapshot.json",
        "metrics": ("127.0.0.1", 9200),
        "external": (("127.0.0.1", 4002), ("127.0.0.1", 9101)),
        "policy": "Kağıt işlem · Salt okunur",
    },
    "binance": {
        "label": "Binance",
        "profile": "paper-binance-testnet",
        "heartbeat": "logs/paper-binance-testnet/heartbeat.jsonl",
        "pid": "logs/.venue-runtime/state/binance.engine.pid",
        "watchdog_pid": "logs/.venue-runtime/state/binance.watchdog.pid",
        "snapshot": "data/profiles/paper-binance-testnet-snapshot.json",
        "metrics": ("127.0.0.1", 9300),
        "policy": "Testnet · Simülasyon",
    },
    "bist": {
        "label": "BIST",
        "profile": "paper-bist-signal",
        "heartbeat": "logs/soak/bist/heartbeat.jsonl",
        "pid": "logs/.venue-runtime/state/bist.engine.pid",
        "watchdog_pid": "logs/.venue-runtime/state/bist.watchdog.pid",
        "snapshot": "logs/soak/bist/snapshot.json",
        "metrics": ("127.0.0.1", 9400),
        "policy": "Yalnızca sinyal · Günlük",
    },
}

def _port_open(host, port):
    import socket
    try:
        with socket.create_connection((host, port), timeout=0.5):
            return True
    except OSError:
        return False

def _pid_alive(path):
    try:
        with open(path, encoding="ascii") as f:
            pid = int(f.read().strip())
        os.kill(pid, 0)
        return pid
    except (OSError, ValueError):
        return None

def _last_json(path):
    lines = _tail(path, 1)
    if not lines:
        return None

def _financial_snapshot(relative_path):
    path = os.path.join(REPO, relative_path)
    try:
        with open(path, encoding="utf-8") as handle:
            snapshot = json.load(handle)
        finance = snapshot.get("finance") or {}
        return {
            "available": True,
            "age_seconds": max(0, int(time.time() - os.path.getmtime(path))),
            "status": finance.get("status", "Unavailable"),
            "equity": finance.get("total_equity"),
            "balance": finance.get("balance"),
            "currency": finance.get("currency"),
            "open_pnl": finance.get("open_pnl", 0),
            "realized_pnl": finance.get("realize_pnl", 0),
            "starting_capital": finance.get("starting_capital"),
            "position_count": len(snapshot.get("positions") or []),
            "closed_trade_count": len(snapshot.get("trade_history") or []),
        }
    except (OSError, ValueError, TypeError):
        return {
            "available": False,
            "age_seconds": None,
            "status": "Unavailable",
        }
    try:
        return json.loads(lines[-1])
    except json.JSONDecodeError:
        return None

def _profile_values(profile):
    values = {}
    for relative_path in ("profiles/_base.conf", f"profiles/{profile}.conf"):
        path = os.path.join(REPO, relative_path)
        try:
            with open(path, encoding="utf-8") as handle:
                for line in handle:
                    line = line.strip()
                    if not line or line.startswith("#") or "=" not in line:
                        continue
                    key, value = line.split("=", 1)
                    values[key.strip()] = value.strip()
        except OSError:
            continue
    return values

def _execution_summary(profile):
    values = _profile_values(profile)
    dry_run = values.get("LIVE_DRY_RUN", "").lower() in {"1", "true", "yes", "on"}
    armed = values.get("LIVE_EXECUTION_ARMED", "0") == "1"
    go = values.get("LIVE_GO_NO_GO", "NO_GO").upper()
    real_orders_enabled = armed and go == "GO" and not dry_run
    if dry_run:
        mode = "DRY_RUN"
    elif real_orders_enabled:
        mode = "ARMED"
    else:
        mode = "BLOCKED"
    if profile == "paper-binance-testnet" and values.get("BINANCE_TESTNET", "0") == "1":
        market = "TESTNET"
    else:
        market = {
            "paper-mt5": "MT5 CFD",
            "paper-ibkr": "IBKR CFD",
            "paper-bist-signal": "BIST SIGNAL",
        }.get(profile, values.get("VENUES", "—"))
    return {
        "mode": mode,
        "market": market,
        "live_execution_armed": armed,
        "go_no_go": go,
        "real_orders_enabled": real_orders_enabled,
    }

def _runtime_status():
    now = time.time()
    result = {}
    for key, config in VENUE_RUNTIME.items():
        heartbeat = os.path.join(REPO, config["heartbeat"])
        heartbeat_data = _last_json(heartbeat)
        try:
            heartbeat_age = max(0, int(now - os.path.getmtime(heartbeat)))
        except OSError:
            heartbeat_age = None
        engine_pid = _pid_alive(os.path.join(REPO, config["pid"]))
        watchdog_pid = _pid_alive(os.path.join(REPO, config["watchdog_pid"]))
        readiness = _venue_readiness(key)
        watchdog_ready = readiness.get("watchdog", {}).get("up", watchdog_pid is not None)
        ports = {}
        if "listener" in config:
            host, port = config["listener"]
            ports["listener"] = {"host": host, "port": port, "open": _port_open(host, port)}
        host, port = config["metrics"]
        ports["metrics"] = {"host": host, "port": port, "open": _port_open(host, port)}
        for index, endpoint in enumerate(config.get("external", ())):
            host, port = endpoint
            ports["external_%s" % index] = {
                "host": host, "port": port, "open": _port_open(host, port)
            }
        ports_ready = all(item["open"] for item in ports.values())
        result[key] = {
            "label": config["label"],
            "profile": config["profile"],
            "policy": config["policy"],
            "execution": _execution_summary(config["profile"]),
            "engine": {"up": engine_pid is not None, "pid": engine_pid},
            "watchdog": {"up": watchdog_ready, "pid": watchdog_pid},
            "dependencies_ready": ports_ready,
            "financial": _financial_snapshot(config["snapshot"]),
            "heartbeat": {
                "age_seconds": heartbeat_age,
                "fresh": heartbeat_age is not None and heartbeat_age <= 180,
                "phase": (heartbeat_data or {}).get("phase"),
                "tick": (heartbeat_data or {}).get("tick"),
                "anomalies": (heartbeat_data or {}).get("anomalies", 0),
                "last_error": (heartbeat_data or {}).get("runtime_lock_diagnostic", {}).get("phase"),
            },
            "ports": ports,
            "readiness": readiness,
        }
    return {"generated_at": time.time(), "venues": result}

def _merge_runtime_financials(snapshot):
    """Aktif snapshot'a diğer izole venue hesap snapshot'larını ekler."""
    accounts = dict(snapshot.get("venue_accounts") or {})
    portfolios = list(snapshot.get("venue_portfolios") or [])
    runtime_accounts = {
        venue: ((item.get("readiness") or {}).get("account") or {})
        for venue, item in _runtime_status().get("venues", {}).items()
        if isinstance(item, dict)
    }
    portfolio_by_key = {}
    for item in portfolios:
        if not isinstance(item, dict):
            continue
        venue_name = str(item.get("venue", "")).lower()
        if venue_name:
            portfolio_by_key[venue_name] = item
    market_by_venue = {
        "mt5": "cfd",
        "ibkr": "cfd",
        "binance": "futures",
        "bist": "spot",
    }
    health_by_venue = {
        str(item.get("venue", "")).lower(): item
        for item in snapshot.get("venue_health", [])
        if isinstance(item, dict)
    }

    for venue, config in VENUE_RUNTIME.items():
        financial = _financial_snapshot(config["snapshot"])
        status = str(financial.get("status") or "").lower()
        equity = financial.get("equity")
        currency = financial.get("currency")
        runtime_account = runtime_accounts.get(venue, {})
        if (
            (status not in {"fresh", "partial"} or not currency or equity is None)
            and runtime_account.get("ready")
            and runtime_account.get("equity") is not None
            and runtime_account.get("currency")
        ):
            financial = {
                **financial,
                "status": runtime_account.get("status", "Fresh"),
                "equity": runtime_account["equity"],
                "balance": runtime_account.get("balance"),
                "currency": runtime_account["currency"],
            }
            status = str(financial["status"]).lower()
            equity = financial["equity"]
            currency = financial["currency"]
        if status not in {"fresh", "partial"} or not currency or equity is None:
            continue

        market = market_by_venue[venue]
        key = f"{venue}:{market}"
        accounts[key] = {
            "currency": currency,
            "balance": financial.get("balance"),
            "equity": equity,
            "timestamp": None,
            "quality": "Reported",
            "status": financial["status"],
        }
        portfolio = portfolio_by_key.get(venue) or portfolio_by_key.get(key)
        if portfolio is None:
            portfolio = {"venue": venue, "account_keys": [key]}
            portfolios.append(portfolio)
            portfolio_by_key[key] = portfolio
        portfolio.update({
            "balance": financial.get("balance"),
            "equity": equity,
            "open_pnl": financial.get("open_pnl", 0),
            "realized_pnl": (
                portfolio.get("realized_pnl")
                if portfolio.get("closed_trade_count", 0)
                else financial.get("realized_pnl", 0)
            ),
            "position_count": financial.get("position_count", 0),
            "closed_trade_count": financial.get("closed_trade_count", 0),
        })

        health = health_by_venue.get(venue)
        if health and str(health.get("overall_status", "")).upper() == "INACTIVE":
            health["overall_status"] = "OK"
            health["account_status"] = str(financial["status"]).upper()
            health["readiness_blockers"] = [
                blocker for blocker in health.get("readiness_blockers", [])
                if "hesap snapshot" not in str(blocker)
            ]

    snapshot["venue_accounts"] = accounts
    snapshot["venue_portfolios"] = portfolios
    return snapshot

def _venue_contexts():
    fields = (
        "finance", "data_coverage", "download_progress", "venue_health",
        "venue_runtime", "pipeline_steps", "runtime_pipeline_steps", "autonomy_counters",
        "decision_trace", "opportunity_snapshots", "positions", "trade_history", "venue_symbols",
        "market_fleet", "bist_data_status",
    )
    contexts = {}
    for venue, config in VENUE_RUNTIME.items():
        try:
            with open(os.path.join(REPO, config["snapshot"]), encoding="utf-8") as handle:
                source = json.load(handle)
        except (OSError, ValueError, TypeError):
            source = {}
        pinned = _profile_pinned_symbols(config["profile"])
        symbols = [
            item for item in (source.get("venue_symbols") or []) if isinstance(item, dict)
        ] + [
            item for item in (source.get("market_fleet") or []) if isinstance(item, dict)
        ]
        pinned_bases = {value.split("@", 1)[0] for value in pinned}
        fleet_by_symbol = {
            str(item.get("symbol", "")).upper(): item
            for item in source.get("market_fleet", [])
            if isinstance(item, dict)
        }
        prices = []
        price_by_base = {}
        for item in symbols:
            if not isinstance(item, dict):
                continue
            symbol = str(item.get("symbol") or "")
            if not symbol:
                continue
            base = symbol.split("@", 1)[0].upper()
            if pinned and symbol.upper() not in pinned and base not in pinned_bases:
                continue
            fleet = fleet_by_symbol.get(symbol.upper(), {})
            candle_at = item.get("last_candle_at") or fleet.get("last_candle_at")
            age_seconds = None
            if candle_at:
                try:
                    parsed = datetime.fromisoformat(str(candle_at).replace("Z", "+00:00"))
                    age_seconds = max(0, int((datetime.now(timezone.utc) - parsed).total_seconds()))
                except ValueError:
                    pass
            if item.get("current_price") in (None, 0, 0.0):
                item = {**fleet, **item}
                if item.get("current_price") in (None, 0, 0.0):
                    item["current_price"] = fleet.get("current_price")
            price = {
                "symbol": symbol,
                "display_symbol": base,
                "price": item.get("current_price"),
                "change_24h": item.get("change_24h"),
                "status": item.get("status") or item.get("candle_status") or "FRESH",
                "source": item.get("source") or "snapshot",
                "last_candle_at": candle_at,
                "age_seconds": age_seconds,
                "support": fleet.get("nearest_support"),
                "resistance": fleet.get("nearest_resistance"),
            }
            existing = price_by_base.get(base)
            tagged = "@" in symbol
            if existing is None or (tagged and "@" not in existing["symbol"]) or (
                price["support"] is not None and existing["support"] is None
            ):
                price_by_base[base] = price
        prices = list(price_by_base.values())
        steps = source.get("pipeline_steps") or []
        runtime_steps = source.get("runtime_pipeline_steps") or []
        decision_steps = [
            step for step in steps
            if isinstance(step, dict) and str(step.get("label", "")).lstrip()[:2].rstrip(".").isdigit()
        ]
        contexts[f"{venue}:{'futures' if venue == 'binance' else 'spot' if venue == 'bist' else 'cfd'}"] = {
            field: source.get(field) for field in fields if field in source
        }
        decision_trace = source.get("decision_trace") or _derived_decision_trace(
            venue, source, pinned
        )
        if decision_trace:
            decision_trace = _enrich_decision_trace(decision_trace, source.get("opportunity_snapshots"))
            contexts[f"{venue}:{'futures' if venue == 'binance' else 'spot' if venue == 'bist' else 'cfd'}"][
                "decision_trace"
            ] = decision_trace
        contexts[f"{venue}:{'futures' if venue == 'binance' else 'spot' if venue == 'bist' else 'cfd'}"].update({
            "pipeline_status": {
                "total": len(decision_steps),
                "done": sum(1 for step in decision_steps if str(step.get("status", "")).lower() in {"done", "ready", "ok"}),
                "stale": sum(1 for step in decision_steps if int(step.get("overdue_secs") or 0) > 0),
            },
            "runtime_pipeline_status": {
                "total": len(runtime_steps),
                "done": sum(1 for step in runtime_steps if str(step.get("status", "")).lower() in {"done", "ready", "ok"}),
                "running": sum(1 for step in runtime_steps if str(step.get("status", "")).lower() == "running"),
                "stale": sum(1 for step in runtime_steps if int(step.get("overdue_secs") or 0) > 0),
            },
            "pinned_prices": prices,
        })
    return contexts


def _enrich_decision_trace(trace: list[dict], opportunities: object) -> list[dict]:
    """Karar trace'ini aynı sembolün fırsat TF/bar ve skoruyla tamamlar."""
    if not isinstance(opportunities, list):
        return trace
    by_symbol = {
        str(item.get("symbol") or "").upper(): item
        for item in opportunities
        if isinstance(item, dict) and item.get("symbol")
    }
    enriched = []
    for item in trace:
        if not isinstance(item, dict):
            continue
        opportunity = by_symbol.get(str(item.get("symbol") or "").upper(), {})
        result = dict(item)
        result.setdefault("signal_interval", opportunity.get("interval"))
        result.setdefault("bar_timestamp", opportunity.get("last_ts"))
        result.setdefault("opportunity_score", opportunity.get("opportunity_score"))
        if not result.get("reason") and opportunity.get("opportunity_score") is not None:
            result["reason"] = f"fırsat skoru {float(opportunity['opportunity_score']):.2f}"
        enriched.append(result)
    return enriched


def _derived_decision_trace(venue: str, source: dict, pinned: set[str]) -> list[dict]:
    """Karar paneli için venue'ler arası ortak, emirden bağımsız görünüm üretir."""
    if venue == "bist":
        report_path = _bist_trend_report_path()
        try:
            with open(report_path, encoding="utf-8") as handle:
                report = json.load(handle)
        except (OSError, ValueError, TypeError):
            return []
        rows = report if isinstance(report, list) else report.get("decisions", [])
        pinned_bases = {value.split("@", 1)[0].upper() for value in pinned}
        trace = []
        for item in rows:
            if not isinstance(item, dict):
                continue
            base = str(item.get("symbol") or "").split("@", 1)[0].upper()
            if pinned_bases and base not in pinned_bases:
                continue
            action = str(item.get("action") or "WATCH").upper()
            signal = {"ENTRY_CANDIDATE": "BUY", "EXIT_WARNING": "SELL"}.get(action, "HOLD")
            freshness, age_secs = _bist_bar_freshness(item.get("as_of"))
            trace.append({
                "symbol": f"{base}@bist",
                "strategy": "BIST trend",
                "regime": item.get("trend"),
                "signal": signal,
                "outcome": action,
                "reason": item.get("reason"),
                "signal_interval": item.get("interval") or "1d",
                "bar_timestamp": item.get("as_of"),
                "data_freshness": freshness,
                "data_age_secs": age_secs,
            })
        return trace

    trace = []
    for item in source.get("opportunity_snapshots") or []:
        if not isinstance(item, dict):
            continue
        symbol = str(item.get("symbol") or "")
        if not symbol:
            continue
        phase = str(item.get("phase") or "").lower()
        if "short" in phase:
            signal, outcome = "SELL", "EXIT_WARNING"
        elif "long" in phase:
            signal, outcome = "BUY", "ENTRY_CANDIDATE"
        else:
            signal, outcome = "HOLD", "WATCH"
        trace.append({
            "symbol": symbol,
            "strategy": "Opportunity değerlendirmesi",
            "regime": item.get("phase"),
            "signal": signal,
            "outcome": outcome,
            "reason": f"fırsat skoru {float(item.get('opportunity_score', 0.0)):.2f}",
            "signal_interval": item.get("interval"),
            "bar_timestamp": item.get("last_ts"),
        })
    return trace


def _bist_bar_freshness(value: object) -> tuple[str, int | None]:
    """Günlük BIST barı güncel değilse fırsat havuzunda aktif aday sayma."""
    if not value:
        return "UNKNOWN", None
    try:
        parsed = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        age_secs = max(0, int((datetime.now(timezone.utc) - parsed).total_seconds()))
    except (TypeError, ValueError):
        return "UNKNOWN", None
    max_age = int(os.environ.get("BIST_DAILY_MAX_AGE_SECS", str(4 * 24 * 3600)))
    return ("FRESH" if age_secs <= max_age else "STALE"), age_secs


def _profile_pinned_symbols(profile: str) -> set[str]:
    path = os.path.join(REPO, "profiles", f"{profile}.conf")
    try:
        with open(path, encoding="utf-8") as handle:
            for line in handle:
                if line.startswith("PINNED_SYMBOLS="):
                    return {
                        token.strip().upper()
                        for token in line.split("=", 1)[1].split(",")
                        if token.strip()
                    }
    except OSError:
        pass
    return set()

def _runtime_paths():
    paths = {"snapshot": DEFAULT_SNAPSHOT, **DEFAULT_LOGS, "metrics_port": DEFAULT_METRICS_PORT}
    configured_conf = os.environ.get(
        "RTC_LAUNCH_CONF", os.path.join(REPO, "scripts", ".launch.conf")
    )
    conf = configured_conf if os.path.isabs(configured_conf) else os.path.join(REPO, configured_conf)
    try:
        with open(conf, encoding="utf-8") as f:
            for line in f:
                match = re.match(
                    r"^(SNAPSHOT_PATH|TRADING_LOG_PATH|TRADES_LOG_PATH|HEARTBEAT_PATH|METRICS_PORT)=(.*)$",
                    line.strip(),
                )
                if not match:
                    continue
                key, value = match.groups()
                value = value.strip().strip("'\"")
                if key == "METRICS_PORT":
                    try:
                        port = int(value)
                    except ValueError:
                        continue
                    if 1 <= port <= 65535:
                        paths["metrics_port"] = port
                    continue
                if not os.path.isabs(value):
                    value = os.path.join(REPO, value)
                if os.path.commonpath((REPO, os.path.abspath(value))) == REPO:
                    paths[{
                        "SNAPSHOT_PATH": "snapshot",
                        "TRADING_LOG_PATH": "engine",
                        "TRADES_LOG_PATH": "trades",
                        "HEARTBEAT_PATH": "heartbeat",
                    }[key]] = os.path.abspath(value)
    except (OSError, ValueError):
        pass
    return paths

def _freshest_snapshot_age():
    """Birleşik dashboard için en güncel venue snapshot yaşını döndürür."""
    now = time.time()
    candidates = []
    for config in VENUE_RUNTIME.values():
        path = os.path.join(REPO, config["snapshot"])
        try:
            candidates.append((max(0, int(now - os.path.getmtime(path))), path))
        except OSError:
            continue
    if not candidates:
        return None, None
    age, path = min(candidates, key=lambda item: item[0])
    return age, path

def _runtime_logs():
    paths = _runtime_paths()
    return {key: paths[key] for key in DEFAULT_LOGS}

def _bist_trend_report_path():
    """Return the configured report only when it remains inside the repository."""
    configured = os.environ.get("BIST_TREND_REPORT", "data/bist_trend.json")
    path = configured if os.path.isabs(configured) else os.path.join(REPO, configured)
    path = os.path.abspath(path)
    try:
        if os.path.commonpath((REPO, path)) != REPO:
            return None
    except ValueError:
        return None
    return path

def _bist_trend_status_path():
    """Return the configured health status only when it remains inside the repository."""
    configured = os.environ.get("BIST_TREND_STATUS", "data/bist_trend_status.json")
    path = configured if os.path.isabs(configured) else os.path.join(REPO, configured)
    path = os.path.abspath(path)
    try:
        if os.path.commonpath((REPO, path)) != REPO:
            return None
    except ValueError:
        return None
    return path

def _tail(path, limit):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.readlines()[-limit:]
    except FileNotFoundError:
        return []

def _venue_detail(query_string):
    params = urllib.parse.parse_qs(query_string, keep_blank_values=False)
    required = ("venue", "symbol")
    if any(key not in params or len(params[key]) != 1 for key in required):
        return 400, {"error": "venue ve symbol zorunludur"}, "application/json; charset=utf-8"
    allowed = {"venue", "symbol", "interval", "limit"}
    if set(params) - allowed:
        return 400, {"error": "gecersiz sorgu parametresi"}, "application/json; charset=utf-8"
    venue = params["venue"][0].strip().lower()
    if venue == "bist:spot":
        return _venue_database_detail(params)
    target = f"http://127.0.0.1:{_runtime_paths()['metrics_port']}/venue-detail?" + urllib.parse.urlencode(
        {key: values[0] for key, values in params.items()}
    )
    request = urllib.request.Request(target, headers={"Accept": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.status, response.read(), "application/json; charset=utf-8"
    except urllib.error.HTTPError as error:
        body = error.read()
        if not body:
            body = json.dumps({"error": f"motor endpointi HTTP {error.code} döndürdü"}).encode()
        fallback = _venue_database_detail(params)
        return fallback if fallback[0] == 200 and fallback[1].get("candles") else (error.code, body, "application/json; charset=utf-8")
    except (urllib.error.URLError, TimeoutError):
        fallback = _venue_database_detail(params)
        return fallback if fallback[0] == 200 and fallback[1].get("candles") else (503, {"error": "motor venue sorgusuna yanit vermiyor"}, "application/json; charset=utf-8")

def _venue_database_detail(params):
    symbol = params["symbol"][0].strip().upper()
    interval = (params.get("interval") or ["1d"])[0]
    try:
        limit = max(1, min(int((params.get("limit") or ["50"])[0]), 500))
    except ValueError:
        return 400, {"error": "limit gecersiz"}, "application/json; charset=utf-8"
    venue_map = {
        "bist:spot": ("bist", "bist"),
        "binance:futures": ("binance", "futures"),
        "mt5:cfd": ("mt5", "cfd"),
        "ibkr:cfd": ("ibkr", "cfd"),
    }
    exchange, market = venue_map.get(params["venue"][0].strip().lower(), ("", ""))
    if not exchange:
        return 400, {"error": "desteklenmeyen venue"}, "application/json; charset=utf-8"
    try:
        with sqlite3.connect(DEFAULT_DATABASE, timeout=2) as db:
            rows = db.execute(
                """
                SELECT timestamp, open, high, low, close, volume
                FROM candles
                WHERE lower(exchange) = ? AND lower(market) = ? AND symbol = ? AND interval = ?
                ORDER BY timestamp DESC LIMIT ?
                """,
                (exchange, market, symbol, interval, limit),
            ).fetchall()
        rows.reverse()
        candles = [
            {
                "timestamp": datetime.fromtimestamp(row[0] / 1000, tz=timezone.utc).isoformat(),
                "open": row[1], "high": row[2], "low": row[3],
                "close": row[4], "volume": row[5],
            }
            for row in rows
        ]
        last = candles[-1]["close"] if candles else None
        return 200, {
            "ok": True,
            "venue": params["venue"][0].strip().lower(),
            "symbol": symbol,
            "source": "trader.db",
            "ticker": {"last": last, "bid": None, "ask": None},
            "candles": candles,
        }, "application/json; charset=utf-8"
    except (OSError, sqlite3.Error) as error:
        return 503, {"error": f"DB piyasa verisi okunamadı: {error}"}, "application/json; charset=utf-8"

def _venue_symbols(query_string):
    params = urllib.parse.parse_qs(query_string, keep_blank_values=False)
    venue = (params.get("venue") or [""])[0].strip().lower()
    if not venue or set(params) - {"venue"}:
        return 400, {"error": "venue zorunlu veya gecersiz sorgu"}, "application/json; charset=utf-8"
    try:
        with sqlite3.connect(DEFAULT_DATABASE, timeout=2) as db:
            catalog_rows = db.execute(
                """
                SELECT symbol
                FROM venue_symbol_catalog
                WHERE lower(venue) = ?
                ORDER BY symbol
                """,
                (venue,),
            ).fetchall()
            if catalog_rows:
                rows = catalog_rows
            else:
                rows = db.execute(
                """
                SELECT symbol FROM candles
                WHERE (? = 'bist:spot' AND lower(exchange) = 'bist')
                   OR (? <> 'bist:spot' AND lower(exchange || ':' || market) = ?)
                ORDER BY symbol
                """,
                    (venue, venue, venue),
                ).fetchall()
        return 200, {"venue": venue, "symbols": [row[0] for row in rows if row[0]]}, "application/json; charset=utf-8"
    except (OSError, sqlite3.Error) as error:
        return 503, {"error": f"sembol kataloğu okunamadı: {error}"}, "application/json; charset=utf-8"

class H(http.server.BaseHTTPRequestHandler):
    def _send(self, code, body, ctype):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        try:
            if path == "/healthz":
                self._send(200, b'{"status":"ok"}', "application/json; charset=utf-8")
            elif path == "/runtime.json":
                self._send(
                    200,
                    json.dumps(_runtime_status(), ensure_ascii=False).encode(),
                    "application/json; charset=utf-8",
                )
            elif path == "/readiness.json":
                readiness = {
                    venue: _venue_readiness(venue)
                    for venue in VENUE_RUNTIME
                }
                self._send(
                    200,
                    json.dumps(
                        {"generated_at": time.time(), "venues": readiness},
                        ensure_ascii=False,
                    ).encode(),
                    "application/json; charset=utf-8",
                )
            elif path == "/" or path == "/index.html":
                with open(INDEX, "rb") as f:
                    self._send(200, f.read(), "text/html; charset=utf-8")
            elif path == "/snapshot.json":
                snapshot_path = _runtime_paths()["snapshot"]
                try:
                    age, freshest_snapshot_path = _freshest_snapshot_age()
                    if age is None:
                        raise FileNotFoundError("venue snapshot yok")
                    with open(snapshot_path, "rb") as f:
                        snapshot = json.load(f)
                    merged = _merge_runtime_financials(snapshot)
                    merged["venue_contexts"] = _venue_contexts()
                    body = json.dumps(merged, ensure_ascii=False).encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json; charset=utf-8")
                    self.send_header("Cache-Control", "no-store")
                    self.send_header("X-Snapshot-Age", str(age))
                    self.send_header(
                        "X-Snapshot-Path",
                        os.path.relpath(snapshot_path, REPO),
                    )
                    if freshest_snapshot_path:
                        self.send_header(
                            "X-Freshest-Snapshot-Path",
                            os.path.relpath(freshest_snapshot_path, REPO),
                        )
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                except FileNotFoundError:
                    self._send(
                        503,
                        json.dumps({
                            "error": "snapshot yok - motor calisiyor mu?",
                            "path": os.path.relpath(snapshot_path, REPO),
                        }).encode(),
                        "application/json",
                    )
            elif path == "/bist-trend.json":
                report_path = _bist_trend_report_path()
                if report_path is None:
                    self._send(
                        503,
                        b'{"error":"BIST trend raporu depo dizini disinda yapilandirilmis"}',
                        "application/json; charset=utf-8",
                    )
                    return
                try:
                    with open(report_path, "rb") as f:
                        body = f.read()
                    # JSON'u sunmadan önce parse etmek, bozuk/yarım raporu
                    # panelde geçerli veri gibi göstermeyi engeller.
                    json.loads(body)
                except FileNotFoundError:
                    self._send(
                        404,
                        json.dumps({
                            "error": "BIST trend raporu bulunamadi",
                            "path": os.path.relpath(report_path, REPO),
                        }).encode(),
                        "application/json; charset=utf-8",
                    )
                    return
                except (OSError, json.JSONDecodeError):
                    self._send(
                        503,
                        json.dumps({
                            "error": "BIST trend raporu okunamiyor veya gecersiz JSON",
                            "path": os.path.relpath(report_path, REPO),
                        }).encode(),
                        "application/json; charset=utf-8",
                    )
                    return
                self.send_response(200)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Cache-Control", "no-store")
                self.send_header(
                    "X-Report-Age",
                    str(max(0, int(time.time() - os.path.getmtime(report_path)))),
                )
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            elif path == "/bist-health.json":
                status_path = _bist_trend_status_path()
                if status_path is None:
                    self._send(
                        503,
                        b'{"error":"BIST status dosyasi depo dizini disinda yapilandirilmis"}',
                        "application/json; charset=utf-8",
                    )
                    return
                try:
                    with open(status_path, "rb") as f:
                        body = f.read()
                    json.loads(body)
                except FileNotFoundError:
                    self._send(
                        404,
                        json.dumps({
                            "error": "BIST veri sağlık durumu bulunamadi",
                            "path": os.path.relpath(status_path, REPO),
                        }).encode(),
                        "application/json; charset=utf-8",
                    )
                    return
                except (OSError, json.JSONDecodeError):
                    self._send(
                        503,
                        json.dumps({
                            "error": "BIST veri sağlık durumu okunamiyor veya gecersiz JSON",
                            "path": os.path.relpath(status_path, REPO),
                        }).encode(),
                        "application/json; charset=utf-8",
                    )
                    return
                self.send_response(200)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Cache-Control", "no-store")
                self.send_header("X-Status-Age", str(max(0, int(time.time() - os.path.getmtime(status_path)))))
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            elif path == "/logs.json":
                query = self.path.split("?", 1)[1] if "?" in self.path else ""
                params = dict(item.split("=", 1) for item in query.split("&") if "=" in item)
                stream = params.get("stream", "engine")
                try:
                    limit = min(max(int(params.get("lines", "80")), 1), 200)
                except ValueError:
                    limit = 80
                path_for_stream = _runtime_logs().get(stream)
                if path_for_stream is None:
                    self._send(400, b'{"error":"gecersiz stream"}', "application/json")
                    return
                payload = {
                    "stream": stream,
                    "path": os.path.relpath(path_for_stream, REPO),
                    "lines": [line.rstrip("\n") for line in _tail(path_for_stream, limit)],
                    "updated_at": os.path.getmtime(path_for_stream) if os.path.exists(path_for_stream) else None,
                }
                self._send(200, json.dumps(payload, ensure_ascii=False).encode(), "application/json; charset=utf-8")
            elif path == "/venue-detail.json":
                status, body, ctype = _venue_detail(
                    self.path.split("?", 1)[1] if "?" in self.path else ""
                )
                if isinstance(body, dict):
                    body = json.dumps(body, ensure_ascii=False).encode()
                self._send(status, body, ctype)
            elif path == "/venue-symbols.json":
                status, body, ctype = _venue_symbols(
                    self.path.split("?", 1)[1] if "?" in self.path else ""
                )
                if isinstance(body, dict):
                    body = json.dumps(body, ensure_ascii=False).encode()
                self._send(status, body, ctype)
            elif path in VENDOR_FILES:
                asset_path, content_type = VENDOR_FILES[path]
                try:
                    with open(asset_path, "rb") as f:
                        self._send(200, f.read(), content_type)
                except FileNotFoundError:
                    self._send(404, b"asset not found", "text/plain; charset=utf-8")
            else:
                self._send(404, b"not found", "text/plain")
        except BrokenPipeError:
            pass

    def log_message(self, *a):  # sessiz (stdout'u kirletme)
        pass

class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True

if __name__ == "__main__":
    with Server((BIND, PORT), H) as httpd:
        print(f"📲 Memos panel: http://{BIND}:{PORT}  (Ctrl-C ile dur)")
        httpd.serve_forever()
