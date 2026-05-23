#!/bin/bash
# scripts/log_doctor.sh — Memos canlı log yorumlayıcı & uyarı motoru
#
# İki mod:
#   Snapshot (default): son N dakikanın log'larını tarar, kategori özeti
#                       + uyarılar + önerilen aksiyonlar. Bir kere koşar.
#   Watch (--watch):    tail -F ile sürekli izler, kritik pattern'da renkli
#                       alarm verir (sahte PnL, komisyon erozyonu, anomaly).
#
# Kullanım:
#   ./scripts/log_doctor.sh                 # snapshot, son 10 dk
#   ./scripts/log_doctor.sh --minutes 60    # snapshot, son 60 dk
#   ./scripts/log_doctor.sh --watch         # real-time alarm
#   ./scripts/log_doctor.sh --log-dir /path/to/logs --watch
#
# Tüm uyarılar PASS / WARN / CRITICAL kategorilerinde; çıkış kodu:
#   0 = PASS, 1 = WARN, 2 = CRITICAL  (CI/cron için)

set -u
export LC_NUMERIC=C  # printf %.2f gibi format Türkçe locale'da virgül koyuyor

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.." || exit 1

# Renkler
RED='\033[0;31m'
YELLOW='\033[1;33m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# Defaults
MODE="snapshot"
MINUTES=10
LOG_DIR="./logs"

# Arg parse
while [ $# -gt 0 ]; do
    case "$1" in
        --watch)    MODE="watch" ;;
        --minutes)  shift; MINUTES="${1:-10}" ;;
        --log-dir)  shift; LOG_DIR="${1:-./logs}" ;;
        -h|--help)
            head -20 "$0" | tail -18
            exit 0 ;;
        *)
            echo "Bilinmeyen argüman: $1 (--help için -h)"
            exit 1 ;;
    esac
    shift
done

ROBOTIC_LOG="${LOG_DIR}/robotic_trading.log"
STDERR_LOG="${LOG_DIR}/engine_stderr.log"
HEARTBEAT="${LOG_DIR}/heartbeat.jsonl"

if [ ! -f "$ROBOTIC_LOG" ]; then
    echo -e "${RED}❌ ${ROBOTIC_LOG} bulunamadı. Bot çalışıyor mu? --log-dir ile yol verin.${NC}"
    exit 2
fi

