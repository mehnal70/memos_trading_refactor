#!/usr/bin/env bash
# scripts/reset_state.sh — Test fazı reset aracı.
#
# İki şeyi (ayrı ayrı ya da birlikte) sıfırlar:
#   • Aktif loglar  : robotic_trading.log + trades.jsonl + heartbeat.jsonl
#                     → logs/archive/run_<ts>/ altına TAŞINIR (silinmez, arşivlenir).
#   • İşlem durumu  : account_state (equity/peak/closed) + open_positions_snapshot
#                     → starting_capital'a sıfırlanır, pozisyonlar boşaltılır.
#
# Mumlar (candles*) ve diğer geçmiş/analitik tablolar HER ZAMAN korunur.
# İşlem durumu sıfırlanmadan önce .dump ile aynı arşiv klasörüne yedeklenir
# (geri dönüş: sqlite3 <db> < account_state_backup.sql).
#
# Kullanım:
#   ./scripts/reset_state.sh                # logs + state (onay sorar)
#   ./scripts/reset_state.sh --logs         # yalnız logları arşivle
#   ./scripts/reset_state.sh --state        # yalnız account_state + pozisyon
#   ./scripts/reset_state.sh --all -y       # ikisi de, onaysız (cron/script)
#   DB_PATH=data/trader.db CAPITAL=10000 ./scripts/reset_state.sh
#
# Güvenlik: bot (rtc_headless/rtc_tui) çalışıyorsa REDDEDER (yazma yarışı).

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.." || exit 1

RED='\033[0;31m'; YELLOW='\033[1;33m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

DB_PATH="${DB_PATH:-data/trader.db}"
DO_LOGS=false
DO_STATE=false
ASSUME_YES=false
EXPLICIT_SCOPE=false

for arg in "$@"; do
    case "$arg" in
        --logs)     DO_LOGS=true;  EXPLICIT_SCOPE=true ;;
        --state)    DO_STATE=true; EXPLICIT_SCOPE=true ;;
        --all)      DO_LOGS=true;  DO_STATE=true; EXPLICIT_SCOPE=true ;;
        -y|--yes)   ASSUME_YES=true ;;
        -h|--help)  sed -n '2,29p' "$0"; exit 0 ;;
        *) echo -e "${RED}⚠️  Bilinmeyen argüman: $arg${NC} (-h için yardım)"; exit 1 ;;
    esac
done
# Kapsam belirtilmediyse default: ikisi de.
if [ "$EXPLICIT_SCOPE" = false ]; then DO_LOGS=true; DO_STATE=true; fi

echo -e "${BOLD}╔══ Memos Reset Aracı (test fazı) ══╗${NC}"
echo -e "  DB        : ${DB_PATH}"
echo -e "  Loglar    : $([ "$DO_LOGS" = true ] && echo "SIFIRLA" || echo "atla")"
echo -e "  İşlem dur.: $([ "$DO_STATE" = true ] && echo "SIFIRLA" || echo "atla")"
echo -e "  Mumlar    : ${GREEN}KORUNUR${NC}"

# ── Güvenlik: bot çalışıyor mu? (pgrep -x → tam binary adı, script'in kendisini eşlemez) ──
if pgrep -x rtc_headless >/dev/null 2>&1 || pgrep -x rtc_tui >/dev/null 2>&1; then
    echo -e "${RED}❌ Bot çalışıyor (rtc_headless/rtc_tui). Önce durdur — yazma yarışı/bozulma riski.${NC}"
    exit 1
fi

if [ "$DO_STATE" = true ] && [ ! -f "$DB_PATH" ]; then
    echo -e "${RED}❌ DB bulunamadı: ${DB_PATH}${NC}"; exit 1
fi

# ── Onay ──
if [ "$ASSUME_YES" = false ]; then
    echo -ne "${YELLOW}Devam edilsin mi? [e/H] ${NC}"
    read -r ans
    case "$ans" in e|E|y|Y|evet|yes) ;; *) echo "İptal."; exit 0 ;; esac
fi

TS="$(date +%Y%m%d_%H%M%S)"
DEST="logs/archive/run_${TS}"
mkdir -p "$DEST"

# ── 1) Loglar: arşivle ──
if [ "$DO_LOGS" = true ]; then
    moved=0
    for f in robotic_trading.log trades.jsonl heartbeat.jsonl; do
        if [ -f "logs/$f" ]; then mv "logs/$f" "$DEST/$f"; moved=$((moved+1)); fi
    done
    echo -e "${GREEN}✓ Loglar arşivlendi:${NC} $DEST ($moved dosya). Bot yeniden oluşturur."
fi

# ── 2) İşlem durumu: yedekle + sıfırla (mumlara dokunma) ──
if [ "$DO_STATE" = true ]; then
    before_candles="$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM candles;" 2>/dev/null)"
    # Geri-dönüş yedeği
    sqlite3 "$DB_PATH" ".dump account_state open_positions_snapshot" > "$DEST/account_state_backup.sql" 2>/dev/null
    # Sermaye: CAPITAL env > mevcut starting_capital > 10000
    CAP="${CAPITAL:-$(sqlite3 "$DB_PATH" "SELECT COALESCE(MAX(starting_capital),10000) FROM account_state;" 2>/dev/null)}"
    CAP="${CAP:-10000}"
    NOW="$(date --iso-8601=seconds)"
    sqlite3 "$DB_PATH" <<SQL
INSERT INTO account_state (id, equity, peak_equity, starting_capital, closed_trades_count, updated_at)
VALUES (1, ${CAP}, ${CAP}, ${CAP}, 0, '${NOW}')
ON CONFLICT(id) DO UPDATE SET
    equity = ${CAP}, peak_equity = ${CAP}, starting_capital = ${CAP},
    closed_trades_count = 0, updated_at = '${NOW}';
INSERT INTO open_positions_snapshot (id, positions, updated_at)
VALUES (1, '[]', '${NOW}')
ON CONFLICT(id) DO UPDATE SET positions = '[]', updated_at = '${NOW}';
SQL
    after_candles="$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM candles;" 2>/dev/null)"
    echo -e "${GREEN}✓ account_state sıfırlandı:${NC} equity=peak=\$${CAP}, closed=0, pozisyon=[]"
    echo -e "  yedek: $DEST/account_state_backup.sql"
    if [ "$before_candles" != "$after_candles" ]; then
        echo -e "${RED}⚠️  Mum sayısı değişti! $before_candles → $after_candles (BEKLENMEDİK)${NC}"; exit 2
    fi
    echo -e "${CYAN}  mumlar korundu: $after_candles satır${NC}"
fi

echo -e "${BOLD}${GREEN}✓ Reset tamam.${NC}"
