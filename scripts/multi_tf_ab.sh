#!/bin/bash
# scripts/multi_tf_ab.sh — Çoklu-TF seed düzeneği A/B doğrulaması (Single vs Multi).
#
# examples/multi_tf_ab'yi release modda koşar: bir edge_sweep raporundaki >1 WF-onaylı edge'li
# sembolleri her izin KENDİ TF mumunda backtest eder, Multi kolunu çakışmasız tek-pozisyon
# arbitrasyonuyla birleştirir, Single (yalnız top-PF iz) ile kıyaslar → EDGE_SEED_MULTI_TF'i
# canlıya açmadan önce NET kazanç var mı ölçer.
#
# Kullanım:
#   ./scripts/multi_tf_ab.sh reports/edge_sweep_<ts>.json [market]
# Env: DB_PATH, EDGE_SEED_MIN_TRADES/MIN_PF/MAX_PF/REQUIRE_WF/MIN_QVOL, EDGE_SEED_MAX_TRACKS,
#      AB_DIRECTION(long|both|regime), AB_EDGE_MIN, AB_CANDLE_LIMIT — seed barı edge_scan/store ile aynı.

set -u
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.." || exit 1

REPORT="${1:-}"
MARKET="${2:-futures}"
if [ -z "${REPORT}" ] || [ ! -f "${REPORT}" ]; then
    echo "❌ Kullanım: ./scripts/multi_tf_ab.sh reports/edge_sweep_<ts>.json [market]"
    echo "   (önce ./scripts/edge_sweep.sh ile taze rapor üret)"
    exit 2
fi

export DB_PATH="${DB_PATH:-data/trader.db}"
export EDGE_SEED_REPORT="${REPORT}"
export TRADE_MARKET="${MARKET}"
# Seed barı default'ları: 1d edge'leri az-işlemli → min_trades=15 mantıklı (operatör override).
export EDGE_SEED_MIN_TRADES="${EDGE_SEED_MIN_TRADES:-15}"

echo "🪢 multi_tf_ab · rapor=${REPORT} · market=${MARKET} · db=${DB_PATH}"
echo "   (release derleniyor; ardından >1 izli sembollerde A/B backtest)"
echo

cargo run --release --quiet --example multi_tf_ab
RC=$?
echo
[ "${RC}" -eq 0 ] || echo "⚠️ A/B hata (RC=${RC})."
exit "${RC}"
