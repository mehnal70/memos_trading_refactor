#!/usr/bin/env bash
# scripts/mt5/setup_and_run.sh — MT5 (Wine) kurulum + MemosBridge EA deploy/derle + köprü+motor.
#
# Memos trading-core ↔ MetaTrader 5 köprüsünü uçtan uca ayağa kaldırır:
#   1) Wine kontrol → MT5 terminalini (yoksa) MetaQuotes'tan indir + sessiz kur
#   2) MemosBridge.mq5 EA'yı MT5'in Experts klasörüne kopyala + MetaEditor ile derle (.ex5)
#   3) MT5 terminalini /portable başlat (veri klasörü kurulum dizininde → deterministik yol)
#   4) Memos motorunu paper-mt5 profiliyle başlat (Rust = köprü TCP SERVER, EA = client)
#
# OTOMATİKLEŞTİRİLEMEYEN (GUI/hesap — script SONUNDA listelenir):
#   • MT5'te ücretsiz DEMO hesap aç/giriş yap (forex/altın verisi + sembol listesi bunsuz gelmez)
#   • MemosBridge EA'yı bir grafiğe SÜRÜKLE; "Allow Algo Trading" + (Faz 2 için) InpEnableExec
#   • Tools › Options › Expert Advisors → soket adres listesine 127.0.0.1 ekle (build ≥1930)
#
# Kullanım:
#   scripts/mt5/setup_and_run.sh            # tüm aşamalar (install→deploy→launch→engine)
#   scripts/mt5/setup_and_run.sh install    # yalnız MT5 kurulumu
#   scripts/mt5/setup_and_run.sh deploy      # yalnız EA kopyala+derle
#   scripts/mt5/setup_and_run.sh launch      # yalnız MT5 terminalini başlat
#   scripts/mt5/setup_and_run.sh engine      # yalnız Memos motorunu (paper-mt5) başlat
#   scripts/mt5/setup_and_run.sh check       # yalnız durum raporu (kurulum yapmaz)
set -euo pipefail

# ── Konum + ayarlar ───────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
export WINEPREFIX="${WINEPREFIX:-$HOME/.wine}"
export WINEDEBUG="${WINEDEBUG:-fixme-all,err-all}"   # log gürültüsünü kıs
MT5_URL="${MT5_URL:-https://download.mql5.com/cdn/web/metaquotes.software.corp/mt5/mt5setup.exe}"
INSTALL_DIR="$WINEPREFIX/drive_c/Program Files/MetaTrader 5"
TERMINAL_EXE="$INSTALL_DIR/terminal64.exe"
EDITOR_EXE="$INSTALL_DIR/metaeditor64.exe"
EA_SRC="$SCRIPT_DIR/MemosBridge.mq5"
EA_DIR="$INSTALL_DIR/MQL5/Experts"
SETUP_EXE="${SETUP_EXE:-/tmp/mt5setup.exe}"
BRIDGE_ADDR="${MT5_BRIDGE_ADDR:-127.0.0.1:9001}"

c_ok()   { printf '\033[32m✅ %s\033[0m\n' "$*"; }
c_warn() { printf '\033[33m⚠️  %s\033[0m\n' "$*"; }
c_err()  { printf '\033[31m❌ %s\033[0m\n' "$*" >&2; }
c_info() { printf '\033[36mℹ️  %s\033[0m\n' "$*"; }

# ── check: durum raporu ─────────────────────────────────────────────────────────
do_check() {
  echo "── MT5 köprü altyapı durumu ─────────────────────────"
  if command -v wine >/dev/null 2>&1; then c_ok "Wine: $(wine --version 2>/dev/null)"; else c_err "Wine kurulu DEĞİL (sudo apt install wine64 / winehq-stable)"; fi
  echo "   WINEPREFIX = $WINEPREFIX"
  [ -f "$TERMINAL_EXE" ] && c_ok "MT5 terminali: $TERMINAL_EXE" || c_warn "MT5 terminali yok ($TERMINAL_EXE) → 'install' gerekli"
  [ -f "$EDITOR_EXE" ]   && c_ok "MetaEditor: $EDITOR_EXE"      || c_warn "MetaEditor yok → EA derlenemez"
  [ -f "$EA_DIR/MemosBridge.mq5" ] && c_ok "EA kaynağı yerleştirilmiş" || c_warn "EA henüz Experts'e kopyalanmadı"
  [ -f "$EA_DIR/MemosBridge.ex5" ] && c_ok "EA derlenmiş (.ex5)"        || c_warn "EA derlenmemiş (.ex5 yok)"
  if ss -tnl 2>/dev/null | grep -q ':9001'; then c_ok "Köprü dinliyor (9001) — Memos motoru ayakta"; else c_warn "Köprü (9001) dinlemiyor → motor 'engine' ile başlat"; fi
  echo "─────────────────────────────────────────────────────"
}

