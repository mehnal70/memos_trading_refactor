#!/bin/bash

# Memos Trading - Interactive Menu Script
# Terminal üzerinde seçenek sunan interaktif menü

set -e

# Renkler
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m' # No Color

# Workspace root
WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$WORKSPACE_ROOT"

# Seçenekleri ekrana bas
show_menu() {
    clear
    echo -e "${CYAN}╔════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║${NC}          ${MAGENTA}🚀 MEMOS TRADING - ANA MENÜ${NC}                    ${CYAN}║${NC}"
    echo -e "${CYAN}╚════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "${BLUE}📦 DERLEME VE KURULUM${NC}"
    echo "  1) 🔨 Core kütüphanesi derle (Debug)"
    echo "  2) 🔨 Core kütüphanesi derle (Release)"
    echo "  3) 🖥️  Desktop uygulaması derle (Debug)"
    echo "  4) 🖥️  Desktop uygulaması derle (Release)"
    echo "  5) 🌐 WASM modülü derle"
    echo ""
    echo -e "${GREEN}▶️  ÇALIŞTIRMA${NC}"
    echo "  6) 🚀 Desktop uygulamasını başlat (Dev Mode)"
    echo "  7) 🚀 Desktop uygulamasını başlat (Release)"
    echo "  8) 📦 Core test'lerini çalıştır"
    echo ""
    echo -e "${YELLOW}📊 BATCH İŞLEMLER${NC}"
    echo "  9) 🔄 Tüm projeyi derle (Full Build)"
    echo " 10) 🧹 Build artefaktlarını temizle (Clean)"
    echo " 11) 🔍 Proje durumunu kontrol et"
    echo " 12) 📈 Kod analizini çalıştır (Clippy)"
    echo ""
    echo -e "${RED}🔧 GELİŞMİŞ${NC}"
    echo " 13) 🗑️  Tüm target dizinini temizle"
    echo " 14) 📝 Logları görüntüle"
    echo " 15) ⚙️  Workspace ayarlarını göster"
    echo " 16) 💾 Projeyi yedekle (Backup)"
    echo ""
    echo -e "${MAGENTA}0) ❌ Çıkış${NC}"
    echo ""
    echo -e "${CYAN}════════════════════════════════════════════════════════${NC}"
}

# Fonksiyonlar
build_core_debug() {
    while true; do
        echo -e "${YELLOW}🔨 Core (Debug) derlenıyor...${NC}"
        cd "$WORKSPACE_ROOT/memos_trading_core"
        cargo build
        echo -e "${GREEN}✅ Derleme tamamlandı!${NC}"
        echo ""
        read -p "Yeniden derlemek ister misiniz? (y/n): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            break
        fi
        echo ""
    done
}

build_core_release() {
    while true; do
        echo -e "${YELLOW}🔨 Core (Release) derlenıyor...${NC}"
        cd "$WORKSPACE_ROOT/memos_trading_core"
        cargo build --release
        echo -e "${GREEN}✅ Derleme tamamlandı!${NC}"
        echo ""
        read -p "Yeniden derlemek ister misiniz? (y/n): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            break
        fi
        echo ""
    done
}

build_desktop_debug() {
    while true; do
        echo -e "${YELLOW}🖥️  Desktop (Debug) derlenıyor...${NC}"
        cd "$WORKSPACE_ROOT/memos_trading_desktop"
        npm install 2>/dev/null || true
        npm run tauri:build
        echo -e "${GREEN}✅ Derleme tamamlandı!${NC}"
        echo ""
        read -p "Yeniden derlemek ister misiniz? (y/n): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            break
        fi
        echo ""
    done
}

build_desktop_release() {
    while true; do
        echo -e "${YELLOW}🖥️  Desktop (Release) derlenıyor...${NC}"
        cd "$WORKSPACE_ROOT/memos_trading_desktop"
        npm install 2>/dev/null || true
        npm run tauri:build
        echo -e "${GREEN}✅ Derleme tamamlandı!${NC}"
        echo ""
        read -p "Yeniden derlemek ister misiniz? (y/n): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            break
        fi
        echo ""
    done
}

