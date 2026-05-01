#!/usr/bin/env bash
# ─── Memos RTC CLI — Otonom Trading Konsolu ───────────────────────────────────
# Kullanım:
#   ./run_rtc.sh               # Kağıt mod (API key gerekmez)
#   ./run_rtc.sh --release     # Release binary ile çalıştır
#   ./run_rtc.sh --build-only  # Sadece derle, çalıştırma
#   BINANCE_API_KEY=xxx BINANCE_API_SECRET=yyy ./run_rtc.sh
# ──────────────────────────────────────────────────────────────────────────────

set -e

WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE_DIR="$WORKSPACE_ROOT/memos_trading_core"
BINARY_DEBUG="$WORKSPACE_ROOT/target/debug/rtc_cli"
BINARY_RELEASE="$WORKSPACE_ROOT/target/release/rtc_cli"

BUILD_MODE="debug"
BUILD_ONLY=false

# Argüman parse
for arg in "$@"; do
    case "$arg" in
        --release)   BUILD_MODE="release" ;;
        --build-only) BUILD_ONLY=true ;;
    esac
done

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║         Memos RTC CLI — Otonom Trading Konsolu               ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# Ortam değişkenleri bilgisi
if [ -z "$BINANCE_API_KEY" ] || [ -z "$BINANCE_API_SECRET" ]; then
    echo "⚠️  BINANCE_API_KEY / BINANCE_API_SECRET bulunamadı → Kağıt mod aktif"
else
    echo "✅  Binance API key yüklendi → Gerçek mod"
fi
echo ""

# Derleme
if [ "$BUILD_MODE" = "release" ]; then
    echo "🔨 Release binary derleniyor..."
    cargo build --release --bin rtc_cli --manifest-path "$CORE_DIR/Cargo.toml"
    BINARY="$BINARY_RELEASE"
else
    echo "🔨 Debug binary derleniyor..."
    cargo build --bin rtc_cli --manifest-path "$CORE_DIR/Cargo.toml"
    BINARY="$BINARY_DEBUG"
fi

echo "✅ Derleme tamamlandı: $BINARY"
echo ""

if [ "$BUILD_ONLY" = true ]; then
    echo "ℹ️  --build-only modu: çalıştırılmıyor."
    exit 0
fi

echo "🚀 RTC CLI başlatılıyor..."
echo "──────────────────────────────────────────────────────────────"
exec "$BINARY"
