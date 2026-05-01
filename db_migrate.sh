#!/usr/bin/env bash
# db_migrate.sh — Trader DB veri bütünlüğü düzeltmesi
# Veri kaybı olmaksızın aşağıdaki sorunları giderir:
#
#  1. candles_binance_spot  : Saniye cinsinden timestamp → ms'e çevir
#                             Çakışan (ms karşılığı zaten var) saniye kayıtları sil
#  2. candles_binance_futures: Aynı işlem
#  3. candles (eski tablo)  : Gerçek duplicate'leri temizle (MIN(id) tut)
#  4. candles UNIQUE index  : Duplicate tekrarını önlemek için UNIQUE index ekle
#  5. VACUUM                : Silinen sayfalardaki boş alanı geri al
#
# Çalıştırmadan önce:
#   bash db_migrate.sh --dry-run   → yalnızca sayıları gösterir, değişiklik yapmaz
#   bash db_migrate.sh             → gerçek migration
#
# Her adım kendi transaction'ında; hata oluşursa o adım otomatik geri alınır.

set -euo pipefail

DEFAULT_DB="/home/ulas/PyCharmMiscProject/memos_trading/data/trader.db"
DRY_RUN=0
DB=""

for arg in "$@"; do
    if [[ "$arg" == "--dry-run" ]]; then
        DRY_RUN=1
    elif [[ -z "$DB" ]]; then
        DB="$arg"
    fi
done
DB="${DB:-$DEFAULT_DB}"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Trader DB Migration"
echo "  DB   : $DB"
echo "  Mode : $([ $DRY_RUN -eq 1 ] && echo 'DRY-RUN (değişiklik yok)' || echo 'LIVE')"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ ! -f "$DB" ]; then
    echo "HATA: DB dosyası bulunamadı: $DB"
    exit 1
fi

# ── Ön kontrol ────────────────────────────────────────────────────────────────
echo ""
echo "▶ Ön analiz..."
sqlite3 "$DB" << 'ANALYSIS'
.mode column
.headers on
SELECT '=== Tablo Boyutları ===' AS info;
SELECT 'candles_binance_spot'    AS tbl, COUNT(*) AS total,
       SUM(CASE WHEN timestamp < 9999999999 THEN 1 ELSE 0 END) AS sec_rows,
       SUM(CASE WHEN timestamp >= 9999999999 THEN 1 ELSE 0 END) AS ms_rows
FROM candles_binance_spot
UNION ALL
SELECT 'candles_binance_futures', COUNT(*),
       SUM(CASE WHEN timestamp < 9999999999 THEN 1 ELSE 0 END),
       SUM(CASE WHEN timestamp >= 9999999999 THEN 1 ELSE 0 END)
FROM candles_binance_futures;

SELECT '=== candles Tablo Duplicate Sayısı ===' AS info;
SELECT COUNT(*) AS toplam,
       (SELECT COUNT(*) FROM (SELECT MIN(id) FROM candles GROUP BY symbol,interval,timestamp)) AS unique_count,
       COUNT(*) - (SELECT COUNT(*) FROM (SELECT MIN(id) FROM candles GROUP BY symbol,interval,timestamp)) AS duplicate_count
FROM candles;

SELECT '=== Spot Çakışma (saniye kaydın ms karşılığı mevcut) ===' AS info;
SELECT COUNT(*) AS spot_collision FROM candles_binance_spot s
WHERE timestamp < 9999999999
  AND EXISTS (SELECT 1 FROM candles_binance_spot m WHERE m.symbol=s.symbol AND m.interval=s.interval AND m.timestamp=s.timestamp*1000);

SELECT '=== Futures Çakışma ===' AS info;
SELECT COUNT(*) AS futures_collision FROM candles_binance_futures s
WHERE timestamp < 9999999999
  AND EXISTS (SELECT 1 FROM candles_binance_futures m WHERE m.symbol=s.symbol AND m.interval=s.interval AND m.timestamp=s.timestamp*1000);

SELECT '=== Fragmentasyon ===' AS info;
SELECT page_count, freelist_count, page_size,
       ROUND(freelist_count * 100.0 / page_count, 1) AS free_pct
FROM pragma_page_count(), pragma_freelist_count(), pragma_page_size();
ANALYSIS

if [ $DRY_RUN -eq 1 ]; then
    echo ""
    echo "DRY-RUN modu: Değişiklik yapılmadı. Çalıştırmak için --dry-run olmadan tekrar çalıştırın."
    exit 0
fi

# ── Backup ────────────────────────────────────────────────────────────────────
BACKUP="${DB%.db}_pre_migration_$(date +%Y%m%d_%H%M%S).db"
echo ""
echo "▶ Yedek oluşturuluyor: $BACKUP"
cp "$DB" "$BACKUP"
echo "  ✓ Yedek hazır ($(du -sh "$BACKUP" | cut -f1))"

# ── ADIM 1: candles_binance_spot — timestamp düzeltme ──────────────────────
echo ""
echo "▶ ADIM 1: candles_binance_spot — karışık timestamp düzeltme"
sqlite3 "$DB" << 'STEP1'
BEGIN;

-- 1a. Saniye kaydı var, ms karşılığı da var → saniye kaydı gereksiz, sil
DELETE FROM candles_binance_spot
WHERE timestamp < 9999999999
  AND EXISTS (
    SELECT 1 FROM candles_binance_spot m
    WHERE m.symbol   = candles_binance_spot.symbol
      AND m.interval = candles_binance_spot.interval
      AND m.timestamp = candles_binance_spot.timestamp * 1000
  );

