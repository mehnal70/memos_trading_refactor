#!/usr/bin/env bash
# scripts/launch.sh — Memos RTC interaktif BAŞLATMA menüsü.
#
# Amaç: run_rtc.sh'i çalıştırmadan önce tüm çalıştırma parametrelerini (env + build/hedef) tek
# ekranda gör/düzenle, başlamadan önce NET ÖZET al → terminalde env'leri tek tek yazıp bir şeyi
# unutma/gözden kaçırma riski biter. Seçimler scripts/.launch.conf'a yazılır → sonraki açılışta
# son ayarlar default gelir. (Engine YÖNETİMİ — start/stop/status/log — ayrı: scripts/engine.sh.)
#
# Kullanım: ./scripts/launch.sh
set -u
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"
CONF="scripts/.launch.conf"

# ── Parametre tanımı: anahtar | grup | tip(bool/text/cycle:a/b/..) | açıklama ───────────────
# (sıra korunur; cycle = sabit seçenekler arası dön; bool = 0/1 toggle; text = serbest gir)
KEYS=(
  BUILD_MODE TARGET
  TRADE_MARKET AUTO_INTERVAL_CANDIDATES
  EDGE_SEED_REPORT EDGE_SEED_MIN_TRADES EDGE_SEED_MAX_PF EDGE_SEED_MIN_QVOL
  EDGE_SEED_REQUIRE_WF EDGE_SEED_MULTI_TF EDGE_SEED_MAX_TRACKS EDGE_SEED_IGNORE_MARKET
  SEED_STRATEGY_PRIORITY REGIME_DIRECTIONAL REGIME_ADAPTIVE_PCTL SCALP_SWING_ENABLE
  USE_LIMIT_ENTRY MAKER_COMMISSION_RATE LET_WINNERS_RUN STALE_FEED_MAX_AGE_SECS
  XS_LIVE_ENABLED XS_LIVE_SYMBOLS XS_LIVE_INTERVAL XS_LIVE_LOOKBACK XS_LIVE_TOP_K
  XS_LIVE_BUFFER XS_LIVE_POSITION_PCT XS_LIVE_LEVERAGE XS_LIVE_REGIME_GATE XS_LIVE_MOMENTUM
  XS_LIVE_MAX_DD_PCT XS_LIVE_CB_COOLDOWN_SECS
  GRADED_ENTRY_ENABLED GRADED_TRANCHE_WEIGHTS GRADED_FAVORABLE_MOVE_PCT
  GRADED_ADVERSE_MOVE_PCT GRADED_REQUIRE_HTF
)
declare -A GROUP TYPE DESC VAL
set_meta(){ GROUP[$1]=$2; TYPE[$1]=$3; DESC[$1]=$4; }
set_meta BUILD_MODE               "Build"   "cycle:release/debug"    "derleme profili"
set_meta TARGET                   "Build"   "cycle:tui/headless"     "tui=arayüz · headless=servis"
set_meta TRADE_MARKET             "Genel"   "cycle:futures/spot"     "işlem piyasası"
set_meta AUTO_INTERVAL_CANDIDATES "Genel"   "text"                   "otonom interval adayları (csv)"
set_meta EDGE_SEED_REPORT         "Seed"    "report"                 "edge_sweep raporu (boş=seed yok)"
set_meta EDGE_SEED_MIN_TRADES     "Seed"    "text"                   "seed min işlem (1d az→15)"
set_meta EDGE_SEED_MAX_PF         "Seed"    "text"                   "fluke cap (PF üst sınır)"
set_meta EDGE_SEED_MIN_QVOL       "Seed"    "text"                   "likidite tabanı USDT/gün (0=kapalı)"
set_meta EDGE_SEED_REQUIRE_WF     "Seed"    "bool"                   "yalnız WF-onaylı seed"
set_meta EDGE_SEED_MULTI_TF       "Seed"    "bool"                   "çoklu-TF iz düzeneği (opt-in)"
set_meta EDGE_SEED_MAX_TRACKS     "Seed"    "text"                   "sembol başına azami iz"
set_meta EDGE_SEED_IGNORE_MARKET  "Seed"    "bool"                   "çapraz-market seed (riskli)"
set_meta SEED_STRATEGY_PRIORITY   "Otonomi" "bool"                   "seed'li sembolde ScalpSwing pas"
set_meta REGIME_DIRECTIONAL       "Otonomi" "bool"                   "rejim-yön teyidi (ters-trend ele)"
set_meta REGIME_ADAPTIVE_PCTL     "Otonomi" "text"                   "adaptif Volatile pctl (boş=sabit)"
set_meta SCALP_SWING_ENABLE       "Otonomi" "bool"                   "ScalpSwing alt-kanalı"
set_meta USE_LIMIT_ENTRY          "İcra"    "bool"                   "maker LIMIT giriş (yoksa taker MARKET)"
set_meta MAKER_COMMISSION_RATE    "İcra"    "text"                   "maker komisyon oranı (boş=taker ile aynı; XS≈0.0002)"
set_meta LET_WINNERS_RUN          "İcra"    "bool"                   "kazananı koştur (trail genişlet)"
set_meta STALE_FEED_MAX_AGE_SECS  "İcra"    "text"                   "bayat-feed eşiği sn (boş=auto)"
# ── Kesitsel adanmış mod (XS momentum — market-nötr long/short kitabı) ──────────────────────
# Doğrulanmış edge (gross OOS p=0.009, net band=1 maker günlük p=0.034, NW-HAC sonrası da anlamlı).
# XS_LIVE_ENABLED=0 → mod kapalı (sıfır regresyon); 1 + sepet ile aktive. Maker net için yukarıdaki
# USE_LIMIT_ENTRY=1 + MAKER_COMMISSION_RATE≈0.0002 ile birlikte kullan.
set_meta XS_LIVE_ENABLED          "Kesitsel" "bool"                  "kesitsel adanmış mod (market-nötr L/S kitabı)"
set_meta XS_LIVE_SYMBOLS          "Kesitsel" "text"                  "sepet (csv; ≥2·top_k derin major)"
set_meta XS_LIVE_INTERVAL         "Kesitsel" "text"                  "sepet TF (doğrulanan: 1d)"
set_meta XS_LIVE_LOOKBACK         "Kesitsel" "text"                  "momentum bar sayısı (boş=14)"
set_meta XS_LIVE_TOP_K            "Kesitsel" "text"                  "bacak başına sembol (long=short=k)"
set_meta XS_LIVE_BUFFER           "Kesitsel" "text"                  "no-trade band (rank histerezisi; 1-2)"
set_meta XS_LIVE_POSITION_PCT     "Kesitsel" "text"                  "bacak başına equity oranı (eşit-ağırlık)"
set_meta XS_LIVE_LEVERAGE         "Kesitsel" "text"                  "sabit kaldıraç (anlamlılık L-invariant)"
set_meta XS_LIVE_REGIME_GATE      "Kesitsel" "bool"                  "yüksek-vol'da kitabı flat çek (kriz koruması)"
set_meta XS_LIVE_MOMENTUM         "Kesitsel" "bool"                  "momentum (kapalı=reversal; doğrulanan: momentum)"
set_meta XS_LIVE_MAX_DD_PCT       "Kesitsel" "text"                  "devre kesici: kitap DD%% eşiği (0=kapalı)"
set_meta XS_LIVE_CB_COOLDOWN_SECS "Kesitsel" "text"                  "devre kesici sonrası flat kalma sn (default 3600)"
# ── Kademeli giriş (XS HARİÇ — pozisyonu N kademede, rejime göre pyramiding/averaging) ───────
set_meta GRADED_ENTRY_ENABLED     "Kademeli" "bool"                  "kademeli giriş (XS dışı pozisyonlar)"
set_meta GRADED_TRANCHE_WEIGHTS   "Kademeli" "text"                  "kademe ağırlıkları csv (örn 0.4,0.3,0.3)"
set_meta GRADED_FAVORABLE_MOVE_PCT "Kademeli" "text"                 "pyramiding eşiği: lehte hareket %% (trend rejim)"
set_meta GRADED_ADVERSE_MOVE_PCT  "Kademeli" "text"                  "averaging eşiği: aleyhte hareket %% (ranging rejim)"
set_meta GRADED_REQUIRE_HTF       "Kademeli" "bool"                  "ek kademe için HTF trend hizası şart"

