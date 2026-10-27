#!/usr/bin/env bash
# scripts/mt5/setup_and_run.sh — MT5 (Wine) kurulum + MemosBridge EA deploy/derle + köprü+motor.
#
# Memos trading-core ↔ MetaTrader 5 köprüsünü uçtan uca ayağa kaldırır:
#   1) Wine kontrol → MT5 terminalini (yoksa) MetaQuotes'tan indir + sessiz kur
#   2) MemosBridge.mq5 EA'yı MT5'in Experts klasörüne kopyala + MetaEditor ile derle (.ex5)
#   3) MT5 terminalini /portable başlat (veri klasörü kurulum dizininde → deterministik yol)
#   4) Memos motorunu paper-mt5 profiliyle başlat (Rust = köprü TCP SERVER, EA = client)
#
# OTOMATİKLEŞTİRİLEMEYEN (GUI/hesap — script SONUNDA listelenir):
#   • MT5'te ücretsiz DEMO hesap aç/giriş yap (forex/altın verisi + sembol listesi bunsuz gelmez)
#   • MemosBridge EA'yı bir grafiğe SÜRÜKLE; "Allow Algo Trading" + (Faz 2 için) InpEnableExec
#   • Tools › Options › Expert Advisors → soket adres listesine 127.0.0.1 ekle (build ≥1930)
#
# Kullanım:
#   scripts/mt5/setup_and_run.sh            # tüm aşamalar (install→deploy→launch→engine)
#   scripts/mt5/setup_and_run.sh install    # yalnız MT5 kurulumu
#   scripts/mt5/setup_and_run.sh deploy      # yalnız EA kopyala+derle
#   scripts/mt5/setup_and_run.sh launch      # yalnız MT5 terminalini başlat
#   scripts/mt5/setup_and_run.sh engine      # yalnız Memos motorunu başlat (aktif profil korunur)
#   scripts/mt5/setup_and_run.sh check       # yalnız durum raporu (kurulum yapmaz)
#   scripts/mt5/setup_and_run.sh self-heal   # eksik/kapalı parçaları tek adımda onar
#   scripts/mt5/setup_and_run.sh install-timer   # user-level systemd timer kur
#   scripts/mt5/setup_and_run.sh uninstall-timer # user-level systemd timer kaldır
#   scripts/mt5/setup_and_run.sh timer-status    # timer durumunu göster
#   scripts/mt5/setup_and_run.sh next-actions    # sadece gerekli manuel adımları yazdır
#   scripts/mt5/setup_and_run.sh probe           # MT5 bridge+EA uçtan uca test
#   scripts/mt5/setup_and_run.sh probe-trade     # MT5 emir kanalı hazır mı (güvenli test)
set -euo pipefail

# ── Konum + ayarlar ───────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
USER_WINEPREFIX="${WINEPREFIX:-}"
BOTTLES_MT5_PREFIX="$HOME/.var/app/com.usebottles.bottles/data/bottles/bottles/mt5"

# EĞER bilgisayarda Bottles MT5 klasörü varsa WINEPREFIX olarak onu ayarla, yoksa eski mantık devam et
if [ -d "$BOTTLES_MT5_PREFIX" ]; then
    export WINEPREFIX="$BOTTLES_MT5_PREFIX"
    echo "ℹ️  Bottles MT5 prefix otomatik olarak WINEPREFIX olarak atandı."
  if [ "${MT5_USE_DXVK:-0}" != "1" ]; then
    export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:-dxgi,d3d11,d3d10core=b}"
  fi
else
    export WINEPREFIX="${WINEPREFIX:-$HOME/.wine}"
fi

export WINEDEBUG="${WINEDEBUG:-fixme-all,err-all}"   # log gürültüsünü kıs
MT5_URL="${MT5_URL:-https://download.mql5.com/cdn/web/metaquotes.software.corp/mt5/mt5setup.exe}"
EA_SRC="$SCRIPT_DIR/MemosBridge.mq5"
SETUP_EXE="${SETUP_EXE:-/tmp/mt5setup.exe}"
BRIDGE_ADDR="${MT5_BRIDGE_ADDR:-127.0.0.1:9001}"
EXECUTOR_ADDR="${MT5_EXECUTOR_ADDR:-127.0.0.1:9002}"
# Mevcut EA protokolünde veri ve emir komutları 9001 üzerinden taşınır.
# Gerçek MT5 execution doğrulaması için MT5_EXECUTION_REQUIRED=1 kullan.
MT5_EXECUTION_REQUIRED="${MT5_EXECUTION_REQUIRED:-0}"
SELF_HEAL_LOG_DIR="$REPO_DIR/logs/mt5-self-heal"


refresh_mt5_paths() {
  INSTALL_DIR="$WINEPREFIX/drive_c/Program Files/MetaTrader 5"
  TERMINAL_EXE="$INSTALL_DIR/terminal64.exe"
  EDITOR_EXE="$INSTALL_DIR/MetaEditor64.exe"
  EA_DIR="$INSTALL_DIR/MQL5/Experts"
}

auto_detect_wineprefix() {
  refresh_mt5_paths
  [ -f "$TERMINAL_EXE" ] && return 0
  # Kullanıcı WINEPREFIX'i açıkça verdiyse onun seçimini bozma.
  [ -n "$USER_WINEPREFIX" ] && return 0

  local bottles_terminal="$BOTTLES_MT5_PREFIX/drive_c/Program Files/MetaTrader 5/terminal64.exe"
  if [ -f "$bottles_terminal" ]; then
    export WINEPREFIX="$BOTTLES_MT5_PREFIX"
    refresh_mt5_paths
    c_info "Bottles MT5 prefix algılandı: $WINEPREFIX"
  fi
}

refresh_mt5_paths

c_ok()   { printf '\033[32m✅ %s\033[0m\n' "$*"; }
c_warn() { printf '\033[33m⚠️  %s\033[0m\n' "$*"; }
c_err()  { printf '\033[31m❌ %s\033[0m\n' "$*" >&2; }
c_info() { printf '\033[36mℹ️  %s\033[0m\n' "$*"; }

