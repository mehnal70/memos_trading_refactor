#!/usr/bin/env bash
# scripts/engine.sh — Memos Trading Engine yönetim CLI'ı
#
# Kullanım:
#   ./scripts/engine.sh              # interaktif menü (varsayılan)
#   ./scripts/engine.sh menu         # aynı menü
#   ./scripts/engine.sh start        # arka planda başlat (debug binary)
#   ./scripts/engine.sh start --release
#   ./scripts/engine.sh stop         # graceful kapat (SIGTERM, 5s sonra SIGKILL)
#   ./scripts/engine.sh status       # process + son heartbeat tick
#   ./scripts/engine.sh restart      # stop + start
#   ./scripts/engine.sh tail         # heartbeat'i okunabilir canlı izle
#   ./scripts/engine.sh trades       # trades.jsonl canlı izle
#   ./scripts/engine.sh logs [N]     # son N satır (default 50)
#   ./scripts/engine.sh build [--release]
#
# Tek-engine garantisi: start çağrısı önce PID dosyasını kontrol eder; canlı
# süreç varsa "zaten çalışıyor" diyerek geri çekilir → çift engine = DB çakışması
# riskine düşmez.

# set -e KULLANILMAZ — read_pid (dosya yok), pid_alive (kill -0 fail), tail -F
# Ctrl+C gibi yerlerde non-zero exit beklenen davranıştır. Manuel hata
# yönetimi `|| true` ve explicit `exit 1` ile yapılır.

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

PID_FILE="logs/.engine.pid"
STDOUT_LOG="logs/engine_stdout.log"
STDERR_LOG="logs/engine_stderr.log"
HEARTBEAT_LOG="logs/heartbeat.jsonl"
TRADES_LOG="logs/trades.jsonl"

mkdir -p logs

# ─── yardımcılar ─────────────────────────────────────────────────────────────

