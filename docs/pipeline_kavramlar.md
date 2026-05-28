# Otonom Trading Pipeline — Kavram Sözlüğü

> Bu belge, motorun **gerçek kod akışına** göre hazırlanmış terimsel haritadır: her kavramın
> tanımı, kısa formülü ve **hangi aşamada / hangi durumda** devreye girdiği.
> Belkemiği: `memos_trading_core/src/robot/data_pipeline/canon.rs` içindeki **7 aşamalı canon
> pipeline**. Her cycle (tur), seçili bir sembol için bu 7 aşamadan baştan geçer.

---

## Zaman ekseni (timeframe mantığı)

| Kavram | Tanım | Ne zaman | Kaynak |
|---|---|---|---|
| **Cycle / cadence** | Motorun bir turu; bir sembol 7 aşamadan geçer | Sürekli; her tur baştan | `master/loop_core.rs` |
| **Base TF** | `config.interval` — sinyal/gösterge bu çözünürlükte | Her cycle | `config` |
| **HTF (üst zaman dilimi)** | `MULTI_TF` / `use_htf` ile rejim ve bias geniş TF'de hesaplanır | Rejim tespiti, screener bias, GBT eğitimi | `htf_loader` |
| **RegimeContext cache TTL** | `REGIME_CONTEXT_TTL_SECS` (default 900s) — rejim her tur değil, TTL dolunca yeniden hesaplanır | Aşama 2; seyrek tespit | `RuntimeTuning` |
| **Candle freshness** | `CANDLE_FRESHNESS_SECS` (300s) — bundan eski mum fiyat referansı sayılmaz | Aşama 1 & 4 | `RuntimeTuning` |

> **Özet:** sinyal base TF'de hızlı, rejim/yön HTF'de yavaş (cache'li) akar. İkisi farklı saat
> hızında dönen iki çarktır.

### Canon aşamaları (özet)

```
1. DataIngest      → Veri Akışı
2. FeatureExtract  → Özellik Çıkarımı (gösterge + rejim)
3. StrategyEval    → Strateji & Sinyal
4. RiskGate        → Risk Kapısı
5. Execute         → İcra
6. Learn           → Öğrenme
7. Optimize        → Optimizasyon
```

---

## 1. DataIngest — *Veri Akışı*

Piyasa verisini içeri alır.

- **Candle (OHLCV):** open/high/low/close/volume. Tüm göstergelerin girdisi.
- **live_price:** `price_poll` 5sn REST snapshot. Entry fiyatının birincil kaynağı; DB mumu yedek.
- **`symbol_eligible_for_live`:** sembol canlı feed'e uygun mu? Tek-kaynak market kapısı (BIST gibi
  feed'siz borsalar cycle'dan dışlanır → anomaly birikimini önler).
- **HTF loader:** üst TF mumlarını TTL cache ile çeker (`MULTI_TF` açıkken).
- **Anomaly guardian:** boş/bayat/sapmalı veride uyarı biriktirir (severity'li).

---

## 2. FeatureExtract — *Özellik Çıkarımı* (göstergeler + rejim)

### Göstergeler (sinyal ve risk hesaplarının ham maddesi)

| Gösterge | Kısa tanım / formül | Kullanım |
|---|---|---|
| **ATR(14)** | ort. *true range* = max(H−L, \|H−Cₚ\|, \|L−Cₚ\|) ortalaması | SL/TP/trailing mesafesi, edge normalizasyonu, leverage noise-floor |
| **ADX** | trend gücü → `AdxRegime{Ranging, Neutral, Trending, Volatile}` | Rejim sınıflama |
| **Supertrend / EMA** | trend yönü | Trend stratejileri |
| **RSI / Bollinger** | aşırı alım-satım, yatay bant | Range / mean-reversion stratejileri |
| **PSAR** | parabolik dur-ve-dön | Trend takip / çıkış |

### RegimeContext (Hedef mimari "Adım 1")

- **`MarketRegime`** = `{Trending, Ranging, Volatile, LowVolatility, Unknown}`.
- Basit örnek formül: `stddev > mean·0.05 → Volatile`, `close > mean → Trending`, aksi `Ranging`.
- **Pluggable detector:** matematik → GBT / ONNX'e takılabilir. TTL'li, HTF-tercihli cache'te
  (`BrainBox.regime_context`).
- **GBT regime direction** (`REGIME_GBT`, default açık): GBT skoru Trending **yönünü** belirler
  (yalnız rejimde; edge saf-matematik kalır). Geri-dönüş: `GBT_EDGE_LEGACY`.

---

## 3. StrategyEval — *Strateji & Sinyal*

- **StrategySelector:** rejim → strateji eşlemesi (Trending→trend, Ranging→range, Volatile→IDLE
  savunması).
- **`STRATEGY_SELECT_EVAL`:** açıksa adayları **mini-backtest skoruyla** seçer (param_spec
  optimizasyonu seçime girer).
- **`Signal` = {Buy, Sell, Hold}:** stratejinin ürettiği karar.
- **Edge score** (giriş hunisi — *bu sinyal gerçekten avantajlı mı?*):

  ```
  edge = dir_match · (mom_strength·mom_w + ml·ml_w)
  mom_strength = |momentum| / ATR%        (kaç ATR yön yapıldı)
  dir_match    = 1.0 (yön uyumlu) | 0.4 (ters yön → ceza) | 0 (Hold)
  ml_w         = 0.5 (ml hazırsa) | 0 (ml≈0 → momentum tek taşıyıcı), mom_w = 1 − ml_w
  ```

  - **Dinamik eşik** (`dynamic_edge_threshold`): `ml_confidence < 0.05 → 0.20`, `< 0.30 → 0.35`,
    değilse `0.55`. ML hazır değilken gevşek, hazırlandıkça katı.
  - **Ne zaman:** `edge < eşik` → giriş yok. Backtest'te de aynı huni (`BACKTEST_EDGE_FILTER`,
    tek-kaynak).
