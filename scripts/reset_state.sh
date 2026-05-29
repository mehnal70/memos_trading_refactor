#!/usr/bin/env bash
# scripts/reset_state.sh — Test fazı reset aracı.
#
# Üç şeyi (ayrı ayrı ya da birlikte) sıfırlar:
#   • Aktif loglar  (--logs)  : robotic_trading.log + trades.jsonl + heartbeat.jsonl
#                               → logs/archive/run_<ts>/ altına TAŞINIR (silinmez).
#   • İşlem durumu  (--state) : account_state (equity/peak/closed) + open_positions_snapshot
#                               → starting_capital'a sıfırlanır, pozisyonlar boşaltılır.
#   • Geçmiş        (--history): trade/sinyal/rapor geçmiş tabloları (trades, signals,
#                               consensus_signals, portfolio, reports, paper_trading_*,
#                               trade_outcomes, active_paper_trading) → TEMİZLENİR.
#                               Dashboard'daki kümülatif geçmiş izlerini de siler.
#
# KORUNANLAR (her zaman): mumlar (candles*) + ÖĞRENİLMİŞ durum
#   (strategy_optimized_params, ml_model_state, rl_q_table, pattern_library,
#    leverage_settings, symbol_status, system_settings, symbols ...).
# State/history sıfırlanmadan önce .dump ile aynı arşiv klasörüne yedeklenir
# (geri dönüş: sqlite3 <db> < <backup>.sql).
#
# Kullanım:
#   ./scripts/reset_state.sh                # logs + state (default; onay sorar)
#   ./scripts/reset_state.sh --logs         # yalnız logları arşivle
#   ./scripts/reset_state.sh --state        # yalnız account_state + pozisyon
#   ./scripts/reset_state.sh --history      # yalnız geçmiş/analitik tablolar
#   ./scripts/reset_state.sh --all -y       # logs + state + history, onaysız
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
DO_HISTORY=false
ASSUME_YES=false
EXPLICIT_SCOPE=false

# Geçmiş/analitik tablolar (mumlar ve öğrenilmiş parametreler DAHİL DEĞİL).
HISTORY_TABLES="trades signals consensus_signals portfolio reports \
paper_trading_results paper_trading_daily_pnl paper_trading_auto_stop \
trade_outcomes active_paper_trading strategy_signal_compatibility"

for arg in "$@"; do
    case "$arg" in
        --logs)     DO_LOGS=true;    EXPLICIT_SCOPE=true ;;
        --state)    DO_STATE=true;   EXPLICIT_SCOPE=true ;;
        --history)  DO_HISTORY=true; EXPLICIT_SCOPE=true ;;
        --all)      DO_LOGS=true; DO_STATE=true; DO_HISTORY=true; EXPLICIT_SCOPE=true ;;
        -y|--yes)   ASSUME_YES=true ;;
        -h|--help)  sed -n '2,37p' "$0"; exit 0 ;;
        *) echo -e "${RED}⚠️  Bilinmeyen argüman: $arg${NC} (-h için yardım)"; exit 1 ;;
    esac
done
# Kapsam belirtilmediyse default: logs + state (history hariç — ağır/opt-in).
if [ "$EXPLICIT_SCOPE" = false ]; then DO_LOGS=true; DO_STATE=true; fi

echo -e "${BOLD}╔══ Memos Reset Aracı (test fazı) ══╗${NC}"
echo -e "  DB        : ${DB_PATH}"
echo -e "  Loglar    : $([ "$DO_LOGS" = true ] && echo "SIFIRLA" || echo "atla")"
echo -e "  İşlem dur.: $([ "$DO_STATE" = true ] && echo "SIFIRLA" || echo "atla")"
echo -e "  Geçmiş    : $([ "$DO_HISTORY" = true ] && echo "SIFIRLA" || echo "atla")"
echo -e "  Mumlar+ML : ${GREEN}KORUNUR${NC}"

# ── Güvenlik: bot çalışıyor mu? (pgrep -x → tam binary adı, script'in kendisini eşlemez) ──
if pgrep -x rtc_headless >/dev/null 2>&1 || pgrep -x rtc_tui >/dev/null 2>&1; then
    echo -e "${RED}❌ Bot çalışıyor (rtc_headless/rtc_tui). Önce durdur — yazma yarışı/bozulma riski.${NC}"
    exit 1
fi

if { [ "$DO_STATE" = true ] || [ "$DO_HISTORY" = true ]; } && [ ! -f "$DB_PATH" ]; then
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

# Mum güvencesi (state/history koşullarında öncesi/sonrası kıyas).
before_candles=""
if [ "$DO_STATE" = true ] || [ "$DO_HISTORY" = true ]; then
    before_candles="$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM candles;" 2>/dev/null)"
fi

# ── 1) Loglar: arşivle ──
if [ "$DO_LOGS" = true ]; then
    moved=0
    for f in robotic_trading.log trades.jsonl heartbeat.jsonl; do
        if [ -f "logs/$f" ]; then mv "logs/$f" "$DEST/$f"; moved=$((moved+1)); fi
    done
    echo -e "${GREEN}✓ Loglar arşivlendi:${NC} $DEST ($moved dosya). Bot yeniden oluşturur."
fi

# ── 2) İşlem durumu: yedekle + sıfırla ──
if [ "$DO_STATE" = true ]; then
    sqlite3 "$DB_PATH" ".dump account_state open_positions_snapshot" > "$DEST/account_state_backup.sql" 2>/dev/null
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
    echo -e "${GREEN}✓ account_state sıfırlandı:${NC} equity=peak=\$${CAP}, closed=0, pozisyon=[]"
    echo -e "  yedek: $DEST/account_state_backup.sql"
fi

# ── 3) Geçmiş/analitik tablolar: yedekle + temizle (mumlar ve ML hariç) ──
if [ "$DO_HISTORY" = true ]; then
    # Yalnız VAR OLAN tabloları işle (şema sürümleri arası güvenli).
    existing=""
    for t in $HISTORY_TABLES; do
        n="$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='$t';" 2>/dev/null)"
        [ "$n" = "1" ] && existing="$existing $t"
    done
    if [ -n "$existing" ]; then
        # shellcheck disable=SC2086
        sqlite3 "$DB_PATH" ".dump$(printf ' %s' $existing)" > "$DEST/history_backup.sql" 2>/dev/null
        total=0
        for t in $existing; do
            c="$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM $t;" 2>/dev/null)"
            total=$((total + ${c:-0}))
            sqlite3 "$DB_PATH" "DELETE FROM $t;" 2>/dev/null
        done
        echo -e "${GREEN}✓ Geçmiş temizlendi:${NC} ${existing# } (${total} satır silindi)"
        echo -e "  yedek: $DEST/history_backup.sql"
        echo -e "${CYAN}  not: dosya boyutu küçülmedi (DELETE alanı geri vermez); gerekirse: sqlite3 $DB_PATH 'VACUUM;'${NC}"
    else
        echo -e "${YELLOW}• Geçmiş tablosu bulunamadı, atlandı.${NC}"
    fi
fi

# ── Mum güvencesi ──
if [ -n "$before_candles" ]; then
    after_candles="$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM candles;" 2>/dev/null)"
    if [ "$before_candles" != "$after_candles" ]; then
        echo -e "${RED}⚠️  Mum sayısı değişti! $before_candles → $after_candles (BEKLENMEDİK)${NC}"; exit 2
    fi
    echo -e "${CYAN}  mumlar korundu: $after_candles satır${NC}"
fi

echo -e "${BOLD}${GREEN}✓ Reset tamam.${NC}"