# ── Default'lar (doğrulanmış temiz futures profili) ─────────────────────────────────────────
defaults(){
  VAL[BUILD_MODE]=release;            VAL[TARGET]=tui
  VAL[TRADE_MARKET]=futures;          VAL[AUTO_INTERVAL_CANDIDATES]="1h,4h,1d"
  VAL[EDGE_SEED_REPORT]="$(latest_report)"
  VAL[EDGE_SEED_MIN_TRADES]=15;       VAL[EDGE_SEED_MAX_PF]=10
  VAL[EDGE_SEED_MIN_QVOL]=0;          VAL[EDGE_SEED_REQUIRE_WF]=1
  VAL[EDGE_SEED_MULTI_TF]=0;          VAL[EDGE_SEED_MAX_TRACKS]=3
  VAL[EDGE_SEED_IGNORE_MARKET]=0
  VAL[SEED_STRATEGY_PRIORITY]=1;      VAL[REGIME_DIRECTIONAL]=0
  VAL[REGIME_ADAPTIVE_PCTL]="";       VAL[SCALP_SWING_ENABLE]=1
  VAL[USE_LIMIT_ENTRY]=0;             VAL[MAKER_COMMISSION_RATE]=""
  VAL[LET_WINNERS_RUN]=0;             VAL[STALE_FEED_MAX_AGE_SECS]=""
  # Kesitsel: ENABLED=0 → mod kapalı (mevcut davranış birebir). Diğerleri doğrulanmış config'in
  # ön-doldurulmuş değerleri → ENABLED'ı 1'e çevirince çalışan bir kitap kurulur (sepet düzenlenebilir).
  VAL[XS_LIVE_ENABLED]=0
  VAL[XS_LIVE_SYMBOLS]="BTCUSDT,ETHUSDT,BCHUSDT,XRPUSDT,TRXUSDT,ADAUSDT,ZECUSDT,BNBUSDT,ONTUSDT,DOGEUSDT,SOLUSDT,UNIUSDT,AVAXUSDT,STORJUSDT,ALPHAUSDT"
  VAL[XS_LIVE_INTERVAL]=1d;           VAL[XS_LIVE_LOOKBACK]=14
  VAL[XS_LIVE_TOP_K]=3;               VAL[XS_LIVE_BUFFER]=1
  VAL[XS_LIVE_POSITION_PCT]=0.10;     VAL[XS_LIVE_LEVERAGE]=1
  VAL[XS_LIVE_REGIME_GATE]=1;         VAL[XS_LIVE_MOMENTUM]=1
  VAL[XS_LIVE_MAX_DD_PCT]=0;          VAL[XS_LIVE_CB_COOLDOWN_SECS]=3600
  # Kademeli giriş: ENABLED=0 → kapalı (tek-fill, mevcut davranış). Diğerleri doğrulanmış default'lar.
  VAL[GRADED_ENTRY_ENABLED]=0;        VAL[GRADED_TRANCHE_WEIGHTS]="0.4,0.3,0.3"
  VAL[GRADED_FAVORABLE_MOVE_PCT]=1.0; VAL[GRADED_ADVERSE_MOVE_PCT]=1.0
  VAL[GRADED_REQUIRE_HTF]=1
}
latest_report(){ ls -t reports/edge_sweep_*.json 2>/dev/null | head -1; }

