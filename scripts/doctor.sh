#!/usr/bin/env bash
set -euo pipefail

# Kisa yol: eski/aliskanlik komutlari kirilmadan IBKR doctor'a yonlendir.
exec bash "$(dirname "$0")/ibkr/doctor.sh" "$@"
