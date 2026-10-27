#!/usr/bin/env bash
# Memos external runtime supervisor.
#
# Bu katman yalnızca dış bağımlılıkları hazırlar ve doğrular. Rust engine'lerini
# başlatmaz; engine başlangıcı, tüm probe'lar başarılı olduktan sonra ayrı bir
# operasyon adımı olarak kalır.
#
# Kullanım:
#   scripts/runtime_supervisor.sh status
#   scripts/runtime_supervisor.sh verify [mt5|ibkr|all]
#   scripts/runtime_supervisor.sh readiness [mt5|ibkr|binance|bist|all]
#   scripts/runtime_supervisor.sh recover [mt5|ibkr|binance|bist|all]
#   scripts/runtime_supervisor.sh start [mt5|ibkr|all]
#   scripts/runtime_supervisor.sh start-gui [mt5|ibkr|all]
#   scripts/runtime_supervisor.sh engine [mt5|ibkr|binance|bist|all]

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

# Kullanıcıya özel GUI/profil ayarları varsa yükle.
. "$REPO_DIR/scripts/lib_launchconf.sh" 2>/dev/null || true
load_launch_conf "$REPO_DIR/scripts/.launch.conf" 2>/dev/null || true
# External dependency settings are stable across venue engine profiles. They must
# override the active engine profile so switching MT5/IBKR cannot hide a ready
# Gateway or bridge from this supervisor.
load_launch_conf "$REPO_DIR/scripts/.autonomy.env" 2>/dev/null || true

MT5_PORT="${MT5_BRIDGE_PORT:-9001}"
IBKR_API_PORT_VALUE="${IBKR_API_PORT:-4002}"
IBKR_BRIDGE_ADDR_VALUE="${IBKR_BRIDGE_ADDR:-127.0.0.1:9101}"
IBKR_BRIDGE_PORT="${IBKR_BRIDGE_ADDR_VALUE##*:}"
MT5_PREFIX="${BOTTLES_MT5_PREFIX:-$HOME/.var/app/com.usebottles.bottles/data/bottles/bottles/mt5}"
IBKR_REMMINA_PROFILE="${IBKR_REMMINA_PROFILE:-}"
BIST_AUTO_PIN_ENABLED="${BIST_AUTO_PIN_ENABLED:-1}"

venue_active() {
  local wanted="$1" token
  IFS=',' read -ra configured_venues <<< "${VENUES:-}"
  for token in "${configured_venues[@]}"; do
    [[ "${token%%:*}" == "$wanted" ]] && return 0
  done
  return 1
}

refresh_bist_pins() {
  [[ "$BIST_AUTO_PIN_ENABLED" == "1" ]] || return 0
  venue_active bist || return 0
  local rc=0
  python3 "$REPO_DIR/scripts/bist/auto_pin_symbols.py" || rc=$?
  if [[ "$rc" -eq 10 ]]; then
    log "BIST pinned evreni değişti; signal-only engine yeniden başlatılıyor"
    "$REPO_DIR/scripts/venue_engine.sh" stop bist || true
    "$REPO_DIR/scripts/venue_engine.sh" start bist || log "BIST auto-pin sonrası engine başlatılamadı"
  elif [[ "$rc" -ne 0 ]]; then
    log "BIST auto-pin başarısız (rc=$rc)"
  fi
}
SUPERVISOR_LOG="${RUNTIME_SUPERVISOR_LOG:-$REPO_DIR/logs/runtime-supervisor.log}"
SUPERVISOR_INTERVAL="${RUNTIME_SUPERVISOR_INTERVAL_SECS:-30}"
RECOVERY_GATE_ENABLED="${RUNTIME_SUPERVISOR_RECOVERY_GATE_ENABLED:-1}"
AUTO_RECOVERY_ENABLED="${VENUE_AUTO_RECOVERY_ENABLED:-0}"

log() {
  mkdir -p "$(dirname "$SUPERVISOR_LOG")"
  printf '[runtime-supervisor %s] %s\n' "$(date -Is)" "$*" | tee -a "$SUPERVISOR_LOG"
}

have() { command -v "$1" >/dev/null 2>&1; }

port_open() {
  local host="$1" port="$2"
  if have nc; then
    nc -z -w 2 "$host" "$port" >/dev/null 2>&1
  else
    timeout 2 bash -lc "</dev/tcp/${host}/${port}" >/dev/null 2>&1
  fi
}

desktop_ready() {
  [[ -n "${DISPLAY:-}" || -n "${WAYLAND_DISPLAY:-}" ]] &&
    [[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]]
}