pid_alive() {
    local pid="$1"
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

read_pid() {
    [ -f "$PID_FILE" ] && cat "$PID_FILE"
}

clear_pid() {
    rm -f "$PID_FILE"
}

# Heartbeat satırını okunabilir biçime çevirir (jq varsa onu kullanır,
# yoksa python3 fallback). Stdin'den satır okur, stdout'a basar.
format_heartbeat() {
    if command -v jq >/dev/null 2>&1; then
        jq -c '{tick,phase,open:.open_positions,closed:.closed_trades,anom:.anomalies,ml:.ml_confidence,gbt:.gbt_ready}'
    else
        python3 -c '
import sys, json
for line in sys.stdin:
    try:
        r = json.loads(line)
        print(f"tick={r[\"tick\"]:>4} phase={r[\"phase\"]:<10} open={r[\"open_positions\"]} closed={r[\"closed_trades\"]} anom={r[\"anomalies\"]:>2} ml={r[\"ml_confidence\"]:.3f} gbt={r[\"gbt_ready\"]}")
        sys.stdout.flush()
    except Exception:
        pass
'
    fi
}

# ─── komutlar ────────────────────────────────────────────────────────────────

cmd_start() {
    local mode="debug"
    for arg in "$@"; do
        [ "$arg" = "--release" ] && mode="release"
    done

    local binary="target/$mode/rtc_headless"
    if [ ! -x "$binary" ]; then
        echo "❌ Binary yok: $binary"
        echo "   Önce derle:  ./scripts/engine.sh build${mode:+ --release}"
        exit 1
    fi

    local existing
    existing=$(read_pid)
    if pid_alive "$existing"; then
        echo "ℹ️  Engine zaten çalışıyor (PID $existing). 'stop' veya 'restart' kullan."
        exit 0
    fi

    # Stale PID temizliği
    clear_pid

    nohup "$binary" > "$STDOUT_LOG" 2> "$STDERR_LOG" &
    local new_pid=$!
    echo "$new_pid" > "$PID_FILE"

    sleep 1
    if pid_alive "$new_pid"; then
        echo "✅ Engine başlatıldı (PID $new_pid, $mode binary)"
        echo "   stderr → $STDERR_LOG"
        echo "   stdout → $STDOUT_LOG"
        echo "   tail   → ./scripts/engine.sh tail"
    else
        echo "❌ Engine 1 saniye içinde çakıldı — stderr son satırlar:"
        tail -10 "$STDERR_LOG" 2>/dev/null
        clear_pid
        exit 1
    fi
}

cmd_stop() {
    local pid
    pid=$(read_pid)
    if ! pid_alive "$pid"; then
        echo "ℹ️  Çalışan engine yok (PID dosya: ${pid:-yok})"
        clear_pid
        return 0
    fi

    echo "⏳ SIGTERM gönderiliyor (PID $pid)…"
    kill -TERM "$pid" 2>/dev/null || true

    # 5 sn graceful bekle
    for i in $(seq 1 5); do
        if ! pid_alive "$pid"; then
            echo "✅ Engine kapandı (graceful, ${i}sn)"
            clear_pid
            return 0
        fi
        sleep 1
    done

    echo "⚠️  Engine SIGTERM'e cevap vermedi, SIGKILL gönderiliyor…"
    kill -KILL "$pid" 2>/dev/null || true
    sleep 1
    if pid_alive "$pid"; then
        echo "❌ Engine hâlâ çalışıyor — manuel müdahale gerek (PID $pid)"
        exit 1
    fi
    echo "✅ Engine zorla kapatıldı"
    clear_pid
}

cmd_status() {
    local pid
    pid=$(read_pid)
    if pid_alive "$pid"; then
        local started
        started=$(ps -o lstart= -p "$pid" 2>/dev/null | xargs -I{} echo {})
        local uptime
        uptime=$(ps -o etime= -p "$pid" 2>/dev/null | tr -d ' ')
        echo "✅ Engine ÇALIŞIYOR — PID $pid · uptime $uptime · başlangıç $started"
    else
        echo "⛔ Engine ÇALIŞMIYOR (PID dosya: ${pid:-yok})"
        return 0
    fi

    if [ -s "$HEARTBEAT_LOG" ]; then
        local last_tick
        last_tick=$(tail -1 "$HEARTBEAT_LOG")
        echo "📡 Son heartbeat:"
        echo "$last_tick" | format_heartbeat
    fi

    if [ -s "$TRADES_LOG" ]; then
        local n_trades
        n_trades=$(wc -l < "$TRADES_LOG")
        local size
        size=$(du -h "$TRADES_LOG" | awk '{print $1}')
        echo "💱 trades.jsonl: $n_trades satır / $size"
    fi
}

cmd_restart() {
    cmd_stop
    sleep 1
    cmd_start "$@"
}

cmd_tail() {
    if [ ! -f "$HEARTBEAT_LOG" ]; then
        echo "ℹ️  Henüz heartbeat yok. Engine başladı mı? './scripts/engine.sh status'"
        exit 1
    fi
    echo "📡 Heartbeat canlı izleme (Ctrl+C ile çık — engine durmaz):"
    tail -F "$HEARTBEAT_LOG" | format_heartbeat
}

cmd_trades() {
    if [ ! -f "$TRADES_LOG" ]; then
        echo "ℹ️  Henüz trades.jsonl yok."
        exit 1
    fi
    echo "💱 Trade event'leri canlı (Ctrl+C ile çık):"
    if command -v jq >/dev/null 2>&1; then
        tail -F "$TRADES_LOG" | jq -c '{t:.event_type,sym:.symbol,sig:.signal,price,pnl,msg:.message}'
    else
        tail -F "$TRADES_LOG" | python3 -c '
import sys, json
for line in sys.stdin:
    try:
        r = json.loads(line)
        print(f"[{r[\"event_type\"]:<11}] {r[\"symbol\"]:<10} {r[\"signal\"]:<6} @{r[\"price\"]:<10.4f} pnl={r[\"pnl\"]:+.4f}  — {r[\"message\"]}")
        sys.stdout.flush()
    except Exception: pass
'
    fi
}

cmd_logs() {
    local n="${1:-50}"
    echo "── stderr son $n satır ──"
    tail -n "$n" "$STDERR_LOG" 2>/dev/null || echo "(stderr boş)"
    echo
    echo "── robotic_trading.log son $n satır ──"
    tail -n "$n" logs/robotic_trading.log 2>/dev/null || echo "(yok)"
}

cmd_build() {
    local profile=""
    for arg in "$@"; do
        [ "$arg" = "--release" ] && profile="--release"
    done
    echo "🔨 cargo build --bin rtc_headless $profile"
    cargo build --bin rtc_headless $profile
}

cmd_help() {
    sed -n '2,18p' "$0"
}

# Engine durumunu menü başlığı için tek satıra sıkıştırır.
status_oneline() {
    local pid
    pid=$(read_pid)
    if pid_alive "$pid"; then
        local up
        up=$(ps -o etime= -p "$pid" 2>/dev/null | tr -d ' ')
        local last_tick="-"
        if [ -s "$HEARTBEAT_LOG" ]; then
            last_tick=$(tail -1 "$HEARTBEAT_LOG" | grep -oE '"tick":[0-9]+' | head -1 | cut -d: -f2)
        fi
        echo "🟢 ÇALIŞIYOR · PID $pid · uptime $up · tick $last_tick"
    else
        echo "🔴 KAPALI"
    fi
}

cmd_menu() {
    while true; do
        clear
        echo "╔══════════════════════════════════════════════════════════╗"
        echo "║   MEMOS TRADING ENGINE — YÖNETİM PANELİ                  ║"
        echo "╚══════════════════════════════════════════════════════════╝"
        echo "Durum:  $(status_oneline)"
        echo
        PS3=$'\nSeçim (numara) > '
        # Bash select kullan; ekstra bağımlılık yok
        select choice in \
            "Start (debug)" \
            "Start (release)" \
            "Stop" \
            "Restart" \
            "Status (detaylı)" \
            "Heartbeat tail (Ctrl+C ile döner)" \
            "Trades tail (Ctrl+C ile döner)" \
            "Logs (son 50 satır)" \
            "Logs (son 200 satır)" \
            "Build (debug)" \
            "Build (release)" \
            "Yenile (menüyü yeniden çiz)" \
            "Çıkış"
        do
            case "$REPLY" in
                1)  cmd_start ;;
                2)  cmd_start --release ;;
                3)  cmd_stop ;;
                4)  cmd_restart ;;
                5)  cmd_status ;;
                6)  cmd_tail || true ;;
                7)  cmd_trades || true ;;
                8)  cmd_logs 50 ;;
                9)  cmd_logs 200 ;;
                10) cmd_build ;;
                11) cmd_build --release ;;
                12) break ;;  # menüyü yenile
                13) echo "Görüşmek üzere."; return 0 ;;
                *)  echo "Geçersiz seçim: $REPLY" ;;
            esac
            echo
            echo "(devam etmek için Enter'a bas)"
            read -r _
            break  # iç select'ten çık, while döngüsüyle menüyü yeniden çiz
        done
    done
}

# ─── dispatch ────────────────────────────────────────────────────────────────

case "${1:-menu}" in
    menu|"") cmd_menu ;;
    start)   shift; cmd_start "$@" ;;
    stop)    cmd_stop ;;
    status)  cmd_status ;;
    restart) shift; cmd_restart "$@" ;;
    tail)    cmd_tail ;;
    trades)  cmd_trades ;;
    logs)    shift; cmd_logs "$@" ;;
    build)   shift; cmd_build "$@" ;;
    help|-h|--help) cmd_help ;;
    *)
        echo "⚠️  Bilinmeyen komut: $1"
        cmd_help
        exit 1
        ;;
esac
