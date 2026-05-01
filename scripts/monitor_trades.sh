#!/bin/bash
# scripts/monitor_trades.sh - Real-time trading monitoring dashboard

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color
BOLD='\033[1m'

LOG_FILE="logs/robotic_trader.log"
JSON_FILE="logs/trade_history.jsonl"

# Header (no clear - output capture friendly)
printf "\033[2J\033[H"  # Reset cursor without clearing buffer
echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BOLD}${CYAN}        📊 MEMOS TRADING - LIVE MONITORING DASHBOARD 📊        ${NC}"
echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
echo ""

# Check files
if [ ! -f "$LOG_FILE" ] && [ ! -f "$JSON_FILE" ]; then
    echo -e "${RED}❌ Trading logları bulunamadı!${NC}"
    echo -e "${YELLOW}Önce robotic trader'ı başlatın: cargo run --bin main_robotic${NC}"
    exit 1
fi

# Function: Parse JSON stats
calculate_stats() {
    if [ ! -f "$JSON_FILE" ]; then
        echo -e "${YELLOW}⚠️  JSON log dosyası henüz oluşmadı${NC}"
        return
    fi

    local total_trades=$(grep '"event_type":"TRADE"' "$JSON_FILE" 2>/dev/null | wc -l)
    local buy_trades=$(grep '"signal":"Buy"' "$JSON_FILE" 2>/dev/null | wc -l)
    local sell_trades=$(grep '"signal":"Sell"' "$JSON_FILE" 2>/dev/null | wc -l)
    local signals=$(grep '"event_type":"SIGNAL"' "$JSON_FILE" 2>/dev/null | wc -l)
    local risk_blocks=$(grep '"event_type":"RISK_BLOCK"' "$JSON_FILE" 2>/dev/null | wc -l)
    local errors=$(grep '"event_type":"ERROR"' "$JSON_FILE" 2>/dev/null | wc -l)

    # Calculate PnL (sum all pnl fields)
    local total_pnl=$(grep '"event_type":"TRADE"' "$JSON_FILE" 2>/dev/null | \
        grep -oP '"pnl":\s*[-+]?\d+\.?\d*' | \
        cut -d':' -f2 | \
        awk '{sum += $1} END {printf "%.2f", sum}')

    # Latest equity
    local latest_equity=$(grep '"equity":' "$JSON_FILE" 2>/dev/null | tail -1 | \
        grep -oP '"equity":\s*[-+]?\d+\.?\d*' | \
        cut -d':' -f2 | \
        awk '{printf "%.2f", $1}')

    # Win rate (approximate - positive PnL trades)
    local winning_trades=$(grep '"event_type":"TRADE"' "$JSON_FILE" 2>/dev/null | \
        grep -oP '"pnl":\s*[+]?\d+\.?\d*' | \
        grep -v '"pnl":0.00' | \
        grep -v '"pnl":-' | \
        wc -l)

    local win_rate=0
    if [ "$total_trades" -gt 0 ]; then
        win_rate=$(awk "BEGIN {printf \"%.1f\", ($winning_trades/$total_trades)*100}")
    fi

    # Display stats
    echo -e "${BOLD}${MAGENTA}📈 TRADING İSTATİSTİKLERİ${NC}"
    echo -e "─────────────────────────────────────────────────────────────"
    echo -e "  ${CYAN}Toplam İşlem:${NC}        ${BOLD}$total_trades${NC} (🟢 $buy_trades BUY | 🔴 $sell_trades SELL)"
    echo -e "  ${CYAN}Sinyal Üretimi:${NC}      ${BOLD}$signals${NC}"
    echo -e "  ${CYAN}Risk Blokajı:${NC}        ${BOLD}$risk_blocks${NC}"
    echo -e "  ${CYAN}Hata Sayısı:${NC}         ${BOLD}$errors${NC}"
    echo ""
    echo -e "  ${GREEN}Toplam PnL:${NC}          ${BOLD}\$$total_pnl${NC}"
    echo -e "  ${GREEN}Mevcut Equity:${NC}       ${BOLD}\$$latest_equity${NC}"
    echo -e "  ${GREEN}Kazanma Oranı:${NC}       ${BOLD}${win_rate}%${NC}"
    echo ""
}