mt5_terminal_path() {
  printf '%s/drive_c/Program Files/MetaTrader 5/terminal64.exe\n' "$MT5_PREFIX"
}

mt5_status() {
  local terminal="down" process="down" bridge="down"
  [[ -f "$(mt5_terminal_path)" ]] && terminal="installed"
  pgrep -af 'terminal64\.exe' >/dev/null 2>&1 && process="up"
  port_open 127.0.0.1 "$MT5_PORT" && bridge="ready"
  printf 'mt5 terminal=%s process=%s bridge=%s port=%s\n' \
    "$terminal" "$process" "$bridge" "$MT5_PORT"
  [[ "$terminal" == "installed" && "$process" == "up" && "$bridge" == "ready" ]]
}

remmina_target() {
  [[ -f "$IBKR_REMMINA_PROFILE" ]] || return 1
  awk -F= '$1 == "server" {print $2; exit}' "$IBKR_REMMINA_PROFILE"
}

ibkr_status() {
  local api="down" bridge="down" gui="unconfigured" target
  if target="$(remmina_target 2>/dev/null)"; then
    gui="$target"
  fi
  port_open 127.0.0.1 "$IBKR_API_PORT_VALUE" && api="open"
  port_open "${IBKR_BRIDGE_ADDR_VALUE%%:*}" "$IBKR_BRIDGE_PORT" && bridge="ready"
  printf 'ibkr gui=%s api=%s port=%s bridge=%s port=%s\n' \
    "$gui" "$api" "$IBKR_API_PORT_VALUE" "$bridge" "$IBKR_BRIDGE_PORT"
  [[ "$api" == "open" && "$bridge" == "ready" ]]
}

start_mt5_gui() {
  desktop_ready || { log "MT5 GUI başlatılamadı: grafik oturumu hazır değil"; return 1; }
  if pgrep -af 'terminal64\.exe' >/dev/null 2>&1; then
    log "MT5 terminali zaten çalışıyor; Bottles yeniden başlatılmadı"
    return 0
  fi
  if have bottles; then
    nohup bottles >/dev/null 2>&1 &
  elif have flatpak; then
    nohup flatpak run com.usebottles.bottles >/dev/null 2>&1 &
  else
    log "Bottles/Flatpak bulunamadı"
    return 1
  fi
  log "Bottles GUI başlatıldı; MT5 terminali ve EA probe bekleniyor"
}

start_ibkr_gui() {
  desktop_ready || { log "IBKR GUI başlatılamadı: grafik oturumu hazır değil"; return 1; }
  [[ -n "$IBKR_REMMINA_PROFILE" ]] ||
    { log "IBKR_REMMINA_PROFILE tanımlı değil; Remmina profili otomatik açılmadı"; return 1; }
  have remmina || { log "remmina bulunamadı"; return 1; }
  [[ -f "$IBKR_REMMINA_PROFILE" ]] ||
    { log "Remmina profili bulunamadı: $IBKR_REMMINA_PROFILE"; return 1; }
  local target host port
  target="$(remmina_target 2>/dev/null || true)"
  host="${target%:*}"
  port="${target##*:}"
  if [[ -n "$host" && -n "$port" ]] && port_open "$host" "$port"; then
    log "Remmina hedefi zaten erişilebilir ($target); yeni bağlantı açılmadı"
    return 0
  fi
  nohup remmina -c "$IBKR_REMMINA_PROFILE" >/dev/null 2>&1 &
  log "Remmina profili başlatıldı: $IBKR_REMMINA_PROFILE"
}

ensure_ibkr_container() {
  have docker || return 0
  local container="${IBKR_GATEWAY_CONTAINER:-ibgateway_otonom}"
  local running
  if ! docker inspect "$container" >/dev/null 2>&1; then
    log "IBKR Gateway container bulunamadı: $container"
    return 1
  fi
  running="$(docker inspect -f '{{.State.Running}}' "$container" 2>/dev/null || true)"
  if [[ "$running" == "true" ]]; then
    return 0
  fi
  log "IBKR Gateway container başlatılıyor: $container"
  docker start "$container" >/dev/null
}

activate_recovery_gate() {
  [[ "$RECOVERY_GATE_ENABLED" == "1" ]] || return 0
  log "Memos recovery gate çalıştırılıyor"
  "$REPO_DIR/memos" recover >>"$REPO_DIR/logs/runtime-recovery.log" 2>&1
}

