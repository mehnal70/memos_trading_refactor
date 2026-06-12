#!/usr/bin/env bash
# ─── Memos RTC — Otonom Trading Konsolu ───────────────────────────────────────
# Kullanım:
#   ./run_rtc.sh                       # TUI · debug · kağıt mod
#   ./run_rtc.sh --release             # TUI · release
#   ./run_rtc.sh --headless            # Headless servis (rtc_headless)
#   ./run_rtc.sh --headless --release  # Headless · release
#   ./run_rtc.sh --healthcheck         # Çift kol smoke (paper + live dry-run)
#   ./run_rtc.sh --build-only          # Sadece derle, çalıştırma
#   BINANCE_API_KEY=xxx BINANCE_API_SECRET=yyy ./run_rtc.sh
# ──────────────────────────────────────────────────────────────────────────────

set -e

WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$WORKSPACE_ROOT"

# .launch.conf RUNTIME env'ini yükle (TEK KAYNAK) → doğrudan ./run_rtc.sh çağrısı da launch.sh
# menüsüyle aynı ayarları alır. launch.sh zaten export ediyorsa çift-yükleme zararsız (aynı değer).
# shellcheck source=scripts/lib_launchconf.sh
. "$WORKSPACE_ROOT/scripts/lib_launchconf.sh"
load_launch_conf "$WORKSPACE_ROOT/scripts/.launch.conf"

# .env (gitignored) — gerçek secret'lar (BINANCE_API_KEY/SECRET) burada, .launch.conf'ta DEĞİL.
# Aynı güvenli satır-satır parser'la yükle ki aşağıdaki mod-banner'ı gerçeği göstersin.
# Asıl binary zaten dotenvy::dotenv() ile .env'i yükler → bu satır yalnız ön-kontrol banner'ı içindir.
load_launch_conf "$WORKSPACE_ROOT/.env"

# Varsayılanlar
TARGET_BIN="rtc_tui"
BUILD_MODE="debug"
BUILD_ONLY=false

# Argüman parse
for arg in "$@"; do
    case "$arg" in
        --release)     BUILD_MODE="release" ;;
        --headless)    TARGET_BIN="rtc_headless" ;;
        --tui)         TARGET_BIN="rtc_tui" ;;
        --healthcheck) TARGET_BIN="rtc_healthcheck" ;;
        --build-only)  BUILD_ONLY=true ;;
        -h|--help)
            sed -n '2,11p' "$0"
            exit 0
            ;;
        *)
            echo "⚠️  Bilinmeyen argüman: $arg (--help için -h)"
            exit 1
            ;;
    esac
done

if [ "$BUILD_MODE" = "release" ]; then
    BINARY="$WORKSPACE_ROOT/target/release/$TARGET_BIN"
    CARGO_FLAGS=(--release)
else
    BINARY="$WORKSPACE_ROOT/target/debug/$TARGET_BIN"
    CARGO_FLAGS=()
fi

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║         Memos RTC — Otonom Trading Konsolu                   ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo "  Hedef     : $TARGET_BIN"
echo "  Profil    : $BUILD_MODE"
echo "  Workspace : $WORKSPACE_ROOT"
echo ""

# Ortam değişkenleri bilgisi
if [ -z "$BINANCE_API_KEY" ] || [ -z "$BINANCE_API_SECRET" ]; then
    echo "⚠️  BINANCE_API_KEY / BINANCE_API_SECRET bulunamadı → Kağıt mod aktif"
else
    echo "✅  Binance API key yüklendi → Gerçek mod"
fi
echo ""

# Derleme — workspace kökünden -p <paket> ile
echo "🔨 $TARGET_BIN ($BUILD_MODE) derleniyor..."
cargo build "${CARGO_FLAGS[@]}" -p "$TARGET_BIN"

echo "✅ Derleme tamamlandı: $BINARY"
echo ""

if [ "$BUILD_ONLY" = true ]; then
    echo "ℹ️  --build-only modu: çalıştırılmıyor."
    exit 0
fi

if [ ! -x "$BINARY" ]; then
    echo "❌ Binary bulunamadı: $BINARY"
    exit 1
fi

echo "🚀 $TARGET_BIN başlatılıyor..."
echo "──────────────────────────────────────────────────────────────"
exec "$BINARY"
