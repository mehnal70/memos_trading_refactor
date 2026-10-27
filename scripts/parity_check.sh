#!/usr/bin/env bash
set -u

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR" || exit 1

. "$REPO_DIR/scripts/lib_launchconf.sh"

FULL=0
PROFILE=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --full)
      FULL=1
      ;;
    --profile)
      [ "$#" -ge 2 ] || { printf 'Kullanim: %s [--full] [--profile PROFIL]\n' "$0" >&2; exit 2; }
      PROFILE="$2"
      shift
      ;;
    --profile=*)
      PROFILE="${1#*=}"
      ;;
    -h|--help)
      printf 'Kullanim: %s [--full] [--profile PROFIL]\n' "$0"
      exit 0
      ;;
    *)
      printf 'Bilinmeyen arguman: %s\nKullanim: %s [--full] [--profile PROFIL]\n' "$1" "$0" >&2
      exit 2
      ;;
  esac
  shift
done

profile_env_tmp=""
if [ -n "$PROFILE" ]; then
  profile_file="$REPO_DIR/profiles/$PROFILE.conf"
  [ -f "$profile_file" ] || { printf 'Profil bulunamadi: %s\n' "$PROFILE" >&2; exit 2; }
  profile_env_tmp="$(mktemp)"
  cat "$REPO_DIR/profiles/_base.conf" "$profile_file" >"$profile_env_tmp"
  load_launch_conf "$profile_env_tmp"
  export PARITY_PROFILE="$PROFILE"
  printf 'PROFILE  %s (global scripts/.launch.conf degistirilmedi)\n' "$PROFILE"
else
  load_launch_conf "$REPO_DIR/scripts/.launch.conf"
fi

FAILS=0
WARNS=0

pass() { printf 'PASS  %s\n' "$1"; }
warn() { printf 'WARN  %s\n' "$1"; WARNS=$((WARNS + 1)); [ -n "${2:-}" ] && printf '      Calistir: %s\n' "$2"; }
fail() { printf 'FAIL  %s\n' "$1"; FAILS=$((FAILS + 1)); [ -n "${2:-}" ] && printf '      Calistir: %s\n' "$2"; }
section() { printf '\n[%s] %s\n' "$1" "$2"; }

section 1 "Config ve sembol yonlendirme"
if [ -n "${VENUES:-}" ]; then pass "VENUES=${VENUES}"; else fail "VENUES bos" "./memos use paper-follower-pool"; fi
python3 - <<'PY' || FAILS=$((FAILS + 1))
import os
venues = {item.split(':', 1)[0] for item in os.getenv('VENUES', '').split(',') if item}
bad = []
for key in ('AUTO_BINANCE_SYMBOLS', 'AUTO_MT5_SYMBOLS', 'AUTO_IBKR_SYMBOLS'):
    for item in filter(None, map(str.strip, os.getenv(key, '').split(','))):
        if '@' not in item or item.rsplit('@', 1)[1] not in venues:
            bad.append(f'{key}:{item}')
if bad:
    print('FAIL  Etiketsiz veya aktif VENUES disinda semboller: ' + ', '.join(bad))
    print('      Calistir: ./memos use paper-follower-pool')
    raise SystemExit(1)
print('PASS  Venue sembol etiketleri tutarli')
PY

section 2 "Kod ve calisma artefakti"
if [ "$FULL" -eq 1 ]; then
  if cargo check --workspace >/tmp/memos-parity-build.log 2>&1; then
    pass "Workspace derleniyor"
  else
    fail "Workspace derleme hatasi" "cargo check --workspace"
  fi
else
  [ -x target/release/rtc_tui ] && pass "Release TUI binary mevcut" || fail "Release TUI binary yok" "./memos build"
  printf 'SKIP  Workspace derleme (--full ile etkin)\n'
fi

section 3 "Process, port ve recovery"
readiness_tmp="$(mktemp)"
trap 'rm -f "$readiness_tmp" "$profile_env_tmp"' EXIT
if python3 scripts/venue_readiness.py all >"$readiness_tmp"; then
  pass "Venue readiness artefakti guncel"
else
  fail "Venue readiness artefakti uretilemedi" "python3 scripts/venue_readiness.py all"
fi
python3 - "$readiness_tmp" <<'PY'
import json
import os
import sys

with open(sys.argv[1], encoding="utf-8") as fh:
    data = json.load(fh)
failed = False
configured = {
    item.split(':', 1)[0].strip()
    for item in os.getenv('VENUES', '').split(',')
    if ':' in item
}
profile_scoped = bool(os.getenv('PARITY_PROFILE'))
for venue, item in data.items():
    if profile_scoped and venue not in configured:
        continue
    overall = item.get("overall")
    if overall == "NOT_ACTIVE":
        continue
    engine = "UP" if item.get("engine", {}).get("up") else "DOWN"
    watchdog = "UP" if item.get("watchdog", {}).get("up") else "DOWN"
    if overall in {"READY", "SIGNAL_READY"} and engine == "UP" and watchdog == "UP":
        print(f"PASS  {venue}: {overall} engine={engine} watchdog={watchdog}")
    else:
        print(f"FAIL  {venue}: {overall} engine={engine} watchdog={watchdog}")
        failed = True