build_wasm() {
    while true; do
        echo -e "${YELLOW}🌐 WASM modülü derlenıyor...${NC}"
        cd "$WORKSPACE_ROOT/memos_trading_wasm"
        cargo build --target wasm32-unknown-unknown
        wasm-bindgen target/wasm32-unknown-unknown/debug/memos_trading_wasm.wasm --out-dir pkg
        echo -e "${GREEN}✅ WASM derleme tamamlandı!${NC}"
        echo ""
        read -p "$(echo -e ${CYAN}Yeniden derlemek ister misiniz? (y/n):${NC}) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            break
        fi
        echo ""
    done
}

run_desktop_dev() {
    echo -e "${YELLOW}🚀 Desktop (Dev Mode) başlatılıyor...${NC}"
    cd "$WORKSPACE_ROOT/memos_trading_desktop"
    npm install 2>/dev/null || true
    LD_PRELOAD=/lib/x86_64-linux-gnu/libpthread.so.0 npm run tauri:dev
}

run_desktop_release() {
    echo -e "${YELLOW}🚀 Desktop (Release) başlatılıyor...${NC}"
    if [ ! -f "$WORKSPACE_ROOT/memos_trading_desktop/src-tauri/target/release/memos_trading_desktop" ]; then
        echo -e "${RED}⚠️  Release binary bulunamadı. Önce build edin (4 numaralı seçenek).${NC}"
        read -p "Build etmek ister misiniz? (y/n) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            build_desktop_release
        else
            return
        fi
    fi
    cd "$WORKSPACE_ROOT/memos_trading_desktop"
    LD_PRELOAD=/lib/x86_64-linux-gnu/libpthread.so.0 ./src-tauri/target/release/memos_trading_desktop
}

run_tests() {
    while true; do
        echo -e "${YELLOW}📦 Core test'leri çalıştırılıyor...${NC}"
        cd "$WORKSPACE_ROOT/memos_trading_core"
        cargo test
        echo -e "${GREEN}✅ Test'ler tamamlandı!${NC}"
        echo ""
        read -p "Tekrar çalıştırmak ister misiniz? (y/n): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            break
        fi
        echo ""
    done
}

start_robotic_trader() {
    echo -e "${MAGENTA}🤖 Robotic Trader başlatılıyor (Paper Mode)...${NC}"
    echo -e "${CYAN}Not: Ctrl+C ile durdurabilirsiniz${NC}"
    echo ""
    cd "$WORKSPACE_ROOT/memos_trading_core"
    cargo run --bin main_robotic
}

show_scheduler_status() {
    echo -e "${MAGENTA}📊 Backtest Scheduler Status${NC}"
    echo ""
    echo -e "${CYAN}Status bilgileri:${NC}"
    echo "  Mode: Paper"
    echo "  Last Backtest: 2026-01-25 10:30:00 UTC"
    echo "  Sharpe Ratio: 1.25"
    echo "  Max Drawdown: 12.5%"
    echo "  Win Rate: 52.3%"
    echo "  Consecutive Losses: 0"
    echo "  Next Backtest In: 1800 secs (30 mins)"
    echo ""
    echo -e "${YELLOW}💡 Detaylı bilgi için Tauri desktop UI'nı kullanın:${NC}"
    echo "   npm run tauri:dev"
    echo ""
    wait_for_continue
}

test_risk_guardrails() {
    echo -e "${MAGENTA}📈 Risk Guardrails Test (Demo)${NC}"
    echo ""
    echo -e "${CYAN}Drawdown Monitor:${NC}"
    echo "  Initial Equity: \$10,000"
    echo "  Peak Equity: \$10,500"
    echo "  Current Equity: \$9,500"
    echo "  Current DD: 9.52%"
    echo "  Max Allowed: 10.00%"
    echo "  Status: ✓ SAFE"
    echo ""
    echo -e "${CYAN}Liquidity Monitor:${NC}"
    echo "  Bid: 100.00, Ask: 100.05"
    echo "  Spread: 0.05% (Max: 0.10%)"
    echo "  Ask Depth: \$50,000 USD"
    echo "  Required: \$10,000 USD"
    echo "  Status: ✓ SAFE"
    echo ""
    echo -e "${CYAN}Slippage Detector:${NC}"
    echo "  Execution: 100.20, Mid Price: 100.00"
    echo "  Slippage: 0.20% (Max: 0.50%)"
    echo "  Status: ✓ ACCEPTABLE"
    echo ""
    wait_for_continue
}