now_iso() {
  date -u +"%Y-%m-%dT%H:%M:%SZ"
}

hm_to_minutes() {
  local hm="${1:-}"
  local hh mm
  [[ "$hm" =~ ^([0-9]{2}):([0-9]{2})$ ]] || return 1
  hh="${BASH_REMATCH[1]}"
  mm="${BASH_REMATCH[2]}"
  hh=$((10#$hh))
  mm=$((10#$mm))
  [ "$hh" -ge 0 ] && [ "$hh" -le 23 ] && [ "$mm" -ge 0 ] && [ "$mm" -le 59 ] || return 1
  echo $((hh * 60 + mm))
}

utc_now_minutes() {
  local hh mm
  hh="$(date -u +%H)"
  mm="$(date -u +%M)"
  echo $((10#$hh * 60 + 10#$mm))
}

utc_day_allowed() {
  local days_csv="${1:-1,2,3,4,5,6,7}"
  local day_utc
  day_utc="$(date -u +%u)"
  case ",${days_csv// /}," in
    *",${day_utc},"*) return 0 ;;
    *) return 1 ;;
  esac
}

time_in_utc_window() {
  local window="${1:-}"
  local start end now start_m end_m
  [[ "$window" =~ ^([0-9]{2}:[0-9]{2})-([0-9]{2}:[0-9]{2})$ ]] || return 1
  start="${BASH_REMATCH[1]}"
  end="${BASH_REMATCH[2]}"
  start_m="$(hm_to_minutes "$start")" || return 1
  end_m="$(hm_to_minutes "$end")" || return 1
  now="$(utc_now_minutes)"

  if [ "$start_m" -le "$end_m" ]; then
    [ "$now" -ge "$start_m" ] && [ "$now" -lt "$end_m" ]
    return
  fi

  [ "$now" -ge "$start_m" ] || [ "$now" -lt "$end_m" ]
}

self_heal_port_owner() {
  local port="$1"
  ss -tnlp 2>/dev/null | grep ":${port}" | head -n1 || true
}

self_heal_repair_cooldown_active() {
  local cooldown_secs="$1"
  local state_file="$2"
  local now_ts last_ts elapsed
  [ "$cooldown_secs" -gt 0 ] || return 1
  [ -f "$state_file" ] || return 1
  last_ts="$(cat "$state_file" 2>/dev/null || true)"
  [[ "$last_ts" =~ ^[0-9]+$ ]] || return 1
  now_ts="$(date +%s)"
  elapsed=$((now_ts - last_ts))
  [ "$elapsed" -lt "$cooldown_secs" ]
}

mark_self_heal_repair_attempt() {
  local state_file="$1"
  mkdir -p "$(dirname "$state_file")"
  date +%s > "$state_file"
}

port_listening() {
  local port="$1"
  ss -tnl 2>/dev/null | grep -q ":${port}"
}

mt5_execution_required() {
  [ "${MT5_EXECUTION_REQUIRED:-0}" = "1" ]
}

mt5_terminal_running() {
  pgrep -af "terminal64.exe" >/dev/null 2>&1
}

wait_for_mt5_ports() {
  local wait_secs="$1"
  local phase="$2"
  local end_ts=$(( $(date +%s) + wait_secs ))
  while [ "$(date +%s)" -lt "$end_ts" ]; do
    if port_listening 9001; then
      c_ok "$phase sonrası veri/emir protokolü hazır (9001)."
      return 0
    fi
    sleep 1
  done
  c_warn "$phase sonrası portlar hâlâ hazır değil (timeout=${wait_secs}s)."
  return 1
}

force_restart_mt5_runtime() {
  c_warn "MT5 runtime zorunlu restart uygulanıyor (terminal + wineserver temizleniyor)..."
  pkill -f "terminal64.exe" >/dev/null 2>&1 || true
  wineserver -k >/dev/null 2>&1 || true
  sleep 1
  do_launch || c_warn "Zorunlu restart: launch başarısız"
  do_engine || c_warn "Zorunlu restart: engine başarısız"
}

mt5_health_status() {
  local has_terminal=0
  local has_ex5=0
  local p9001=0
  local p9002=0

  [ -f "$TERMINAL_EXE" ] && has_terminal=1
  [ -f "$EA_DIR/MemosBridge.ex5" ] && has_ex5=1
  port_listening 9001 && p9001=1
  port_listening 9002 && p9002=1

  if [ "$has_terminal" -eq 1 ] && [ "$has_ex5" -eq 1 ] && [ "$p9001" -eq 1 ]; then
    echo "healthy"
  elif [ "$has_terminal" -eq 1 ] || [ "$has_ex5" -eq 1 ] || [ "$p9001" -eq 1 ] || [ "$p9002" -eq 1 ]; then
    echo "degraded"
  else
    echo "failed"
  fi
}

mt5_health_report_line() {
  local has_terminal="no"
  local terminal_process="down"
  local has_ex5="no"
  local p9001="down"
  local p9002="down"

  [ -f "$TERMINAL_EXE" ] && has_terminal="yes"
  mt5_terminal_running && terminal_process="up"
  [ -f "$EA_DIR/MemosBridge.ex5" ] && has_ex5="yes"
  port_listening 9001 && p9001="up"
  port_listening 9002 && p9002="up"

  echo "terminal=${has_terminal} terminal_process=${terminal_process} ex5=${has_ex5} port9001=${p9001} port9002=${p9002}"
}

append_self_heal_log() {
  local level="$1"
  local status="$2"
  local attempt="$3"
  local message="$4"
  mkdir -p "$SELF_HEAL_LOG_DIR"
  local log_file="$SELF_HEAL_LOG_DIR/self_heal_$(date +%Y%m%d).jsonl"
  printf '{"ts":"%s","level":"%s","status":"%s","attempt":%s,"message":"%s","health":"%s"}\n' \
    "$(now_iso)" "$level" "$status" "$attempt" "${message//\"/\\\"}" "$(mt5_health_report_line)" >> "$log_file"
}

mt5_self_heal_degraded_count() {
  local log_file="$SELF_HEAL_LOG_DIR/self_heal_$(date +%Y%m%d).jsonl"
  [ -f "$log_file" ] || { echo 0; return 0; }
  grep -c '"status":"degraded"' "$log_file" 2>/dev/null || echo 0
}

mt5_self_heal_escalate_if_needed() {
  local status="$1"
  local threshold="${MT5_SELF_HEAL_ESCALATE_AFTER:-3}"
  local degraded_count
  degraded_count="$(mt5_self_heal_degraded_count)"

  if [ "$status" = "healthy" ]; then
    return 0
  fi

  if [ "$degraded_count" -ge "$threshold" ]; then
    append_self_heal_log "error" "$status" "0" "escalation threshold aşıldı"
    c_err "MT5 self-heal escalation: durum=$status, degraded_count=$degraded_count, eşik=$threshold"
    echo "  • Son log: $SELF_HEAL_LOG_DIR/self_heal_$(date +%Y%m%d).jsonl"
    echo "  • Öneri: ./scripts/mt5/setup_and_run.sh next-actions"
    echo "  • Öneri: ./scripts/mt5/setup_and_run.sh check"
  fi
}

install_self_heal_timer() {
  command -v systemctl >/dev/null 2>&1 || {
    c_err "systemctl bulunamadı; systemd timer kurulamadı"
    return 1
  }

  local unit_dir="$HOME/.config/systemd/user"
  local src_service="$SCRIPT_DIR/systemd/mt5-self-heal.service"
  local src_timer="$SCRIPT_DIR/systemd/mt5-self-heal.timer"
  local dst_service="$unit_dir/mt5-self-heal.service"
  local dst_timer="$unit_dir/mt5-self-heal.timer"
  local interval="${MT5_SELF_HEAL_TIMER_INTERVAL:-10min}"

  [ -f "$src_service" ] || { c_err "service şablonu yok: $src_service"; return 1; }
  [ -f "$src_timer" ] || { c_err "timer şablonu yok: $src_timer"; return 1; }

  mkdir -p "$unit_dir"

  sed \
    -e "s|__REPO_DIR__|$REPO_DIR|g" \
    -e "s|__SCRIPT_PATH__|$SCRIPT_DIR/setup_and_run.sh|g" \
    "$src_service" > "$dst_service"

  sed -e "s|__INTERVAL__|$interval|g" "$src_timer" > "$dst_timer"

  systemctl --user daemon-reload
  systemctl --user enable --now mt5-self-heal.timer
  c_ok "MT5 self-heal timer kuruldu ve başlatıldı (interval=$interval)"
  systemctl --user status mt5-self-heal.timer --no-pager || true
}

uninstall_self_heal_timer() {
  command -v systemctl >/dev/null 2>&1 || {
    c_err "systemctl bulunamadı"
    return 1
  }

  local unit_dir="$HOME/.config/systemd/user"
  systemctl --user disable --now mt5-self-heal.timer 2>/dev/null || true
  rm -f "$unit_dir/mt5-self-heal.timer" "$unit_dir/mt5-self-heal.service"
  systemctl --user daemon-reload
  c_ok "MT5 self-heal timer kaldırıldı"
}

show_self_heal_timer_status() {
  command -v systemctl >/dev/null 2>&1 || {
    c_err "systemctl bulunamadı"
    return 1
  }
  systemctl --user status mt5-self-heal.timer --no-pager || true
  systemctl --user status mt5-self-heal.service --no-pager || true
  systemctl --user list-timers --all --no-pager | grep -E "mt5-self-heal|NEXT|LAST|LEFT" || true
}

show_next_actions() {
  auto_detect_wineprefix
  echo "── En Az Manuel Müdahale Adımları ─────────────────"
  if ! mt5_terminal_running; then
    echo "1) MT5 terminalini aç: scripts/mt5/setup_and_run.sh launch"
  fi
  if ! port_listening 9001; then
    echo "2) Motoru/köprüyü ayağa kaldır: scripts/mt5/setup_and_run.sh engine"
  fi
  if [ ! -f "$EA_DIR/MemosBridge.ex5" ]; then
    echo "3) EA'yı derle: scripts/mt5/setup_and_run.sh deploy"
  fi
  echo "4) MT5 içinde tek zorunlu GUI: EA'yı grafiğe ekle + AutoTrading aç + demo hesap girişini doğrula"
  echo "5) Son durumu gör: scripts/mt5/setup_and_run.sh check"
  echo "6) Uçtan uca probe: scripts/mt5/setup_and_run.sh probe"
  echo "─────────────────────────────────────────────────────"
}

do_probe() {
  auto_detect_wineprefix
  local symbol="${MT5_PROBE_SYMBOL:-EURUSD}"
  local interval="${MT5_PROBE_INTERVAL:-1h}"
  local limit="${MT5_PROBE_LIMIT:-5}"
  local timeout_secs="${MT5_PROBE_TIMEOUT_SECS:-90}"
  local build_timeout_secs="${MT5_PROBE_BUILD_TIMEOUT_SECS:-240}"
  local probe_log="$SELF_HEAL_LOG_DIR/probe_$(date +%Y%m%d_%H%M%S).log"
  local probe_bin="$REPO_DIR/target/release/examples/download_mt5"

  mkdir -p "$SELF_HEAL_LOG_DIR"
  c_info "MT5 probe başlatılıyor: ${symbol}@mt5 ${interval} limit=${limit} run_timeout=${timeout_secs}s"

  if command -v timeout >/dev/null 2>&1; then
    c_info "Probe derlemesi: timeout=${build_timeout_secs}s"
    if ! (cd "$REPO_DIR" && timeout "${build_timeout_secs}s" cargo build --release --example download_mt5 >>"$probe_log" 2>&1); then
      c_err "MT5 probe derlemesi başarısız/timeout (ayrıntı: $probe_log)"
      append_self_heal_log "error" "probe-build-failed" "0" "mt5 probe derleme başarısız"
      return 1
    fi
  else
    c_warn "timeout komutu bulunamadı; probe derlemesi sınırsız sürebilir"
    if ! (cd "$REPO_DIR" && cargo build --release --example download_mt5 >>"$probe_log" 2>&1); then
      c_err "MT5 probe derlemesi başarısız (ayrıntı: $probe_log)"
      append_self_heal_log "error" "probe-build-failed" "0" "mt5 probe derleme başarısız"
      return 1
    fi
  fi

  local saw_bind=0
  local run_rc=0
  if command -v timeout >/dev/null 2>&1; then
    (cd "$REPO_DIR" && timeout "${timeout_secs}s" "$probe_bin" mt5 "$interval" "$symbol" "$limit" >>"$probe_log" 2>&1) &
  else
    (cd "$REPO_DIR" && "$probe_bin" mt5 "$interval" "$symbol" "$limit" >>"$probe_log" 2>&1) &
  fi
  local probe_pid=$!

  while kill -0 "$probe_pid" >/dev/null 2>&1; do
    if port_listening 9001; then
      saw_bind=1
    fi
    sleep 1
  done

  if wait "$probe_pid"; then
    run_rc=0
  else
    run_rc=$?
  fi

  if [ "$run_rc" -eq 0 ]; then
    c_ok "MT5 probe başarılı: bridge+EA veri akışı yanıt verdi (log: $probe_log)"
    append_self_heal_log "info" "probe-ok" "0" "mt5 probe başarılı (${symbol}@${interval})"
    return 0
  fi

  if [ "$saw_bind" -eq 1 ]; then
    c_err "MT5 probe başarısız: bridge bind oldu ama EA yanıtı gelmedi (log: $probe_log)"
    append_self_heal_log "error" "probe-ea-no-response" "0" "mt5 probe: bridge bind var, EA yanıt yok"
  else
    c_err "MT5 probe başarısız: bridge bind gözlenmedi (log: $probe_log)"
    append_self_heal_log "error" "probe-bind-missing" "0" "mt5 probe: bridge bind gözlenmedi"
  fi
  return 1
}

do_probe_trade() {
  auto_detect_wineprefix
  local symbol="${MT5_TRADE_PROBE_SYMBOL:-EURUSD}"
  local leverage="${MT5_TRADE_PROBE_LEVERAGE:-1}"
  local timeout_secs="${MT5_TRADE_PROBE_TIMEOUT_SECS:-90}"
  local build_timeout_secs="${MT5_TRADE_PROBE_BUILD_TIMEOUT_SECS:-240}"
  local auto_toggle_runtime="${MT5_TRADE_PROBE_AUTO_TOGGLE_RUNTIME:-1}"
  local runtime_was_toggled=0
  local runtime_was_up=0
  local probe_log="$SELF_HEAL_LOG_DIR/probe_trade_$(date +%Y%m%d_%H%M%S).log"
  local probe_bin="$REPO_DIR/target/release/examples/probe_mt5_trade"

  mkdir -p "$SELF_HEAL_LOG_DIR"
  c_info "MT5 trade-probe başlatılıyor: ${symbol} leverage=${leverage} timeout=${timeout_secs}s"

  if [ "$auto_toggle_runtime" = "1" ] && port_listening 9001; then
    if tmux has-session -t "${MEMOS_TUI_SESSION:-memos}" 2>/dev/null; then
      c_info "probe-trade: 9001 rtc_tui tarafindan kullaniliyor; probe icin runtime gecici down/up uygulanacak."
      runtime_was_up=1
      if "$REPO_DIR/memos" down >/dev/null 2>&1; then
        runtime_was_toggled=1
      else
        c_warn "probe-trade: runtime down basarisiz, probe mevcut durumla denenecek."
      fi
    fi
  fi

  restore_runtime_after_probe() {
    if [ "$runtime_was_toggled" = "1" ] && [ "$runtime_was_up" = "1" ]; then
      c_info "probe-trade: runtime tekrar ayaga kaldiriliyor."
      if "$REPO_DIR/memos" up >/dev/null 2>&1; then
        if ! tmux has-session -t "${MEMOS_TUI_SESSION:-memos}" 2>/dev/null; then
          "$REPO_DIR/scripts/tui_daemon.sh" start --release >/dev/null 2>&1 || c_warn "probe-trade: TUI restore basarisiz, elle './memos up' calistir."
        fi
      else
        c_warn "probe-trade: runtime up basarisiz, elle './memos up' calistir."
      fi
    fi
  }

  if command -v timeout >/dev/null 2>&1; then
    if ! (cd "$REPO_DIR" && timeout "${build_timeout_secs}s" cargo build --release --example probe_mt5_trade >>"$probe_log" 2>&1); then
      c_err "MT5 trade-probe derlemesi başarısız/timeout (ayrıntı: $probe_log)"
      append_self_heal_log "error" "trade-probe-build-failed" "0" "mt5 trade-probe derleme başarısız"
      return 1
    fi
    if (cd "$REPO_DIR" && timeout "${timeout_secs}s" "$probe_bin" "$symbol" "$leverage" >>"$probe_log" 2>&1); then
      c_ok "MT5 trade-probe BAŞARILI: emir kanalı hazır (ayrıntı: $probe_log)"
      append_self_heal_log "info" "trade-probe-ok" "0" "mt5 trade-probe başarılı"
      restore_runtime_after_probe
      return 0
    else
      c_err "MT5 trade-probe BAŞARISIZ: emir kanalı hazır değil (ayrıntı: $probe_log)"
      append_self_heal_log "error" "trade-probe-failed" "0" "mt5 trade-probe başarısız"
      restore_runtime_after_probe
      return 1
    fi
  else
    c_warn "timeout komutu bulunamadı; trade-probe sınırsız sürebilir"
    if ! (cd "$REPO_DIR" && cargo build --release --example probe_mt5_trade >>"$probe_log" 2>&1); then
      c_err "MT5 trade-probe derlemesi başarısız (ayrıntı: $probe_log)"
      append_self_heal_log "error" "trade-probe-build-failed" "0" "mt5 trade-probe derleme başarısız"
      return 1
    fi
    if (cd "$REPO_DIR" && "$probe_bin" "$symbol" "$leverage" >>"$probe_log" 2>&1); then
      c_ok "MT5 trade-probe BAŞARILI: emir kanalı hazır (ayrıntı: $probe_log)"
      append_self_heal_log "info" "trade-probe-ok" "0" "mt5 trade-probe başarılı"
      restore_runtime_after_probe
      return 0
    else
      c_err "MT5 trade-probe BAŞARISIZ: emir kanalı hazır değil (ayrıntı: $probe_log)"
      append_self_heal_log "error" "trade-probe-failed" "0" "mt5 trade-probe başarısız"
      restore_runtime_after_probe
      return 1
    fi
  fi
}

# ── check: durum raporu ─────────────────────────────────────────────────────────
do_check() {
  auto_detect_wineprefix
  echo "── MT5 köprü altyapı durumu ─────────────────────────"
  if command -v wine >/dev/null 2>&1; then c_ok "Wine: $(wine --version 2>/dev/null)"; else c_err "Wine kurulu DEĞİL (sudo apt install wine64 / winehq-stable)"; fi
  echo "   WINEPREFIX = $WINEPREFIX"
  [ -f "$TERMINAL_EXE" ] && c_ok "MT5 terminali: $TERMINAL_EXE" || c_warn "MT5 terminali yok ($TERMINAL_EXE) → 'install' gerekli"
  [ -f "$EDITOR_EXE" ]   && c_ok "MetaEditor: $EDITOR_EXE"      || c_warn "MetaEditor yok → EA derlenemez"
  [ -f "$EA_DIR/MemosBridge.mq5" ] && c_ok "EA kaynağı yerleştirilmiş" || c_warn "EA henüz Experts'e kopyalanmadı"
  [ -f "$EA_DIR/MemosBridge.ex5" ] && c_ok "EA derlenmiş (.ex5)"        || c_warn "EA derlenmemiş (.ex5 yok)"
  if port_listening 9001; then c_ok "Köprü dinliyor (9001) — Memos motoru ayakta"; else c_warn "Köprü (9001) dinlemiyor → motor 'engine' ile başlat"; fi
  if port_listening 9002; then
    c_info "Eski ayrı emir portu (9002) açık; mevcut EA protokolünde emirler yine 9001 üzerinden doğrulanır."
  else
    c_info "9002 ayrı listener değil; mevcut EA emirleri 9001 üzerinden işler."
  fi
  if mt5_execution_required; then
    c_info "MT5_EXECUTION_REQUIRED=1: gerçek yürütmeyi doğrulamak için 'scripts/mt5/setup_and_run.sh probe-trade' çalıştır."
  fi
  if mt5_terminal_running && ! port_listening 9001; then
    c_info "Not: MT5 köprüsü lazy-bind çalışabilir; idle durumda port görünmeyebilir. Uçtan uca teyit için 'scripts/mt5/setup_and_run.sh probe' kullan."
  fi
  c_info "Sağlık durumu: $(mt5_health_status) ($(mt5_health_report_line))"
  echo "─────────────────────────────────────────────────────"
}

# ── install: MT5 indir + kur ─────────────────────────────────────────────────────
do_install() {
  auto_detect_wineprefix
  command -v wine >/dev/null 2>&1 || { c_err "Wine yok — önce kur."; exit 1; }
  if [ -f "$TERMINAL_EXE" ]; then c_ok "MT5 zaten kurulu: $TERMINAL_EXE"; return 0; fi
  if [ ! -f "$SETUP_EXE" ]; then
    c_info "MT5 kurulumu indiriliyor: $MT5_URL"
    if command -v curl >/dev/null 2>&1; then curl -fL --retry 3 -o "$SETUP_EXE" "$MT5_URL";
    else wget -O "$SETUP_EXE" "$MT5_URL"; fi
  fi
  c_info "MT5 sessiz kuruluyor (wine mt5setup.exe /auto) — birkaç dakika sürebilir..."
  wine "$SETUP_EXE" /auto || c_warn "Kurulumcu çıkış kodu ≠0 (sihirbaz GUI'si manuel ilerletme isteyebilir)"
  # terminal64.exe oluşana kadar bekle (≤120sn)
  for _ in $(seq 1 60); do [ -f "$TERMINAL_EXE" ] && break; sleep 2; done
  [ -f "$TERMINAL_EXE" ] && c_ok "MT5 kuruldu: $TERMINAL_EXE" || { c_err "Kurulum tamamlanamadı (terminal64.exe yok). Kurulum sihirbazını GUI'de tamamlayıp tekrar dene."; exit 1; }
}

# ── deploy: EA kopyala + derle ───────────────────────────────────────────────────
do_deploy() {
  auto_detect_wineprefix
  [ -f "$EA_SRC" ] || { c_err "EA kaynağı yok: $EA_SRC"; exit 1; }
  mkdir -p "$EA_DIR"
  cp -f "$EA_SRC" "$EA_DIR/MemosBridge.mq5"
  c_ok "EA kopyalandı → $EA_DIR/MemosBridge.mq5"
  if [ -f "$EDITOR_EXE" ]; then
    c_info "EA derleniyor (MetaEditor /compile)..."
    # MetaEditor Windows-yolu ister; portable veri klasörü kurulum dizininde.
    wine "$EDITOR_EXE" /compile:"C:\\Program Files\\MetaTrader 5\\MQL5\\Experts\\MemosBridge.mq5" /log 2>/dev/null || true
    sleep 2
    [ -f "$EA_DIR/MemosBridge.ex5" ] && c_ok "EA derlendi: MemosBridge.ex5" || c_warn "Otomatik derleme doğrulanamadı → MetaEditor'da F7 ile elle derle"
  else
    c_warn "MetaEditor yok → EA'yı MT5 içindeki MetaEditor'da (F7) elle derle"
  fi
}

# ── launch: MT5 terminalini başlat ───────────────────────────────────────────────
do_launch() {
  auto_detect_wineprefix
  [ -f "$TERMINAL_EXE" ] || { c_err "MT5 kurulu değil → önce 'install'"; exit 1; }
  c_info "MT5 terminali başlatılıyor (/portable)..."
  mkdir -p "$SELF_HEAL_LOG_DIR"
  local terminal_log="$SELF_HEAL_LOG_DIR/terminal_$(date +%Y%m%d).log"
  if command -v systemd-run >/dev/null 2>&1; then
    systemd-run --user --scope --quiet \
      --unit=mt5-terminal \
      --property=CollectMode=inactive \
      --property=KillMode=process \
      /usr/bin/env bash -lc "cd '$INSTALL_DIR' && exec wine '$TERMINAL_EXE' /portable >>'$terminal_log' 2>&1" || {
      c_warn "systemd-run ile başlatma başarısız; fallback kullanılıyor"
      ( cd "$INSTALL_DIR" && nohup setsid wine "$TERMINAL_EXE" /portable >>"$terminal_log" 2>&1 < /dev/null & )
    }
  else
    ( cd "$INSTALL_DIR" && nohup setsid wine "$TERMINAL_EXE" /portable >>"$terminal_log" 2>&1 < /dev/null & )
  fi
  sleep 2
  if mt5_terminal_running; then
    c_ok "MT5 başlatıldı (log: $terminal_log). GUI'de demo hesaba giriş yap + EA'yı grafiğe ekle."
  else
    c_warn "MT5 terminali başlatma sinyali verildi ama süreç henüz görünmüyor; log: $terminal_log"
  fi
}

# ── engine: Memos motorunu başlat (= köprü server). Profil varsayılan korunur. ───
do_engine() {
  cd "$REPO_DIR"
  if [ -n "${MT5_ENGINE_PROFILE:-}" ]; then
    c_info "Memos motoru '${MT5_ENGINE_PROFILE}' profiliyle başlatılıyor (Rust = köprü TCP server :${BRIDGE_ADDR##*:})..."
    ./memos use "$MT5_ENGINE_PROFILE"
  else
    c_info "Memos motoru aktif profille başlatılıyor (profil korunur). İstersen MT5_ENGINE_PROFILE=paper-mt5 ile zorlayabilirsin."
  fi
  ./memos up
  c_ok "Motor ayakta. EA bağlanınca köprü ${BRIDGE_ADDR} üzerinden veri akar."
}

# ── manuel adımlar hatırlatıcısı ────────────────────────────────────────────────
do_self_heal() {
  auto_detect_wineprefix
  local dry_run="${MT5_SELF_HEAL_DRY_RUN:-0}"
  local check_only="${MT5_SELF_HEAL_CHECK_ONLY:-0}"
  local policy="${MT5_SELF_HEAL_POLICY:-legacy}"
  local repair_window_utc="${MT5_SELF_HEAL_REPAIR_WINDOW_UTC:-}"
  local repair_mode="${MT5_SELF_HEAL_REPAIR_MODE:-auto}"
  local soft_window_utc="${MT5_SELF_HEAL_SOFT_WINDOW_UTC:-$repair_window_utc}"
  local full_window_utc="${MT5_SELF_HEAL_FULL_WINDOW_UTC:-}"
  local repair_days_utc="${MT5_SELF_HEAL_REPAIR_DAYS_UTC:-1,2,3,4,5,6,7}"
  local soft_days_utc="${MT5_SELF_HEAL_SOFT_DAYS_UTC:-$repair_days_utc}"
  local full_days_utc="${MT5_SELF_HEAL_FULL_DAYS_UTC:-$repair_days_utc}"
  local repair_cooldown_secs="${MT5_SELF_HEAL_REPAIR_COOLDOWN_SECS:-21600}"
  local repair_state_file="${MT5_SELF_HEAL_REPAIR_STATE_FILE:-$REPO_DIR/logs/.mt5-self-heal-repair.last}"
  local port_guard="${MT5_SELF_HEAL_PORT_GUARD:-1}"
  local max_attempts="${MT5_SELF_HEAL_MAX_ATTEMPTS:-3}"
  local auto_restart="${MT5_SELF_HEAL_AUTO_RESTART:-1}"
  local restart_budget="${MT5_SELF_HEAL_RESTART_BUDGET:-1}"
  local port_wait_secs="${MT5_SELF_HEAL_PORT_WAIT_SECS:-20}"
  local force_restart_on_degraded="${MT5_SELF_HEAL_FORCE_RESTART_ON_DEGRADED:-1}"
  local restart_count=0
  local attempt=1
  local need_install=0
  local need_deploy=0
  local need_launch=0
  local need_engine=0
  local repair_actions_taken=0
  local selected_repair_mode="none"
  local mode_state_file=""

  case "$policy" in
    legacy|smart|check-only|always-repair) ;;
    *)
      c_warn "Bilinmeyen MT5_SELF_HEAL_POLICY='$policy'; legacy kullanılacak."
      policy="legacy"
      ;;
  esac

  if [ "$policy" = "check-only" ]; then
    check_only="1"
  elif [ "$policy" = "always-repair" ]; then
    check_only="0"
  fi

  if [ "$check_only" = "1" ]; then
    local status
    status="$(mt5_health_status)"
    append_self_heal_log "info" "$status" "0" "self-heal check-only durum raporu"
    c_info "Check-only mod: onarım adımları atlandı (MT5_SELF_HEAL_CHECK_ONLY=1)."
    do_check
    if [ "$status" = "healthy" ]; then
      return 0
    fi
    if [ "$status" = "degraded" ]; then
      return 2
    fi
    return 1
  fi

  if [ "$policy" = "smart" ]; then
    local status_smart owner_9001
    status_smart="$(mt5_health_status)"
    if [ "$status_smart" = "healthy" ]; then
      append_self_heal_log "info" "healthy" "0" "self-heal smart: healthy, repair atlandı"
      do_check
      return 0
    fi

    if ! utc_day_allowed "$repair_days_utc"; then
      append_self_heal_log "warn" "$status_smart" "0" "self-heal smart: gün penceresi dışı, check-only"
      c_info "Smart policy: bugün repair günü değil, check-only raporlandı."
      do_check
      [ "$status_smart" = "degraded" ] && return 2
      return 1
    fi

    case "$repair_mode" in
      auto)
        if utc_day_allowed "$full_days_utc" && [ -n "$full_window_utc" ] && time_in_utc_window "$full_window_utc"; then
          selected_repair_mode="full"
        elif utc_day_allowed "$soft_days_utc" && [ -n "$soft_window_utc" ] && time_in_utc_window "$soft_window_utc"; then
          selected_repair_mode="soft"
        else
          selected_repair_mode="none"
        fi
        ;;
      soft)
        if utc_day_allowed "$soft_days_utc" && [ -n "$soft_window_utc" ] && time_in_utc_window "$soft_window_utc"; then
          selected_repair_mode="soft"
        else
          selected_repair_mode="none"
        fi
        ;;
      full)
        if utc_day_allowed "$full_days_utc" && [ -n "$full_window_utc" ] && time_in_utc_window "$full_window_utc"; then
          selected_repair_mode="full"
        else
          selected_repair_mode="none"
        fi
        ;;
      *)
        c_warn "Bilinmeyen MT5_SELF_HEAL_REPAIR_MODE='$repair_mode'; auto kullanılacak."
        if utc_day_allowed "$full_days_utc" && [ -n "$full_window_utc" ] && time_in_utc_window "$full_window_utc"; then
          selected_repair_mode="full"
        elif utc_day_allowed "$soft_days_utc" && [ -n "$soft_window_utc" ] && time_in_utc_window "$soft_window_utc"; then
          selected_repair_mode="soft"
        else
          selected_repair_mode="none"
        fi
        ;;
    esac

    if [ "$selected_repair_mode" = "none" ]; then
      append_self_heal_log "warn" "$status_smart" "0" "self-heal smart: mod penceresi dışı, check-only"
      c_info "Smart policy: repair penceresi dışında, check-only raporlandı."
      do_check
      [ "$status_smart" = "degraded" ] && return 2
      return 1
    fi

    mode_state_file="${repair_state_file}.${selected_repair_mode}"
    if self_heal_repair_cooldown_active "$repair_cooldown_secs" "$mode_state_file"; then
      append_self_heal_log "warn" "$status_smart" "0" "self-heal smart: cooldown aktif, check-only"
      c_info "Smart policy: repair cooldown aktif, check-only raporlandı."
      do_check
      [ "$status_smart" = "degraded" ] && return 2
      return 1
    fi

    if [ "$port_guard" = "1" ] && port_listening 9001; then
      owner_9001="$(self_heal_port_owner 9001)"
      if [ -n "$owner_9001" ] && ! echo "$owner_9001" | grep -q "rtc_tui"; then
        append_self_heal_log "error" "$status_smart" "0" "self-heal smart: 9001 yabancı owner guard"
        c_err "Smart policy: 9001 beklenmeyen owner tarafından tutuluyor; repair güvenlik nedeniyle atlandı."
        do_check
        [ "$status_smart" = "degraded" ] && return 2
        return 1
      fi
    fi

    append_self_heal_log "info" "$status_smart" "0" "self-heal smart: kontrollü repair penceresi aktif (mode=${selected_repair_mode})"
  fi

  while [ "$attempt" -le "$max_attempts" ]; do
    need_install=0
    need_deploy=0
    need_launch=0
    need_engine=0

    if [ ! -f "$TERMINAL_EXE" ]; then
      need_install=1
    fi

    if [ ! -f "$EA_DIR/MemosBridge.ex5" ]; then
      need_deploy=1
    fi

    if ! mt5_terminal_running; then
      need_launch=1
    fi

    if ! port_listening 9001; then
      need_launch=1
      need_engine=1
    fi

    if [ "${MT5_AUTO_REPAIR:-0}" = "1" ]; then
      need_install=1
      need_deploy=1
      need_launch=1
      need_engine=1
    elif [ "${MT5_AUTO_LAUNCH:-0}" = "1" ]; then
      need_launch=1
      need_engine=1
    elif [ "${MT5_AUTO_SETUP:-0}" = "1" ]; then
      need_install=1
      need_deploy=1
    fi

    if [ "$policy" = "smart" ] && [ "$selected_repair_mode" = "soft" ]; then
      # Soft modda sadece runtime toparlama (launch/engine) uygulanır.
      need_install=0
      need_deploy=0
    fi

    echo "── MT5 self-heal plan (deneme $attempt/$max_attempts) ───"
    echo "  install: $([ "$need_install" -eq 1 ] && echo yes || echo no)"
    echo "  deploy : $([ "$need_deploy" -eq 1 ] && echo yes || echo no)"
    echo "  launch : $([ "$need_launch" -eq 1 ] && echo yes || echo no)"
    echo "  engine : $([ "$need_engine" -eq 1 ] && echo yes || echo no)"
    echo "  health : $(mt5_health_status) ($(mt5_health_report_line))"
    echo "──────────────────────────────────────────────────"

    if [ "$dry_run" = "1" ]; then
      append_self_heal_log "info" "dry-run" "$attempt" "self-heal dry-run plan üretildi"
      c_info "Dry-run modu: gerçek adım atlanıyor."
      return 0
    fi

    if [ "$need_install" -eq 1 ]; then
      repair_actions_taken=1
      if ! do_install; then
        c_warn "install adımı başarısız oldu; self-heal'e devam ediliyor"
      fi
    else
      c_ok "MT5 kurulu görünüyordu; install atlanıyor"
    fi

    if [ "$need_deploy" -eq 1 ]; then
      repair_actions_taken=1
      if ! do_deploy; then
        c_warn "deploy adımı başarısız oldu; self-heal'e devam ediliyor"
      fi
    else
      c_ok "EA derlenmiş görünüyordu; deploy atlanıyor"
    fi

    if [ "$need_launch" -eq 1 ]; then
      repair_actions_taken=1
      if ! do_launch; then
        c_warn "launch adımı başarısız oldu; self-heal'e devam ediliyor"
      fi
      wait_for_mt5_ports "$port_wait_secs" "launch" || true
    else
      c_ok "MT5 terminali görünür durumda; launch atlanıyor"
    fi

    if [ "$need_engine" -eq 1 ]; then
      repair_actions_taken=1
      if ! do_engine; then
        c_warn "engine adımı başarısız oldu; self-heal'e devam ediliyor"
        append_self_heal_log "warn" "engine-failed" "$attempt" "engine adımı başarısız"
      fi
      wait_for_mt5_ports "$port_wait_secs" "engine" || true
    else
      c_ok "Köprü/port görünür durumda; engine atlanıyor"
    fi

    local status
    status="$(mt5_health_status)"
    append_self_heal_log "info" "$status" "$attempt" "self-heal denemesi tamamlandı"

    if [ "$status" = "healthy" ]; then
      if [ "$policy" = "smart" ] && [ "$repair_actions_taken" -eq 1 ]; then
        mark_self_heal_repair_attempt "${mode_state_file:-$repair_state_file}"
      fi
      c_ok "MT5 self-heal başarıyla sağlık kontrolünü geçti (healthy)."
      do_check
      return 0
    fi

    if [ "$status" = "degraded" ] && [ "$auto_restart" = "1" ] \
      && [ "$restart_count" -lt "$restart_budget" ] && ! port_listening 9001; then
      c_warn "Sağlık durumu degraded; yeniden başlatma uygulanıyor ($((restart_count + 1))/$restart_budget)..."
      append_self_heal_log "warn" "degraded" "$attempt" "otomatik yeniden başlatma tetiklendi"
      if [ "$force_restart_on_degraded" = "1" ]; then
        repair_actions_taken=1
        force_restart_mt5_runtime
      else
        repair_actions_taken=1
        do_launch || c_warn "Auto-restart launch adımı başarısız"
        do_engine || c_warn "Auto-restart engine adımı başarısız"
      fi
      wait_for_mt5_ports "$port_wait_secs" "auto-restart" || true
      restart_count=$((restart_count + 1))
      status="$(mt5_health_status)"
      append_self_heal_log "info" "$status" "$attempt" "otomatik yeniden başlatma sonrası durum"
      if [ "$status" = "healthy" ]; then
        if [ "$policy" = "smart" ] && [ "$repair_actions_taken" -eq 1 ]; then
          mark_self_heal_repair_attempt "${mode_state_file:-$repair_state_file}"
        fi
        c_ok "Auto-restart sonrası durum healthy."
        do_check
        return 0
      fi
    fi

    if [ "$attempt" -lt "$max_attempts" ]; then
      c_warn "MT5 sağlık kontrolü henüz tamamlanmadı; yeniden deneniyor..."
      append_self_heal_log "warn" "$status" "$attempt" "sağlık tamamlanmadı, tekrar denenecek"
      attempt=$((attempt + 1))
      sleep 2
    else
      if [ "$status" = "degraded" ]; then
        c_warn "MT5 self-heal degraded durumda sonlandı."
        append_self_heal_log "warn" "degraded" "$attempt" "max deneme sonrası degraded"
        mt5_self_heal_escalate_if_needed "$status"
      else
        c_err "MT5 self-heal failed durumda sonlandı."
        append_self_heal_log "error" "failed" "$attempt" "max deneme sonrası failed"
        mt5_self_heal_escalate_if_needed "$status"
      fi
      if [ "$policy" = "smart" ] && [ "$repair_actions_taken" -eq 1 ]; then
        mark_self_heal_repair_attempt "${mode_state_file:-$repair_state_file}"
      fi
      do_check
      if [ "$status" = "degraded" ]; then
        return 2
      else
        return 1
      fi
    fi
  done

  if [ "$policy" = "smart" ] && [ "$repair_actions_taken" -eq 1 ]; then
    mark_self_heal_repair_attempt "${mode_state_file:-$repair_state_file}"
  fi
}

