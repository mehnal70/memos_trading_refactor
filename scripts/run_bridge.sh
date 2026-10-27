#!/usr/bin/env bash
set -euo pipefail

# Kisa yol: eski/aliskanlik komutlari kirilmadan IBKR bridge scriptine yonlendir.
exec bash "$(dirname "$0")/ibkr/run_bridge.sh" "$@"
