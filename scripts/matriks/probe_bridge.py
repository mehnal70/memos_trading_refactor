#!/usr/bin/env python3
"""Matriks bridge transport probe.

This tool verifies only the Ubuntu <-> Windows TCP/NDJSON transport. It does
not automate MatriksIQ, submit orders, or treat a GUI session as an API.
"""

from __future__ import annotations

import argparse
import json
import os
import socket
import sys
import time
from typing import Any


def parse_addr(raw: str) -> tuple[str, int]:
    host, separator, port = raw.rpartition(":")
    if not separator or not host:
        raise ValueError(f"invalid bridge address: {raw!r}")
    parsed_port = int(port)
    if not 1 <= parsed_port <= 65535:
        raise ValueError(f"invalid bridge port: {parsed_port}")
    return host, parsed_port


def request(sock: socket.socket, command: str, request_id: str) -> dict[str, Any]:
    payload = json.dumps(
        {"id": request_id, "cmd": command},
        ensure_ascii=True,
        separators=(",", ":"),
    ).encode("utf-8") + b"\n"
    sock.sendall(payload)
    response = b""
    while not response.endswith(b"\n"):
        chunk = sock.recv(4096)
        if not chunk:
            raise ConnectionError("bridge closed the connection")
        response += chunk
        if len(response) > 1_048_576:
            raise ValueError("bridge response exceeds 1 MiB")
    decoded = json.loads(response.decode("utf-8"))
    if not isinstance(decoded, dict):
        raise ValueError("bridge response must be a JSON object")
    return decoded


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--addr",
        default=os.getenv("MATRIKS_BRIDGE_ADDR", "127.0.0.1:9010"),
        help="bridge host:port",
    )
    parser.add_argument("--timeout", type=float, default=3.0)
    args = parser.parse_args()

    try:
        host, port = parse_addr(args.addr)
    except (ValueError, TypeError) as error:
        print(f"ERROR {error}", file=sys.stderr)
        return 2

    try:
        with socket.create_connection((host, port), timeout=args.timeout) as sock:
            sock.settimeout(args.timeout)
            for index, command in enumerate(
                ("health", "session_status", "accounts"),
                start=1,
            ):
                result = request(sock, command, f"probe-{int(time.time())}-{index}")
                print(json.dumps({"command": command, "response": result}, ensure_ascii=True))
    except (OSError, TimeoutError, ValueError, json.JSONDecodeError) as error:
        print(f"UNAVAILABLE {host}:{port} — {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