- **ScalpSwing:** ayrı kanal (scalp / swing); kendi skoru + `auto_tune`. `SCALP_SWING_ENABLE=0` ile
  kapanır.

---

## 4. RiskGate — *Risk Kapısı*

İlk **Deny**'de kısa devre (RiskFilter chain). Geçerse pozisyon boyutu / kaldıraç burada hesaplanır.

- **RiskFilter chain:** drawdown, ardışık zarar (loss streak), exposure, `blocked_symbols`. İlk veto
  trade'i durdurur (`RiskDecision::Deny`).
- **Kelly Criterion** (pozisyon büyüklüğü):

  ```
  f* = (p·b − q) / b           p = kazanma olasılığı, q = 1 − p, b = avg_win / avg_loss
  base_alloc  = equity · 0.10 · risk_appetite
  alloc       = Kelly dinamik ölçek(base_alloc, loss_streak, ml_confidence)
  ```

  `loss_streak` ve `ml_confidence` ile dinamik küçültme / büyütme.
- **Leverage resolve** (`LEVERAGE_ENABLED`, default kapalı → 1.0 = spot):

  ```
  lev = base × regime_mult × conf_boost × winrate_feedback
  regime_mult : Volatile 0.5 · StrongTrend 1.3 · Ranging/LowVol 1.0 · diğer 0.9
  conf_boost  : ml_confidence ≥ eşik → ×1.2
  winrate     : win_rate ≥ 0.6 → ×1.15 ; 0 < win_rate ≤ 0.4 → ceza ; 0 = nötr
  ```

- **Price sanity guard** (`MAX_ENTRY_PRICE_DEVIATION_PCT`, %5): entry ↔ son mum sapması eşiği aşarsa
  açma (bayat fiyatla sahte kâr döngüsünü önler). 0 → guard kapalı.
- **Exchange filtreleri:** `apply_filters` → LOT_SIZE (stepSize'a yuvarla), MIN_NOTIONAL, PRICE_FILTER
  (tickSize). Binance −1013 reddini önler.
- **`live_max_notional` tavanı:** üstündeki emir veto edilir.

---

## 5. Execute — *İcra*

İcra-öncesi politika + emir + koruma.

- **ExecutionPolicy chain** (Skip): `MarketHours → IdleStrategy → BasketEmpty`. İlk Skip sembolü
  atlar, kalanlarla devam.
- **Giriş emri (dispatch):**
  - **Maker LIMIT** (`USE_LIMIT_ENTRY=1`, opt-in): POST_ONLY (futures GTX / spot LIMIT_MAKER),
    best_bid/ask'e katıl, spread guard (`LIMIT_ENTRY_MAX_SPREAD_BPS`) + N deneme
    (`LIMIT_ENTRY_MAX_ATTEMPTS` / `_TIMEOUT_MS`). Dolmazsa `LIMIT_ENTRY_FALLBACK_MARKET`'e göre
    taker'a düş ya da trade atla.
  - **Taker MARKET** (default): anında dolum.
  - **Fill reconciliation:** gerçek dolum fiyatından entry/SL/TP/trailing yeniden hesaplanır
    (maker yolunda).
- **Protection orders:** SL (`STOP_MARKET`+reduceOnly / spot `STOP_LOSS`), TP
  (`TAKE_PROFIT_MARKET`). Bot ölse / network kopsa bile pozisyon korumalı kalır.
- **Trailing stop:** `entry ∓ ATR·atr_mult`; `LET_WINNERS_RUN` ile sabit TP uzağa itilip çıkışı
  trailing yönetir.
- **Komisyon:** `commission_rate` (taker) / `maker_commission_rate` (maker dolum); entry + exit
  simetrik düşülür.

---

## 6. Learn — *Öğrenme*

Kapanış geri-beslemesi.

- **IntelligenceHub:** açılışta `track_trade(pos_id, regime, strategy)` ↔ kapanışta
  `learn_from_exit`. Hangi rejimde hangi strateji kazandı / kaybetti.
- **ScalpSwingStats / win_rate:** kanal-bazlı istatistik; win_rate düşükse kanal kapanır.
- **Regime drift gözlemi** (`observe_regime_drift`): rejim önceki tura göre değiştiyse store patch'i
  bir kademe sıkılaştırır + cooldown + Telegram/UI uyarısı. İlk gözlem değişim sayılmaz (cold start).
- **PnL muhasebesi:** closed_trades, equity, execution costs.

---

## 7. Optimize — *Optimizasyon*

Periyodik; öğrenilenleri parametrelere yazar (scheduler ile seyrek).

- **HyperOpt / ParameterStore:** rejim-aware `trade_risk` patch'leri (TP/SL/edge eşikleri rejime
  göre).
