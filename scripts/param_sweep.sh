#!/bin/bash
# scripts/param_sweep.sh — strateji/indikatör/osilatör PARAMETRE optimizasyon taraması (tek komut).
#
# examples/param_optimize'ı release modda koşar → seçili sembol sepetinde her TF için her
# stratejinin KENDİ param_spec uzayını (RSI period/overbought, MACD fast/slow, BB period/std_dev,
# CCI eşikleri…) holdout IS/OOS ile optimize eder. edge_sweep.sh'in TAMAMLAYICISI: o çıkış
# ekseni (TP/SL/PS) + havuz seçimi tarar, bu indikatör paramlarını tarar.
#
# Çıktı (uygulamaya entegre için):
#   reports/param_sweep_<ts>.json          → tam rapor (champions + tüm satırlar, yapısal params)
#   reports/param_sweep_champions_<ts>.csv → sembol başına ŞAMPİYON (kötünün iyisi), düz tablo
# Stdout'a sıralı tablo + şampiyon tablosu basılır.
#
# Kullanım:
#   ./scripts/param_sweep.sh                                  # 12-sembol sepeti · 4h (default)
#   ./scripts/param_sweep.sh futures 15m,1h,4h,1d            # aynı sepet, çok-TF
#   ./scripts/param_sweep.sh futures 4h BTCUSDT,ETHUSDT 300  # özel sepet · n=300
#   DB_PATH=data/trader_4h_test.db ./scripts/param_sweep.sh  # 4h test DB'si (12×4h×10950 bar)
#
# Argümanlar: market(futures|spot|…) · intervals(csv|all→15m,1h,4h,1d) · symbols(csv) · n_örnek · limit
# Env: DB_PATH (default data/trader.db), IS_PCT (holdout, default 70), MIN_OOS_TRADES (robust, default 5),
#      SEED (determinizm, default 12345), PARAM_OPT_OUT (rapor yolu — boşsa otomatik ts'li).

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.." || exit 1

# Seçili 12-sembol sepeti (XS momentum + edge majörleri; data/trader_4h_test.db ile aynı).
DEFAULT_SYMBOLS="ADAUSDT,AVAXUSDT,BCHUSDT,BNBUSDT,BTCUSDT,DOGEUSDT,ETHUSDT,SOLUSDT,TRXUSDT,UNIUSDT,XRPUSDT,ZECUSDT"

MARKET="${1:-futures}"
INTERVALS="${2:-4h}"
SYMBOLS="${3:-$DEFAULT_SYMBOLS}"
N="${4:-200}"
LIMIT="${5:-11000}"

export DB_PATH="${DB_PATH:-data/trader.db}"
TS="$(date +%Y%m%d_%H%M%S)"
export PARAM_OPT_OUT="${PARAM_OPT_OUT:-reports/param_sweep_${TS}.json}"
# CSV'yi JSON yolundan TÜRET → çağıran (sweep_pipeline) PARAM_OPT_OUT'u önceden set etse de tutarlı.
CSV_OUT="${PARAM_OPT_OUT%.json}_champions.csv"

echo "🎯 param_sweep · market=${MARKET} · interval=${INTERVALS} · n=${N} · limit=${LIMIT}"
echo "   db=${DB_PATH} · holdout %${IS_PCT:-70} IS/OOS · rapor=${PARAM_OPT_OUT}"
echo "   sembol=${SYMBOLS}"
echo "   (release derleniyor; ardından optimizasyon — ilerleme stderr'e akacak)"
echo

cargo run --release --quiet --example param_optimize -- "${MARKET}" "${INTERVALS}" "${SYMBOLS}" "${N}" "${LIMIT}"
RC=$?

echo
if [ "${RC}" -ne 0 ] || [ ! -f "${PARAM_OPT_OUT}" ]; then
    echo "⚠️ Tarama hata/eksik (RC=${RC})."
    exit "${RC}"
fi

# ─── Şampiyonları düz CSV'ye çıkar (uygulamaya entegre / spreadsheet için) ──────────
if command -v python3 >/dev/null 2>&1; then
    python3 - "${PARAM_OPT_OUT}" "${CSV_OUT}" <<'PY'
import json, sys, csv
rep = json.load(open(sys.argv[1]))
champs = rep.get("champions", [])
with open(sys.argv[2], "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["symbol","interval","strategy","wf_robust","robust",
                "oos_score","wf_pooled_pf","wf_consistency","wf_pvalue","wf_windows",
                "oos_pnl_pct","oos_win_rate","oos_trades","params"])
    for c in champs:
        params = ";".join(f"{p['name']}={p['value']:g}" for p in c.get("params", []))
        w.writerow([c["symbol"], c["interval"], c["strategy"],
                    int(c.get("wf_robust", False)), int(c["robust"]),
                    f"{c['oos_score']:.4f}", f"{c.get('wf_pooled_pf',0):.4f}",
                    f"{c.get('wf_consistency',0):.4f}", f"{c.get('wf_pvalue',1):.4f}",
                    c.get("wf_windows",0),
                    f"{c['oos_pnl_pct']:.4f}", f"{c['oos_win_rate']:.2f}", c["oos_trades"], params])
print(f"📋 Şampiyon CSV: {sys.argv[2]} ({len(champs)} sembol)")
PY
else
    echo "ℹ️ python3 yok → CSV atlandı (JSON champions alanı yine de hazır)."
fi

echo "✅ Tarama bitti → ${PARAM_OPT_OUT}"
echo "   (reports/ gitignore'lu; tekrar koşularda karşılaştırmak için sakla. JSON champions[].params = entegrasyon için yapısal param)"
exit "${RC}"