load_conf(){ [ -f "$CONF" ] || return 0; while IFS='=' read -r k v; do [ -n "${k:-}" ] && VAL[$k]="$v"; done < "$CONF"; }
save_conf(){ : > "$CONF"; for k in "${KEYS[@]}"; do printf '%s=%s\n' "$k" "${VAL[$k]}" >> "$CONF"; done; }

# ── Menü çizimi ─────────────────────────────────────────────────────────────────────────────
draw(){
  clear 2>/dev/null || true
  echo "╔══════════════════════════════════════════════════════════════════════╗"
  echo "║   Memos RTC — Başlatma Parametreleri (düzenle → 's' başlat)          ║"
  echo "╚══════════════════════════════════════════════════════════════════════╝"
  local last_group="" i=1
  for k in "${KEYS[@]}"; do
    if [ "${GROUP[$k]}" != "$last_group" ]; then
      printf "\n  ── %s ──────────────────────────────\n" "${GROUP[$k]}"
      last_group="${GROUP[$k]}"
    fi
    printf "  %2d) %-26s %-22s  %s\n" "$i" "$k" "$(show_val "$k")" "${DESC[$k]}"
    i=$((i+1))
  done
  echo
  echo "  ─────────────────────────────────────────────────────────────────────"
  echo "   numara=düzenle · s=BAŞLAT · d=default'a dön · q=çık"
}
show_val(){
  local k=$1 v="${VAL[$k]}"
  case "${TYPE[$k]}" in
    bool) [ "$v" = "1" ] && echo "[✓ açık]" || echo "[· kapalı]" ;;
    report) [ -n "$v" ] && echo "$(basename "$v")" || echo "(yok)" ;;
    *) [ -n "$v" ] && echo "$v" || echo "(boş/auto)" ;;
  esac
}

