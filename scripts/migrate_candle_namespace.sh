#!/usr/bin/env bash
# candles tablosu namespace konsolidasyonu.
#
# SORUN: yazma ve okuma yolları (exchange, market) çifti üzerinde anlaşmıyordu; ayrıca
# semboller hem çıplak (AUDUSD) hem etiketli (AUDUSD@mt5) yazılmıştı. Sonuç: aynı
# enstrümanın geçmişi 5 kovaya parçalandı ve canlı döngü en ince kovayı okuyordu.
#
# KANONİK KURAL (VenueAdapter::storage_namespace ile aynı):
#   exchange = venue.exchange().as_str()      (binance | mt5 | ibkr)
#   market   = venue.market().as_str()        (futures | cfd | spot | coinm)
#   symbol   = ÇIPLAK (venue etiketi yok)
# `exchange` mt5 ile ibkr'ı ayırdığı için ikisi de "cfd" olabilir.
#
# Kullanım:
#   ./scripts/migrate_candle_namespace.sh            → DRY-RUN (hiçbir şey yazmaz)
#   ./scripts/migrate_candle_namespace.sh --apply    → uygular (önce yedek alır)
set -uo pipefail
cd "$(dirname "$0")/.."
DB="${DB_PATH:-data/trader.db}"
APPLY=0; [ "${1:-}" = "--apply" ] && APPLY=1

[ -f "$DB" ] || { echo "❌ DB yok: $DB"; exit 2; }
command -v sqlite3 >/dev/null || { echo "❌ sqlite3 gerekli"; exit 2; }

echo "=== ÖNCE: kova dağılımı ==="
sqlite3 -column -header "$DB" "
SELECT exchange, market,
       SUM(CASE WHEN symbol LIKE '%@%' THEN 1 ELSE 0 END) AS tagli,
       SUM(CASE WHEN symbol LIKE '%@%' THEN 0 ELSE 1 END) AS ciplak,
       COUNT(*) AS toplam
FROM candles GROUP BY exchange, market ORDER BY toplam DESC;"

echo
echo "=== PLANLANAN 3 DÖNÜŞÜM ==="
n1=$(sqlite3 "$DB" "SELECT COUNT(*) FROM candles WHERE exchange='mt5' AND market='mt5';")
n2=$(sqlite3 "$DB" "SELECT COUNT(*) FROM candles WHERE exchange='ibkr' AND market='futures';")
n3=$(sqlite3 "$DB" "SELECT COUNT(*) FROM candles WHERE symbol LIKE '%@%';")
echo "  1) mt5|mt5      → mt5|cfd        : $n1 satır"
echo "  2) ibkr|futures → ibkr|cfd       : $n2 satır"
echo "  3) sembol etiketi soyma (@ sil)  : $n3 satır"
n4=$(sqlite3 "$DB" "SELECT COUNT(*) FROM candles WHERE exchange='binance' AND market='futures' AND symbol IN ('EURUSD','GBPUSD','USDJPY','XAUUSD');")
echo "  4) binance|futures forex/metal → mt5|cfd (BOŞLUK-DOLDURMA, ezme YOK): $n4 satır"

echo
echo "=== ÇAKIŞMA ANALİZİ (kanonik satır korunur, eski yalnız boşluk doldurur) ==="
sqlite3 "$DB" "
WITH kanonik AS (
  SELECT exchange,
         CASE WHEN exchange='mt5' AND market='mt5' THEN 'cfd'
              WHEN exchange='ibkr' AND market='futures' THEN 'cfd'
              ELSE market END AS market,
         CASE WHEN instr(symbol,'@')>0 THEN substr(symbol,1,instr(symbol,'@')-1)
              ELSE symbol END AS symbol,
         interval, timestamp
  FROM candles
)
SELECT 'toplam satır: ' || (SELECT COUNT(*) FROM candles)
     || ' → kanonik benzersiz: ' || (SELECT COUNT(*) FROM (SELECT DISTINCT exchange,market,symbol,interval,timestamp FROM kanonik))
     || '  (fark = birleşen kopya)';"

if [ "$APPLY" -eq 0 ]; then
  echo
  echo "ℹ️  DRY-RUN — hiçbir şey yazılmadı. Uygulamak için: $0 --apply"
  exit 0
fi

BK="$HOME/memos_backups/trader_db_$(date +%Y%m%d_%H%M%S).db"
mkdir -p "$(dirname "$BK")"
sqlite3 "$DB" ".backup '$BK'"
echo "✅ yedek: $BK ($(du -h "$BK" | cut -f1))"

sqlite3 "$DB" <<'SQL'
PRAGMA foreign_keys=OFF;
BEGIN IMMEDIATE;
-- Kanonik hedef satırı varsa KORU; eski namespace yalnız eksik anahtarları doldursun.
INSERT OR IGNORE INTO candles (exchange, market, symbol, interval, timestamp, open, high, low, close, volume)
SELECT exchange, 'cfd', symbol, interval, timestamp, open, high, low, close, volume
  FROM candles WHERE exchange='mt5' AND market='mt5';
DELETE FROM candles WHERE exchange='mt5' AND market='mt5';

INSERT OR IGNORE INTO candles (exchange, market, symbol, interval, timestamp, open, high, low, close, volume)
SELECT exchange, 'cfd', symbol, interval, timestamp, open, high, low, close, volume
  FROM candles WHERE exchange='ibkr' AND market='futures';
DELETE FROM candles WHERE exchange='ibkr' AND market='futures';

INSERT OR IGNORE INTO candles (exchange, market, symbol, interval, timestamp, open, high, low, close, volume)
SELECT exchange, market, substr(symbol, 1, instr(symbol,'@')-1), interval, timestamp, open, high, low, close, volume
  FROM candles WHERE instr(symbol,'@') > 0;
DELETE FROM candles WHERE instr(symbol,'@') > 0;

-- 4) Yanlış etiketli forex/metal serilerini mt5|cfd'ye BOŞLUK-DOLDURARAK taşı.
--    `INSERT OR IGNORE` KASITLI: mt5|cfd'de o timestamp zaten VARSA dokunulmaz →
--    gerçek MT5 broker feed'i ASLA ezilmez, yalnız MT5'te olmayan tarihler eklenir.
--    (Bu satırlar Yahoo/TwelveData kaynaklı; aynı enstrüman için iki farklı feed'i
--     üst üste yazmak backtest'i bozardı. Bu yüzden REPLACE değil IGNORE.)
INSERT OR IGNORE INTO candles (exchange, market, symbol, interval, timestamp, open, high, low, close, volume)
SELECT 'mt5', 'cfd', symbol, interval, timestamp, open, high, low, close, volume
  FROM candles
 WHERE exchange='binance' AND market='futures'
   AND symbol IN ('EURUSD','GBPUSD','USDJPY','XAUUSD');

DELETE FROM candles
 WHERE exchange='binance' AND market='futures'
   AND symbol IN ('EURUSD','GBPUSD','USDJPY','XAUUSD');
COMMIT;
SQL
echo "✅ migration uygulandı"

echo
echo "=== SONRA: kova dağılımı ==="
sqlite3 -column -header "$DB" "
SELECT exchange, market,
       SUM(CASE WHEN symbol LIKE '%@%' THEN 1 ELSE 0 END) AS tagli,
       COUNT(DISTINCT symbol) AS sembol, COUNT(*) AS toplam
FROM candles GROUP BY exchange, market ORDER BY toplam DESC;"