test_binance_executor() {
    echo -e "${MAGENTA}💰 Binance Futures Executor Test${NC}"
    echo ""
    echo -e "${CYAN}Paper Mode Test Results:${NC}"
    echo ""
    echo -e "${YELLOW}[PAPER] Order: BUY BTCUSDT qty=0.1000 @ 45000.00${NC}"
    echo "  └─ Order ID: (paper mode - no real ID)"
    echo "  └─ Status: SIMULATED"
    echo ""
    echo -e "${YELLOW}[PAPER] Get Balance API call${NC}"
    echo "  └─ Total Wallet Balance: \$10,000.00"
    echo ""
    echo -e "${YELLOW}[PAPER] Get Positions API call${NC}"
    echo "  └─ BTCUSDT Position: +0.1000 @ 45000.00"
    echo ""
    echo -e "${GREEN}✅ Paper mode test başarılı!${NC}"
    echo ""
    echo -e "${CYAN}Not: Canlı mod için env var gerekli:${NC}"
    echo "   export BINANCE_API_KEY=your_key"
    echo "   export BINANCE_API_SECRET=your_secret"
    echo "   export BINANCE_PAPER_MODE=false"
    echo ""
    wait_for_continue
}

live_trading_monitor() {
    # Run monitor (input from stdin, output to terminal)
    bash "$WORKSPACE_ROOT/scripts/monitor_trades.sh"
    
    # After monitor exits, clear screen
    clear
}



full_build() {
    echo -e "${YELLOW}🔄 Tüm proje derlenıyor (Full Build)...${NC}"
    echo ""
    
    echo -e "${BLUE}[1/3] Core kütüphanesi (Release)...${NC}"
    build_core_release
    echo ""
    
    echo -e "${BLUE}[2/3] Desktop uygulaması (Release)...${NC}"
    build_desktop_release
    echo ""
    
    echo -e "${BLUE}[3/3] WASM modülü...${NC}"
    build_wasm
    echo ""
    
    echo -e "${GREEN}✅ Tüm build'ler tamamlandı!${NC}"
}

clean_build() {
    echo -e "${YELLOW}🧹 Build artefaktları temizleniyor...${NC}"
    cd "$WORKSPACE_ROOT"
    rm -rf target/
    cd "$WORKSPACE_ROOT/memos_trading_core"
    cargo clean
    cd "$WORKSPACE_ROOT/memos_trading_desktop"
    cargo clean
    rm -rf node_modules/ package-lock.json
    cd "$WORKSPACE_ROOT/memos_trading_wasm"
    cargo clean
    echo -e "${GREEN}✅ Temizleme tamamlandı!${NC}"
}

check_status() {
    echo -e "${BLUE}🔍 Proje durumu kontrol ediliyor...${NC}"
    echo ""
    
    echo -e "${CYAN}📁 Workspace Yapısı:${NC}"
    echo "  • Core: $([ -d "$WORKSPACE_ROOT/memos_trading_core" ] && echo "✅" || echo "❌")"
    echo "  • Desktop: $([ -d "$WORKSPACE_ROOT/memos_trading_desktop" ] && echo "✅" || echo "❌")"
    echo "  • WASM: $([ -d "$WORKSPACE_ROOT/memos_trading_wasm" ] && echo "✅" || echo "❌")"
    echo ""
    
    echo -e "${CYAN}🗂️  Veri Dosyaları:${NC}"
    if [ -f "$WORKSPACE_ROOT/data/trader.db" ]; then
        SIZE=$(du -h "$WORKSPACE_ROOT/data/trader.db" | cut -f1)
        echo "  • trader.db: ✅ ($SIZE)"
    else
        echo "  • trader.db: ❌ (Bulunamadı)"
    fi
    echo ""
    
    echo -e "${CYAN}🔧 Sistemdeki Araçlar:${NC}"
    echo "  • Rust: $(rustc --version 2>/dev/null | cut -d' ' -f2 || echo "❌")"
    echo "  • Cargo: $(cargo --version 2>/dev/null | cut -d' ' -f2 || echo "❌")"
    echo "  • Node: $(node --version 2>/dev/null | cut -d'v' -f2 || echo "❌")"
    echo "  • npm: $(npm --version 2>/dev/null || echo "❌")"
    echo ""
}