# Function: Show last 10 events
show_recent_events() {
    echo -e "${BOLD}${YELLOW}🕒 SON 10 EVENT${NC}"
    echo -e "─────────────────────────────────────────────────────────────"
    
    if [ -f "$JSON_FILE" ]; then
        tail -10 "$JSON_FILE" | while read -r line; do
            local event_type=$(echo "$line" | grep -oP '"event_type":"\K[^"]+')
            local timestamp=$(echo "$line" | grep -oP '"timestamp":"\K[^"]+' | cut -d'T' -f2 | cut -d'+' -f1)
            local symbol=$(echo "$line" | grep -oP '"symbol":"\K[^"]+')
            local signal=$(echo "$line" | grep -oP '"signal":"\K[^"]+')
            local message=$(echo "$line" | grep -oP '"message":"\K[^"]+')

            case "$event_type" in
                "TRADE")
                    echo -e "  ${GREEN}[${timestamp}]${NC} 💰 ${BOLD}$event_type${NC} $symbol $signal - $message"
                    ;;
                "SIGNAL")
                    echo -e "  ${BLUE}[${timestamp}]${NC} 📡 ${BOLD}$event_type${NC} $symbol $signal - $message"
                    ;;
                "RISK_BLOCK")
                    echo -e "  ${RED}[${timestamp}]${NC} 🚨 ${BOLD}$event_type${NC} $symbol - $message"
                    ;;
                "ERROR")
                    echo -e "  ${YELLOW}[${timestamp}]${NC} ⚠️  ${BOLD}$event_type${NC} - $message"
                    ;;
                *)
                    echo -e "  ${NC}[${timestamp}] $event_type - $message"
                    ;;
            esac
        done
    else
        echo -e "${YELLOW}⚠️  JSON log dosyası henüz oluşmadı${NC}"
    fi
    echo ""
}

# Function: Show live tail (continuous monitoring)
live_tail() {
    echo -e "${BOLD}${CYAN}🔴 CANLI AKIŞ (Son 5 event)${NC}"
    echo -e "─────────────────────────────────────────────────────────────"
    echo ""
    
    if [ -f "$JSON_FILE" ]; then
        tail -5 "$JSON_FILE" | while read -r line; do
            timestamp=$(echo "$line" | grep -oP '"timestamp":"\K[^"]+' | cut -d'T' -f2 | cut -d'+' -f1)
            event_type=$(echo "$line" | grep -oP '"event_type":"\K[^"]+' | head -1)
            symbol=$(echo "$line" | grep -oP '"symbol":"\K[^"]+' | head -1)
            message=$(echo "$line" | grep -oP '"message":"\K[^"]+' | head -1)
            
            case "$event_type" in
                TRADE)
                    echo -e "  ${GREEN}[LIVE] [$timestamp] 💰 TRADE $symbol - $message${NC}"
                    ;;
                SIGNAL)
                    echo -e "  ${BLUE}[LIVE] [$timestamp] 📡 SIGNAL $symbol - $message${NC}"
                    ;;
                RISK_BLOCK)
                    echo -e "  ${RED}[LIVE] [$timestamp] 🚨 RISK_BLOCK $symbol - $message${NC}"
                    ;;
                ERROR)
                    echo -e "  ${YELLOW}[LIVE] [$timestamp] ⚠️  ERROR - $message${NC}"
                    ;;
                *)
                    echo -e "  [LIVE] [$timestamp] $event_type $symbol"
                    ;;
            esac
        done
    else
        echo -e "${YELLOW}⚠️  JSON log dosyası henüz oluşmadı${NC}"
    fi
    echo ""
}

# Main menu
echo -e "${BOLD}Ne yapmak istiyorsunuz?${NC}"
echo "  1) 📊 İstatistikler + Son 10 Event (Tek Snapshot)"
echo "  2) 🔴 Canlı Akış (Real-time Tail -f)"
echo "  3) 📁 Ham Logları Görüntüle"
echo "  0) Çıkış"
echo ""
if [ -t 0 ]; then
    read -p "Seçim (1/2/3/0): " -t 10 choice || choice="0"
else
    read choice || choice="0"
fi

case $choice in
    1)
        calculate_stats
        show_recent_events
        ;;
    2)
        live_tail
        ;;
    3)
        if [ -f "$LOG_FILE" ]; then
            less "$LOG_FILE"
        else
            echo -e "${RED}Log dosyası bulunamadı${NC}"
        fi
        ;;
    0)
        echo -e "${CYAN}Çıkış...${NC}"
        exit 0
        ;;
    *)
        echo -e "${RED}Geçersiz seçim${NC}"
        exit 1
        ;;
esac

echo ""
echo -e "${CYAN}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${CYAN}              Monitoring tamamlandı - $(date '+%Y-%m-%d %H:%M:%S')${NC}"
echo -e "${CYAN}═══════════════════════════════════════════════════════════════${NC}"
echo ""