# ── install: MT5 indir + kur ─────────────────────────────────────────────────────
do_install() {
  command -v wine >/dev/null 2>&1 || { c_err "Wine yok — önce kur."; exit 1; }
  if [ -f "$TERMINAL_EXE" ]; then c_ok "MT5 zaten kurulu: $TERMINAL_EXE"; return 0; fi
  if [ ! -f "$SETUP_EXE" ]; then
    c_info "MT5 kurulumu indiriliyor: $MT5_URL"
    if command -v curl >/dev/null 2>&1; then curl -fL --retry 3 -o "$SETUP_EXE" "$MT5_URL";
    else wget -O "$SETUP_EXE" "$MT5_URL"; fi
  fi
  c_info "MT5 sessiz kuruluyor (wine mt5setup.exe /auto) — birkaç dakika sürebilir..."
  wine "$SETUP_EXE" /auto || c_warn "Kurulumcu çıkış kodu ≠0 (sihirbaz GUI'si manuel ilerletme isteyebilir)"
  # terminal64.exe oluşana kadar bekle (≤120sn)
  for _ in $(seq 1 60); do [ -f "$TERMINAL_EXE" ] && break; sleep 2; done
  [ -f "$TERMINAL_EXE" ] && c_ok "MT5 kuruldu: $TERMINAL_EXE" || { c_err "Kurulum tamamlanamadı (terminal64.exe yok). Kurulum sihirbazını GUI'de tamamlayıp tekrar dene."; exit 1; }
}

# ── deploy: EA kopyala + derle ───────────────────────────────────────────────────
do_deploy() {
  [ -f "$EA_SRC" ] || { c_err "EA kaynağı yok: $EA_SRC"; exit 1; }
  mkdir -p "$EA_DIR"
  cp -f "$EA_SRC" "$EA_DIR/MemosBridge.mq5"
  c_ok "EA kopyalandı → $EA_DIR/MemosBridge.mq5"
  if [ -f "$EDITOR_EXE" ]; then
    c_info "EA derleniyor (MetaEditor /compile)..."
    # MetaEditor Windows-yolu ister; portable veri klasörü kurulum dizininde.
    wine "$EDITOR_EXE" /compile:"C:\\Program Files\\MetaTrader 5\\MQL5\\Experts\\MemosBridge.mq5" /log 2>/dev/null || true
    sleep 2
    [ -f "$EA_DIR/MemosBridge.ex5" ] && c_ok "EA derlendi: MemosBridge.ex5" || c_warn "Otomatik derleme doğrulanamadı → MetaEditor'da F7 ile elle derle"
  else
    c_warn "MetaEditor yok → EA'yı MT5 içindeki MetaEditor'da (F7) elle derle"
  fi
}

# ── launch: MT5 terminalini başlat ───────────────────────────────────────────────
do_launch() {
  [ -f "$TERMINAL_EXE" ] || { c_err "MT5 kurulu değil → önce 'install'"; exit 1; }
  c_info "MT5 terminali başlatılıyor (/portable)..."
  ( cd "$INSTALL_DIR" && nohup wine "$TERMINAL_EXE" /portable >/tmp/mt5_terminal.log 2>&1 & )
  c_ok "MT5 başlatıldı (log: /tmp/mt5_terminal.log). GUI'de demo hesaba giriş yap + EA'yı grafiğe ekle."
}

# ── engine: Memos motorunu paper-mt5 ile başlat (= köprü server) ─────────────────
do_engine() {
  cd "$REPO_DIR"
  c_info "Memos motoru paper-mt5 profiliyle başlatılıyor (Rust = köprü TCP server :${BRIDGE_ADDR##*:})..."
  ./memos use paper-mt5
  ./memos up
  c_ok "Motor ayakta. EA bağlanınca köprü ${BRIDGE_ADDR} üzerinden veri akar."
}

# ── manuel adımlar hatırlatıcısı ────────────────────────────────────────────────
print_manual() {
  cat <<EOF

╔══════════════════════════════════════════════════════════════════╗
║  KALAN MANUEL ADIMLAR (GUI/hesap — script otomatikleştiremez)      ║
╚══════════════════════════════════════════════════════════════════╝
  1) MT5'te ÜCRETSİZ DEMO hesap aç (File › Open an Account → MetaQuotes
     Demo) — forex/altın verisi + sembol listesi hesapsız GELMEZ.
  2) Market Watch'a sembolleri ekle: EURUSD, GBPUSD, XAUUSD, ... (sağ tık
     › Show All) — profildeki PINNED_SYMBOLS ile eşleşmeli.
  3) Soket izni: Tools › Options › Expert Advisors → "Allow algorithmic
     trading" + adres listesine 127.0.0.1 ekle.
  4) MemosBridge EA'yı bir grafiğe SÜRÜKLE. Girdiler: InpHost=127.0.0.1,
     InpPort=${BRIDGE_ADDR##*:}. (Faz 2 emir yürütme için InpEnableExec=true + AutoTrading.)
  5) Geçmiş veri çek (motor ayaktayken):
       cargo run --release --example download_mt5 -- mt5 1h EURUSD,GBPUSD,XAUUSD 2000
  6) Durumu izle: ./memos attach   ·   köprü kontrol: scripts/mt5/setup_and_run.sh check
EOF
}

case "${1:-all}" in
  check)   do_check ;;
  install) do_install ;;
  deploy)  do_deploy ;;
  launch)  do_launch ;;
  engine)  do_engine ;;
  all)     do_check; do_install; do_deploy; do_launch; do_engine; print_manual ;;
  *)       c_err "bilinmeyen komut: $1"; echo "kullanım: $0 [check|install|deploy|launch|engine|all]"; exit 1 ;;
esac
