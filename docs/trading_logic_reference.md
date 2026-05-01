# Memos Trading — Trading Mantığı Referans Dökümanı

**Sürüm:** 1.0 | **Tarih:** 2026-03-24 | **Kapsam:** memos_trading_core

---

## İçindekiler

1. [Sistem Mimarisi Genel Bakış](#1-sistem-mimarisi-genel-bakış)
2. [Veri Akışı — Candle Pipeline](#2-veri-akışı--candle-pipeline)
3. [Teknik Göstergeler ve Formüller](#3-teknik-göstergeler-ve-formüller)
4. [Strateji Katmanı](#4-strateji-katmanı)
5. [Sinyal Seçimi — Dinamik Sıralama ve Konsensüs Oylama](#5-sinyal-seçimi--dinamik-sıralama-ve-konsensüs-oylama)
6. [Sinyal Filtre Zinciri](#6-sinyal-filtre-zinciri)
7. [Pozisyon Boyutu ve Kaldıraç](#7-pozisyon-boyutu-ve-kaldıraç)
8. [Stop-Loss ve Take-Profit Mekanizmaları](#8-stop-loss-ve-take-profit-mekanizmaları)
9. [PnL Hesabı](#9-pnl-hesabı)
10. [Risk Kapıları ve Otonom Kontrol](#10-risk-kapıları-ve-otonom-kontrol)
11. [Backtest Motoru ve Performans Metrikleri](#11-backtest-motoru-ve-performans-metrikleri)
12. [Evrimsel Optimizasyon (Genetik Algoritma)](#12-evrimsel-optimizasyon-genetik-algoritma)
13. [ML Katmanı — Anomali Tespiti](#13-ml-katmanı--anomali-tespiti)
14. [Destek/Direnç (S/R) Dedektörü](#14-destek--direnç-sr-dedektörü)
15. [Tespit Edilen Sorunlar ve Geliştirme Tavsiyeleri](#15-tespit-edilen-sorunlar-ve-geliştirme-tavsiyeleri)

---

## 1. Sistem Mimarisi Genel Bakış

```
┌────────────────────────────────────────────────────────────────┐
│                        rtc_cli (TUI)                           │
│  Tab0:Dashboard  Tab1:Evo  Tab2:ML  Tab3:Pozisyon  ...         │
└──────────────────────────┬─────────────────────────────────────┘
                           │  Arc<Mutex<AppState>>
          ┌────────────────┼──────────────────────┐
          ▼                ▼                      ▼
   WorkerOrchestrator  RoboticLoop         AutonomousController
   (per-symbol WS)     (candle tick)       (FSM: Active/SafeMode/Halt)
          │                │                      │
          │         ┌──────┴──────┐               │
          │         ▼             ▼               ▼
          │    StrategyRanker  Filters       AdaptiveBrain
          │    (composite)     (7 katman)    (Q-Learning)
          │         │             │               │
          │         └──────┬──────┘               │
          │                ▼                      ▼
          │         Consensus Vote          PopulationManager
          │         (BUY/SELL/HOLD)         (Genetik Evrim)
          │                │
          ▼                ▼
    Binance WS      TradeExecutor
    (fiyat güncelle) (paper/live)
                           │
                    SQLite DB (candles,
                    patterns, evolution)
```

**Ana akış özeti:**
Her interval sonunda `process_symbol()` çağrılır. Bu fonksiyon; veri çeker, strateji puanlar, oy toplar, filtreden geçirir, kaldıraç hesaplar ve emri çalıştırır. WS fiyatları her saniye SL/TP kontrolü için bağımsız olarak işlenir.

---

## 2. Veri Akışı — Candle Pipeline

### 2.1 Candle Çekme

**Kaynak:** `RoboticLoop::process_symbol()` → `fetcher.fetch_latest()`

```
Binance REST API
  └─ /api/v3/klines (spot)
  └─ /fapi/v1/klines (futures)
  └─ /dapi/v1/klines (coinm)
```

**Tazelik Kontrolü:**
```
max_lag = interval_secs × 3
if (now - last_candle.timestamp) > max_lag → log uyarı
```

### 2.2 HTF (Üst Zaman Dilimi) Eşleşmesi

Mevcut interval'den bir üst zaman dilimine otomatik eşleme:

| LTF (Ana) | HTF (Filtre) |
|-----------|-------------|
| 1m, 5m    | 1h          |
| 15m, 30m  | 4h          |
| 1h        | 4h          |
| 4h, 1d    | 1d          |

HTF candle'ları SQLite DB'den yüklenir (`load_htf_candles_from_db`, son 200 bar).

### 2.3 Veri Normalizasyonu

```
Candle { timestamp, open, high, low, close, volume, symbol }
```

Tüm göstergeler `close` fiyatı üzerinden çalışır; ATR, Stochastic, ADX için `high`/`low` da kullanılır.

---

## 3. Teknik Göstergeler ve Formüller

### 3.1 SMA — Basit Hareketli Ortalama

```
SMA(n) = Σ(close[i], i=len-n..len) / n
```

**Kaynak:** `calculate_sma()` — `indicators.rs:7`

### 3.2 EMA — Üssel Hareketli Ortalama

```
k = 2 / (n + 1)
EMA₀ = SMA(n)  [seed: son n barın ortalaması]
EMAᵢ = closeᵢ × k + EMAᵢ₋₁ × (1 - k)
```

**Kaynak:** `calculate_ema()` — `indicators.rs:14`

> **SORUN:** Mevcut implementasyon seed ve iterasyonu aynı N-pencere üzerinde yapıyor (çift sayım). Düzgün EMA seed için ilk N barı kullanıp N+1'den devam etmelidir. Bkz. §15.1.

### 3.3 RSI — Göreceli Güç Endeksi

```
Gain = Σ(closeᵢ₊₁ - closeᵢ, eğer > 0) / n
Loss = Σ(closeᵢ - closeᵢ₊₁, eğer < 0) / n
RS   = Gain / Loss
RSI  = 100 - (100 / (1 + RS))
```

**Varsayılan periyot:** 14 | OB: 70 | OS: 30

**Kaynak:** `calculate_rsi()` — `indicators.rs:25`

> **SORUN:** Wilder'ın orijinal RSI smoothing'i (SMMA = EMA α=1/n) kullanılmıyor; basit ortalama kullanılıyor. Standart platform değerlerinden sapma oluşur. Bkz. §15.2.

### 3.4 MACD — Hareketli Ortalama Yakınsama/Iraksama

```
k_fast   = 2 / (fast + 1)    [varsayılan: fast=12]
k_slow   = 2 / (slow + 1)    [varsayılan: slow=26]
k_signal = 2 / (signal + 1)  [varsayılan: signal=9]

MACD_line    = EMA(fast) - EMA(slow)
Signal_line  = EMA(MACD_line, signal)
Histogram    = MACD_line - Signal_line
```

**Kaynak:** `calculate_macd()` — `indicators.rs:38`

> **SORUN:** Signal EMA macd_series[0] = 0 ile seed ediliyor; standart hesaplama ilk `signal` barın SMA'sını kullanır. Bkz. §15.3.

### 3.5 Bollinger Bands

```
SMA  = SMA(period)          [varsayılan: period=20]
σ    = StdDev(son period bar)
Upper = SMA + (std_dev × σ)  [varsayılan: std_dev=2.0]
Lower = SMA - (std_dev × σ)
```

**Kaynak:** `calculate_bollinger()` — `indicators.rs:62`

### 3.6 Stochastic Oscillator

```
%K = (close - min_low[n]) / (max_high[n] - min_low[n]) × 100
```

**Varsayılan:** k=6, OB=70, OS=20 (DB kanıtlı değerler)

**Kaynak:** `calculate_stochastic()` — `indicators.rs:73`

### 3.7 ATR — Ortalama Gerçek Aralık

```
TR = max(High - Low, |High - prev_close|, |Low - prev_close|)
ATR = Σ(TR[i], i=len-period..len) / period
```

**Kaynak:** `calculate_atr()` — `indicators.rs:82`

> **NOT:** Gerçek ATR Wilder's SMMA (EMA α=1/n) kullanır; burada basit ortalama kullanılıyor. Normalize edilmiş kullanımda (ATR%) fark az önemlidir.

### 3.8 ADX — Ortalama Yönlü Endeks

```
TR = max(H-L, |H-PC|, |L-PC|)
+DM = max(H - prev_H, 0) [eğer down_move'dan büyükse]
-DM = max(prev_L - L, 0) [eğer up_move'dan büyükse]

+DI = 100 × Σ(+DM) / Σ(TR)
-DI = 100 × Σ(-DM) / Σ(TR)
DX  = 100 × |+DI - (-DI)| / (+DI + (-DI))
ADX ≈ DX  [son periyod]
```

**Kaynak:** `calculate_adx()` — `indicators.rs:135`

> **SORUN:** ADX standarda göre DX serisinin Wilder SMMA'sıdır, burada sadece son periyodun DX değeri alınıyor. Trende geç tepki verir. Bkz. §15.4.

### 3.9 VWAP — Hacim Ağırlıklı Ortalama Fiyat

```
Typical_Price = (High + Low + Close) / 3
VWAP = Σ(Typical × Volume) / Σ(Volume)
```

**Kaynak:** `calculate_vwap()` — `indicators.rs:166`

### 3.10 Williams %R

```
%R = (HH[n] - close) / (HH[n] - LL[n]) × (-100)
```

Aralık: -100 (oversold) .. 0 (overbought).

### 3.11 Supertrend

```
HL2 = (High + Low) / 2
Upper_Band = HL2 + (multiplier × ATR)   [varsayılan: mult=3.0]
Lower_Band = HL2 - (multiplier × ATR)
```

Trend: fiyat Upper_Band üstündeyse +1 (bullish), altındaysa -1 (bearish).

### 3.12 Stochastic RSI

```
RSI_series = RSI hesapla (tüm barlar)
StochRSI = (RSI - min(RSI[k])) / (max(RSI[k]) - min(RSI[k]))
%K = StochRSI × 100
%D = SMA(%K, 3)  [sinyal çizgisi]
```

### 3.13 CCI — Emtia Kanal Endeksi

```
Typical = (H + L + C) / 3
Mean     = SMA(Typical, period)
MeanDev  = Σ|Typical - Mean| / period
CCI      = (Typical - Mean) / (0.015 × MeanDev)
```

---

## 4. Strateji Katmanı

### 4.1 Mevcut Stratejiler

| İsim        | Gösterge         | BUY sinyali koşulu                     | SELL sinyali koşulu                    |
|-------------|------------------|-----------------------------------------|-----------------------------------------|
| MA          | SMA fast/slow    | fast_sma > slow_sma (crossover)         | fast_sma < slow_sma                     |
| EMA         | EMA fast/slow    | fast_ema > slow_ema                     | fast_ema < slow_ema                     |
| RSI         | RSI(14)          | RSI < oversold (30) → alım              | RSI > overbought (70) → satım           |
| MACD        | MACD(12,26,9)    | macd_line > signal_line (crossover)     | macd_line < signal_line                 |
| BB          | BB(20, 2σ)       | close < lower_band                      | close > upper_band                      |
| DONCHIAN    | Donchian Ch(20)  | close > upper_channel (kırılım)         | close < lower_channel                   |
| SUPERTREND  | ATR(10, 3x)      | supertrend_dir = +1                     | supertrend_dir = -1                     |
| STOCH_RSI   | StochRSI(14)     | %K < 20 (oversold)                      | %K > 80 (overbought)                    |
| STOCHASTIC  | Stoch(6)         | %K < 20                                 | %K > 70                                 |
| CCI         | CCI(20)          | CCI < -100                              | CCI > +100                              |
| WILLIAMS    | %R(14)           | %R < -80 (oversold)                     | %R > -20 (overbought)                   |
| ADX         | ADX(14)          | ADX > 25 + +DI > -DI                   | ADX > 25 + -DI > +DI                   |
| VWAP        | VWAP             | close > VWAP (fiyat üstünde)            | close < VWAP                            |

### 4.2 Strateji Parametreleri — Statik Grid

Her tick'te 6 parametre kombinasyonu denenir (hesap maliyeti düşük, hız yüksek):

```
[0] HyperOpt optimize edilmiş (config'den, +0.1 bonus)
[1] fast=9,  slow=21, period=10, OB=80, OS=20
[2] fast=8,  slow=25, period=14, OB=70, OS=30
[3] fast=12, slow=26, period=14, OB=75, OS=25
[4] fast=5,  slow=15, period=9,  OB=70, OS=30
[5] fast=5,  slow=20, period=14, OB=70, OS=30
```

---

## 5. Sinyal Seçimi — Dinamik Sıralama ve Konsensüs Oylama

### 5.1 Composite Score Hesabı

Her strateji için `HyperOptimizer::simulate_score_htf()` çağrılır. Composite skor:

```
composite = w_sharpe × Sharpe
          + w_sortino × Sortino
          + w_wr × Win_Rate
          + w_pf × Profit_Factor
          + w_calmar × Calmar
```

Ağırlıklar interval kategorisine göre dinamik:

| Interval    | Öncelikli Metrik      | Strateji Grubu    |
|-------------|-----------------------|-------------------|
| 1m, 5m      | Win Rate, Profit Factor | Momentum (RSI, Stoch, CCI) |
| 15m, 30m    | Sharpe, Sortino       | Hybrid             |
| 1h          | Sharpe, Sortino       | Trend (MACD, EMA, ST) |
| 4h, 1d      | Calmar, PF            | Structural (ADX, BB) |

### 5.2 Top-5 Ağırlıklı Oylama

```
Toplam Top-5 strateji belirlenir (rank_strategies_for_interval)

BUY_weight  = Σ(composite_score, BUY  veren stratejiler)
SELL_weight = Σ(composite_score, SELL veren stratejiler)
total_weight = BUY_weight + SELL_weight

buy_pct  = BUY_weight  / total_weight
sell_pct = SELL_weight / total_weight

Eşik: 0.50 (≥ %50 ağırlık)

Karar:
  buy_pct  >= 0.50 ve buy_pct  > sell_pct  → BUY
  sell_pct >= 0.50 ve sell_pct > buy_pct   → SELL
  aksi halde                               → HOLD
```

**Neden bu yöntem?** Tek bir stratejinin hatalı sinyali tüm sistemi etkilemez. Ağırlıklı konsensüs, piyasa rejimi bazlı en iyi strateji grubuyla kararı verir.

### 5.3 En İyi Parametre Seçimi (Grid Search)

```
for params in [optimized, grid[1..5]]:
    score = simulate_score_htf(strategy, candles, params)
    if params == optimized: score += 0.1  # bias: optimize edilmişi tercih et

best_params = argmax(score)
```

---

## 6. Sinyal Filtre Zinciri

Bir BUY/SELL sinyali aşağıdaki 10 filtrenin tamamından geçmek zorundadır. Her engel `HOLD`'a döndürür ve loglara kaydedilir.

```
Konsensüs Sinyal
      │
      ▼ [1] HyperOpt Güvensizlik Filtresi
      │   raw_score < 0.05 → ENGEL (parametreler geçmişte zararlı)
      │
      ▼ [2] R/R Oranı Filtresi
      │   rr = take_profit_pct / stop_loss_pct
      │   rr < min_rr → ENGEL
      │
      ▼ [3] Volatilite Bant Filtresi
      │   avg_range_pct = Σ(|H-L|/C × 100) / 20 bar
      │   avg_range_pct < vol_min OR > vol_max → ENGEL
      │
      ▼ [4] HTF Trend Filtresi
      │   LTF=1m → HTF=1h | LTF=15m → HTF=4h ...
      │   BUY  + HTF Bearish → ENGEL
      │   SELL + HTF Bullish → ENGEL
      │
      ▼ [5] LTF Trend Filtresi
      │   SMA_short > SMA_long → Bullish
      │   SMA_short < SMA_long → Bearish
      │   |fark| < margin_pct(0.5%) → Neutral (geçer)
      │   BUY + Bearish → ENGEL | SELL + Bullish → ENGEL
      │
      ▼ [6] S/R Kalite Filtresi
      │   buy_quality = compute_buy_quality(dist_support, dist_resistance)
      │   buy_quality < min_buy_quality → ENGEL
      │
      ▼ [7] Futures Flip Kontrolü
      │   Ters yönde açık pozisyon var → önce kapat, sonra devam
      │   (Sadece Futures/CoinM, Spot'ta bu adım yok)
      │
      ▼ [8] HyperOpt Skor Kapısı
      │   hyperopt_score < 0.0 → ENGEL (zararlı parametre rejimi)
      │
      ▼ [9] Seans/Saat Filtresi
      │   blocked_hours listesinde → ENGEL
      │   allowed_hours listesi doluysa ve saatte yok → ENGEL
      │   long_preferred_hours'da SELL → ENGEL
      │
      ▼ [10] Pattern Kapısı (opsiyonel, varsayılan kapalı)
      │   DB'deki geçmiş pattern (trend|vol|momentum) eşleşmesi
      │   win_rate < 55% veya trade_count < 10 → ENGEL
      │   confidence = f(win_rate, trade_count, avg_pnl) < 0.20 → ENGEL
      │
      ▼ [11] SL Cooldown
          Son SL kapanışından < 1800 sn (30 dk) → ENGEL
```

### 6.1 Trend Bias Formülü

```
SMA_short = SMA(trend_short_period)  [varsayılan: 20]
SMA_long  = SMA(trend_long_period)   [varsayılan: 50]
diff_pct  = |SMA_short - SMA_long| / SMA_long × 100

if diff_pct < margin_pct (0.5%) → Neutral
elif SMA_short > SMA_long        → Bullish
else                             → Bearish
```

### 6.2 Volatilite Filtresi

```
avg_range = Σ((Hᵢ - Lᵢ) / Cᵢ × 100) / n  [n=20 bar]

Geçerli aralık: [volatility_min_pct, volatility_max_pct]
```

**Neden?** Çok düşük volatilite = sahte kırılım riski. Çok yüksek volatilite = spread ve slippage riski.

### 6.3 Pattern Confidence Formülü

```
conf_base = win_rate × 0.6 + (avg_pnl / 5.0).clamp(0,1) × 0.4
conf = conf_base × min(1.0, trade_count / 20.0)
```

Sample penalty: 20 trade'e kadar lineer düşüş (az veriyle aşırı güven engellenir).

---

## 7. Pozisyon Boyutu ve Kaldıraç

### 7.1 Temel Pozisyon Boyutu

**Kelly Kriteri (opsiyonel):**
```
f* = (b × p - q) / b
   b = win_loss_ratio (ort kazanç / ort kayıp)
   p = win_rate
   q = 1 - p

max_risk = capital × max_portfolio_risk_pct
qty = (max_risk × f*) / entry_price
```

**Klasik (varsayılan):**
```
max_risk = capital × max_portfolio_risk_pct
qty = min(max_risk, capital × max_position_size_pct) / entry_price
```

### 7.2 Dinamik Kaldıraç (Sadece Futures)

Spot piyasada kaldıraç her zaman 1x'dir.

```
lev = base_leverage   [varsayılan: 7x]

+1.0  HTF trend sinyalle aynı yönde
-2.0  ATR% > 2.5  (yüksek volatilite)
-1.0  ATR% > 1.5  (orta volatilite)
-1.0  drawdown% > 5
→base drawdown% > 10 (koruma modu)
+0.5  HyperOpt_score > 0.70 (güçlü sinyal)
+0.5  session_rr > 2.0 (çok iyi RR)
-1.0  session_rr < 0.8 (kötü RR)
-0.5  loss_streak >= 2
-1.5  loss_streak >= 3
→base loss_streak >= 5 (ciddi koruma)
-1.0  open_count >= 3 (risk konsantrasyonu)
-2.0  open_count >= 5

effective_leverage = clamp(lev, base, max)
```

### 7.3 Kaldıraçlı Lot Hesabı

```
base_qty = calculate_position_size(equity, entry_price)
qty      = base_qty × effective_leverage
notional = qty × entry_price
```

---

## 8. Stop-Loss ve Take-Profit Mekanizmaları

### 8.1 Statik SL/TP

```
Long  SL = entry × (1 - stop_loss_pct / 100)
Long  TP = entry × (1 + take_profit_pct / 100)
Short SL = entry × (1 + stop_loss_pct / 100)
Short TP = entry × (1 - take_profit_pct / 100)
```

### 8.2 SL Güvenlik Klampı (Tasfiye Koruması)

```
max_sl_pct = 80.0 / effective_leverage

Örn: 10x kaldıraç → max SL = %8
     7x  kaldıraç → max SL = %11.4
```

**Neden?** Binance Futures tasfiye marjı ~%5-10 aralığındadır. SL her zaman tasfiyeden önce tetiklenmeli.

### 8.3 Trailing Stop-Loss

```
Başlangıç: trailing_sl = None (kâra geçmeden aktif değil)

Her fiyat güncellemesinde:
  Long:  best_price = max(best_price, current_price)
         trailing_sl = best_price × (1 - trailing_pct / 100)
         Tetiklenir: current_price <= trailing_sl

  Short: best_price = min(best_price, current_price)
         trailing_sl = best_price × (1 + trailing_pct / 100)
         Tetiklenir: current_price >= trailing_sl
```

### 8.4 S/R Tabanlı Dinamik SL/TP

```
SL (Long)  = max_strength_support.price_low × (1 - buffer_pct / 100)
TP (Long)  = nearest_resistance.price_low × (1 - buffer_pct / 100)

SL (Short) = max_strength_resistance.price_high × (1 + buffer_pct / 100)
TP (Short) = nearest_support.price_high × (1 + buffer_pct / 100)
```

ATR uyarlaması (indicator_adjusted_sl_tp):
- SL_dist < 0.8×ATR → genişlet (gürültüden kapanma)
- SL_dist > 3.0×ATR → daralt (aşırı risk)
- ADX > 25 → TP + 0.5×ATR (trend devam edebilir)
- ADX < 20 → TP max 2×ATR (range ortalaması)
- BB_width > %4 → SL + 0.3×ATR buffer

### 8.5 SL/TP Tetikleme Önceliği

```
1. Trailing SL (en öncelikli — kâr koruma)
2. Static SL
3. Static TP
```

**Çıkış fiyatı:** SL/TP seviyesi kullanılır, WS anlık fiyatı değil (slippage adil simülasyonu).

### 8.6 Çıkış Fiyatı Slippage Düzeltmesi

```
adj = slippage_pct / 100 / 2

BUY  exit: price × (1 + adj)   [daha pahalı çıkış]
SELL exit: price × (1 - adj)   [daha ucuz çıkış]
```

---

## 9. PnL Hesabı

### 9.1 Brüt PnL

```
Long:  gross_pnl = (exit_price - entry_price) × qty
Short: gross_pnl = (entry_price - exit_price) × qty
```

### 9.2 Net PnL (Komisyon Dahil)

```
entry_notional = entry_price × qty
exit_notional  = exit_price  × qty
commission     = (entry_notional + exit_notional) × commission_pct
                                                    [varsayılan: 0.001 = %0.1]
net_pnl = gross_pnl - commission
```

Binance Spot Maker/Taker: %0.1 | VIP ile düşer.
Binance Futures: %0.02 Maker / %0.05 Taker (typik).

### 9.3 Marjin Bazlı PnL Yüzdesi

```
margin = entry_price × qty / leverage
pnl_pct = (net_pnl / margin) × 100
        = net_pnl × leverage / (entry_price × qty) × 100

Örn: %1 fiyat hareketi × 8x kaldıraç = %8 pnl_pct
```

### 9.4 Kümülatif Takip

```
current_equity = (capital + cumulative_pnl).max(0.0)
peak_equity    = max(peak_equity, current_equity)
drawdown_pct   = (peak_equity - current_equity) / peak_equity × 100
```

---

## 10. Risk Kapıları ve Otonom Kontrol

### 10.1 AutonomousController FSM

```
Durumlar: Active → SafeMode → Halt → Recovery → Active

Active:    Normal işlem
SafeMode:  Kademeli azaltma (consecutive_failures >= safe_mode_threshold)
Halt:      İşlem durduruldu (consecutive_failures >= halt_threshold)
Recovery:  Manuel veya otomatik kurtarma bekleniyor

Varsayılan: max_failures_before_safe_mode=3, max_failures_before_halt=5
```

### 10.2 RiskGate

```
Girdi:
  account_equity, day_start_equity, peak_equity,
  requested_notional_usd, model_confidence

Kurallar:
  DENY eğer notional > max_notional_usd
  DENY eğer (equity - day_start_equity) / day_start_equity < -max_daily_loss_pct
  DENY eğer drawdown_pct > max_drawdown_pct
  DENY eğer model_confidence < min_model_confidence
```

Varsayılan policy:
```
max_notional_usd     = 10_000
max_daily_loss_pct   = 5.0%
max_drawdown_pct     = 15.0%
min_model_confidence = 0.5
```

### 10.3 CircuitBreaker

```
Durumlar: Closed (normal) → Open (devre kesik) → HalfOpen (deneme)

Open: API çağrısı başarısız → n saniye bekle → HalfOpen
HalfOpen: Başarısızsa → Open | Başarılıysa → Closed
```

### 10.4 DrawdownMonitor

```
current_dd = (peak - current) / peak × 100

Status:
  Normal   : dd < warning_threshold (varsayılan: %10)
  Warning  : %10 ≤ dd < %20
  Critical : dd ≥ %20  → işleme ara ver
```

### 10.5 SL Cooldown

```
SL tetiklenince: sl_cooldown_map[symbol] = now()
Sonraki giriş denemesinde:
  elapsed = now() - sl_time
  elapsed < sl_cooldown_secs (1800 = 30dk) → ENGEL
```

**Neden?** SL sonrası piyasa genellikle aynı yönde devam eder. 30 dakika beklemek aynı hatanın tekrarını önler.

---

## 11. Backtest Motoru ve Performans Metrikleri

### 11.1 Backtest Simülasyonu

```
for candle in sorted_candles:
    signal = strategy.generate_signal(candles[0..i])

    if signal == BUY and no position:
        entry_price = candle.close
        fee = position_size × entry_price × commission_pct
        balance -= fee

    if position:
        if candle.low  <= SL → çıkış (SL)
        if candle.high >= TP → çıkış (TP)

    if signal == SELL and has position:
        exit_price = candle.close
        gross_pnl = (exit - entry) × size
        fee = size × (entry + exit) × commission_pct
        net_pnl = gross_pnl - fee
        balance += net_pnl

Açık pozisyon kalırsa son fiyattan kapat.
```

### 11.2 Performans Metrikleri Formülleri

**Win Rate:**
```
WR = winning_trades / total_trades × 100
```

**Profit Factor:**
```
PF = Σ(kazançlar) / Σ|kayıplar|
PF > 1 → kârlı | PF > 1.5 → iyi | PF > 2.0 → mükemmel
```

**Sharpe Ratio (trade bazlı, risk-free=0):**
```
mean_R = Σ(pnl_pct) / n
σ_R    = StdDev(pnl_pct)
Sharpe = mean_R / σ_R
```

**Sortino Ratio:**
```
downside_returns = [r for r in pnl_pcts if r < mean_R]
σ_down = StdDev(downside_returns)
Sortino = mean_R / σ_down
```

**Calmar Ratio:**
```
Calmar = total_pnl_pct / max_drawdown_pct
```

**Max Drawdown:**
```
peak = 0; max_dd = 0
for r in pnl_pcts:
    cumulative += r
    peak = max(peak, cumulative)
    max_dd = max(max_dd, peak - cumulative)
```

### 11.3 Trade Profilleri

| Profil         | TP%  | SL%  | Max Pozisyon |
|----------------|------|------|--------------|
| Conservative   | 5.0  | 1.0  | %5 sermaye   |
| Balanced       | 8.0  | 2.0  | %10 sermaye  |
| Aggressive     | 15.0 | 3.0  | %20 sermaye  |
| Scalper        | 2.0  | 0.5  | %15 sermaye  |
| SwingTrading   | 20.0 | 5.0  | %8 sermaye   |

---

## 12. Evrimsel Optimizasyon (Genetik Algoritma)

### 12.1 StrategyGenome

Her birey (genome) bir strateji parametresi kümesidir:
```
{
  strategy_name,
  fast_period, slow_period,
  rsi_period, rsi_ob, rsi_os,
  stop_loss_pct, take_profit_pct,
  fitness: FitnessScore
}
```

### 12.2 Fitness Fonksiyonu

```
FitnessScore = profit_component    (max: 50)
             + risk_component      (max: 30)
             + consistency_component (max: 20)
             + sharpe_component    (max: 30)
             + survival_bonus      (max: 20)
             ─────────────────────────────────
             Toplam range: 0–150
```

**Profit component (0-50):**
```
PF_normalized = min(profit_factor / 3.0, 1.0) × 30
WR_normalized = max(0, win_rate - 50) / 50 × 20
profit_component = PF_normalized + WR_normalized
```

**Risk component (0-30):**
```
DD_score = max(0, 1 - max_drawdown_pct / 30) × 30
risk_component = DD_score
```

**Consistency (0-20):**
```
consistency = trade_count_bonus + streak_penalty
```

### 12.3 Evrim Döngüsü

```
1. Başlangıç popülasyonu: random genome × population_size
2. Fitness değerlendirme: backtest → FitnessScore
3. Seleksiyon: tournament selection (en iyi %top_k hayatta)
4. Crossover: iki ebeveynin parametrelerini karıştır (nokta çaprazlaması)
5. Mutasyon: rastgele parametre ±delta ile boz (mutasyon_rate)
6. Yeni nesil → 2'ye dön

evolve_every_n_cycles: her N trading döngüsünde bir evrim adımı
```

### 12.4 AdaptiveBrain Q-Learning

```
State: MarketRegime (StrongUptrend / WeakUptrend / Ranging / HighVol / LowVol / ...)
Action: StrategyName

Q(s, a) := Q(s, a) + α × [reward - Q(s, a)]
α = 0.1 (öğrenme hızı)
reward = pnl_pct / 10.0

Regime tespiti:
  trend_pct = (last - first) / first × 100  [son 20 bar]

  |trend_pct| > 5% → Strong Trend
  |trend_pct| > 2% → Weak Trend
  (trend önce kontrol edilir, ardından volatilite)
  vol_pct > 3% → HighVolatility
  vol_pct < 0.5% → LowVolatility
  aksi → Ranging
```

---

## 13. ML Katmanı — Anomali Tespiti

### 13.1 Z-Score Tabanlı Anomali Tespiti

**ONNX kaldırıldı**, saf Rust z-score implementasyonu:

```
baseline = input[0..n-1]  // son eleman hariç
mean = Σ(baseline) / (n-1)
σ    = StdDev(baseline)
z    = |input[n-1] - mean| / σ

anomali = z > ZSCORE_THRESHOLD (3.0)
```

**Neden baseline = son eleman hariç?** Spike kendisini mean'e dahil ederse z-score seyreltilir ve tespit edilemez.

**Minimum sample:** MIN_SAMPLES = 5 (5 veri noktasından az varsa tespit yok).

### 13.2 Feature Extractor

```
Feature vektörü (6 özellik):
[0] RSI normalize (0-1)
[1] MACD histogram normalize
[2] BB %B = (close - lower) / (upper - lower)
[3] Stochastic %K (0-100) normalize
[4] ATR% (volatilite)
[5] Volume ratio = volume / avg_volume(20)
```

### 13.3 ML Worker Döngüsü

```
elapsed := train_every_secs  // ilk çalıştırma hemen
loop:
  sleep(1 sn)
  elapsed += 1
  if elapsed >= train_every_secs:
    → DB'den son N candle çek
    → Feature extraction
    → Z-score anomali tespiti
    → AppState.ml_summary güncelle
    elapsed = 0
```

---

## 14. Destek / Direnç (S/R) Dedektörü

### 14.1 Swing Nokta Tespiti

```
Swing High (Direnç adayı):
  candles[i].high >= tüm candles[i-lb..i+lb].high
  (±swing_lookback komşusu)

Swing Low (Destek adayı):
  candles[i].low <= tüm candles[i-lb..i+lb].low

Hacim ağırlığı: vw = candle.volume / mean_volume
```

### 14.2 Kümeleme

```
Noktalara göre sırala (fiyat)
Eğer |price - ref_price| / ref_price × 100 ≤ cluster_pct → aynı kümeye ekle

Midpoint = Σ(price × vw) / Σ(vw)  [hacim ağırlıklı]
price_low  = min(prices) - midpoint × 0.001
price_high = max(prices) + midpoint × 0.001
strength   = Σ(vw) × touch_count
```

### 14.3 Kalite Puanı (Buy Quality)

```
Birincil kriter: destekten uzaklık
  dist_support_pct > 3.0% → q = 0.15  (destekten uzak → kötü alım)
  dist_support_pct > 1.5% → q = 0.30
  dist_support_pct ≤ 1.5% → ikincil kritere geç

İkincil kriter: direnç mesafesi
  dist_resistance_pct < 0.5% → q = 0.10  (neredeyse dirençte)
  dist_resistance_pct < 1.5% → q = 0.40
  dist_resistance_pct < 3.0% → q = 0.60
  dist_resistance_pct ≥ 3.0% → q = 0.75  (geniş hareket alanı)

Özel durumlar:
  in_resistance_zone → q = 0.05  (kesinlikle alma)
  in_support_zone    → q = 0.90  (ideal giriş)
  dist_support < 0   → q = 0.15  (destek altına düştü)
```

---

## 15. Tespit Edilen Sorunlar ve Geliştirme Tavsiyeleri

### 15.1 EMA Çift Sayım Sorunu [ÖNCELİK: ORTA]

**Mevcut kod** (`indicators.rs:14-21`):
```rust
let mut ema = candles[len-n..].iter().map(|c| c.close).sum::<f64>() / n as f64;
for c in &candles[len-n..] {  // Aynı pencere!
    ema = c.close * k + ema * (1.0 - k);
}
```

**Sorun:** Seed için kullanılan N bar, iterasyonda da işleniyor. EMA ilk N bar üzerinde zaten oluşmuş, onları tekrar işlemek değeri öne çekiyor.

**Düzeltme:**
```rust
let mut ema = candles[0..n].iter().map(|c| c.close).sum::<f64>() / n as f64;
for c in &candles[n..] {  // N+1'den devam
    ema = c.close * k + ema * (1.0 - k);
}
```

**Etki:** EMA ve EMA tabanlı tüm stratejiler (MACD, EMA Crossover, Supertrend) farklı değer üretir. Backtest vs live tutarsızlığına yol açar.

---

### 15.2 RSI Wilder Smoothing Eksikliği [ÖNCELİK: ORTA]

**Mevcut:** Basit ortalama (SMA of gains/losses)

**Standart Wilder RSI:** Her adım için `SMMA(n) = (prev_SMMA × (n-1) + current) / n`

**Sonuç:** Mevcut RSI standart platform değerlerinden (TradingView, Binance) %5-10 sapabilir. Backtest ile canlı işlem arasında sinyal farklılığına yol açar.

**Öneri:**
```rust
// Wilder smoothing için:
let mut avg_gain = gains[0..period].iter().sum::<f64>() / period as f64;
let mut avg_loss = losses[0..period].iter().sum::<f64>() / period as f64;
for i in period..closes.len() {
    avg_gain = (avg_gain * (period - 1) as f64 + gains[i]) / period as f64;
    avg_loss = (avg_loss * (period - 1) as f64 + losses[i]) / period as f64;
}
```

---

### 15.3 MACD Signal Seed Sorunu [ÖNCELİK: DÜŞÜK]

**Mevcut:** Signal EMA `macd_series[0]` (≈0) ile seed ediliyor.

**Standart:** İlk `signal_period` bar MACD değerlerinin SMA'sı ile seed edilmeli.

**Etki:** İlk birkaç bar için signal line yanlış; büyük veri setlerinde (200+ bar) ihmal edilebilir.

---

### 15.4 ADX RMA Yerine Basit Ortalama [ÖNCELİK: DÜŞÜK]

**Mevcut:** `ADX ≈ son periyodun DX değeri`

**Standart:** `ADX = RMA(DX, period)` — Wilder Moving Average

**Etki:** ADX değerleri gerçek trend gücünü daha kaba ölçer, gecikme artar.

---

### 15.5 Sharpe/Sortino Zaman Normalizasyonu Eksik [ÖNCELİK: ORTA]

**Mevcut:** Trade bazlı hesaplama (zaman boyutu yok).

**Sorun:**
- 1m interval'de 100 trade vs 1d interval'de 100 trade aynı Sharpe çıkarsa karşılaştırılamaz.
- Standart yıllık Sharpe: `Sharpe_annual = Sharpe_trade × √(trades_per_year)`

**Öneri:**
```
trades_per_year = (365 × 24 × 60) / interval_minutes
annualized_sharpe = trade_sharpe × sqrt(trades_per_year)
```

---

### 15.6 Calmar = Recovery Factor Tekrarı [ÖNCELİK: DÜŞÜK]

**Mevcut:** Her ikisi de `total_pnl / max_drawdown` kullanıyor.

**Standart Calmar:** `annualized_return / max_drawdown` (3 yıllık periyot)

Bu iki metrik birbirinden farklı olmalı; şu an birbirinin kopyası.

---

### 15.7 Kelly Kriteri Binary Varsayım [ÖNCELİK: ORTA]

**Mevcut formül:** `f* = (b×p - q) / b`

Bu formül yalnızca sabit kazanç/kayıp oranı (binary outcome) için doğru.

**Gerçekçi trading için:** `f* = μ / σ²` (continuous Kelly) veya fraksiyonel Kelly:
```
f* = E[R] / E[R²]  →  pratik uygulamada f/2 veya f/4 kullanılır
```

**Risk:** Tam Kelly çok agresiftir; %50 Kelly (f*/2) önerilir.

---

### 15.8 Drawdown Koruma — İşlem Durdurma Yok [ÖNCELİK: YÜKSEK]

**Mevcut:** `dd_pct > 10% → lev = base` (kaldıraç düşürür ama işlem devam eder)

**Eksik:** Drawdown kritik seviyeye ulaştığında (örn. %20) işlemlerin tamamen durdurulması gerekir.

**Öneri:**
```
dd > 10% → leverage = base
dd > 15% → tüm pozisyon boyutlarını %50 azalt
dd > 20% → SafeMode: sadece mevcut pozisyonları kapat, yeni giriş yok
dd > 25% → Halt: tüm işlemleri durdur, manuel onay bekle
```

---

### 15.9 Konsensüs Eşiği Sabit [ÖNCELİK: ORTA]

**Mevcut:** `threshold = 0.50` (sabit)

**Sorun:** HighVolatility rejiminde %50 eşiği çok düşük olabilir; yanlış sinyaller artar.

**Öneri:** Piyasa rejimine göre adaptif eşik:
```
Ranging       → threshold = 0.60  (daha sıkı)
HighVolatility→ threshold = 0.65  (çok sıkı)
StrongTrend   → threshold = 0.45  (biraz gevşek; trend sinyaller net)
```

---

### 15.10 Pattern Gate Varsayılan Kapalı [ÖNCELİK: ORTA]

Pattern Gate (`pattern_gate_enabled = false` varsayılan) açıldığında güçlü bir süzgeç. Ancak yeterli DB verisi birikene kadar çok az sinyal üretir.

**Öneri:** `min_trade_count = 30` yerine `10` ile başlayıp kademeli olarak artır. Veri birikimini hızlandırmak için backtester çıktısını pattern_library'ye besle.

---

### 15.11 Orphan Pozisyon Slippage Yok [ÖNCELİK: DÜŞÜK]

`process_orphans()` içinde slippage hesabı yapılmıyor:
```rust
let pnl = pos.realized_pnl_with_commission(exit_price, self.config.commission_pct);
```

`adjusted_price()` çağrılmıyor. Orphan pozisyonlar için PnL hafif overestimate edilir.

---

### 15.12 HTF Candle Yoksa Filtre Atlanıyor [ÖNCELİK: ORTA]

HTF candle DB'de yoksa `htf_bias = None` döner ve HTF filtresi **geçilmiş sayılır**. Bu, henüz veri birikmeyen semboller için büyük trende karşı giriş yapılmasına izin verir.

**Öneri:** HTF verisi yoksa HOLD döndür veya giriş boyutunu %50 küçült.

---

### 15.13 Genel Tavsiyeler

#### A. Volume Confirmation
Tüm kırılım stratejileri (Donchian, BB kırılımı) **hacim onayı olmadan** tetikleniyor. Sahte kırılım oranını düşürmek için:
```
BUY sinyalinde: current_volume > avg_volume(20) × 1.5 (hacim artışı onayı)
```

#### B. Multi-Timeframe Konfirmasyon Genişletme
Şu an sadece 1 üst TF (HTF) kullanılıyor. Daha güçlü filtre için:
```
LTF: giriş sinyali
MTF: trend teyit
HTF: büyük resim bias
(tümü aynı yönde olursa giriş güçlü)
```

#### C. Bollinger Band Squeeze Tespiti
BB genişliği daralıyorsa (squeeze) kırılım yaklaşmaktadır. Mevcut BB stratejisi bunu kullanmıyor.

#### D. Trailing Stop İyileştirmesi
Şu anki trailing sadece fiyat tabanlı. ATR tabanlı trailing:
```
trailing_distance = k × ATR  (örn. k=2)
```
Bu, düşük volatilite döneminde daha dar, yüksek volatilitede daha geniş stop sağlar.

#### E. Position Sizing — Volatilite Uyarlamalı
Kelly veya sabit % yerine:
```
position_size = risk_amount / (ATR × entry_price)
```
Bu yöntem (Van Tharp'ın "Position Sizing by Volatility") tüm piyasalarda eşit risk alınmasını sağlar.

#### F. Funding Rate Stratejisi (Futures)
`FundingRateContrarian` stratejisi mevcuttur ancak ana döngüde aktif değil. Aşırı funding (±0.1%+) piyasa tersine dönüş sinyalidir; bu sinyal ML/filtre katmanına entegre edilebilir.

---

## Ekler

### Ek A — Konfigürasyon Dosyaları

| Dosya | İçerik |
|-------|--------|
| `config/rtc_config.json` | Exchange, symbol, interval, SL/TP |
| `config/trade_quality.json` | TradeQualityConfig (min_rr, vol band, trend filtre) |
| `config/robotic_profiles.json` | Position/security profilleri |
| `config/evolution_state.json` | AdaptiveBrain Q-table + PopulationManager (otomatik yazılır) |

### Ek B — Environment Değişkenleri

| Değişken | Varsayılan | Açıklama |
|----------|-----------|----------|
| `BINANCE_API_KEY` | - | API anahtarı |
| `BINANCE_API_SECRET` | - | API gizli anahtarı |
| `BINANCE_PAPER_MODE` | `true` | `false` = canlı emir |
| `TRADE_SYMBOL` | `BTCUSDT` | İşlem sembolü |
| `TRADE_MARKET` | `spot` | `spot` / `futures` / `coinm` |
| `AUTONOMOUS_ENABLED` | `false` | RiskGate + FSM aktif |
| `USE_ML_SIGNAL` | `false` | ML sinyali sinyal kanalına dahil et |
| `TRADE_CAPITAL` | `10000` | Başlangıç sermayesi (USDT) |
| `TRADE_AMOUNT` | `0.01` | Sabit lot miktarı (qty override) |

### Ek C — Onaylanan İyi Parametre Değerleri

Backtest ve canlı performansa dayanarak DB'de kanıtlanmış:

| Strateji | Parametre | Değer |
|----------|-----------|-------|
| Stochastic | K period | 6 |
| Stochastic | Oversold | 20 |
| Stochastic | Overbought | 80 |
| RSI | Period | 14 |
| MA Crossover | Fast | 5 |
| MA Crossover | Slow | 20 |
| MACD | Fast/Slow/Signal | 12/26/9 |
| BB | Period / StdDev | 20 / 2.0 |

---

*Bu döküman kaynak koddan otomatik analiz edilerek hazırlanmıştır.*
*Referans dosyalar: `robot/robotic_loop.rs`, `robot/indicators.rs`, `robot/signal_evaluator.rs`,*
*`robot/sr_detector.rs`, `robot/backtester/engine.rs`, `evolution/fitness_evaluator.rs`,*
*`evolution/adaptive_brain.rs`, `ml_anomaly.rs`*
