#!/usr/bin/env bash
# Canlı execution öncesi güvenlik kapısı.
#
# Bu script canlı mod açmaz. Aktif profilin paper/dry-run güvenliğini veya
# canlı profil için açık operatör arm/go-no-go kararını doğrular.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

die() {
  echo "prelive_gate: ❌ $*" >&2
  exit 1
}

read_active_profile() {
  if [[ -f scripts/.active ]]; then
    tr -d '[:space:]' < scripts/.active
  else
    echo "none"
  fi
}

load_runtime_config() {
  if [[ -f scripts/lib_launchconf.sh && -f scripts/.launch.conf ]]; then
    # shellcheck disable=SC1091
    . scripts/lib_launchconf.sh
    load_launch_conf scripts/.launch.conf
  fi
}

is_true() {
  [[ "${1,,}" == "true" || "$1" == "1" ]]
}

check_policy() {
  load_runtime_config

  local profile mode dry_run armed go_no_go venues symbols max_notional
  profile="$(read_active_profile)"
  mode="${TRADING_MODE:-}"
  dry_run="${LIVE_DRY_RUN:-}"
  armed="${LIVE_EXECUTION_ARMED:-0}"
  go_no_go="${LIVE_GO_NO_GO:-NO_GO}"
  venues="${VENUES:-}"
  symbols="${PINNED_SYMBOLS:-}"
  max_notional="${LIVE_MAX_NOTIONAL_USD:-}"

  printf 'profil=%s mod=%s dry_run=%s armed=%s go_no_go=%s\n' \
    "$profile" "${mode:-unset}" "${dry_run:-unset}" "$armed" "$go_no_go"

  [[ -n "$mode" ]] || die "TRADING_MODE eksik"
  [[ -n "$venues" ]] || die "VENUES eksik"

  if [[ "${mode,,}" == "paper" || "${dry_run,,}" == "true" || "$dry_run" == "1" ]]; then
    is_true "$dry_run" || [[ "${mode,,}" == "paper" ]] ||
      die "paper profil dry-run olmadan çalıştırılamaz"
    ! is_true "$armed" || die "paper/dry-run profili canlı olarak arm edilmiş"
    [[ "${go_no_go,,}" != "go" ]] || die "paper/dry-run profili GO ilan edilmiş"
    echo "✅ Güvenli paper/dry-run politikası doğrulandı"
    return 0
  fi

  [[ "${mode,,}" == "live" ]] || die "tanımsız TRADING_MODE=$mode"
  is_true "$armed" || die "canlı mod için LIVE_EXECUTION_ARMED=true gerekli"
  [[ "${go_no_go,,}" == "go" ]] || die "canlı mod için LIVE_GO_NO_GO=GO gerekli"
  [[ "${dry_run,,}" == "false" || "$dry_run" == "0" ]] ||
    die "canlı mod LIVE_DRY_RUN=false olmalı"
  [[ -n "$symbols" ]] || die "canlı mod PINNED_SYMBOLS allowlist'i eksik"
  [[ "$max_notional" =~ ^[0-9]+([.][0-9]+)?$ ]] || die "LIVE_MAX_NOTIONAL_USD geçersiz"
  awk "BEGIN { exit !($max_notional > 0) }" ||
    die "LIVE_MAX_NOTIONAL_USD sıfırdan büyük olmalı"
  [[ -n "${VENUE_MAX_CONCURRENT_OPEN:-}" ]] ||
    die "VENUE_MAX_CONCURRENT_OPEN eksik"
  [[ -n "${VENUE_DAILY_LOSS_PCT:-}" ]] ||
    die "VENUE_DAILY_LOSS_PCT eksik"

  if [[ "$venues" == *"bist"* && "${BIST_EXECUTION_ENABLED:-0}" != "1" ]]; then
    die "BIST execution bridge etkin değil; BIST signal-only kalmalı"
  fi

  echo "✅ Canlı profil biçimsel risk politikası doğrulandı"
}

check_readiness() {
  python3 scripts/venue_readiness.py
}

case "${1:-policy}" in
  policy) check_policy ;;
  readiness) check_readiness ;;
  all)
    check_policy
    check_readiness
    ;;
  *)
    echo "Kullanım: scripts/prelive_gate.sh [policy|readiness|all]" >&2
    exit 2
    ;;
esac