print_manual() {
  cat <<EOF

╔══════════════════════════════════════════════════════════════════╗
║  KALAN MANUEL ADIMLAR (GUI/hesap — script otomatikleştiremez)      ║
╚══════════════════════════════════════════════════════════════════╝
  1) MT5'te ÜCRETSİZ DEMO hesap aç (File › Open an Account → MetaQuotes
     Demo) — forex/altın verisi + sembol listesi hesapsız GELMEZ.
  2) Market Watch'a sembolleri ekle: EURUSD, GBPUSD, XAUUSD, ... (sağ tık
     › Show All) — profildeki PINNED_SYMBOLS ile eşleşmeli.
  3) Soket izni: Tools › Options › Expert Advisors → "Allow algorithmic
     trading" + adres listesine 127.0.0.1 ekle.
  4) MemosBridge EA'yı bir grafiğe SÜRÜKLE. Girdiler: InpHost=127.0.0.1,
     InpPort=${BRIDGE_ADDR##*:} (veri köprüsü), ve emir motoru için 127.0.0.1:${EXECUTOR_ADDR##*:}.
     (Faz 2 emir yürütme için InpEnableExec=true + AutoTrading.)
  5) Geçmiş veri çek (motor ayaktayken):
       cargo run --release --example download_mt5 -- mt5 1h EURUSD,GBPUSD,XAUUSD 2000
  6) Durumu izle: ./memos attach   ·   köprü kontrol: scripts/mt5/setup_and_run.sh check
EOF
}

case "${1:-all}" in
  check)   do_check ;;
  install) do_install ;;
  deploy)  do_deploy ;;
  launch)  do_launch ;;
  engine)  do_engine ;;
  self-heal) do_self_heal ;;
  install-timer) install_self_heal_timer ;;
  uninstall-timer) uninstall_self_heal_timer ;;
  timer-status) show_self_heal_timer_status ;;
  next-actions) show_next_actions ;;
  probe) do_probe ;;
  probe-trade) do_probe_trade ;;
  all)     do_check; do_install; do_deploy; do_launch; do_engine; print_manual ;;
  *)       c_err "bilinmeyen komut: $1"; echo "kullanım: $0 [check|install|deploy|launch|engine|self-heal|install-timer|uninstall-timer|timer-status|next-actions|probe|probe-trade|all]"; exit 1 ;;
esac
