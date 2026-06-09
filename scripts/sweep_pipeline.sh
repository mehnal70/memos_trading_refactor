#!/bin/bash
# scripts/sweep_pipeline.sh — uçtan uca EDGE→(onay)→İNDİR→PARAM tarama orkestratörü.
#
# Akış:
#   1) edge_sweep.sh koşar → gross-edge survey'i reports/edge_sweep_<ts>.json'a yazar.
#   2) Rapordan edge'in BELİRLEDİĞİ semboller (WF-robust → yoksa kârlı → yoksa girdi sepeti) ekrana gelir.
#   3) Onay verirsen (param_sweep çalıştır?) devam; vermezsen edge JSON'u zaten EDGE_SEED_REPORT'a hazır.
#   4) O semboller için TF'leri (küçükten büyüğe) sen belirlersin.
#   5) Önce o sembol×TF mumları indirilir (download_candles), sonra param_sweep taraması koşar.
#   6) Çıktı HEM XS HEM parametre entegrasyonuna hazır:
#        - reports/edge_sweep_<ts>.json            → EDGE_SEED_REPORT (per-sembol strateji+TF+WF seed)
#        - reports/param_sweep_<ts>.json + champions CSV → indikatör/osilatör paramları (kötünün iyisi)
#        - reports/integration_<ts>.env            → kaynak-alınabilir env (EDGE_SEED_REPORT + XS_LIVE_*)
#
# Kullanım (interaktif):
#   ./scripts/sweep_pipeline.sh                      # tüm DB edge taraması → onay → TF sor → indir → param
#   ./scripts/sweep_pipeline.sh futures 4h           # edge'i futures/4h ile daralt
#   ./scripts/sweep_pipeline.sh futures 1h,4h SYM1,SYM2
#
# edge_sweep argümanları: market(all|futures|…) · intervals(csv|all) · symbols(csv|all) · limit
#
# Scriptlenebilir (promptları atla — CI/test):
#   RUN_PARAM_SWEEP=e PIPELINE_TFS=4h PIPELINE_DOWNLOAD=0 PIPELINE_MARKET=futures \
#     DB_PATH=data/trader_4h_test.db ./scripts/sweep_pipeline.sh futures 4h SYM1,SYM2
# Env: DB_PATH, EDGE_REPORT (mevcut edge JSON'u yeniden kullan, edge_sweep'i atla),
#      RUN_PARAM_SWEEP(e/h), PIPELINE_TFS(csv), PIPELINE_MARKET, PIPELINE_SYMBOLS(aday override),
#      PIPELINE_DOWNLOAD(1/0), PIPELINE_YEARS, PARAM_N, PARAM_LIMIT.

set -u
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.." || exit 1

export DB_PATH="${DB_PATH:-data/trader.db}"
TS="$(date +%Y%m%d_%H%M%S)"

# Promptu env-override ile atlayabilen yardımcı (read /dev/tty → stdout redirect'te de çalışır).
ask() {  # ask VARNAME "soru" "default" "envoverride"
    local __var="$1" __prompt="$2" __def="$3" __ovr="${4:-}" __ans=""
    if [ -n "${__ovr}" ]; then printf -v "${__var}" '%s' "${__ovr}"; return; fi
    if [ -r /dev/tty ]; then read -r -p "${__prompt} [${__def}]: " __ans </dev/tty || __ans=""; fi
    printf -v "${__var}" '%s' "${__ans:-${__def}}"
}
is_yes() { case "${1,,}" in e|evet|y|yes|1) return 0;; *) return 1;; esac; }

command -v python3 >/dev/null 2>&1 || { echo "✗ python3 gerekli (rapor ayrıştırma)."; exit 3; }

# ─── 1) EDGE TARAMASI (veya mevcut raporu yeniden kullan) ────────────────────────
EDGE_JSON="${EDGE_REPORT:-}"
if [ -n "${EDGE_JSON}" ]; then
    [ -f "${EDGE_JSON}" ] || { echo "✗ EDGE_REPORT bulunamadı: ${EDGE_JSON}"; exit 1; }
    echo "♻️  Mevcut edge raporu kullanılıyor: ${EDGE_JSON} (edge_sweep atlandı)"
