#!/bin/bash
# memos_trading yedekleme scripti
# Kullanim:
#   ./scripts/backup.sh                    -> varsayilan hedef: ~/PyCharmMiscProject/
#   ./scripts/backup.sh /media/flashdisk   -> flash disk veya baska dizin
#   ./scripts/backup.sh --no-db            -> DB dahil etme (hizli, sadece kaynak)

set -e

# ---- Ayarlar ----
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
DEST_DIR="${1:-$HOME/PyCharmMiscProject}"
INCLUDE_DB=true

# --no-db bayragi
for arg in "$@"; do
  if [ "$arg" = "--no-db" ]; then
    INCLUDE_DB=false
    DEST_DIR="${2:-$HOME/PyCharmMiscProject}"
  fi
done

ARCHIVE_NAME="memos_trading_backup_${TIMESTAMP}.tar.gz"
ARCHIVE_PATH="${DEST_DIR}/${ARCHIVE_NAME}"

# ---- Kontroller ----
if [ ! -d "$DEST_DIR" ]; then
  echo "Hata: Hedef dizin bulunamadi: $DEST_DIR"
  exit 1
fi

# ---- Kaynak listesi ----
cd "$PROJECT_ROOT"

SOURCES=(
  "memos_trading_core/src"
  "memos_trading_core/Cargo.toml"
  "config"
  "scripts"
  "Cargo.toml"
  "Cargo.lock"
  "data/symbols_futures.json"
  "data/symbols_spot.json"
)

if [ "$INCLUDE_DB" = true ]; then
  [ -f "data/trader.db" ]     && SOURCES+=("data/trader.db")
  [ -f "data/trader_old.db" ] && SOURCES+=("data/trader_old.db")
fi

# ---- Bilgi ----
echo "Kaynak : $PROJECT_ROOT"
echo "Hedef  : $ARCHIVE_PATH"
echo "DB     : $INCLUDE_DB"
echo "Basliyor..."

# ---- Paketleme ----
tar -czf "$ARCHIVE_PATH" "${SOURCES[@]}"

# ---- Sonuc ----
SIZE="$(du -sh "$ARCHIVE_PATH" | cut -f1)"
echo ""
echo "Tamamlandi: $ARCHIVE_PATH"
echo "Boyut     : $SIZE"
