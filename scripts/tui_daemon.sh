#!/usr/bin/env bash
# scripts/tui_daemon.sh — TUI'yi (rtc_tui) DETACHED tmux oturumunda 7/24 koş + yönet.
#
# TUI bir TTY ister; tmux detached oturum sayesinde terminal/SSH kapansa da yaşar. İstediğin an
# `attach` ile panelleri canlı izlersin (Ctrl-b d ile bırak → motor çalışmaya devam eder).
# watchdog.sh WATCHDOG_TARGET=tui ile bu script'i kullanarak ölü/hung TUI'yi yeniden başlatır.
#
# ⚠️ TEK-MOTOR: Aynı anda headless (engine.sh) ÇALIŞMAMALI — ikisi aynı DB'ye yazar (bozulma riski).
#
# Kullanım:
#   ./scripts/tui_daemon.sh start [--release]   # detached tmux'ta başlat
#   ./scripts/tui_daemon.sh attach              # panelleri izle (Ctrl-b d ile bırak)
#   ./scripts/tui_daemon.sh stop                # oturumu kapat
#   ./scripts/tui_daemon.sh restart [--release] # stop + start
#   ./scripts/tui_daemon.sh status              # oturum + son heartbeat

set -u
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR" || exit 1

SESSION="${MEMOS_TUI_SESSION:-memos}"
HEARTBEAT_LOG="logs/heartbeat.jsonl"
ENGINE_PID_FILE="logs/.engine.pid"
mkdir -p logs

alive() { tmux has-session -t "$SESSION" 2>/dev/null; }

cmd_start() {
    local mode="debug"; for a in "$@"; do [ "$a" = "--release" ] && mode="release"; done
    local binary="target/$mode/rtc_tui"
    if [ ! -x "$binary" ]; then
        echo "❌ Binary yok: $binary  → ./scripts/run_rtc.sh --tui${mode:+ --release} --build-only (ya da cargo build)"
        exit 1
    fi
    # Tek-motor koruması: headless çalışıyorsa reddet.
    if [ -f "$ENGINE_PID_FILE" ] && kill -0 "$(cat "$ENGINE_PID_FILE" 2>/dev/null)" 2>/dev/null; then
        echo "❌ Headless motor çalışıyor (PID $(cat "$ENGINE_PID_FILE")). Önce: ./scripts/engine.sh stop"
        exit 1
    fi
    if alive; then echo "ℹ️  TUI zaten çalışıyor (tmux '$SESSION'). 'attach' ya da 'restart'."; exit 0; fi
    tmux new-session -d -s "$SESSION" "$binary"
    sleep 1
    if alive; then
        echo "✅ TUI başlatıldı (tmux '$SESSION', $mode). İzle: ./scripts/tui_daemon.sh attach"
    else
        echo "❌ TUI 1sn içinde çıktı (binary/TTY sorunu?)."; exit 1
    fi
}

cmd_stop()   { if alive; then tmux kill-session -t "$SESSION"; echo "🛑 TUI durduruldu ('$SESSION')."; else echo "ℹ️  Çalışan TUI yok."; fi; }
cmd_attach() { if alive; then tmux attach -t "$SESSION"; else echo "ℹ️  Çalışan TUI yok ('$SESSION'). Önce 'start'."; fi; }
cmd_status() {
    if alive; then echo "✅ TUI canlı (tmux '$SESSION')"; else echo "❌ TUI yok ('$SESSION')"; fi
    if [ -f "$HEARTBEAT_LOG" ]; then
        local age=$(( $(date +%s) - $(stat -c %Y "$HEARTBEAT_LOG" 2>/dev/null || echo 0) ))
        echo "   heartbeat yaşı: ${age}s (son satır):"; tail -1 "$HEARTBEAT_LOG" 2>/dev/null
    fi
}

case "${1:-status}" in
    start)   shift; cmd_start "$@" ;;
    stop)    cmd_stop ;;
    restart) shift; cmd_stop; sleep 1; cmd_start "$@" ;;
    attach)  cmd_attach ;;
    status)  cmd_status ;;
    *) echo "kullanım: $0 {start [--release]|attach|stop|restart [--release]|status}"; exit 1 ;;
esac