raise SystemExit(1 if failed else 0)
PY
[ "$?" -eq 0 ] || FAILS=$((FAILS + 1))

for spec in "mt5:9001:MT5 bridge" "ibkr:9101:IBKR bridge" "ibkr:4002:IBKR API"; do
  venue="${spec%%:*}"; rest="${spec#*:}"
  port="${rest%%:*}"; label="${rest#*:}"
  echo "${VENUES:-}" | grep -Eq "(^|,)${venue}:" || { printf 'SKIP  %s (venue etkin degil)\n' "$label"; continue; }
  if ss -ltn 2>/dev/null | grep -q ":${port} "; then pass "$label portu acik ($port)"; else warn "$label portu kapali ($port)" "./memos recover"; fi
done

section 4 "Venue protokolu"
doctor_out="$(./memos doctor 2>&1)"
if echo "${VENUES:-}" | grep -Eq '(^|,)ibkr:'; then
  printf '%s\n' "$doctor_out" | grep -q 'OK  - IB API handshake basarili' \
    && pass "IBKR API handshake" || warn "IBKR API handshake yok" "./memos recover"
else
  printf 'SKIP  IBKR API handshake (venue etkin degil)\n'
fi
printf '%s\n' "$doctor_out" | grep -q 'Kopru dinliyor (9001)\|Köprü dinliyor (9001)' \
  && {
    if echo "${VENUES:-}" | grep -Eq '(^|,)mt5:'; then
      pass "MT5 bridge listener"
    else
      printf 'SKIP  MT5 bridge listener (venue etkin degil)\n'
    fi
  } || {
    if echo "${VENUES:-}" | grep -Eq '(^|,)mt5:'; then
      warn "MT5 bridge listener dogrulanamadi" "./memos recover"
    else
      printf 'SKIP  MT5 bridge listener (venue etkin degil)\n'
    fi
  }

section 5 "Canli fiyat parity"
python3 - <<'PY'
import json, os, re, time
from datetime import datetime
path = 'logs/events_replay.jsonl'
latest = {}
try:
    with open(path, encoding='utf-8', errors='replace') as fh:
        size = os.fstat(fh.fileno()).st_size
        if size > 8 * 1024 * 1024:
            fh.seek(size - 8 * 1024 * 1024)
            fh.readline()
        for line in fh:
            try: event = json.loads(line)
            except Exception: continue
            if event.get('event_type') == 'MARKET_PRICE_UPDATE':
                latest[event.get('venue', '')] = event
except FileNotFoundError:
    pass
expected = []
for token in filter(None, os.getenv('VENUES', '').split(',')):
    exchange, _, market = token.partition(':')
    expected.append(f'{exchange}:{market}')
failed = False
warned = False
now = time.time()
for venue in expected:
    event = latest.get(venue)
    if not event:
        print(f'FAIL  {venue}: canli fiyat eventi yok')
        failed = True
        continue
    try:
      stamp = re.sub(r'(\.\d{6})\d+', r'\1', event['timestamp']).replace('Z', '+00:00')
      age = max(0, int(now - datetime.fromisoformat(stamp).timestamp()))
    except Exception:
        age = 999999
    if age <= 120:
      print(f'PASS  {venue}: son fiyat {age}s once ({event.get("symbol", "?")})')
    elif age <= 600:
      print(f'WARN  {venue}: son fiyat {age}s once ({event.get("symbol", "?")})')
      warned = True
    else:
      print(f'FAIL  {venue}: son fiyat {age}s once ({event.get("symbol", "?")})')
      print('      Calistir: ./memos recover')
      failed = True
raise SystemExit(1 if failed else 2 if warned else 0)
PY
parity_rc=$?
if [ "$parity_rc" -eq 1 ]; then FAILS=$((FAILS + 1)); elif [ "$parity_rc" -eq 2 ]; then WARNS=$((WARNS + 1)); fi

section 6 "DB namespace ve sembol kapsami"
python3 - <<'PY'
import os, sqlite3
db = os.getenv('DB_PATH', 'data/trader.db')
con = sqlite3.connect(db)
expected = {}
for token in filter(None, os.getenv('VENUES', '').split(',')):
    exchange, _, market = token.partition(':')
    expected[exchange] = market
rows = con.execute('select exchange, market, count(*) from candles group by exchange, market').fetchall()
failed = False
warned = False
retained_historical = {('binance', 'spot'), ('mt5', 'spot')}
for exchange, market, count in rows:
    wanted = expected.get(exchange)
    if wanted and market != wanted:
        if (exchange, market) in retained_historical:
            print(f'INFO  Retained historical namespace: {exchange}:{market} {count} mum; kanonik={exchange}:{wanted}')
            print('      Ayrı spot/CFD geçmişi korunuyor; aktif venue kapsamı kanonik namespace üzerinden kontrol edildi.')
        else:
            print(f'WARN  Namespace drift: {exchange}:{market} {count} mum; kanonik={exchange}:{wanted}')
            print('      Calistir: ./scripts/migrate_candle_namespace.sh  # once dry-run')
            warned = True
    elif wanted:
        print(f'PASS  Namespace {exchange}:{market}: {count} mum')
