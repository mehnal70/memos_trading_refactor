#!/usr/bin/env bash
# scripts/lib_launchconf.sh — .launch.conf RUNTIME env yükleyici (TEK KAYNAK).
#
# launch.sh menüsünün yazdığı scripts/.launch.conf'taki çalışma-zamanı env'lerini export eder.
# İnteraktif (run_rtc.sh) ve daemon (tui_daemon.sh/engine.sh) yolları AYNI env'i alsın diye ORTAK
# nokta — aksi halde daemon binary'i çıplak koşar ve XS/graded/rejim ayarları sessizce kapanırdı
# ([[project_daemon_env_pending]] kökü). BUILD_MODE/TARGET build-meta'dır → runtime env değil, atlanır.
# Satır-satır okunur (keyfi bash eval YOK) → conf değerleri güvenle export edilir.
#
# Kullanım:  . scripts/lib_launchconf.sh ; load_launch_conf [conf_yolu]
load_launch_conf() {
  local conf="${1:-scripts/.launch.conf}"
  [ -f "$conf" ] || return 0
  local k v
  while IFS='=' read -r k v; do
    [ -z "${k:-}" ] && continue
    case "$k" in
      \#*) continue ;;                # yorum satırı
      BUILD_MODE|TARGET) continue ;;  # build-meta, runtime env değil
    esac
    [ -n "${v:-}" ] && export "$k=$v"
  done < "$conf"
}