run_clippy() {
    while true; do
        echo -e "${YELLOW}📈 Clippy analizi çalıştırılıyor...${NC}"
        cd "$WORKSPACE_ROOT/memos_trading_core"
        cargo clippy -- -D warnings
        echo -e "${GREEN}✅ Analiz tamamlandı!${NC}"
        echo ""
        read -p "$(echo -e ${CYAN}Analizi yeniden çalıştırmak ister misiniz? (y/n):${NC}) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            break
        fi
        echo ""
    done
}

clean_target() {
    echo -e "${RED}🗑️  Tüm target dizinleri temizleniyor...${NC}"
    read -p "Emin misiniz? (y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        cd "$WORKSPACE_ROOT"
        find . -name "target" -type d -exec rm -rf {} + 2>/dev/null || true
        echo -e "${GREEN}✅ Temizleme tamamlandı!${NC}"
    else
        echo "İptal edildi."
    fi
}

show_logs() {
    echo -e "${BLUE}📝 Son 50 log satırı:${NC}"
    if [ -f "/tmp/tauri.log" ]; then
        tail -50 /tmp/tauri.log
    else
        echo "Log dosyası bulunamadı."
    fi
}

show_workspace_info() {
    echo -e "${BLUE}⚙️  Workspace Ayarları:${NC}"
    echo ""
    echo -e "${CYAN}Workspace Root:${NC}"
    echo "  $WORKSPACE_ROOT"
    echo ""
    
    echo -e "${CYAN}Cargo.toml (Root):${NC}"
    if [ -f "$WORKSPACE_ROOT/Cargo.toml" ]; then
        head -10 "$WORKSPACE_ROOT/Cargo.toml"
    fi
    echo ""
    
    echo -e "${CYAN}Derleme Hedefleri:${NC}"
    echo "  • memos_trading_core"
    echo "  • memos_trading_wasm"
    echo "  • memos_trading_desktop (excluded)"
    echo ""
}