# ─────────────────────────────────────────────────────────────────────────────
# WATCH MODU — real-time renkli alarm
# ─────────────────────────────────────────────────────────────────────────────
if [ "$MODE" = "watch" ]; then
    echo -e "${BOLD}${CYAN}🔭 Memos Log Watch — ${ROBOTIC_LOG}${NC}"
    echo -e "${DIM}Ctrl+C ile çık. Renkler: ${GREEN}açılış${NC} ${YELLOW}kapanış/uyarı${NC} ${RED}kritik${NC}${NC}"
    echo ""
    tail -F "$ROBOTIC_LOG" "$STDERR_LOG" 2>/dev/null | awk \
        -v R="$RED" -v Y="$YELLOW" -v G="$GREEN" -v B="$BLUE" -v C="$CYAN" -v N="$NC" -v D="$DIM" '
        BEGIN { last_min=0; strategy_count=0 }
        {
            # Rolling counter: dakikada STRATEGY_SIGNAL close > 20 → EROZYON
            cmd = "date +%s"; cmd | getline now; close(cmd)
            cur_min = int(now / 60)
            if (cur_min != last_min) {
                if (strategy_count > 20) {
                    printf "%s⚠️  EROZYON ALARMI: son dakikada %d STRATEGY_SIGNAL kapanış (>20). ScalpSwing aç/kapa loop olası.%s\n", R, strategy_count, N
                }
                last_min = cur_min
                strategy_count = 0
            }
            if (/Reason=STRATEGY_SIGNAL/) strategy_count++

            # Pattern matching
            if (/TRADE_OPEN.*SCP_/)           printf "%s⚡ %s%s\n", G, $0, N
            else if (/TRADE_OPEN.*SWG_/)      printf "%s⚡ %s%s\n", G, $0, N
            else if (/TRADE_OPEN/)            printf "%s🟢 %s%s\n", G, $0, N
            else if (/Reason=TAKE_PROFIT/)    printf "%s💰 %s%s\n", G, $0, N
            else if (/Reason=STOP_LOSS/)      printf "%s🔻 %s%s\n", Y, $0, N
            else if (/Reason=TRAILING_STOP/)  printf "%s🎯 %s%s\n", B, $0, N
            else if (/Reason=STRATEGY_SIGNAL/) printf "%s🔄 %s%s\n", D, $0, N
            else if (/Auto-Disabled/)         printf "%s🛑 %s%s\n", R, $0, N
            else if (/🎚️.*tuner/)             printf "%s%s%s\n", C, $0, N
            else if (/erken kapanış reddedildi/) printf "%s⏳ %s%s\n", D, $0, N
            else if (/RISK_BLOCK/)            printf "%s%s%s\n", D, $0, N
            else if (/ERROR/)                 printf "%s🔥 %s%s\n", R, $0, N
            else if (/anomaly\[Critical/)     printf "%s🚨 %s%s\n", R, $0, N
            else if (/anomaly\[Warning/)      printf "%s⚠️  %s%s\n", Y, $0, N
            else if (/panic/)                 printf "%s💥 %s%s\n", R, $0, N
            else                              print $0
            fflush()
        }'
    exit 0
fi

# ─────────────────────────────────────────────────────────────────────────────
# SNAPSHOT MODU — kategori raporu + uyarılar
# ─────────────────────────────────────────────────────────────────────────────
since_ts=$(date -u -d "$MINUTES minutes ago" +%Y-%m-%dT%H:%M 2>/dev/null \
    || date -u -v-${MINUTES}M +%Y-%m-%dT%H:%M)  # macOS uyumlu

echo -e "${BOLD}${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}${CYAN}║   Memos Log Doctor — son ${MINUTES} dakika (${since_ts}+ UTC)         ║${NC}"
echo -e "${BOLD}${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Pencere filtresi: timestamp string karşılaştırması (RFC3339 lexicographic)
recent_lines=$(awk -v since="$since_ts" '$0 >= "["since {print}' "$ROBOTIC_LOG" 2>/dev/null)
recent_stderr=$(awk -v since="$since_ts" '$0 ~ "^\\["since {print} $0 !~ "^\\[" {print}' "$STDERR_LOG" 2>/dev/null)

# Sayım yardımcısı — grep -c boş eşleşmede de "0" yazar ama exit 1 döner;
# `|| echo 0` ikinci sayı yazıp birleştirir → patlama. Sadece stdout al.
count()        { local n; n=$(printf '%s\n' "$recent_lines"  | grep -c "$1" 2>/dev/null || true); echo "${n:-0}"; }
count_stderr() { local n; n=$(printf '%s\n' "$recent_stderr" | grep -c "$1" 2>/dev/null || true); echo "${n:-0}"; }

# ── Genel istatistikler
OPEN_TOTAL=$(count "TRADE_OPEN")
CLOSE_TOTAL=$(count "TRADE_CLOSE")
TP=$(count "Reason=TAKE_PROFIT")
SL=$(count "Reason=STOP_LOSS")
TRAIL=$(count "Reason=TRAILING_STOP")
STRAT_SIG=$(count "Reason=STRATEGY_SIGNAL")
SCP_OPEN=$(count "Strat=SCP_")
SWG_OPEN=$(count "Strat=SWG_")
SUPERTREND=$(count "Strat=SUPERTREND")
RISK_BLOCK=$(count "RISK_BLOCK")
ERKEN_REDDED=$(count "erken kapanış reddedildi")
AUTO_DISABLED=$(count "Auto-Disabled")
TUNER_RUN=$(count "🎚️")

echo -e "${BOLD}📊 İŞLEM AKIŞI${NC}"
printf "  Açılış:       %3d  (SCP=%d  SWG=%d  SUPERTREND=%d  Diğer=%d)\n" \
    "$OPEN_TOTAL" "$SCP_OPEN" "$SWG_OPEN" "$SUPERTREND" "$((OPEN_TOTAL - SCP_OPEN - SWG_OPEN - SUPERTREND))"
printf "  Kapanış:      %3d  (TP=%d  SL=%d  Trail=%d  StrategySignal=%d)\n" \
    "$CLOSE_TOTAL" "$TP" "$SL" "$TRAIL" "$STRAT_SIG"
printf "  Risk block:   %3d   Erken kapanış reddedildi: %d   Auto-Disabled: %d\n" \
    "$RISK_BLOCK" "$ERKEN_REDDED" "$AUTO_DISABLED"
echo ""

# ── Heartbeat son durumu
EXIT_CODE=0
WARN_LINES=()
CRIT_LINES=()

if [ -f "$HEARTBEAT" ]; then
    last_hb=$(tail -1 "$HEARTBEAT")
    if [ -n "$last_hb" ]; then
        # JSON field okuyucu; eşleşme yoksa default 0 / false
        jf() { local v; v=$(echo "$last_hb" | grep -oP "\"$1\":\\K(true|false|[0-9.]+)" | head -1); echo "${v:-${2:-0}}"; }
        EQUITY=$(jf equity 0)
        PEAK=$(jf peak_equity 0)
        DD=$(jf drawdown_pct 0)
        OPEN_POS=$(jf open_positions 0)
        CLOSED=$(jf closed_trades 0)
        ANOM=$(jf anomalies 0)
        ML=$(jf ml_confidence 0)
        GBT=$(jf gbt_ready false)

        echo -e "${BOLD}💰 SERMAYE & PİPELİNE (son heartbeat)${NC}"
        printf "  Equity: \$%.2f    Peak: \$%.2f    Drawdown: %.2f%%\n" "$EQUITY" "$PEAK" "$DD"
        printf "  Açık pozisyon: %s    Kapanan toplam: %s    Anomaly: %s\n" "$OPEN_POS" "$CLOSED" "$ANOM"
        printf "  ML confidence: %.3f    GBT hazır: %s\n" "$ML" "$GBT"
        echo ""

        # Drawdown alarmı
        if [ -n "$DD" ] && awk "BEGIN{exit !($DD > 5.0)}"; then
            CRIT_LINES+=("🚨 DRAWDOWN %${DD} (>%5 kritik) — risk yönetimi devrede mi? Pozisyon büyüklüklerini sorgula.")
            EXIT_CODE=2
        elif [ -n "$DD" ] && awk "BEGIN{exit !($DD > 2.0)}"; then
            WARN_LINES+=("⚠️  Drawdown %${DD} (>%2 dikkat) — birkaç tur daha izle.")
            [ "$EXIT_CODE" -lt 1 ] && EXIT_CODE=1
        fi

        # Anomaly birikimi
        if [ -n "$ANOM" ] && [ "$ANOM" -gt 50 ]; then
            CRIT_LINES+=("🚨 ANOMALY BİRİKİMİ: ${ANOM} (>50 kritik) — pipeline tıkanması olası, anomaly_by_kind'a bak.")
            EXIT_CODE=2
        elif [ -n "$ANOM" ] && [ "$ANOM" -gt 20 ]; then
            WARN_LINES+=("⚠️  Anomaly sayısı ${ANOM} (>20). Belirli bir hata kaynağı birikiyor olabilir.")
            [ "$EXIT_CODE" -lt 1 ] && EXIT_CODE=1
        fi
    fi
fi

# ── Sahte PnL detektörü (entry≈exit aynı tickte yüksek PnL)
# Bizim bulduğumuz bug: entry $74k, exit $87k → +%17 PnL aynı saniyede.
# Sezgi: <10sn yaşam + |PnL_pct| > 3% şüpheli
SAHTE_COUNT=$(echo "$recent_lines" | grep "Reason=STRATEGY_SIGNAL" | grep -E "PnL: \\\$[0-9]{2,}\\." | wc -l)
if [ "$SAHTE_COUNT" -gt 5 ]; then
    CRIT_LINES+=("🚨 SAHTE PnL ŞÜPHESİ: ${SAHTE_COUNT} STRATEGY_SIGNAL kapanışı \$10+ PnL ile. Entry/exit fiyat asimetrisi bug'ı geri dönmüş olabilir (fix d5636ac).")
    EXIT_CODE=2
fi

# ── Komisyon erozyonu (aç/kapa loop)
# Hız: STRATEGY_SIGNAL/dk > 20 → erozyon
if [ "$STRAT_SIG" -gt $((MINUTES * 20)) ]; then
    CRIT_LINES+=("🚨 KOMİSYON EROZYONU: dakikada ortalama $((STRAT_SIG / MINUTES))+ STRATEGY_SIGNAL kapanışı (>20/dk). MIN_HOLDING_SECS_STRATEGY çalışıyor mu? (fix a372e96)")
    EXIT_CODE=2
elif [ "$STRAT_SIG" -gt $((MINUTES * 10)) ]; then
    WARN_LINES+=("⚠️  STRATEGY_SIGNAL kapanış hızı yüksek (~$((STRAT_SIG / MINUTES))/dk). MIN_HOLDING_SECS_STRATEGY artırılabilir.")
    [ "$EXIT_CODE" -lt 1 ] && EXIT_CODE=1
fi

# ── DataIngest empty patlaması
DATA_EMPTY=$(echo "$recent_stderr" | grep "DataIngest empty" | grep -oE "[A-Z]+USDT|[A-Z]+USDC" | sort -u | wc -l)
if [ "$DATA_EMPTY" -gt 20 ]; then
    WARN_LINES+=("⚠️  DataIngest empty: ${DATA_EMPTY} unique sembolde 1m mum yok. İndirme job (E2) tetiklendi mi? 15dk bekle veya manuel download.")
    [ "$EXIT_CODE" -lt 1 ] && EXIT_CODE=1
fi

# ── ApiError birikimi
API_ERR=$(echo "$recent_stderr" | grep -c "anomaly\[Warning/ApiError\]")
if [ "$API_ERR" -gt 50 ]; then
    WARN_LINES+=("⚠️  ApiError ${API_ERR} kayıt. Belirli sembol(ler) Binance'dan veri çekemiyor (BLESSUSDT/BEATUSDT yaygın).")
    [ "$EXIT_CODE" -lt 1 ] && EXIT_CODE=1
fi

# ── Hedge çakışması (açık pozisyon LONG+SHORT aynı sembol)
HEDGE=$(echo "$recent_lines" | grep "TRADE_OPEN" | awk '{
    for (i=1; i<=NF; i++) {
        if ($i == "LONG"  || $i == "SHORT") side[$(i-1)] = side[$(i-1)] " " $i
    }
} END { for (s in side) { if (side[s] ~ "LONG" && side[s] ~ "SHORT") print s }}' | wc -l)
# Not: bu sayım açık+kapalı tüm geçmişi görüyor; gerçek hedge için
# (Açılış - Kapanış) net pozitif lazım. Basit sezgi olarak bırakıyoruz.
if [ "$HEDGE" -gt 3 ]; then
    WARN_LINES+=("⚠️  ${HEDGE} sembol pencerede hem LONG hem SHORT açılış almış. SlotGuard hedge engeli kapsam dışı (ScalpSwing yalnız kendi kanalını kontrol ediyor) olabilir.")
    [ "$EXIT_CODE" -lt 1 ] && EXIT_CODE=1
fi

# ── ScalpSwing aktivite durumu
echo -e "${BOLD}⚡ SCALPSWING DURUM${NC}"
if [ "$SCP_OPEN" -eq 0 ] && [ "$SWG_OPEN" -eq 0 ]; then
    echo -e "  ${DIM}Henüz SCP/SWG açılış yok — rejim uygun değil veya kanallar pasif.${NC}"
    if [ "$AUTO_DISABLED" -gt 0 ]; then
        WARN_LINES+=("⚠️  ScalpSwing Auto-Disabled tetiklendi (${AUTO_DISABLED} olay) — win_rate < 0.30 kanal kapanmış. config'i kontrol et.")
        [ "$EXIT_CODE" -lt 1 ] && EXIT_CODE=1
    fi
else
    echo "  Scalp:  $SCP_OPEN açılış"
    echo "  Swing:  $SWG_OPEN açılış"
fi
if [ "$TUNER_RUN" -gt 0 ]; then
    echo -e "  ${GREEN}🎚️ Tuner tetiklendi: ${TUNER_RUN} olay${NC}"
elif [ "$MINUTES" -ge 5 ]; then
    echo -e "  ${DIM}Tuner henüz çalışmadı (5dk default periyod; ${MINUTES}dk yetersiz olabilir).${NC}"
fi
echo ""

# ── Multi-TF durum (default debug seviyede log atmıyor — env kontrolü)
MTF_DEBUG=$(grep -c "MTF Filter" "$STDERR_LOG" 2>/dev/null || echo 0)
echo -e "${BOLD}🌐 MULTI-TF${NC}"
if [ "$MTF_DEBUG" -gt 0 ]; then
    echo -e "  ${GREEN}HTF filter tetiklenmesi: ${MTF_DEBUG} kayıt${NC} (RUST_LOG=debug açık görünüyor)"
else
    echo -e "  ${DIM}HTF filter debug seviyede sessiz (varsayılan). MULTI_TF_ENABLED=true zaten default; htf_loader çalışır.${NC}"
fi
echo ""

# ── Uyarılar / önerilen aksiyonlar
if [ ${#CRIT_LINES[@]} -eq 0 ] && [ ${#WARN_LINES[@]} -eq 0 ]; then
    echo -e "${BOLD}${GREEN}✅ DURUM: PASS — herhangi bir alarm yok.${NC}"
else
    if [ ${#CRIT_LINES[@]} -gt 0 ]; then
        echo -e "${BOLD}${RED}🚨 KRİTİK UYARILAR${NC}"
        for line in "${CRIT_LINES[@]}"; do
            echo -e "  ${RED}${line}${NC}"
        done
        echo ""
    fi
    if [ ${#WARN_LINES[@]} -gt 0 ]; then
        echo -e "${BOLD}${YELLOW}⚠️  UYARILAR${NC}"
        for line in "${WARN_LINES[@]}"; do
            echo -e "  ${YELLOW}${line}${NC}"
        done
        echo ""
    fi
    case $EXIT_CODE in
        2) echo -e "${BOLD}${RED}DURUM: CRITICAL — derhal incele, gerekirse SCALP_SWING_ENABLE=0 ile dispatch'i kapat.${NC}" ;;
        1) echo -e "${BOLD}${YELLOW}DURUM: WARN — birkaç tur daha gözle.${NC}" ;;
    esac
fi

echo ""
echo -e "${DIM}── log_doctor.sh tamam · çıkış kodu: $EXIT_CODE ──${NC}"
exit $EXIT_CODE
