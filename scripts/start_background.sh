#!/usr/bin/env bash
# Memos Trading — arka planda tmux ile başlatır
# Masaüstü bildirimleri çalışmaya devam eder (DBUS_SESSION_BUS_ADDRESS korunur)

SESSION="memos_trading"
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN_RELEASE="$REPO_DIR/target/release/rtc_cli"
BIN_DEBUG="$REPO_DIR/target/debug/rtc_cli"

if ! command -v tmux &>/dev/null; then
    echo "HATA: tmux bulunamadı. Kurmak için: sudo apt install tmux"
    exit 1
fi

# Release varsa onu tercih et, yoksa debug'a düş
if [ -f "$BIN_RELEASE" ]; then
    BIN="$BIN_RELEASE"
    echo "Binary: release"
elif [ -f "$BIN_DEBUG" ]; then
    BIN="$BIN_DEBUG"
    echo "Binary: debug (release için: cargo build --release --bin rtc_cli)"
else
    echo "Binary bulunamadı."
    echo "Derlemek için: cargo build --release --bin rtc_cli"
    exit 1
fi

# DBUS ve DISPLAY bilgisini kaydet — systemd veya arka plan süreçlerinde notify-send için
mkdir -p "$HOME/.config/memos_trading"
echo "export DBUS_SESSION_BUS_ADDRESS='$DBUS_SESSION_BUS_ADDRESS'" > "$HOME/.config/memos_trading/env"
echo "export DISPLAY='${DISPLAY:-:0}'"                            >> "$HOME/.config/memos_trading/env"

if tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "Oturum zaten çalışıyor: $SESSION"
    echo ""
    echo "  Bağlanmak   : tmux attach -t $SESSION"
    echo "  Durdurmak   : tmux kill-session -t $SESSION"
    exit 0
fi

# Yeni tmux oturumu oluştur (detached, 220×55 boyut)
tmux new-session -d -s "$SESSION" -x 220 -y 55

# Pane crash sonrası kapanmasın — hata mesajı okunabilsin
tmux set-window-option -t "$SESSION" remain-on-exit on

# Log dizinini hazırla
mkdir -p "$REPO_DIR/logs"
LOGFILE="$REPO_DIR/logs/rtc_cli_$(date +%Y%m%d_%H%M%S).log"

# Ortam değişkenlerini aktar
tmux send-keys -t "$SESSION" "export DBUS_SESSION_BUS_ADDRESS='$DBUS_SESSION_BUS_ADDRESS'" Enter
tmux send-keys -t "$SESSION" "export DISPLAY='${DISPLAY:-:0}'" Enter

# Binary'yi çalıştır — stderr log dosyasına da yazılır; crash sonrası pane açık kalır
tmux send-keys -t "$SESSION" "cd '$REPO_DIR' && '$BIN' 2>>'$LOGFILE'; echo ''; echo '=== Çıkış kodu: '\$?' | Log: $LOGFILE ==='; echo 'Son hatalar:'; tail -20 '$LOGFILE'" Enter

echo "Memos Trading arka planda başlatıldı."
echo ""
echo "  Bağlanmak   : tmux attach -t $SESSION"
echo "  Durdurmak   : tmux kill-session -t $SESSION"
echo "  Hata logu   : tail -f $REPO_DIR/logs/rtc_cli_*.log"
echo ""
echo "Masaüstü bildirimleri otomatik olarak görünecek."