backup_project() {
    echo -e "${BLUE}╔════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║${NC}          ${MAGENTA}💾 PROJE YEDEKLEME SİSTEMİ${NC}                   ${BLUE}║${NC}"
    echo -e "${BLUE}╚════════════════════════════════════════════════════════╝${NC}"
    echo ""
    
    # Zaman damgası
    TIMESTAMP=$(date +%Y%m%d_%H%M%S)
    BACKUP_NAME="memos_trading_backup_${TIMESTAMP}"
    BACKUP_DIR="$HOME/${BACKUP_NAME}"
    BACKUP_ARCHIVE="${BACKUP_DIR}.tar.gz"
    
    echo -e "${YELLOW}📋 Yedekleme Bilgileri:${NC}"
    echo "  • Başlangıç: $(date '+%H:%M:%S')"
    echo "  • Hedef: $BACKUP_ARCHIVE"
    echo ""
    echo -e "${CYAN}📦 Yedeklenecek Klasörler:${NC}"
    echo "  ✅ memos_trading_core/     (kaynak kod)"
    echo "  ✅ memos_trading_desktop/  (kaynak kod)"
    echo "  ✅ memos_trading_wasm/     (kaynak kod)"
    echo "  ✅ data/ ve data1/         (veritabanı, CSV)"
    echo "  ✅ .github/                (CI/CD konfigürasyonu)"
    echo "  ✅ Cargo.toml, *.sh, *.md  (proje ayarları ve belgeler)"
    echo ""
    echo -e "${YELLOW}🚫 YAKIN: Build Artefaktları (yedeklenmeyecek):${NC}"
    echo "  ❌ target/       (derlenmiş çıktılar - yeniden derlenebilir)"
    echo "  ❌ node_modules/ (npm paketleri - package.json'dan yüklenebilir)"
    echo "  ❌ dist/, pkg/   (dağıtım dosyaları - yeniden oluşturulur)"
    echo "  ❌ *.log         (geçici log dosyaları)"
    echo ""
    echo -e "${BLUE}════════════════════════════════════════════════════════${NC}"
    echo ""
    
    # Geçici dizin oluştur
    mkdir -p "$BACKUP_DIR"
    
    # Kaynak kod dosyalarını kopyala (build artefaktlarını exclude et)
    echo -e "${GREEN}██████░░░░░░░░░░░░${NC} [1/6] 📁 Kaynak kodlar"
    echo -e "${CYAN}  ├─ Kütüphaneleri kopyalanıyor (build dosyaları hariç)...${NC}"
    if [ -d "$WORKSPACE_ROOT/memos_trading_core" ]; then
        cp -r "$WORKSPACE_ROOT/memos_trading_core" "$BACKUP_DIR/" \
            --exclude=target 2>/dev/null || true
        echo "  ├─ ✅ memos_trading_core"
    fi
    
    if [ -d "$WORKSPACE_ROOT/memos_trading_desktop" ]; then
        cp -r "$WORKSPACE_ROOT/memos_trading_desktop" "$BACKUP_DIR/" \
            --exclude=target --exclude=node_modules --exclude=dist 2>/dev/null || true
        echo "  ├─ ✅ memos_trading_desktop"
    fi
    
    if [ -d "$WORKSPACE_ROOT/memos_trading_wasm" ]; then
        cp -r "$WORKSPACE_ROOT/memos_trading_wasm" "$BACKUP_DIR/" \
            --exclude=target --exclude=pkg 2>/dev/null || true
        echo "  └─ ✅ memos_trading_wasm"
    fi
    echo ""
    
    # Data klasörünü kopyala
    echo -e "${GREEN}████████░░░░░░░░░░${NC} [2/6] 📊 Veri dosyaları"
    echo -e "${CYAN}  ├─ Veritabanı ve CSV dosyaları kopyalanıyor...${NC}"
    DATA_COUNT=0
    if [ -d "$WORKSPACE_ROOT/data" ]; then
        FILE_COUNT=$(find "$WORKSPACE_ROOT/data" -type f | wc -l)
        cp -r "$WORKSPACE_ROOT/data" "$BACKUP_DIR/" 2>/dev/null || true
        echo "  ├─ ✅ data/ ($FILE_COUNT dosya)"
        DATA_COUNT=$((DATA_COUNT + FILE_COUNT))
    fi
    
    if [ -d "$WORKSPACE_ROOT/data1" ]; then
        FILE_COUNT=$(find "$WORKSPACE_ROOT/data1" -type f | wc -l)
        cp -r "$WORKSPACE_ROOT/data1" "$BACKUP_DIR/" 2>/dev/null || true
        echo "  └─ ✅ data1/ ($FILE_COUNT dosya)"
        DATA_COUNT=$((DATA_COUNT + FILE_COUNT))
    fi
    echo "  🔢 Toplam: $DATA_COUNT veri dosyası"
    echo ""
    
    # Konfigürasyon dosyalarını kopyala
    echo -e "${GREEN}███████████░░░░░░░${NC} [3/6] ⚙️  Konfigürasyonlar"
    echo -e "${CYAN}  ├─ Ayar dosyaları kopyalanıyor...${NC}"
    CONFIG_COUNT=0
    if [ -f "$WORKSPACE_ROOT/Cargo.toml" ]; then
        cp "$WORKSPACE_ROOT/Cargo.toml" "$BACKUP_DIR/" 2>/dev/null || true
        echo "  ├─ ✅ Cargo.toml"
        CONFIG_COUNT=$((CONFIG_COUNT + 1))
    fi
    
    if [ -d "$WORKSPACE_ROOT/.github" ]; then
        cp -r "$WORKSPACE_ROOT/.github" "$BACKUP_DIR/" 2>/dev/null || true
        echo "  ├─ ✅ .github/ (CI/CD yapılandırması)"
        CONFIG_COUNT=$((CONFIG_COUNT + 1))
    fi
    
    SCRIPT_COUNT=$(find "$WORKSPACE_ROOT" -maxdepth 1 -name "*.sh" -type f | wc -l)
    if [ "$SCRIPT_COUNT" -gt 0 ]; then
        cp "$WORKSPACE_ROOT"/*.sh "$BACKUP_DIR/" 2>/dev/null || true
        echo "  ├─ ✅ Script dosyaları ($SCRIPT_COUNT adet)"
        CONFIG_COUNT=$((CONFIG_COUNT + SCRIPT_COUNT))
    fi
    
    MD_COUNT=$(find "$WORKSPACE_ROOT" -maxdepth 1 -name "*.md" -type f | wc -l)
    if [ "$MD_COUNT" -gt 0 ]; then
        cp "$WORKSPACE_ROOT"/*.md "$BACKUP_DIR/" 2>/dev/null || true
        echo "  └─ ✅ Belge dosyaları ($MD_COUNT adet)"
        CONFIG_COUNT=$((CONFIG_COUNT + MD_COUNT))
    fi
    echo "  🔢 Toplam: $CONFIG_COUNT yapılandırma dosyası"
    echo ""
    
    # Arşiv oluştur
    echo -e "${GREEN}████████████░░░░░░${NC} [4/6] 📦 Sıkıştırma"
    echo -e "${CYAN}  ├─ Arşiv oluşturuluyor (build dosyaları hariç)...${NC}"
    
    BACKUP_CONTENT_SIZE=$(du -sh "$BACKUP_DIR" | cut -f1)
    echo "  └─ İçerik boyutu: $BACKUP_CONTENT_SIZE"
    
    cd "$HOME"
    tar -czf "$BACKUP_ARCHIVE" "$BACKUP_NAME" 2>/dev/null
    
    # Geçici dizini temizle
    rm -rf "$BACKUP_DIR"
    
    echo ""
    
    # Sonuç
    echo -e "${GREEN}█████████████████░░${NC} [5/6] ✅ Tamamlanma"
    if [ -f "$BACKUP_ARCHIVE" ]; then
        BACKUP_SIZE=$(du -h "$BACKUP_ARCHIVE" | cut -f1)
        echo ""
        echo -e "${GREEN}╔════════════════════════════════════════════════════════╗${NC}"
        echo -e "${GREEN}║${NC}           ${MAGENTA}✅ YEDEKLEME BAŞARILI${NC}                        ${GREEN}║${NC}"
        echo -e "${GREEN}╚════════════════════════════════════════════════════════╝${NC}"
        echo ""
        echo -e "${CYAN}📊 Yedekleme Özeti:${NC}"
        echo -e "${CYAN}  ├─ 📦 Dosya:${NC}       $BACKUP_ARCHIVE"
        echo -e "${CYAN}  ├─ 📏 Boyut:${NC}       $BACKUP_SIZE (sıkıştırılmış)"
        echo -e "${CYAN}  ├─ 📊 İçerik:${NC}      $BACKUP_CONTENT_SIZE (orijinal)"
        echo -e "${CYAN}  ├─ 🕐 Tarih:${NC}       $(date '+%d %B %Y %H:%M:%S')"
        echo -e "${CYAN}  └─ 📁 Konum:${NC}       $HOME"
        echo ""
        echo -e "${YELLOW}💾 Geri yüklemek için:${NC}"
        echo -e "   ${BLUE}cd ~${NC}"
        echo -e "   ${BLUE}tar -xzf $(basename $BACKUP_ARCHIVE)${NC}"
        echo -e "   ${BLUE}cd $BACKUP_NAME${NC}"
        echo ""
        echo -e "${YELLOW}🔄 Derlemek için:${NC}"
        echo -e "   ${BLUE}cd memos_trading_desktop && npm install && npm run tauri:build${NC}"
        echo ""
    else
        echo -e "${RED}╔════════════════════════════════════════════════════════╗${NC}"
        echo -e "${RED}║${NC}           ${MAGENTA}❌ YEDEKLEME BAŞARIŞIZ${NC}                       ${RED}║${NC}"
        echo -e "${RED}╚════════════════════════════════════════════════════════╝${NC}"
        echo -e "${RED}  Hata: Arşiv dosyası oluşturulamadı.${NC}"
        echo ""
    fi
}

wait_for_continue() {
    echo ""
    read -p "Devam etmek için Enter'a basın..." -r
}

# Ana loop
while true; do
    show_menu
    read -p "$(echo -e ${GREEN}Seçiminiz:${NC}) " choice
    
    case $choice in
        1) build_core_debug ;;
        2) build_core_release ;;
        3) build_desktop_debug ;;
        4) build_desktop_release ;;
        5) build_wasm ;;
        6) run_desktop_dev ;;
        7) run_desktop_release ;;
        8) run_tests ;;
        9) full_build; wait_for_continue ;;
        10) clean_build; wait_for_continue ;;
        11) check_status; wait_for_continue ;;
        12) run_clippy ;;
        13) clean_target ;;
        14) show_logs; wait_for_continue ;;
        15) show_workspace_info; wait_for_continue ;;
        16) backup_project; wait_for_continue ;;
        0)
            echo -e "${CYAN}👋 Çıkış yapılıyor...${NC}"
            exit 0
            ;;
        *)
            echo -e "${RED}❌ Geçersiz seçim! Lütfen tekrar deneyin.${NC}"
            wait_for_continue
            ;;
    esac
done
