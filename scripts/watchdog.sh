#!/usr/bin/env bash
# scripts/watchdog.sh — Memos engine 7/24 DENETLEYİCİ (root gerektirmez).
#
# Motor ÖLÜ (süreç yok) VEYA HUNG (heartbeat bayat) ise engine.sh ile yeniden başlatır.
# İki başarısızlık modunu da yakalar: (1) crash → PID ölü; (2) takılma → süreç canlı ama
# heartbeat.jsonl güncellenmiyor. Crash-loop koruması: pencere içinde çok restart olursa
# DURAKLAR (sonsuz hammer yerine operatöre bırakır).
#
# Kullanım:
#   ./scripts/watchdog.sh --release                 # ön planda (Ctrl+C ile dur)
#   nohup ./scripts/watchdog.sh --release >logs/watchdog.log 2>&1 &   # arka plan
#   (kalıcı 7/24 için: scripts/memos-watchdog.service — systemd --user, aşağıdaki kurulum)
#
# Env (default):
#   WATCHDOG_TARGET=headless   # headless (engine.sh+PID) | tui (tui_daemon.sh+tmux)
#   WATCHDOG_CHECK_SECS=60     # kontrol aralığı
#   WATCHDOG_STALE_SECS=180    # heartbeat bu kadar saniyedir güncellenmiyorsa "hung" say
#   WATCHDOG_BOOT_GRACE_SECS=600  # motor BU kadar saniyeden genç ise HUNG sayma (boot/indirme
#                                 # ~6 dk sürer, bu sürede heartbeat yazılmaz → bayat heartbeat
#                                 # taze motoru öldürmesin; yoksa kapat-aç sonrası crash-loop)
#   WATCHDOG_MAX_RESTARTS=5    # pencerede bu kadar restart aşılırsa DURAKLA (crash-loop)
#   WATCHDOG_WINDOW_SECS=900   # crash-loop penceresi (15 dk)
#
# TUI modu (panelleri izlemek isteyenler için): WATCHDOG_TARGET=tui ./scripts/watchdog.sh --release
#   → TUI'yi tmux 'memos' oturumunda tutar; izlemek: ./scripts/tui_daemon.sh attach
#
# Bakım/duraklatma: `logs/.watchdog.pause` dosyası varsa watchdog restart YAPMAZ (kasıtlı
#   bakım: `touch logs/.watchdog.pause`; geri al: `rm logs/.watchdog.pause`).

set -u
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR" || exit 1

# Aktif profilin çalışma-zamanı env'ini yükle → HEARTBEAT_PATH profil-doğru olsun. Watchdog
# motorla AYNI heartbeat dosyasına bakmalı; yoksa profil (logs/<p>/heartbeat.jsonl) yazarken
# watchdog kök logs/heartbeat.jsonl'e bakıp "bayat" sanır → sağlıklı motoru HUNG-öldürür.
# shellcheck source=scripts/lib_launchconf.sh
. "$REPO_DIR/scripts/lib_launchconf.sh" 2>/dev/null \
    && load_launch_conf "$REPO_DIR/scripts/.launch.conf" 2>/dev/null || true

PID_FILE="logs/.engine.pid"
HEARTBEAT_LOG="${HEARTBEAT_PATH:-logs/heartbeat.jsonl}"
PAUSE_FILE="logs/.watchdog.pause"
ENGINE="./scripts/engine.sh"
TUI_DAEMON="./scripts/tui_daemon.sh"
TUI_SESSION="${MEMOS_TUI_SESSION:-memos}"
TARGET="${WATCHDOG_TARGET:-headless}"   # headless | tui

MODE_FLAG=""
for a in "$@"; do [ "$a" = "--release" ] && MODE_FLAG="--release"; done

# Hedefe-göre canlılık + restart soyutlaması (heartbeat-bayatlık kontrolü ikisinde de ortak).
engine_alive() {
    if [ "$TARGET" = "tui" ]; then
        tmux has-session -t "$TUI_SESSION" 2>/dev/null
    else
        local p=""; [ -f "$PID_FILE" ] && p="$(cat "$PID_FILE" 2>/dev/null)"
        [ -n "$p" ] && kill -0 "$p" 2>/dev/null
    fi
}
engine_restart() {
    if [ "$TARGET" = "tui" ]; then
        "$TUI_DAEMON" restart $MODE_FLAG >>logs/watchdog.log 2>&1
    else
        "$ENGINE" restart $MODE_FLAG >>logs/watchdog.log 2>&1
    fi
}