else
    echo "═══ 1/5 · EDGE TARAMASI (edge_sweep.sh) ═══"
    "${SCRIPT_DIR}/edge_sweep.sh" "${1:-all}" "${2:-all}" "${3:-all}" "${4:-5000}"
    ERC=$?
    [ "${ERC}" -eq 0 ] || { echo "✗ edge_sweep başarısız (RC=${ERC})."; exit "${ERC}"; }
    EDGE_JSON="$(ls -t reports/edge_sweep_*.json 2>/dev/null | head -1)"
    [ -n "${EDGE_JSON}" ] && [ -f "${EDGE_JSON}" ] || { echo "✗ edge raporu bulunamadı."; exit 1; }
fi

# ─── 2) EDGE'İN BELİRLEDİĞİ SEMBOLLER ────────────────────────────────────────────
# Öncelik: WF-robust → yoksa kârlı → yoksa raporda geçen tüm semboller. PIPELINE_SYMBOLS override eder.
CANDIDATES="${PIPELINE_SYMBOLS:-}"
EDGE_MARKET="$(python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print((d.get('market_filter') or 'futures'))" "${EDGE_JSON}" 2>/dev/null)"
if [ -z "${CANDIDATES}" ]; then
    CANDIDATES="$(python3 - "${EDGE_JSON}" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
rows = d.get("rows", [])
def syms(pred):
    seen=[]
    for r in rows:
        if pred(r) and r["symbol"] not in seen: seen.append(r["symbol"])
    return seen
out = syms(lambda r: r.get("wf_robust")) or syms(lambda r: r.get("profitable")) or syms(lambda r: True)
print(",".join(out))
PY
)"
fi
echo
echo "═══ 2/5 · EDGE'İN BELİRLEDİĞİ SEMBOLLER ═══"
if [ -z "${CANDIDATES}" ]; then
    echo "  ⚠️ edge raporunda sembol yok (DB boş/filtre dar). Çıkılıyor."
    exit 0
fi
# Sembolleri stratejisi+TF+PF ile listele (WF-robust işaretli).
python3 - "${EDGE_JSON}" "${CANDIDATES}" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); want=set(sys.argv[2].split(","))
best={}
for r in d.get("rows", []):
    if r["symbol"] in want:
        k=r["symbol"]
        if k not in best or r.get("wf",{}).get("pooled_pf",0) > best[k].get("wf",{}).get("pooled_pf",0):
            best[k]=r
print(f"  {'symbol':<10}{'iv':<5}{'strateji':<14}{'PF':>6}{'wfPF':>7}  WF")
for s in sys.argv[2].split(","):
    r=best.get(s)
    if not r: print(f"  {s:<10}{'?':<5}{'-':<14}{'-':>6}{'-':>7}"); continue
    wf="✅" if r.get("wf_robust") else "—"
    print(f"  {s:<10}{r['interval']:<5}{r['best_strategy']:<14}{r['profit_factor']:>6.2f}{r.get('wf',{}).get('pooled_pf',0):>7.2f}  {wf}")
PY
echo "  → aday sembol sayısı: $(echo "${CANDIDATES}" | tr ',' '\n' | grep -c .)"
echo "  → market: ${EDGE_MARKET}"

# ─── 3) ONAY: param_sweep çalıştırılsın mı? ──────────────────────────────────────
echo
ask APPROVE "═══ 3/5 · Bu sembollerde param_sweep (indikatör param optimizasyonu) çalıştırılsın mı? (e/h)" "h" "${RUN_PARAM_SWEEP:-}"
if ! is_yes "${APPROVE}"; then
    echo "  ⏹  param_sweep atlandı. edge raporu entegrasyona hazır:"
    echo "      EDGE_SEED_REPORT=${EDGE_JSON}"
    exit 0
fi

# ─── 4) TF SEÇİMİ + VERİ İNDİRME ─────────────────────────────────────────────────
echo
ask TFS "═══ 4/5 · Bu semboller için TF'leri gir (küçükten büyüğe, csv)" "15m,1h,4h,1d" "${PIPELINE_TFS:-}"
ask MARKET "    market" "${EDGE_MARKET:-futures}" "${PIPELINE_MARKET:-}"
ask DODL  "    Veriler indirilsin mi? (1=evet/0=hayır — veri zaten varsa 0)" "1" "${PIPELINE_DOWNLOAD:-}"
YEARS="${PIPELINE_YEARS:-8}"