- **ParamSpec araması:** strateji parametre uzayı; **canlı + backtest tek kaynak** (kaçak kapalı).
- **Backtester:** gerçek `Strategy` trait'iyle, HTF-hizalı, look-ahead'siz; aynı edge hunisi.
- **Screener:** sembol seçimi; `SCREENER_HTF_BIAS` ile üst TF eğilimi.
- **GBT training:** HTF mumlarıyla eğitilir (train/infer tutarlı), Aşama 2'ye yön besler.
- **Scheduler periyotları:** `SCHEDULER_SCREENER_EVERY_MINS`, `SCHEDULER_ML_EVERY_MINS`, vb.

---

## Akışın tek cümlelik özeti

> **Veri gelir (1)** → **gösterge + rejim çıkarılır (2)** → **rejime uygun strateji sinyal üretir,
> edge hunisinden geçer (3)** → **risk filtreleri + Kelly / kaldıraç boyutlandırır (4)** →
> **politika + maker/market emir + SL/TP icra eder (5)** → **kapanış öğretir (6)** →
> **öğrenilen parametreye / optimizasyona döner (7)**.

---

## İlgili env bayrakları (hızlı referans)

> 📋 **Tüm env'lerin default'lu, gruplu tam listesi:** [`.env.example`](../.env.example)
> (repo kökünde). Kullanım: `cp .env.example .env`. Aşağıdaki tablo yalnızca
> pipeline aşamalarıyla eşleşen seçili bayrakları gösterir.

| Env | Aşama | İşlev |
|---|---|---|
| `MULTI_TF`, `REGIME_CONTEXT_TTL_SECS` | 1–2 | HTF veri + rejim cache |
| `CANDLE_FRESHNESS_SECS` | 1, 4 | Mum tazelik eşiği |
| `REGIME_GBT`, `GBT_EDGE_LEGACY` | 2 | GBT'nin rejim yönü / eski edge yolu |
| `STRATEGY_SELECT_EVAL` | 3 | Mini-backtest ile strateji seçimi |
| `BACKTEST_EDGE_FILTER` | 3, 7 | Edge hunisi (canlı + backtest tek kaynak) |
| `SCALP_SWING_ENABLE` | 3 | ScalpSwing kanalı |
| `LEVERAGE_ENABLED` | 4 | Otonom kaldıraç |
| `STARTING_CAPITAL`, `BASE_ALLOC_FRACTION`, `ALLOC_FLOOR_FRACTION` | 4 | Sermaye + pozisyon boyutlama |
| `KELLY_LOSS_STREAK_WINDOW`, `KELLY_STATS_WINDOW` | 4 | Kelly istatistik pencereleri |
| `FALLBACK_TP_PCT`, `FALLBACK_SL_PCT` | 4 | ParameterStore okunamazsa son-çare TP/SL |
| `MAX_ENTRY_PRICE_DEVIATION_PCT` | 4 | Price sanity guard |
| `LIVE_MAX_NOTIONAL_USD` | 4 | Notional tavanı |
| `USE_LIMIT_ENTRY`, `LIMIT_ENTRY_*`, `MAKER_COMMISSION_RATE` | 5 | Maker LIMIT girişi |
| `LET_WINNERS_RUN` | 5 | Sabit TP yerine trailing yönetimi |
| `COMMISSION_RATE` | 5 | Taker komisyon oranı |
| `SCREENER_HTF_BIAS`, `SCHEDULER_*` | 7 | Screener / optimizasyon periyotları |
