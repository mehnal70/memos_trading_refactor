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
#   WATCHDOG_CHECK_SECS=60     # kontrol aralığı
#   WATCHDOG_STALE_SECS=180    # heartbeat bu kadar saniyedir güncellenmiyorsa "hung" say
#   WATCHDOG_MAX_RESTARTS=5    # pencerede bu kadar restart aşılırsa DURAKLA (crash-loop)
#   WATCHDOG_WINDOW_SECS=900   # crash-loop penceresi (15 dk)
#
# Bakım/duraklatma: `logs/.watchdog.pause` dosyası varsa watchdog restart YAPMAZ (kasıtlı
#   bakım: `touch logs/.watchdog.pause`; geri al: `rm logs/.watchdog.pause`).

set -u
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR" || exit 1

PID_FILE="logs/.engine.pid"
HEARTBEAT_LOG="logs/heartbeat.jsonl"
PAUSE_FILE="logs/.watchdog.pause"
ENGINE="./scripts/engine.sh"

MODE_FLAG=""
for a in "$@"; do [ "$a" = "--release" ] && MODE_FLAG="--release"; done

CHECK_SECS="${WATCHDOG_CHECK_SECS:-60}"
STALE_SECS="${WATCHDOG_STALE_SECS:-180}"
MAX_RESTARTS="${WATCHDOG_MAX_RESTARTS:-5}"
WINDOW_SECS="${WATCHDOG_WINDOW_SECS:-900}"

mkdir -p logs
log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] watchdog: $*"; }

pid_alive() { local p="$1"; [ -n "$p" ] && kill -0 "$p" 2>/dev/null; }
hb_age() { # heartbeat.jsonl son değişiklikten bu yana saniye; dosya yoksa çok büyük
    [ -f "$HEARTBEAT_LOG" ] || { echo 999999; return; }
    echo $(( $(date +%s) - $(stat -c %Y "$HEARTBEAT_LOG" 2>/dev/null || echo 0) ))
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

do_restart() { # neden → engine.sh restart; crash-loop ise duraklat
    local reason="$1"
    log "⚠️ $reason → engine restart ($MODE_FLAG)"
    "$ENGINE" restart $MODE_FLAG >>logs/watchdog.log 2>&1 || log "restart komutu hata döndürdü"
    if record_and_check_loop; then
        log "🛑 CRASH-LOOP: ${WINDOW_SECS}sn içinde ≥${MAX_RESTARTS} restart → DURAKLATILIYOR (touch $PAUSE_FILE)."
        log "   Binary/ortam bozuk olabilir. Düzelt + 'rm $PAUSE_FILE' ile devam ettir."
        : > "$PAUSE_FILE"
    fi
}

log "başladı (check=${CHECK_SECS}s stale=${STALE_SECS}s max_restarts=${MAX_RESTARTS}/${WINDOW_SECS}s mode=${MODE_FLAG:-debug})"
trap 'log "durduruldu (sinyal)"; exit 0' INT TERM

while true; do
    if [ -f "$PAUSE_FILE" ]; then
        log "⏸️  duraklatıldı ($PAUSE_FILE var) — restart yok"
        sleep "$CHECK_SECS"; continue
    fi
    pid=""; [ -f "$PID_FILE" ] && pid="$(cat "$PID_FILE" 2>/dev/null)"
    if ! pid_alive "$pid"; then
        do_restart "motor ÖLÜ (PID ${pid:-yok} canlı değil)"
    else
        age=$(hb_age)
        if [ "$age" -gt "$STALE_SECS" ]; then
            do_restart "motor HUNG (heartbeat ${age}s bayat > ${STALE_SECS}s)"
        fi
    fi
    sleep "$CHECK_SECS"
done
