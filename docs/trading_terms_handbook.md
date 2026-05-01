# Memos Trading — Teknik Terimler El Kitabı

**Sürüm:** 1.0 · **Tarih:** 2026-03-24
**Kapsam:** Sistemde kullanılan tüm teknik terimlerin tanımı, formülü, amacı, alternatifleri ve kullanım sırası/zamanlaması.

---

## İçindekiler

1. [Mum / Candle Kavramları](#1-mum--candle-kavramları)
2. [Trend İndikatörleri](#2-trend-i̇ndikatörleri)
3. [Momentum İndikatörleri](#3-momentum-i̇ndikatörleri)
4. [Volatilite İndikatörleri](#4-volatilite-i̇ndikatörleri)
5. [Hacim / Volume İndikatörleri](#5-hacim--volume-i̇ndikatörleri)
6. [Seviye İndikatörleri](#6-seviye-i̇ndikatörleri)
7. [Sinyal Üretimi ve Konsensüs](#7-sinyal-üretimi-ve-konsensüs)
8. [Pozisyon Boyutlandırma](#8-pozisyon-boyutlandırma)
9. [Stop-Loss / Take-Profit Mekanizmaları](#9-stop-loss--take-profit-mekanizmaları)
10. [Risk Yönetimi](#10-risk-yönetimi)
11. [Backtest ve Performans Metrikleri](#11-backtest-ve-performans-metrikleri)
12. [Makine Öğrenmesi Terimleri](#12-makine-öğrenmesi-terimleri)
13. [Evrimsel/Adaptif Bileşenler](#13-evrimseladaptif-bileşenler)
14. [Piyasa Yapısı Terimleri](#14-piyasa-yapısı-terimleri)
15. [Sistem Mimarisi Terimleri](#15-sistem-mimarisi-terimleri)

---

## 1. Mum / Candle Kavramları

### OHLCV

| Alan | Açıklama |
|------|----------|
| **Open** (Açılış) | Seansın ilk işlem fiyatı |
| **High** (En yüksek) | Seans boyunca ulaşılan zirve |
| **Low** (En düşük) | Seans boyunca düşülen dip |
| **Close** (Kapanış) | Seansın son işlem fiyatı — çoğu indikatörün girdisi |
| **Volume** (Hacim) | Seans içinde el değiştiren miktar |

**Kullanım zamanı:** Her indikatör hesaplaması öncesinde ham veri olarak.

---

### True Range (TR)

```
TR = max(High - Low,  |High - PrevClose|,  |Low - PrevClose|)
```

**Amaç:** Gece boşluklarını (gap) da kapsayan gerçek fiyat aralığını ölçer.
**Kullanım:** ATR'nin ham girdisi.
**Alternatif:** Basit `High - Low` (gap'leri görmez, düşük kaliteli).

---

### HTF (Higher Timeframe)

Analiz edilen mumdan daha uzun periyot. Örn: 15 dk analiz yapılırken 1 saatlik mum.

```
HTF eşlemesi:
  1m  → 15m     5m → 1h     15m → 4h
  30m → 4h      1h → 1d     4h → 1w
```

**Amaç:** Ana trendin yönünü doğrular, LTF sinyallerini filtreler.
**Kullanım zamanı:** Sinyal onayı aşamasında (4. aşama).
**Etkisi:** HTF veri yoksa pozisyon boyutu × 0.5 olarak küçültülür.

---

## 2. Trend İndikatörleri

### SMA — Simple Moving Average (Basit Hareketli Ortalama)

```
SMA(n) = (C₁ + C₂ + ... + Cₙ) / n
```

**Amaç:** Gürültüyü filtreler, orta vadeli trend yönünü verir.
**Alternatifleri:** EMA (daha duyarlı), WMA (ağırlıklı), HMA (hull).
**Kullanım zamanı:** Bollinger Bands hesabında orta bant olarak; MA Crossover stratejisinde yavaş çizgi olarak.
**Zayıflığı:** Gecikmeli (lagging). Ani dönüşleri geç yakalar.

---

### EMA — Exponential Moving Average (Üstel Hareketli Ortalama)

```
k   = 2 / (n + 1)
EMA₀ = SMA(ilk n bar)          ← seed
EMAᵢ = Closeᵢ × k + EMAᵢ₋₁ × (1 - k)
```

**Amaç:** Yakın geçmişe daha fazla ağırlık verir. Trendleri SMA'dan daha hızlı yakalar.
**Alternatifleri:** SMA (daha stabil), DEMA/TEMA (daha az gecikme), Kijun-Sen (Ichimoku).
**Kullanım zamanı:** MACD'nin fast/slow çizgisi; MA Crossover stratejisinde; HTF trend filtresi.
**Dikkat:** Seed değeri SMA ile başlatılmalı; ilk bardan başlatılırsa ilk yüzlerce barda hatalı değer üretir.

---

### MACD — Moving Average Convergence Divergence

```
MACD Line  = EMA(12) - EMA(26)
Signal Line = EMA(9) of MACD Line   ← seed: SMA(ilk 9 MACD değeri)
Histogram  = MACD Line - Signal Line
```

**Amaç:** Trend ivmesini ve yön dönüşlerini tespit eder.
**Sinyal kuralları:**
- MACD > Signal → BUY
- MACD < Signal → SELL
- Histogram sıfır geçişi → güçlü sinyal

**Alternatifleri:** PPO (fiyattan bağımsız yüzde versiyon), TRIX.
**Kullanım zamanı:** Trend grubunda; 1h+ zaman dilimlerinde daha güvenilir.
**Dikkat:** Signal line'ın seed değeri 0'dan değil, ilk N MACD değerinin SMA'sından başlamalıdır.

---

### ADX — Average Directional Index

```
+DM = max(Highᵢ - Highᵢ₋₁, 0)  (yalnızca pozitifse)
-DM = max(Lowᵢ₋₁ - Lowᵢ, 0)   (yalnızca pozitifse)

+DI = 100 × RMA(+DM, 14) / ATR(14)
-DI = 100 × RMA(-DM, 14) / ATR(14)

DX  = 100 × |+DI - -DI| / (+DI + -DI)
ADX = RMA(DX, 14)
```

**Amaç:** Trendin **gücünü** ölçer (yönünü değil).
- ADX < 20 → yatay piyasa
- ADX 20-40 → orta güçte trend
- ADX > 40 → güçlü trend

**Alternatifleri:** Aroon Oscillator, VHF (Vertical Horizontal Filter).
**Kullanım zamanı:** Trend filtresi olarak; düşük ADX'te trend stratejileri kapatılabilir.
**Dikkat:** Sistemdeki mevcut ADX hesabı RMA yerine SMA kullanıyor (daha az hassas).

---

### Supertrend

```
HL2        = (High + Low) / 2
UpperBand  = HL2 + multiplier × ATR
LowerBand  = HL2 - multiplier × ATR

Trend UP   ← fiyat UpperBand üzerindeyken (LowerBand destek olur)
Trend DOWN ← fiyat LowerBand altındayken (UpperBand direnç olur)
```

**Amaç:** Dinamik destek/direnç ile trend yönünü tek çizgide gösterir.
**Alternatifleri:** Parabolic SAR, Chandelier Exit, MA-based systems.
**Kullanım zamanı:** Trend onay filtresi; ATR ile birlikte SL hesabında.

---

### Donchian Channel

```
Upper = max(High, n bar)
Lower = min(Low, n bar)
Mid   = (Upper + Lower) / 2
```

**Amaç:** Breakout tespiti. Fiyat kanalı dışına çıkarsa yeni trend başladığına işaret eder.
**Alternatifleri:** Bollinger Bands (volatilite tabanlı), Keltner Channel.
**Kullanım zamanı:** Breakout stratejisinde; turtle trading sistemlerinde (20/55 bar).

---

## 3. Momentum İndikatörleri

### RSI — Relative Strength Index

```
Wilder SMMA (ilk periyot):
  AvgGain₀ = Σ(pozitif farklar, 1..n) / n
  AvgLoss₀ = Σ(|negatif farklar|, 1..n) / n

Wilder SMMA (devam):
  AvgGain  = (AvgGain_prev × (n-1) + currentGain) / n
  AvgLoss  = (AvgLoss_prev × (n-1) + currentLoss) / n

RS  = AvgGain / AvgLoss
RSI = 100 - (100 / (1 + RS))
```

**Amaç:** Aşırı alım (>70) / aşırı satım (<30) bölgelerini tespit eder.
**Alternatifleri:** Stochastic RSI (RSI'nin RSI'si), CCI, Williams %R.
**Kullanım zamanı:** Momentum grubunda; 5m-30m zaman dilimlerinde etkili.
**Dikkat:** Basit ortalama yerine Wilder SMMA kullanılmalı. Basit ortalama yaklaşık %10 hata üretir.

---

### Stochastic Oscillator (%K)

```
%K = 100 × (Close - Lowest Low(n)) / (Highest High(n) - Lowest Low(n))
%D = SMA(%K, 3)    ← sinyal çizgisi
```

**Amaç:** Fiyatın n-bar aralığı içindeki göreceli konumunu verir.
- %K > 80 → aşırı alım
- %K < 20 → aşırı satım

**Alternatifleri:** RSI (daha yaygın), Williams %R (ters skala).
**Kullanım zamanı:** RSI ile birlikte momentum onayı için; %K ve %D kesişmeleri sinyal üretir.

---

### Stochastic RSI

```
RSI_min = min(RSI, n)
RSI_max = max(RSI, n)
StochRSI = (RSI - RSI_min) / (RSI_max - RSI_min)
```

**Amaç:** RSI'nin kendisini momentum göstergesi olarak değerlendirir. RSI'den daha duyarlı.
**Alternatifleri:** RSI, Stochastic Oscillator.
**Kullanım zamanı:** Aşırı bölgelerin erken tespitinde; kısa vadeli scalping'de.

---

### CCI — Commodity Channel Index

```
TP    = (High + Low + Close) / 3
SMA_TP = SMA(TP, n)
MAD   = mean(|TP - SMA_TP|)
CCI   = (TP - SMA_TP) / (0.015 × MAD)
```

**Amaç:** Fiyatın ortalamadan standart sapmasını ölçer.
- CCI > +100 → güçlü yükseliş momentum
- CCI < -100 → güçlü düşüş momentum

**Alternatifleri:** RSI, Stochastic.
**Kullanım zamanı:** Trend takibinde ek momentum onayı olarak.

---

### Williams %R

```
%R = -100 × (Highest High(n) - Close) / (Highest High(n) - Lowest Low(n))
```

**Amaç:** Stochastic'in ters versiyonu.
- %R > -20 → aşırı alım
- %R < -80 → aşırı satım

**Alternatifleri:** Stochastic %K.
**Kullanım zamanı:** RSI/Stochastic ile beraber çift onay için.

---

## 4. Volatilite İndikatörleri

### ATR — Average True Range

```
ATR(n) = SMA(TR₁, TR₂, ..., TRₙ)     ← basit versiyon
       = RMA(TR, n)                    ← Wilder versiyonu (daha yaygın)
```

**Amaç:** Piyasanın ortalama hareketini ölçer. Stop-loss mesafesi için temel referans.
**Kullanım alanları:**
1. SL mesafesi: `SL = entry ± k × ATR`  (k genellikle 1.5-3.0)
2. Pozisyon boyutu: risk/ATR
3. Supertrend band genişliği
4. Kaldıraç hesabı (yüksek ATR → düşük kaldıraç)

**Alternatifleri:** Bollinger bandwidth, fiyat yüzdesi olarak sabit SL.
**Kullanım zamanı:** Sinyal üretildikten sonra SL/TP hesabında; pozisyon boyutlandırmada.

---

### Bollinger Bands

```
Middle = SMA(Close, n)
σ      = std_dev(Close, n)
Upper  = Middle + k × σ     (k genellikle 2.0)
Lower  = Middle - k × σ
```

**Amaç:** Fiyatın standart sapma merkezli bandını çizer.
- Fiyat Upper band → olası aşırı alım / breakout
- Fiyat Lower band → olası aşırı satım / breakout
- Band daralması (squeeze) → yaklaşan büyük hareket

**Alternatifleri:** Keltner Channel (ATR tabanlı, daha sakin), Donchian Channel.
**Kullanım zamanı:** Range piyasalarda ortalamaya dönüş stratejisinde; squeeze breakout'ta.

---

## 5. Hacim / Volume İndikatörleri

### VWAP — Volume Weighted Average Price

```
TP_t   = (High_t + Low_t + Close_t) / 3
VWAP_t = Σ(TP × Volume) / Σ(Volume)   ← gün başından itibaren kümülatif
```

**Amaç:** Kurumsal alıcı/satıcıların gün içi referans fiyatı.
- Fiyat > VWAP → alıcılar baskın
- Fiyat < VWAP → satıcılar baskın

**Alternatifleri:** OBV (On-Balance Volume), TWAP (zaman ağırlıklı).
**Kullanım zamanı:** Gün içi sinyal onayı filtresi olarak; entry kalitesini artırır.

---

### Funding Rate (Finansman Oranı)

Perpetual futures'da long/short pozisyon dengesizliğini dengeleyen periyodik ödeme.

```
Funding Rate > 0 → Long'lar short'lara ödeme yapar (piyasa aşırı bull)
Funding Rate < 0 → Short'lar long'lara ödeme yapar (piyasa aşırı bear)
```

**FundingRateContrarian stratejisi:**
- Yüksek pozitif funding → kontre sat (kısa)
- Yüksek negatif funding → kontre al (uzun)

**Kullanım zamanı:** Kripto perpetual piyasalarda; aşırı kalabalık pozisyonların tersine dönüşünü yakalamak için.

---

## 6. Seviye İndikatörleri

### Destek / Direnç (S/R — Support / Resistance)

**Destek:** Fiyatın düşüşünü durduran alt seviye.
**Direnç:** Fiyatın yükselişini durduran üst seviye.

#### Swing High / Swing Low Tespiti

```
Swing High: candles[i].high > candles[i-1].high  ve
            candles[i].high > candles[i+1].high

Swing Low:  candles[i].low < candles[i-1].low  ve
            candles[i].low < candles[i+1].low
```

**SrZone yapısı:**
```
SrZone {
  price_low:  f64,    ← zone alt sınırı
  price_high: f64,    ← zone üst sınırı
  midpoint:   f64,    ← (price_low + price_high) / 2
  strength:   f64,    ← test sayısı × hacim faktörü
  zone_type:  Support | Resistance | Both
}
```

**Alternatifleri:** Fibonacci retracement, pivot points, VWAP bands.
**Kullanım zamanı:** Entry kalitesi hesabında; SL fiyatı belirlemede (destek altına koy).

---

### Fibonacci Seviyeleri

```
Swing hareketi = High - Low

Retracement seviyeleri: 23.6%, 38.2%, 50.0%, 61.8%, 78.6%
Extension seviyeleri:  100%, 127.2%, 161.8%, 261.8%
```

**Amaç:** Düzeltme hareketlerinde potansiyel destek/direnç bölgelerini öngörür.
**Kullanım zamanı:** HTF analizi tamamlandıktan sonra entry noktası hassasiyeti için.

---

### Pivot Points

```
PP   = (High + Low + Close) / 3
R1   = 2×PP - Low           S1 = 2×PP - High
R2   = PP + (High - Low)    S2 = PP - (High - Low)
R3   = High + 2×(PP - Low)  S3 = Low - 2×(High - PP)
```

**Kullanım zamanı:** Günlük açılışta seviye referansı olarak.

---

## 7. Sinyal Üretimi ve Konsensüs

### Strateji Sıralaması (rank_strategies_for_interval)

Her strateji için **composite skor** hesaplanır:

```
Composite Score = w₁×win_rate + w₂×profit_factor + w₃×sharpe_ratio
```

Zaman dilimine göre strateji grubu ön planda tutulur:
- 1m-5m → Momentum (RSI, Stochastic, PriceAction)
- 15m-30m → Karma
- 1h-4h → Trend (MACD, EMA, Supertrend)
- 4h+ → Yapısal (ICT_FVG, SMC)

**Top-5 strateji seçilir**, ağırlıklı oylama yapılır.

---

### Yüzde Tabanlı Konsensüs Oylama

```
BUY_weight  = Σ(composite_score, BUY sinyali veren stratejiler)
SELL_weight = Σ(composite_score, SELL sinyali veren stratejiler)
HOLD_weight = kalan

BUY%  = BUY_weight  / total_weight
SELL% = SELL_weight / total_weight
```

**Eşik (Adaptif, §15.9):**

| Rejim | Eşik |
|-------|------|
| StrongUptrend / StrongDowntrend | 0.45 (gevşek) |
| Normal / Unknown | 0.50 |
| Ranging | 0.60 (katı) |
| HighVolatility | 0.65 (en katı) |

**Karar:** `BUY% >= eşik` → BUY; `SELL% >= eşik` → SELL; aksi → HOLD.

**Neden adaptif?** Yatay ve yüksek volatilite dönemlerinde yanlış sinyal riski artar; daha yüksek eşik bu riski azaltır.

---

### Parametre Grid Search (HyperOptimizer)

Her ana strateji için önceden tanımlanmış parametre seti üzerinde en iyi kombinasyon aranır.

```
score = simulate_score_htf(strategy, candles, params, htf_candles)
```

Mevcut konfigürasyon parametresine +0.10 bonus verilir (kararlılık için).

---

## 8. Pozisyon Boyutlandırma

### Kelly Kriteri (Half-Kelly)

```
f* = (b×p - q) / b

b = win/loss oranı (kazanılan ortalama / kaybedilen ortalama)
p = kazanma olasılığı
q = 1 - p

Uygulamalı Half-Kelly:
f_applied = f* × 0.5
```

**Amaç:** Teorik olarak maksimum logaritmik büyümeyi sağlar.
**Neden Half-Kelly?** Tam Kelly son derece agresif olup drawdown'a karşı hassastır. Yarı Kelly, büyüme ve güvenlik dengesi sunar.
**Alternatifleri:** Sabit yüzde (1-2%), ATR-bazlı risk, optimal F.
**Kullanım zamanı:** `use_kelly_criterion = true` olduğunda; yeterli geçmiş veri (>50 trade) varsa güvenilir.

---

### Klasik Risk Bazlı Pozisyon Boyutu

```
max_risk = capital × (max_portfolio_risk_pct / 100)
qty      = max_risk / entry_price
```

**Kullanım zamanı:** Kelly aktif değilse veya kazanma istatistikleri yetersizse.

---

### Dinamik Kaldıraç Hesabı

```
base_leverage  = konfigürasyon değeri

# ATR düzeltmesi: yüksek volatilite → düşük kaldıraç
atr_pct        = ATR / Close × 100
atr_factor     = 1.0 - (atr_pct - 1.0) × 0.3    [1.0..2.0 aralığında]

# Drawdown düzeltmesi: max_dd'nin yarısını geçince azalt
dd_pct         = (peak - current) / peak × 100
dd_factor      = 1.0 - (dd_pct / (max_dd/2)) × 0.5

# Kayıp serisi düzeltmesi: ardarda kayıp artınca düşür
streak_factor  = 1.0 - loss_streak × 0.1

# Risk/Ödül düzeltmesi: iyi RR → hafif artış
rr_factor      = 1.0 + (session_rr - 2.0) × 0.1

effective_lev  = base_leverage × atr_factor × dd_factor × streak_factor × rr_factor
               ← [1.0, max_leverage] aralığına sıkıştırılır
```

**Kaldıraç güvenlik klampı:**
```
max_sl_pct = 80.0 / effective_lev
```
Örn: 10x kaldıraç → maks %8 SL; tasfiye riski minimize edilir.

---

### Kademeli Drawdown Koruması (§15.8)

```
current_dd = (peak_equity - current_equity) / peak_equity × 100

dd > 20% → Yeni pozisyon ENGELLENDI (return)
dd > 15% → Pozisyon boyutu × 0.5
dd ≤ 15% → Normal boyut
```

**Amaç:** Büyük kayıp dönemlerinde kademeli olarak risk azaltılır.
**Alternatifleri:** Sabit maksimum drawdown eşiği (tek basamaklı), circuit breaker.

---

### HTF Veri Yoksa Boyut Küçültme (§15.12)

```
htf_candles yoksa → base_qty × 0.5
htf_candles varsa  → base_qty (normal)
```

**Amaç:** HTF teyidi olmadan açılan pozisyon daha küçük tutulur; belirsizlik riski azaltılır.

---

## 9. Stop-Loss / Take-Profit Mekanizmaları

### Statik SL/TP

```
SL (long) = entry × (1 - stop_loss_pct / 100)
TP (long) = entry × (1 + take_profit_pct / 100)
```

**Kullanım zamanı:** Parametre olarak tanımlanmış sabit yüzde mevcut olduğunda.

---

### ATR Tabanlı Dinamik SL

```
SL (long) = entry - atr_multiplier × ATR
TP (long) = entry + rr_ratio × atr_multiplier × ATR
```

**Amaç:** Volatiliteye orantılı SL mesafesi. Yüksek ATR → geniş SL.
**Alternatifleri:** Statik yüzde SL, destek altına SL.
**Kullanım zamanı:** Sinyal onaylandıktan sonra, kaldıraç hesabından önce.

---

### Destek/Direnç Bazlı SL

```
SL (long) = nearest_support_zone.price_low × (1 - buffer_pct)
```

**Amaç:** Yapısal bir seviyenin hemen altında SL koyar; "anlamsız" tetiklenme riskini azaltır.
**Kullanım zamanı:** S/R zonu tespit edildiyse; ATR SL'ye göre daha anlamlı bir level sağlar.

---

### Trailing Stop-Loss (Takipli SL)

```
Long pozisyon için:
  Fiyat yukarı her hareket ettiğinde:
    trailing_sl = max(trailing_sl, fiyat - trailing_distance)

  Fiyat trailing_sl'ye gelirse: kapat
```

**Amaç:** Kar kilitleme. Fiyat yukarı gittikçe SL de yukarı taşınır.
**Alternatifleri:** Zamanlı TP (belirli süre sonra kapat), Chandelier Exit.
**Kullanım zamanı:** Pozisyon açıldıktan sonra, her mum kapanışında güncellenir.

---

### SL Güvenlik Klampı

```
max_sl_pct = 80.0 / effective_lev

Hesaplanan SL bu sınırı aşıyorsa → max_sl_pct uygulanır
```

**Amaç:** Kaldıraçlı pozisyonda tasfiye öncesi çıkışı garantiler.

---

## 10. Risk Yönetimi

### DrawdownMonitor

```
current_dd = (peak_equity - current_equity) / peak_equity × 100

Durum:
  Safe          → normal işlem devam
  LimitExceeded → stop_loop = true, tüm işlem durur
```

**Kullanım:** Her pozisyon kapanışında güncellenir.

---

### RiskGate (FSM tabanlı)

Risk kuralları hiyerarşisi:

| Kural | Koşul | Eylem |
|-------|-------|-------|
| DailyLossLimit | Günlük zarar > eşik | DENY + safe_mode |
| DrawdownLimit | DD > max_dd | DENY + halt |
| PositionSizeLimit | Notional > max_notional | DENY |
| ConfidenceFilter | model_confidence < min | DENY |

**Kullanım zamanı:** Pozisyon açılmadan hemen önce.

---

### CircuitBreaker

Ardarda kayıp veya hata durumunda işlemi geçici olarak kesen mekanizma.

```
Tetikleyiciler:
  - Ardarda kayıp sayısı > threshold
  - API hataları
  - Slippage > max_slippage

Durumlar: Closed → HalfOpen → Open (→ Closed: cooldown sonrası)
```

**Alternatifleri:** Basit loss_streak kontrolü, manuel müdahale.

---

### SL Cooldown

```
SL ile kapanan pozisyon sonrası:
  sl_cooldown_map[symbol] = now()

  Yeni sinyal gelirse:
    if elapsed < sl_cooldown_secs → sinyali atla (cooldown)
```

**Amaç:** Aynı sembolde "ard arda SL yeme" döngüsünü kırar.

---

### Risk/Ödül Oranı (RR — Risk/Reward Ratio)

```
RR = (TP fiyatı - Entry) / (Entry - SL fiyatı)

Minimum tavsiye: RR >= 2.0
```

**Yorumu:** RR=2 demek 1 risk için 2 ödül beklemek demektir. Uzun vadede kazanma yüzdesi %50'nin altında olsa bile kârlı olunabilir.

---

### Şeridi (Loss Streak) Takibi

```
Ardarda kayıp sayısı = loss_streak

streak_factor = 1.0 - loss_streak × 0.1
```

Örn: 3 ardarda kayıp → kaldıraç %30 küçülür.

---

## 11. Backtest ve Performans Metrikleri

### Sharpe Ratio

```
Sharpe = (PortfolioReturn - RiskFreeRate) / σ(returns)
```

**Amaç:** Birim risk başına düşen fazla getiriyi ölçer.
- Sharpe > 1.0 → iyi
- Sharpe > 2.0 → mükemmel
- Sharpe < 0 → risksiz varlıktan kötü

**Alternatifleri:** Sortino Ratio (yalnızca negatif volatiliteyi cezalandırır), Calmar Ratio.

---

### Sortino Ratio

```
Sortino = (PortfolioReturn - RiskFreeRate) / σ(negatif_returns)
```

**Amaç:** Sharpe'dan farkı: yalnızca olumsuz oynaklığı cezalandırır. Yüksek ödül/düşük zarar stratejilerini daha iyi değerlendirir.

---

### Max Drawdown (Maksimum Düşüş)

```
MaxDD = max( (peak - trough) / peak ) × 100%
```

**Amaç:** Stratejinin yaşadığı en büyük tepe-dip kayıp yüzdesi.
**Kullanım zamanı:** Backtest sonucunda; canlı trading'de DrawdownMonitor ile izlenir.

---

### Profit Factor

```
Profit Factor = Toplam Kâr / Toplam Zarar (mutlak)
```

- PF > 1.5 → iyi
- PF > 2.0 → mükemmel
- PF < 1.0 → sistem zararda

---

### Win Rate

```
Win Rate = Kazanan Trade Sayısı / Toplam Trade Sayısı × 100%
```

**Tek başına yeterli değildir.** Win Rate %40 bile olsa RR=3 ile kârlı olunabilir.

---

### CAGR — Compound Annual Growth Rate

```
CAGR = (Son Değer / İlk Değer)^(1/yıl) - 1
```

**Amaç:** Yıllık bileşik getiri. Farklı sürelerdeki backtestleri karşılaştırmak için.

---

### Calmar Ratio

```
Calmar = CAGR / MaxDrawdown
```

Yüksek Calmar → düşük drawdownla yüksek büyüme.

---

## 12. Makine Öğrenmesi Terimleri

### Z-Score Anomali Tespiti

```
baseline = son bar hariç tüm veri
μ        = mean(baseline)
σ        = std_dev(baseline)

z = (son_değer - μ) / σ

|z| > 3.0 → anomali (spike tespit edildi)
```

**Amaç:** Fiyat veya indikatör serisinde olağandışı hareket tespiti.
**Dikkat:** Son bar baseline'a dahil edilmemelidir (spike kendisini dilüe eder).
**Alternatifleri:** IQR yöntemi, LSTM tabanlı anomali, Isolation Forest.
**Kullanım zamanı:** Sinyal filtreleme aşamasında; ani spike'larda işlem engellenir.

---

### Feature Extractor (Özellik Çıkarıcı)

Ham OHLCV veriden ML girdisi için normalize edilmiş özellik vektörü türetir:

```
Özellikler:
  - returns        (fiyat değişim yüzdeleri)
  - volume_ratio   (son hacim / ortalama hacim)
  - rsi_norm       (RSI / 100)
  - bb_position    ((close - lower) / (upper - lower))
  - atr_norm       (ATR / close)
```

**Kullanım zamanı:** ML sinyal üretiminden önce (USE_ML_SIGNAL=true).

---

### Linear Regression Sinyal (ML Engine)

```
Özellik vektörü → [w₁, w₂, ..., wₙ] ağırlıkları ile doğrusal kombinasyon
score = Σ(wᵢ × featureᵢ) + bias

score > 0 → BUY
score < 0 → SELL
```

**Kullanım zamanı:** Diğer strateji sinyalleriyle birleştirilir; AUTONOMOUS_ENABLED=true ile aktif.

---

## 13. Evrimsel/Adaptif Bileşenler

### Q-Learning (AdaptiveBrain)

Takviyeli öğrenme (Reinforcement Learning) tabanlı strateji seçimi.

```
Q(s, a) ← Q(s, a) + α × [reward - Q(s, a)]

s (state)  = piyasa rejimi (StrongUptrend, Ranging, HighVolatility, ...)
a (action) = seçilen strateji (MA, RSI, MACD, ...)
α          = öğrenme hızı (0.10)
reward     = PnL% / 10    (normalleştirilmiş)
```

**Exploration vs Exploitation:**
```
ε-greedy politikası:
  P(ε) → rastgele strateji seç (keşif)
  P(1-ε) → Q tablosundan en iyi stratejiyi seç (sömürü)

ε başlangıç = 0.20 → her 100 adımda %1 azalır → minimum 0.05
```

**Alternatifleri:** UCB (Upper Confidence Bound), Bayesian optimization, Thompson sampling.

---

### Piyasa Rejimi Tespiti (MarketRegime)

```
son 20 bar kapanışına bakılır:
  trend_pct     = (last - first) / first × 100
  volatility_pct = std_dev(closes) / mean(closes) × 100

Sıra önemlidir (önce trend kontrol edilir):
  trend_pct > 5%   → StrongUptrend
  trend_pct > 2%   → WeakUptrend
  trend_pct < -5%  → StrongDowntrend
  trend_pct < -2%  → WeakDowntrend
  volatility > 3%  → HighVolatility
  volatility < 0.5%→ LowVolatility
  aksi             → Ranging
```

**Kullanım zamanı:** Her trading döngüsü başında; konsensüs eşiğini ve strateji seçimini etkiler.

---

### Genetik Algoritma (PopulationManager)

Strateji parametrelerini evrimsel yöntemle optimize eder.

```
Genome = {fast, slow, period, std_dev, overbought, oversold, ...}

Fitness(0-150 puan):
  +0-50   → Win Rate skoru
  +0-40   → Profit Factor skoru
  +0-30   → Sharpe Ratio skoru
  +0-15   → CAGR skoru
  +0-15   → Max Drawdown skoru (düşük DD = yüksek puan)
  Ceza:   → Trade sayısı < 10 ise -20 puan

Nesil işlemleri:
  Seçilim (Selection)  → En fit %50
  Çaprazlama (Crossover) → İki parent'tan yeni genome
  Mutasyon (Mutation)  → Küçük rastgele değişim
```

---

## 14. Piyasa Yapısı Terimleri

### Trend Bias (Trend Eğilimi)

```
Bullish: Short MA > Long MA (fiyat artıyor)
Bearish: Short MA < Long MA (fiyat düşüyor)
Neutral: |kısa - uzun| / uzun < margin_pct
```

**Kullanım:** HTF trend filtresi; LTF sinyali HTF yönüne karşıysa engellenir.

---

### Order Book Depth (Emir Defteri Derinliği)

```
Bid side depth: toplam alış emirleri (USD notional)
Ask side depth: toplam satış emirleri (USD notional)

Likidite oranı = min(bid_depth, ask_depth) / max(bid_depth, ask_depth)
```

**Kullanım:** Slippage riskini değerlendirmek için; yetersiz derinlikte işlem engellenir.

---

### Slippage (Kayma)

```
Slippage% = |executed_price - expected_price| / expected_price × 100
```

**Amaç:** Emirlerin beklenen fiyattan sapma yüzdesi. Likit olmayan piyasalarda artar.
**Kullanım zamanı:** Her emir sonrasında kontrol edilir; eşik aşılırsa cooldown başlar.

---

### Spread

```
Spread = Ask - Bid
Spread% = Spread / Mid × 100
```

**Kullanım:** Yüksek spread → maliyet artışı; belirlenen eşiği aşarsa işlem engellenir.

---

### Pozisyon Tipleri

| Terim | Açıklama |
|-------|----------|
| **Long** | Fiyat artacak beklentisiyle alım |
| **Short** | Fiyat düşecek beklentisiyle açığa satış |
| **Spot** | Anlık alım/satım; kaldıraçsız |
| **Futures/Perpetual** | Sözleşme bazlı; kaldıraç kullanılabilir |

---

### Paper Trading

Gerçek para kullanmadan simüle edilmiş işlem.
`BINANCE_PAPER_MODE=true` → tüm emirler sisteme kayıt edilir, Binance API'ye iletilmez.

---

## 15. Sistem Mimarisi Terimleri

### Data Pipeline

```
DataSource (fetch) → Normalizer → Cache → Candle[]
```

Desteklenen kaynaklar: Binance, Yahoo Finance (BIST), Bybit, KuCoin, Coinbase.
Cache: periyot bazlı TTL; aynı sembol için tekrar fetch engellenir.

---

### FSM — Finite State Machine (Sonlu Durum Makinesi)

**AutonomousController durumları:**

```
Idle → Active → Paused → Stopped
          ↓
       Evolution (genetik algoritma döngüsü)
```

Her geçiş koşulları ve tetikleyici eventleri tanımlıdır.

---

### Hot Reload (Sıcak Yeniden Yükleme)

Sistemi durdurmadan strateji parametrelerini güncelleme.

```
Akış:
  1. Yeni konfigürasyon yüklenir
  2. Aktif pozisyonlar korunur
  3. Yeni cycle yeni parametrelerle başlar
```

**Kullanım zamanı:** Canlı trading sırasında backtest sonuçlarına göre parametre güncellemesi.

---

### Snapshot (Anlık Görüntü)

Trading durumunun diske kaydı. Restart sonrası kurtarma için.

```
Kaydedilen:
  - Açık pozisyonlar
  - AdaptiveBrain Q-table
  - PopulationManager genomları
  - Equity ve drawdown bilgisi
```

---

### Signal (Sinyal) Hiyerarşisi

```
Sıralama (her aşama bir sonrakini onaylamalı):

  1. İndikatör sinyali     → strateji mantığı (Buy/Sell/Hold)
  2. Konsensüs oylama      → çoklu strateji onayı (>= eşik)
  3. Filtre katmanı        → RSI aşırı bölge, ATR filtresi, spread, slippage
  4. HTF filtresi          → üst timeframe trend onayı
  5. S/R kalite puanı      → destek/direnç bölgesine yakınlık
  6. Risk gate             → drawdown, günlük kayıp, pozisyon büyüklüğü
  7. Emir iletimi          → gerçek veya paper execution
```

---

### İşlem Kalitesi Skoru (Trade Quality)

```
Quality = f(dist_support, dist_resistance, volume_ratio, atr_condition)

0.0 - 0.3 → düşük kalite (işlem engellenir / boyut küçültülür)
0.3 - 0.6 → orta kalite
0.6 - 1.0 → yüksek kalite (tam boyut ile işlem)
```

---

## Hızlı Referans: Kullanım Sırası

```
[Veri Alımı]
  OHLCV fetch → normaliz → cache

[İndikatör Hesabı]
  SMA → EMA → RSI (Wilder) → MACD (SMA seed) → BB → ATR → ADX
  Stochastic → Williams %R → CCI → VWAP → Supertrend

[Strateji & Konsensüs]
  rank_strategies → top-5 → ağırlıklı oylama → adaptif eşik

[Filtreler]
  RSI aşırı bölge → ATR min → spread → slippage → HTF bias → S/R kalite

[Boyutlandırma]
  Kelly/Klasik → Half-Kelly → DD kademeli → HTF yok → 0.5× → kaldıraç

[SL/TP]
  ATR tabanlı → S/R bazlı → klamp → trailing SL

[Risk Gate]
  DrawdownMonitor → RiskGate → CircuitBreaker → SL Cooldown

[Execution]
  Paper veya canlı → pozisyon kayıt → PnL hesap → AdaptiveBrain öğren
```

---

*Bu el kitabı `docs/trading_logic_reference.md` dokümanının teknik terminoloji özetidir.*
*Son güncelleme: 2026-03-24*