for key, exchange in [('AUTO_BINANCE_SYMBOLS','binance'),('AUTO_MT5_SYMBOLS','mt5'),('AUTO_IBKR_SYMBOLS','ibkr')]:
    market = expected.get(exchange)
    for tagged in filter(None, map(str.strip, os.getenv(key, '').split(','))):
        symbol = tagged.split('@', 1)[0]
        count = con.execute('select count(*) from candles where exchange=? and market=? and symbol=?', (exchange, market, symbol)).fetchone()[0]
        if count == 0:
            print(f'FAIL  DB eksik: {tagged} ({exchange}:{market})')
            failed = True
if not failed:
    print('PASS  Configured sembollerin kanonik DB kapsami var')
raise SystemExit(1 if failed else 2 if warned else 0)
PY
parity_rc=$?
if [ "$parity_rc" -eq 1 ]; then FAILS=$((FAILS + 1)); elif [ "$parity_rc" -eq 2 ]; then WARNS=$((WARNS + 1)); fi

section 7 "Strateji, risk ve pyramid politikasi"
for key in STRATEGY_BY_EXCHANGE STRATEGY_POOL_BY_EXCHANGE STRATEGY_SELECT_EVAL VENUE_MAX_CONCURRENT_OPEN VENUE_DAILY_LOSS_PCT PYRAMID_MAX_TRANCHES_BY_EXCHANGE PYRAMID_MAX_ATR_PCT_BY_EXCHANGE MAIN_LOOP_STALL_SECS; do
  value="${!key:-}"
  [ -n "$value" ] && pass "$key tanimli" || warn "$key tanimsiz" "./memos use paper-follower-pool"
done
if [[ "${STRATEGY_BY_EXCHANGE:-}" == *":AUTO"* ]]; then
  case "${STRATEGY_SELECT_EVAL:-0}" in
    1|true|TRUE|True|yes|YES|on|ON) pass "Venue AUTO mini-backtest secimi aktif" ;;
    0|false|FALSE|False|no|NO|off|OFF|'') pass "Venue AUTO mini-backtest secimi kapali (varsayilan davranis)" ;;
    *) fail "STRATEGY_SELECT_EVAL gecersiz: ${STRATEGY_SELECT_EVAL}" "STRATEGY_SELECT_EVAL=0 veya 1" ;;
  esac
  case "${LIVE_STRATEGY:-AUTO}" in
    AUTO|auto|DEFAULT|Default|default|'') pass "Global strateji venue AUTO secimini engellemiyor" ;;
    *) fail "LIVE_STRATEGY=${LIVE_STRATEGY} venue AUTO secimini eziyor" "LIVE_STRATEGY=AUTO" ;;
  esac
fi
python3 - <<'PY'
import os

pools = {}
for item in os.getenv('STRATEGY_POOL_BY_EXCHANGE', '').split(';'):
    if ':' not in item:
        continue
    venue, raw_pool = item.split(':', 1)
    pools[venue.strip().lower()] = {name.strip().upper() for name in raw_pool.split('|') if name.strip()}

strict_enabled = os.getenv('TREND_STRICT_MODE', '').lower() in {'1', 'true', 'yes', 'on'}
strict = {name.strip().upper() for name in os.getenv('TREND_STRICT_STRATEGY_POOL', '').split(',') if name.strip()}
failed = False
for venue, pool in sorted(pools.items()):
    effective = pool & strict if strict_enabled else pool
    if len(effective) < 2:
        print(f'FAIL  {venue} efektif strateji havuzu tekillesti: {sorted(effective)}')
        failed = True
    else:
        print(f'PASS  {venue} efektif strateji havuzu: {"|".join(sorted(effective))}')
raise SystemExit(1 if failed else 0)
PY
[ "$?" -eq 0 ] || FAILS=$((FAILS + 1))

section 8 "Hesap, emir ve pozisyon mutabakati"
case "${TRADING_MODE:-Paper}:${LIVE_DRY_RUN:-true}:${IBKR_READ_ONLY:-1}" in
  Paper:*|*:true:*|*:True:*|*:TRUE:*)
    printf 'SKIP  Gercek hesap/emir parity: Paper veya dry-run mod\n'
    ;;
  *:*:1)
    warn "IBKR read-only; emir ve pozisyon parity sinirli" "IBKR_READ_ONLY=0 ./memos recover"
    ;;
  *)
    warn "Canli mutabakat runtime reconcile loglariyla izleniyor; salt-okuma parity probe henuz yok" "./memos logs errors"
    ;;
esac

printf '\nSONUC  fail=%d warn=%d\n' "$FAILS" "$WARNS"
[ "$FAILS" -eq 0 ]