SELECT 'spot: çakışan saniye kayıt silindi →' || changes() AS step1a;

-- 1b. Kalan saniye kayıtlarını ms'e çevir (UPDATE OR IGNORE: çakışma olursa atla)
UPDATE OR IGNORE candles_binance_spot
SET timestamp = timestamp * 1000
WHERE timestamp < 9999999999;

SELECT 'spot: saniye→ms güncellendi →' || changes() AS step1b;

-- 1c. Güncellenemeyen (çok nadir çakışma) saniye kayıtları kaldır
DELETE FROM candles_binance_spot WHERE timestamp < 9999999999;
SELECT 'spot: kalan saniye kayıt silindi →' || changes() AS step1c;

COMMIT;

-- Doğrulama
SELECT 'spot timestamp doğrulama:',
       SUM(CASE WHEN timestamp < 9999999999 THEN 1 ELSE 0 END) AS sec_remaining,
       SUM(CASE WHEN timestamp >= 9999999999 THEN 1 ELSE 0 END) AS ms_count
FROM candles_binance_spot;
STEP1
echo "  ✓ Adım 1 tamamlandı"

# ── ADIM 2: candles_binance_futures — timestamp düzeltme ───────────────────
echo ""
echo "▶ ADIM 2: candles_binance_futures — karışık timestamp düzeltme"
sqlite3 "$DB" << 'STEP2'
BEGIN;

DELETE FROM candles_binance_futures
WHERE timestamp < 9999999999
  AND EXISTS (
    SELECT 1 FROM candles_binance_futures m
    WHERE m.symbol   = candles_binance_futures.symbol
      AND m.interval = candles_binance_futures.interval
      AND m.timestamp = candles_binance_futures.timestamp * 1000
  );

SELECT 'futures: çakışan saniye kayıt silindi →' || changes() AS step2a;

UPDATE OR IGNORE candles_binance_futures
SET timestamp = timestamp * 1000
WHERE timestamp < 9999999999;

SELECT 'futures: saniye→ms güncellendi →' || changes() AS step2b;

DELETE FROM candles_binance_futures WHERE timestamp < 9999999999;
SELECT 'futures: kalan saniye kayıt silindi →' || changes() AS step2c;

COMMIT;

SELECT 'futures timestamp doğrulama:',
       SUM(CASE WHEN timestamp < 9999999999 THEN 1 ELSE 0 END) AS sec_remaining,
       SUM(CASE WHEN timestamp >= 9999999999 THEN 1 ELSE 0 END) AS ms_count
FROM candles_binance_futures;
STEP2
echo "  ✓ Adım 2 tamamlandı"

# ── ADIM 3: candles tablosu — gerçek duplicate'leri temizle ────────────────
echo ""
echo "▶ ADIM 3: candles — gerçek duplicate temizleme (MIN(id) korunur)"
sqlite3 "$DB" << 'STEP3'
BEGIN;

-- Her (symbol, interval, timestamp) grubu için sadece MIN(id)'yi tut
DELETE FROM candles
WHERE id NOT IN (
  SELECT MIN(id)
  FROM candles
  GROUP BY symbol, interval, timestamp
);

SELECT 'candles: duplicate silindi →' || changes() AS step3;

COMMIT;

SELECT 'candles doğrulama: kalan satır →' || COUNT(*) FROM candles;
STEP3
echo "  ✓ Adım 3 tamamlandı"

# ── ADIM 4: candles UNIQUE index ekle ──────────────────────────────────────
echo ""
echo "▶ ADIM 4: candles — UNIQUE index oluştur (gelecekteki duplicate'leri önler)"
sqlite3 "$DB" << 'STEP4'
-- Mevcut non-unique index'i kaldır, UNIQUE olanı ekle
DROP INDEX IF EXISTS idx_candles_unique;
CREATE UNIQUE INDEX IF NOT EXISTS idx_candles_dedup
  ON candles (symbol, interval, timestamp);

SELECT 'idx_candles_dedup oluşturuldu' AS step4;
STEP4
echo "  ✓ Adım 4 tamamlandı"

# ── ADIM 5: VACUUM ──────────────────────────────────────────────────────────
echo ""
echo "▶ ADIM 5: VACUUM — silinen sayfaları geri al (bu işlem birkaç dakika sürebilir...)"
sqlite3 "$DB" "VACUUM;"
echo "  ✓ VACUUM tamamlandı"

# ── Son doğrulama ─────────────────────────────────────────────────────────
echo ""
echo "▶ Son doğrulama..."
sqlite3 "$DB" << 'FINAL'
.mode column
.headers on

SELECT 'candles_binance_spot' AS tbl, COUNT(*) AS rows,
       SUM(CASE WHEN timestamp < 9999999999 THEN 1 ELSE 0 END) AS sec_rows_remaining
FROM candles_binance_spot
UNION ALL
SELECT 'candles_binance_futures', COUNT(*),
       SUM(CASE WHEN timestamp < 9999999999 THEN 1 ELSE 0 END)
FROM candles_binance_futures
UNION ALL
SELECT 'candles', COUNT(*), 0 FROM candles;

SELECT '=== integrity_check ===' AS check;
PRAGMA integrity_check(50);

SELECT '=== Boyut (VACUUM sonrası) ===' AS check;
SELECT page_count * page_size / 1024 / 1024 AS size_mb,
       freelist_count AS free_pages
FROM pragma_page_count(), pragma_freelist_count(), pragma_page_size();
FINAL

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Migration tamamlandı."
echo "  Yedek : $BACKUP"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