CHECK_SECS="${WATCHDOG_CHECK_SECS:-60}"
STALE_SECS="${WATCHDOG_STALE_SECS:-180}"
BOOT_GRACE_SECS="${WATCHDOG_BOOT_GRACE_SECS:-600}"
MAX_RESTARTS="${WATCHDOG_MAX_RESTARTS:-5}"
WINDOW_SECS="${WATCHDOG_WINDOW_SECS:-900}"

mkdir -p logs
log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] watchdog: $*"; }

pid_alive() { local p="$1"; [ -n "$p" ] && kill -0 "$p" 2>/dev/null; }
hb_age() { # heartbeat.jsonl son değişiklikten bu yana saniye; dosya yoksa çok büyük
    [ -f "$HEARTBEAT_LOG" ] || { echo 999999; return; }
    echo $(( $(date +%s) - $(stat -c %Y "$HEARTBEAT_LOG" 2>/dev/null || echo 0) ))
}

engine_pid() { # çalışan motor sürecinin PID'i (hedefe göre); bulunamazsa boş
    if [ "$TARGET" = "tui" ]; then
        pgrep -f 'target/release/rtc_tui' | head -1
    else
        [ -f "$PID_FILE" ] && cat "$PID_FILE" 2>/dev/null
    fi
}
engine_age() { # motor sürecinin yaşı (sn); bulunamazsa 999999 (= yaşlı say → boot-grace devre dışı)
    local p; p="$(engine_pid)"
    if [ -n "$p" ] && kill -0 "$p" 2>/dev/null; then
        local a; a="$(ps -o etimes= -p "$p" 2>/dev/null | tr -d ' ')"
        [ -n "$a" ] && echo "$a" || echo 999999
    else
        echo 999999
    fi
}

# Crash-loop penceresi: son restart epoch'ları (boşlukla ayrılmış).
restart_stamps=""
record_and_check_loop() { # restart kaydet; pencerede sınır aşıldıysa 1 döndür (durakla)
    local now; now=$(date +%s)
    local kept=""
    for t in $restart_stamps; do [ $((now - t)) -lt "$WINDOW_SECS" ] && kept="$kept $t"; done
    kept="$kept $now"
    restart_stamps="$kept"
    local count; count=$(echo $restart_stamps | wc -w)
    [ "$count" -ge "$MAX_RESTARTS" ]
}

do_restart() { # neden → hedefe-göre restart; crash-loop ise duraklat
    local reason="$1"
    log "⚠️ $reason → $TARGET restart ($MODE_FLAG)"
    engine_restart || log "restart komutu hata döndürdü"
    if record_and_check_loop; then
        log "🛑 CRASH-LOOP: ${WINDOW_SECS}sn içinde ≥${MAX_RESTARTS} restart → DURAKLATILIYOR (touch $PAUSE_FILE)."
        log "   Binary/ortam bozuk olabilir. Düzelt + 'rm $PAUSE_FILE' ile devam ettir."
        : > "$PAUSE_FILE"
    fi
}

log "başladı (target=${TARGET} check=${CHECK_SECS}s stale=${STALE_SECS}s boot_grace=${BOOT_GRACE_SECS}s max_restarts=${MAX_RESTARTS}/${WINDOW_SECS}s hb=${HEARTBEAT_LOG} mode=${MODE_FLAG:-debug})"
trap 'log "durduruldu (sinyal)"; exit 0' INT TERM

while true; do
    if [ -f "$PAUSE_FILE" ]; then
        log "⏸️  duraklatıldı ($PAUSE_FILE var) — restart yok"
        sleep "$CHECK_SECS"; continue
    fi
    if ! engine_alive; then
        do_restart "motor ÖLÜ ($TARGET canlı değil)"
    else
        age=$(hb_age)
        if [ "$age" -gt "$STALE_SECS" ]; then
            eage=$(engine_age)
            if [ "$eage" -lt "$BOOT_GRACE_SECS" ]; then
                log "⏳ boot-grace: motor ${eage}s genç (<${BOOT_GRACE_SECS}s) — heartbeat ${age}s bayat ama HUNG sayılmıyor (boot/indirme sürüyor)"
            else
                do_restart "motor HUNG (heartbeat ${age}s bayat > ${STALE_SECS}s)"
            fi
        fi
    fi
    sleep "$CHECK_SECS"
done
