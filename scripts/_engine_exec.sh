#!/usr/bin/env bash
# scripts/_engine_exec.sh — DAHİLİ: .launch.conf env'ini yükle → verilen binary'i exec et.
#
# Daemon yolları (tui_daemon.sh tmux + engine.sh nohup) bunu sarmalayıcı olarak kullanır → daemon
# motoru interaktif çalıştırmayla AYNI .launch.conf env'ini alır (XS/graded/rejim sessizce kapanmaz).
# Doğrudan çağırma; tui_daemon/engine üzerinden gelir. Kullanım: _engine_exec.sh <binary> [arg...]
set -u
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR" || exit 1
# shellcheck source=scripts/lib_launchconf.sh
. "$REPO_DIR/scripts/lib_launchconf.sh"
load_launch_conf "$REPO_DIR/scripts/.launch.conf"
exec "$@"