verify_ibkr() {
  IBKR_API_PORT="$IBKR_API_PORT_VALUE" IBKR_BRIDGE_ADDR="$IBKR_BRIDGE_ADDR_VALUE" \
    bash "$REPO_DIR/scripts/ibkr/doctor.sh" paper
}

verify_one() {
  case "$1" in
    mt5) mt5_status ;;
    ibkr) verify_ibkr ;;
    *) log "geçersiz hedef: $1"; return 2 ;;
  esac
}

start_one() {
  case "$1" in
    mt5)
      if mt5_status; then
        log "MT5 zaten hazır; yeniden başlatma yapılmadı"
        return 0
      fi
      start_mt5_gui
      "$REPO_DIR/scripts/mt5/setup_and_run.sh" check
      ;;
    ibkr)
      if ibkr_status; then
        log "IBKR zaten hazır; recovery/Remmina/bridge işlemleri atlandı"
        return 0
      fi
      activate_recovery_gate
      ensure_ibkr_container || return 1
      start_ibkr_gui
      if ! port_open 127.0.0.1 "$IBKR_BRIDGE_PORT"; then
        nohup bash "$REPO_DIR/scripts/ibkr/bridge_service.sh" \
          >>"$REPO_DIR/logs/ibkr-bridge-supervisor.log" 2>&1 &
        log "IBKR bridge service başlatıldı"
      else
        log "IBKR bridge zaten hazır; yeni instance başlatılmadı"
      fi
      ;;
    *) log "geçersiz hedef: $1"; return 2 ;;
  esac
}

daemon_loop() {
  log "daemon başlıyor (interval=${SUPERVISOR_INTERVAL}s)"
  if [[ "$AUTO_RECOVERY_ENABLED" == "1" ]]; then
    bash "$REPO_DIR/scripts/venue_recovery.sh" all || log "venue auto-recovery başlangıç turu başarısız"
  else
    if venue_active mt5; then
      start_one mt5 || log "MT5 GUI başlatma/doğrulama başarısız"
    fi
    if venue_active ibkr; then
      start_one ibkr || log "IBKR GUI/bridge başlatma başarısız"
    fi
  fi
  while true; do
    refresh_bist_pins
    python3 "$REPO_DIR/scripts/venue_readiness.py" all >/dev/null \
      || log "ortak venue readiness artefaktı güncellenemedi"
    if [[ "$AUTO_RECOVERY_ENABLED" == "1" ]]; then
      bash "$REPO_DIR/scripts/venue_recovery.sh" all || log "venue auto-recovery turu başarısız"
    else
      if venue_active mt5; then
        mt5_status || log "MT5 readiness degraded"
      fi
      if venue_active ibkr; then
        ibkr_status || log "IBKR readiness degraded"
      fi
    fi
    sleep "$SUPERVISOR_INTERVAL"
  done
}

targets_for() {
  case "${1:-all}" in
    all) printf '%s\n' mt5 ibkr ;;
    mt5|ibkr) printf '%s\n' "$1" ;;
    *) log "geçersiz hedef: ${1:-}"; return 2 ;;
  esac
}

command_name="${1:-status}"
target="${2:-all}"
case "$command_name" in
  status)
    desktop_ready && echo "desktop=ready" || echo "desktop=unavailable"
    mt5_status || true
    ibkr_status || true
    ;;
  verify)
    failed=0
    while read -r item; do
      verify_one "$item" || failed=1
    done < <(targets_for "$target")
    exit "$failed"
    ;;
  start|start-gui)
    if [[ "$command_name" == "start-gui" ]]; then
      while read -r item; do start_one "$item" || exit 1; done < <(targets_for "$target")
    else
      log "start yalnızca dış bağımlılık servislerini başlatır; GUI için start-gui kullanın"
      while read -r item; do
        [[ "$item" == "ibkr" ]] && start_one "$item" || true
      done < <(targets_for "$target")
    fi
    ;;
  daemon)
    daemon_loop
    ;;
  engine)
    "$REPO_DIR/scripts/venue_engine.sh" start "$target"
    ;;
    readiness)
      exec python3 "$REPO_DIR/scripts/venue_readiness.py" "$target"
      ;;
    recover)
      exec bash "$REPO_DIR/scripts/venue_recovery.sh" "$target"
      ;;
      *)
        echo "Kullanim: $0 status | verify [mt5|ibkr|all] | readiness [venue|all] | recover [venue|all] | start [ibkr|all] | start-gui [mt5|ibkr|all] | engine [mt5|ibkr|all] | daemon"
    exit 2
    ;;
esac
