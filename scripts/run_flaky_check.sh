#!/bin/bash
# scripts/run_flaky_check.sh - Flaky test tespiti: tüm testleri N tur sıralı koşar.
#
# BIST testleri (bist_async_cli_test + bist_realtime_test) external API'ye
# bağımlı oldukları için #[ignore] ile işaretli — bu koşumda zaten dahil edilmez.
# (Onları manuel denemek için: cargo test --test bist_async_cli_test -- --ignored)
#
# Kullanım:
#   ./scripts/run_flaky_check.sh         # default 5 tur
#   ./scripts/run_flaky_check.sh 20      # 20 tur

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.." || exit 1

RUNS="${1:-5}"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

echo -e "${BOLD}🔁 Flaky check — ${RUNS} tur · cargo test --workspace --no-fail-fast${NC}"
echo -e "   (Network testleri zaten #[ignore], dahil edilmez: BIST + Yahoo + Binance download)"
echo

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

declare -i green_runs=0
declare -i red_runs=0
declare -A fail_counts

for i in $(seq 1 "$RUNS"); do
    start=$(date +%s)
    out="$TMP/run_$i.log"
    if cargo test --workspace --no-fail-fast > "$out" 2>&1; then
        elapsed=$(( $(date +%s) - start ))
        echo -e "▸ Tur $i/$RUNS  ${GREEN}✓${NC} hepsi yeşil (${elapsed}s)"
        green_runs+=1
    else
        elapsed=$(( $(date +%s) - start ))
        # "test foo::bar ... FAILED" satırlarını yakala (test result: FAILED özet satırı değil)
        mapfile -t fails < <(grep -E '^test .* \.\.\. FAILED$' "$out" \
                             | sed -E 's/^test (.*) \.\.\. FAILED$/\1/' \
                             | sort -u)
        n=${#fails[@]}
        echo -e "▸ Tur $i/$RUNS  ${RED}✗${NC} ${n} test fail (${elapsed}s)"
        for f in "${fails[@]}"; do
            echo "      • $f"
            fail_counts["$f"]=$(( ${fail_counts["$f"]:-0} + 1 ))
        done
        red_runs+=1
    fi
done

echo
echo -e "${BOLD}📊 Özet${NC}"
echo -e "  - ${GREEN}${green_runs}/${RUNS}${NC} tur tamamen yeşil"
if [ "$red_runs" -gt 0 ]; then
    echo -e "  - ${RED}${red_runs}${NC} turda fail"
    echo
    echo -e "${YELLOW}En az bir kez patlayan testler (kaç/${RUNS} tur):${NC}"
    for k in "${!fail_counts[@]}"; do
        printf "      %s — ${RED}%d/${RUNS}${NC}\n" "$k" "${fail_counts[$k]}"
    done
    echo
    echo -e "   Detay log: ${TMP}/run_<N>.log (script çıkışında silinir)"
fi

exit "$red_runs"
