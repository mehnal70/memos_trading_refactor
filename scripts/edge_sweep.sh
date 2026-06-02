#!/bin/bash
# scripts/edge_sweep.sh — DB-geneli GROSS-EDGE toplu ön-tespit taraması (tek komut).
#
# examples/edge_scan'ı release modda, TÜM marketler × geniş interval kümesinde, yüksek
# seri tavanıyla koşar → "hangi seri+strateji NET KÂRLI (PF≥1.0) edge taşıyor" survey'i.
# Uzun sürer (yüzlerce seri × strateji havuzu × TP/SL/PS ızgarası); ilerleme stderr'e akar.
# Rapor reports/edge_sweep_<ts>.json'a mühürlenir (market×interval özet + PF-sıralı satırlar).
#
# Kullanım:
#   ./scripts/edge_sweep.sh                          # tüm DB (tüm market/interval/sembol)
#   ./scripts/edge_sweep.sh futures                  # yalnız futures
#   ./scripts/edge_sweep.sh all 1h,4h,1d             # tüm market, yalnız bu interval'ler
#   ./scripts/edge_sweep.sh futures 1h BTCUSDT,ETHUSDT
#
# Argümanlar: market(all|futures|spot|…) · intervals(csv|all) · symbols(csv|all) · limit
# Env: DB_PATH (default data/trader.db), EDGE_SCAN_MAX_SERIES (default 5000 = pratikte hepsi).

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.." || exit 1

MARKET="${1:-all}"
INTERVALS="${2:-all}"
SYMBOLS="${3:-all}"
LIMIT="${4:-5000}"

export DB_PATH="${DB_PATH:-data/trader.db}"
export EDGE_SCAN_MAX_SERIES="${EDGE_SCAN_MAX_SERIES:-5000}"  # toplu sweep → tavanı kaldır
TS="$(date +%Y%m%d_%H%M%S)"
export EDGE_SCAN_OUT="reports/edge_sweep_${TS}.json"

echo "🔬 edge_sweep · market=${MARKET} · interval=${INTERVALS} · symbol=${SYMBOLS} · limit=${LIMIT}"
echo "   db=${DB_PATH} · max_series=${EDGE_SCAN_MAX_SERIES} · rapor=${EDGE_SCAN_OUT}"
echo "   (release derleniyor; ardından tarama — uzun sürebilir, ilerleme akacak)"
echo

cargo run --release --quiet --example edge_scan -- "${MARKET}" "${INTERVALS}" "${SYMBOLS}" "${LIMIT}"
RC=$?

echo
if [ "${RC}" -eq 0 ] && [ -f "${EDGE_SCAN_OUT}" ]; then
    echo "✅ Tarama bitti → ${EDGE_SCAN_OUT}"
    echo "   (tekrar koşularda karşılaştırmak için raporu sakla; reports/ gitignore'lu)"
else
    echo "⚠️ Tarama hata/eksik (RC=${RC})."
fi
exit "${RC}"