# ── Bir parametreyi düzenle ─────────────────────────────────────────────────────────────────
edit_key(){
  local k=$1
  local t="${TYPE[$k]}"
  case "$t" in
    bool)  [ "${VAL[$k]}" = "1" ] && VAL[$k]=0 || VAL[$k]=1 ;;
    cycle:*) cycle_val "$k" "${t#cycle:}" ;;
    report) pick_report ;;
    text)  printf "  %s (şu an: '%s') yeni değer [boş=değişme/temizle için bir boşluk]: " "$k" "${VAL[$k]}"
           read -r nv; [ -n "$nv" ] && VAL[$k]="$(echo "$nv" | sed 's/^ *$//')" ;;
  esac
}
cycle_val(){ # k "a/b/c" → mevcut değerin bir sonrakine geç
  local k=$1; IFS='/' read -ra opts <<< "$2"; local cur="${VAL[$k]}" n=${#opts[@]} j
  for j in "${!opts[@]}"; do [ "${opts[$j]}" = "$cur" ] && { VAL[$k]="${opts[$(((j+1)%n))]}"; return; }; done
  VAL[$k]="${opts[0]}"
}
pick_report(){
  local reports=(); while IFS= read -r f; do reports+=("$f"); done < <(ls -t reports/edge_sweep_*.json 2>/dev/null)
  echo; echo "  Raporlar (yeni→eski):"
  echo "   0) (seed YOK — boş bırak)"
  local i=1; for f in "${reports[@]}"; do printf "  %2d) %s  (%s)\n" "$i" "$(basename "$f")" "$(date -r "$f" '+%m-%d %H:%M' 2>/dev/null)"; i=$((i+1)); done
  printf "  Seç (numara, boş=değişme): "; read -r s
  [ -z "$s" ] && return
  [ "$s" = "0" ] && { VAL[EDGE_SEED_REPORT]=""; return; }
  [ "$s" -ge 1 ] 2>/dev/null && [ "$s" -le "${#reports[@]}" ] && VAL[EDGE_SEED_REPORT]="${reports[$((s-1))]}"
}

# ── Başlatma: özet + onay + run_rtc.sh ──────────────────────────────────────────────────────
launch(){
  clear 2>/dev/null || true
  echo "╔══════════════════════════════════════════════════════════════════════╗"
  echo "║   BAŞLATMA ÖZETİ — son kontrol                                       ║"
  echo "╚══════════════════════════════════════════════════════════════════════╝"
  echo "  Build  : ${VAL[BUILD_MODE]} · hedef ${VAL[TARGET]}"
  if [ -n "${VAL[EDGE_SEED_REPORT]}" ]; then
    if [ -f "${VAL[EDGE_SEED_REPORT]}" ]; then echo "  Seed   : $(basename "${VAL[EDGE_SEED_REPORT]}") ✓"
    else echo "  Seed   : ⚠️ DOSYA YOK → ${VAL[EDGE_SEED_REPORT]}"; fi
  else echo "  Seed   : (yok — global/auto strateji)"; fi
  echo "  Market : ${VAL[TRADE_MARKET]} · interval ${VAL[AUTO_INTERVAL_CANDIDATES]}"
  echo
  echo "  Aktif env:"
  local envline=""
  for k in "${KEYS[@]}"; do
    case "$k" in BUILD_MODE|TARGET) continue;; esac
    local v="${VAL[$k]}"; [ -z "$v" ] && continue
    printf "    %-26s = %s\n" "$k" "$v"
    envline+="$k=$v "
  done
  [ -z "${BINANCE_API_KEY:-}" ] && echo "  ⚠️ BINANCE_API_KEY yok → KAĞIT mod" || echo "  ✅ API key var → GERÇEK mod"
  echo
  printf "  Başlat? [E/h] (k=config kaydet+çık, geri için h): "; read -r ok
  case "$ok" in
    k|K) save_conf; echo "  ✓ scripts/.launch.conf kaydedildi."; exit 0 ;;
    h|H|n|N) return ;;
  esac
  save_conf
  # Kayıt: seçilen env'i zaman damgalı dosyaya da bas (sonradan "ne ile başlattım" denetimi).
  mkdir -p logs; local stamp; stamp="$(date '+%Y%m%d_%H%M%S')"
  { echo "# launch $stamp"; for k in "${KEYS[@]}"; do echo "$k=${VAL[$k]}"; done; } > "logs/launch_${stamp}.env"
  echo "  ✓ logs/launch_${stamp}.env"

  # env'i export et (boş olanlar atlanır → kod default'u geçerli kalır).
  for k in "${KEYS[@]}"; do
    case "$k" in BUILD_MODE|TARGET) continue;; esac
    [ -n "${VAL[$k]}" ] && export "$k=${VAL[$k]}"
  done
  local flags=(); [ "${VAL[BUILD_MODE]}" = "release" ] && flags+=(--release)
  [ "${VAL[TARGET]}" = "headless" ] && flags+=(--headless) || flags+=(--tui)
  echo "  → ./run_rtc.sh ${flags[*]}"; echo
  exec ./run_rtc.sh "${flags[@]}"
}

# ── Ana döngü ───────────────────────────────────────────────────────────────────────────────
defaults
load_conf
# conf'ta rapor yoksa/silinmişse en taze raporu öner.
{ [ -z "${VAL[EDGE_SEED_REPORT]}" ] || [ ! -f "${VAL[EDGE_SEED_REPORT]}" ]; } && VAL[EDGE_SEED_REPORT]="$(latest_report)"
while true; do
  draw
  printf "  > "; read -r cmd
  case "$cmd" in
    s|S) launch ;;
    q|Q) echo "çıkıldı."; exit 0 ;;
    d|D) defaults; VAL[EDGE_SEED_REPORT]="$(latest_report)" ;;
    ''|*[!0-9]*) ;; # boş/sayı-olmayan → yoksay
    *) idx=$((cmd-1)); [ "$idx" -ge 0 ] && [ "$idx" -lt "${#KEYS[@]}" ] && edit_key "${KEYS[$idx]}" ;;
  esac
done
