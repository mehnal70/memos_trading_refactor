#!/usr/bin/env bash
set -euo pipefail

# Uyumluluk kisayolu: run_engine.sh -> engine.sh
exec bash "$(dirname "$0")/engine.sh" "$@"
