#!/usr/bin/env bash
# Golden davranış kontrolü — dikey-dilim refactor'ünün regresyon ağı.
#
# Test suite derlenmediği için (tests/risk_filter_chain.rs, trade_summary_report.rs)
# bu betik onun yerine geçer: algoritmik çekirdeğin deterministik parmak izini
# golden/BASELINE.txt ile karşılaştırır. HER refactor adımından sonra çalıştır.
#
#   ./scripts/golden_check.sh            → karşılaştır (fark varsa exit 1)
#   ./scripts/golden_check.sh --accept   → mevcut davranışı yeni temel kabul et
set -uo pipefail
cd "$(dirname "$0")/.."
BASE="golden/BASELINE.txt"
CUR="golden/current.txt"

cargo run --release --quiet --example golden_baseline -p memos_trading_core > "$CUR" 2>/dev/null || {
  echo "❌ golden_baseline derlenmedi/çalışmadı"; exit 2; }

if [ "${1:-}" = "--accept" ]; then
  cp "$CUR" "$BASE"; sha256sum "$BASE" > golden/BASELINE.sha256
  echo "✅ yeni temel kabul edildi: $(sha256sum "$BASE" | cut -c1-16)"; exit 0
fi

if [ ! -f "$BASE" ]; then echo "❌ $BASE yok — önce --accept ile temel oluştur"; exit 2; fi

if diff -q "$BASE" "$CUR" >/dev/null; then
  echo "✅ GOLDEN OK — davranış değişmedi ($(wc -l < "$CUR") satır)"
  exit 0
else
  echo "❌ DAVRANIŞ DEĞİŞTİ — $(diff "$BASE" "$CUR" | grep -c '^[<>]') satır farklı"
  echo "--- bölüm bazında fark ---"
  for s in REGISTRY BACKTEST INDICATORS KELLY EXIT EXIT_WALK RISKGATE; do
    n=$(diff <(sed -n "/===== $s /,/=====/p" "$BASE") <(sed -n "/===== $s /,/=====/p" "$CUR") | grep -c '^[<>]')
    [ "$n" -gt 0 ] && echo "  $s: $n satır"
  done
  echo "--- ilk 25 fark ---"; diff "$BASE" "$CUR" | head -25
  exit 1
fi
