#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=scripts/ibkr/lib_single_owner.sh
. "${SCRIPT_DIR}/lib_single_owner.sh"

ACTION="${1:-enforce}"

case "$ACTION" in
  status)
    echo "IBKR single-owner status"
    echo "------------------------"
    ibkr_single_owner_report
    if ibkr_single_owner_violation; then
      echo "RESULT: VIOLATION"
      exit 2
    fi
    echo "RESULT: OK"
    ;;
  enforce)
    if ibkr_single_owner_enforce; then
      echo "RESULT: OK"
      exit 0
    fi
    echo "RESULT: FAIL"
    exit 3
    ;;
  *)
    echo "Kullanim: $0 [status|enforce]"
    exit 2
    ;;
esac