if [ "${DODL}" = "1" ]; then
    echo
    echo "  ⬇️  İndiriliyor: ${MARKET} · TF=[${TFS}] · ${YEARS} yıl · semboller=${CANDIDATES}"
    IFS=',' read -ra _TFARR <<< "${TFS}"
    for tf in "${_TFARR[@]}"; do
        tf="$(echo "$tf" | tr -d ' ')"; [ -z "$tf" ] && continue
        echo "  --- ${tf} ---"
        cargo run --release --quiet --example download_candles -- "${MARKET}" "${tf}" "${CANDIDATES}" "${YEARS}" \
            || echo "  ⚠️ ${tf} indirmede uyarı (devam)."
    done
else
    echo "  ⏭  İndirme atlandı (mevcut DB kullanılacak)."
fi

# ─── 5) PARAM TARAMASI ───────────────────────────────────────────────────────────
echo
echo "═══ 5/5 · PARAM TARAMASI (param_sweep.sh) ═══"
export PARAM_OPT_OUT="reports/param_sweep_${TS}.json"
"${SCRIPT_DIR}/param_sweep.sh" "${MARKET}" "${TFS}" "${CANDIDATES}" "${PARAM_N:-200}" "${PARAM_LIMIT:-11000}"
PRC=$?
[ "${PRC}" -eq 0 ] || { echo "✗ param_sweep başarısız (RC=${PRC})."; exit "${PRC}"; }
PARAM_JSON="${PARAM_OPT_OUT}"
CHAMP_CSV="${PARAM_OPT_OUT%.json}_champions.csv"  # param_sweep ile AYNI türetme (TS uyumsuzluğu yok)

# ─── ENTEGRASYON SNIPPET'İ (XS + parametre) ──────────────────────────────────────
# XS interval = en küçük (ilk) TF. XS_LIVE_SYMBOLS = aday sepet. EDGE_SEED_REPORT = edge JSON.
XS_IV="$(echo "${TFS}" | cut -d',' -f1 | tr -d ' ')"
INTEG="reports/integration_${TS}.env"
{
    echo "# sweep_pipeline ${TS} — entegrasyona hazır env (kaynak-al: set -a; . ${INTEG}; set +a)"
    echo "# Parametre/seed entegrasyonu: engine bu raporu okuyup per-sembol strateji+TF+WF seed yükler."
    echo "EDGE_SEED_REPORT=${EDGE_JSON}"
    echo "# İndikatör/osilatör paramları (kötünün iyisi): champions[].params makine-okunur."
    echo "# PARAM_SWEEP_REPORT=${PARAM_JSON}   # (bilgi amaçlı; otonom besleme A/B sonrası opt-in)"
    echo "# XS entegrasyonu: kesitsel market-nötr kitap bu sepetle çalışır."
    echo "XS_LIVE_SYMBOLS=${CANDIDATES}"
    echo "XS_LIVE_INTERVAL=${XS_IV}"
    echo "# XS_LIVE_ENABLED=1   # açmak için (opt-in; default kapalı)"
} > "${INTEG}"

echo
echo "════════════════════ ✅ PIPELINE TAMAM ════════════════════"
echo "  edge raporu     : ${EDGE_JSON}        (EDGE_SEED_REPORT)"
echo "  param raporu    : ${PARAM_JSON}"
echo "  şampiyon CSV    : ${CHAMP_CSV}"
echo "  entegrasyon env : ${INTEG}"
echo
echo "  → Parametre+seed entegrasyonu:  export EDGE_SEED_REPORT=${EDGE_JSON}"
echo "  → XS entegrasyonu:              set -a; . ${INTEG}; set +a   (sonra XS_LIVE_ENABLED=1)"
echo "  ⚠️ Otonom ParameterStore'a besleme A/B kazanç kanıtlamadan opt-in kalsın ([[project_edge_scan]])."
