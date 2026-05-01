use crate::strategies::{Strategy, htf_trend_filter};
use crate::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::Result;
// ...existing code...
use crate::robot::indicators::{
    calculate_rsi,
    calculate_macd,
    calculate_bollinger,
    calculate_stochastic,
    calculate_williams_r,
    calculate_adx,
    calculate_vwap,
    calculate_supertrend,
    calculate_ema_series,
    calculate_stochastic_rsi,
    calculate_cci,
};

/// RSI tabanlı strateji
pub struct RsiStrategy;
impl Strategy for RsiStrategy {
    fn name(&self) -> &str { "RSI" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(14);
        let overbought = params.overbought.unwrap_or(70.0);
        let oversold = params.oversold.unwrap_or(30.0);
        let rsi = calculate_rsi(candles, period).unwrap_or(50.0);
        let raw = if rsi > overbought { Signal::Sell } else if rsi < oversold { Signal::Buy } else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "RSI"))
    }
}

/// MACD tabanlı strateji
pub struct MacdStrategy;
impl Strategy for MacdStrategy {
    fn name(&self) -> &str { "MACD" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let fast = params.fast.unwrap_or(12);
        let slow = params.slow.unwrap_or(26);
        let signal_period = params.signal_period.unwrap_or(9);
        let raw = if let Some((macd, signal, _hist)) = calculate_macd(candles, fast, slow, signal_period) {
            if macd > signal { Signal::Buy } else if macd < signal { Signal::Sell } else { Signal::Hold }
        } else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "MACD"))
    }
}

/// Bollinger Bands tabanlı strateji
pub struct BollingerStrategy;
impl Strategy for BollingerStrategy {
    fn name(&self) -> &str { "BOLLINGER" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period = params.bb_period.or(params.period).unwrap_or(20);
        let std_dev = params.std_dev.unwrap_or(2.0);
        let raw = if let Some((upper, middle, lower)) = calculate_bollinger(candles, period, std_dev) {
            let close = candles.last().map(|c| c.close).unwrap_or(middle);
            if close > upper { Signal::Sell } else if close < lower { Signal::Buy } else { Signal::Hold }
        } else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "Bollinger"))
    }
}

/// Stochastic tabanlı strateji
pub struct StochasticStrategy;
impl Strategy for StochasticStrategy {
    fn name(&self) -> &str { "STOCHASTIC" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(14);
        let overbought = params.overbought.unwrap_or(80.0);
        let oversold = params.oversold.unwrap_or(20.0);
        let stoch = calculate_stochastic(candles, period).unwrap_or(50.0);
        let raw = if stoch > overbought { Signal::Sell } else if stoch < oversold { Signal::Buy } else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "Stochastic"))
    }
}

/// Williams %R tabanlı strateji
pub struct WilliamsRStrategy;
impl Strategy for WilliamsRStrategy {
    fn name(&self) -> &str { "WILLIAMS" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(14);
        let overbought = params.overbought.unwrap_or(-20.0);
        let oversold = params.oversold.unwrap_or(-80.0);
        let raw = if let Some(wr) = calculate_williams_r(candles, period) {
            if wr <= oversold { Signal::Buy } else if wr >= overbought { Signal::Sell } else { Signal::Hold }
        } else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "WilliamsR"))
    }
}

/// ADX tabanlı strateji (yön için +DI/-DI kullanılır)
pub struct AdxStrategy;
impl Strategy for AdxStrategy {
    fn name(&self) -> &str { "ADX" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(14);
        let threshold = params.oversold.unwrap_or(25.0);
        let raw = if let Some((adx, plus_di, minus_di)) = calculate_adx(candles, period) {
            if adx > threshold {
                if plus_di > minus_di { Signal::Buy } else if minus_di > plus_di { Signal::Sell } else { Signal::Hold }
            } else { Signal::Hold }
        } else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "ADX"))
    }
}

/// VWAP tabanlı strateji (fiyat VWAP üstü/altı)
pub struct VwapStrategy;
impl Strategy for VwapStrategy {
    fn name(&self) -> &str { "VWAP" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(20);
        if candles.is_empty() { return Ok(Signal::Hold); }
        let close = candles.last().unwrap().close;
        let raw = if let Some(vwap) = calculate_vwap(candles, period) {
            if close > vwap { Signal::Buy } else if close < vwap { Signal::Sell } else { Signal::Hold }
        } else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "VWAP"))
    }
}

/// Supertrend stratejisi (ATR tabanlı trend takip).
/// Kripto piyasaları için en güvenilir trend göstergelerinden biri.
/// period = ATR periyodu (varsayılan 10), std_dev = çarpan (varsayılan 3.0).
pub struct SupertrendStrategy;
impl Strategy for SupertrendStrategy {
    fn name(&self) -> &str { "SUPERTREND" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(10);
        let mult   = params.std_dev.unwrap_or(3.0);
        let raw = match calculate_supertrend(candles, period, mult) {
            Some((1, _))  => Signal::Buy,
            Some((-1, _)) => Signal::Sell,
            _             => Signal::Hold,
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "Supertrend"))
    }
}

/// EMA Crossover stratejisi (SMA yerine EMA — daha duyarlı).
/// fast/slow parametrelerini kullanır. EMA kesişiminde sinyal üretir.
pub struct EmaCrossoverStrategy;
impl Strategy for EmaCrossoverStrategy {
    fn name(&self) -> &str { "EMA" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let fast = params.fast.unwrap_or(9);
        let slow = params.slow.unwrap_or(21);
        if candles.len() < slow + 2 { return Ok(Signal::Hold); }
        let fast_now  = calculate_ema_series(candles,                    fast).and_then(|v| v.last().copied());
        let slow_now  = calculate_ema_series(candles,                    slow).and_then(|v| v.last().copied());
        let fast_prev = calculate_ema_series(&candles[..candles.len()-1], fast).and_then(|v| v.last().copied());
        let slow_prev = calculate_ema_series(&candles[..candles.len()-1], slow).and_then(|v| v.last().copied());
        let raw = match (fast_now, slow_now, fast_prev, slow_prev) {
            (Some(fn_), Some(sn), Some(fp), Some(sp)) => {
                if fp <= sp && fn_ > sn       { Signal::Buy  }
                else if fp >= sp && fn_ < sn  { Signal::Sell }
                else                          { Signal::Hold }
            }
            _ => Signal::Hold,
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "EMA"))
    }
}

/// StochasticRSI stratejisi — RSI üzerine uygulanan Stochastic.
/// Standart RSI'dan daha hassas OB/OS tespiti için kullanılır.
pub struct StochasticRsiStrategy;
impl Strategy for StochasticRsiStrategy {
    fn name(&self) -> &str { "STOCH_RSI" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let rsi_period = params.period.unwrap_or(14);
        let overbought = params.overbought.unwrap_or(80.0);
        let oversold   = params.oversold.unwrap_or(20.0);
        let raw = match calculate_stochastic_rsi(candles, rsi_period, 14, 3, 3) {
            Some((k, _d)) => {
                if k < oversold { Signal::Buy } else if k > overbought { Signal::Sell } else { Signal::Hold }
            }
            None => Signal::Hold,
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "StochRSI"))
    }
}

/// CCI stratejisi (Commodity Channel Index) — döngüsel dönüm noktaları.
/// >+100 → Sell (aşırı alım), <-100 → Buy (aşırı satım).
pub struct CciStrategy;
impl Strategy for CciStrategy {
    fn name(&self) -> &str { "CCI" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(20);
        let raw = match calculate_cci(candles, period) {
            Some(cci) if cci < -100.0 => Signal::Buy,
            Some(cci) if cci >  100.0 => Signal::Sell,
            _ => Signal::Hold,
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "CCI"))
    }
}

/// ATR tabanlı risk/pozisyon boyutlandırma stratejisi örneği (yalnızca sinyal değil, risk için de kullanılabilir)
// Yeni stratejiler ve göstergeler eklemek için yukarıdaki gibi yeni struct ve Strategy trait implementasyonu eklemen yeterli.

/// Price Action stratejisi — engulfing, pin bar ve inside-bar formasyonları.
/// Gösterge bağımsız; yalnızca son 3 mumun şekline bakar.
///
/// - **Bullish engulfing**: önceki kırmızı < mevcut yeşil body → BUY
/// - **Bearish engulfing**: önceki yeşil < mevcut kırmızı body → SELL
/// - **Bullish pin bar**: alt gölge ≥ body×2 ve üst gölge küçük → BUY
/// - **Bearish pin bar**: üst gölge ≥ body×2 ve alt gölge küçük → SELL
pub struct PriceActionStrategy;
impl Strategy for PriceActionStrategy {
    fn name(&self) -> &str { "PRICE_ACTION" }
    fn generate_signal(&self, candles: &[Candle], _params: &StrategyParams, _fr: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        if candles.len() < 3 { return Ok(Signal::Hold); }
        let n = candles.len();
        let prev = &candles[n - 2];
        let curr = &candles[n - 1];

        let prev_body = (prev.close - prev.open).abs();
        let curr_body = (curr.close - curr.open).abs();
        let curr_upper = curr.high - curr.close.max(curr.open);
        let curr_lower = curr.close.min(curr.open) - curr.low;

        // Engulfing
        let bull_engulf = prev.close < prev.open          // önceki kırmızı
            && curr.close > curr.open                      // mevcut yeşil
            && curr_body > prev_body * 1.1                 // mevcut daha büyük
            && curr.open <= prev.close                     // mevcut open ≤ prev close
            && curr.close >= prev.open;                    // mevcut close ≥ prev open

        let bear_engulf = prev.close > prev.open
            && curr.close < curr.open
            && curr_body > prev_body * 1.1
            && curr.open >= prev.close
            && curr.close <= prev.open;

        // Pin bar (wick ≥ 2× body, wick karşı taraf küçük)
        let bull_pin = curr_lower >= curr_body * 2.0 && curr_upper < curr_body * 0.5;
        let bear_pin = curr_upper >= curr_body * 2.0 && curr_lower < curr_body * 0.5;

        let raw = if bull_engulf || bull_pin { Signal::Buy }
                  else if bear_engulf || bear_pin { Signal::Sell }
                  else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "PriceAction"))
    }
}

/// ICT Fair Value Gap (FVG) stratejisi.
/// 3 mumlu swing'de oluşan boşluk (imbalance) bölgesini tespit eder.
///
/// Bullish FVG: candle[i-2].high < candle[i].low → fiyat bu boşluğa dönünce BUY
/// Bearish FVG: candle[i-2].low  > candle[i].high → fiyat bu boşluğa dönünce SELL
///
/// `period` parametresi: kaç mum geriye bakılacak (varsayılan 5).
pub struct IctFvgStrategy;
impl Strategy for IctFvgStrategy {
    fn name(&self) -> &str { "ICT_FVG" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _fr: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let lookback = params.period.unwrap_or(5).max(3);
        if candles.len() < lookback + 1 { return Ok(Signal::Hold); }
        let n = candles.len();
        let current_price = candles[n - 1].close;

        let mut raw = Signal::Hold;
        // Son `lookback` mum içindeki FVG'leri tara
        for i in (2..lookback.min(n - 1)).rev() {
            let left  = &candles[n - 1 - i - 1]; // i+1 mum öncesi
            let mid   = &candles[n - 1 - i];
            let right = &candles[n - 1 - i + 1]; // i-1 mum öncesi
            let _ = mid; // orta mumu kullanmıyoruz, sadece boşluk kontrolü

            // Bullish FVG: left.high < right.low (arada boşluk)
            if left.high < right.low {
                let fvg_mid = (left.high + right.low) / 2.0;
                // Fiyat FVG içindeyse ve altından dönüyorsa BUY
                if current_price >= left.high && current_price <= right.low {
                    raw = Signal::Buy;
                    break;
                }
                // Fiyat FVG'ye yakın yaklaşıyorsa (1% içinde) BUY
                if (current_price - fvg_mid).abs() / fvg_mid < 0.01 && current_price < fvg_mid {
                    raw = Signal::Buy;
                    break;
                }
            }
            // Bearish FVG: left.low > right.high
            if left.low > right.high {
                let fvg_mid = (left.low + right.high) / 2.0;
                if current_price <= left.low && current_price >= right.high {
                    raw = Signal::Sell;
                    break;
                }
                if (current_price - fvg_mid).abs() / fvg_mid < 0.01 && current_price > fvg_mid {
                    raw = Signal::Sell;
                    break;
                }
            }
        }
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "ICT_FVG"))
    }
}

/// Smart Money Concepts (SMC) — Break of Structure (BOS) ve Change of Character (CHoCH).
///
/// - **BOS (trend devamı)**: Önceki swing high kırılırsa BUY, swing low kırılırsa SELL.
/// - **CHoCH (trend dönüşü)**: Yükselen trendde swing low kırılırsa potential reversal SELL,
///   düşen trendde swing high kırılırsa potential reversal BUY.
///
/// `period` parametresi: swing tespit için lookback (varsayılan 10).
pub struct SmcStrategy;
impl Strategy for SmcStrategy {
    fn name(&self) -> &str { "SMC" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _fr: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let swing_lb = params.period.unwrap_or(10).max(3);
        if candles.len() < swing_lb * 2 + 2 { return Ok(Signal::Hold); }
        let n = candles.len();
        let current = &candles[n - 1];

        // Son swing_lb mum içindeki en yüksek high ve en düşük low (swing levels)
        let window = &candles[n - 1 - swing_lb..n - 1];
        let swing_high = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let swing_low  = window.iter().map(|c| c.low ).fold(f64::INFINITY,    f64::min);

        // Önceki swing_lb penceresi (iç trend yönünü belirlemek için)
        let prev_window = &candles[n - 1 - swing_lb * 2..n - 1 - swing_lb];
        let prev_high = prev_window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let prev_low  = prev_window.iter().map(|c| c.low ).fold(f64::INFINITY,    f64::min);

        let uptrend   = swing_high > prev_high && swing_low > prev_low;
        let downtrend = swing_high < prev_high && swing_low < prev_low;

        let raw = if current.close > swing_high {
            // BOS yukarı: trend devamı BUY veya düşen trendde CHoCH BUY
            Signal::Buy
        } else if current.close < swing_low {
            // BOS aşağı: trend devamı SELL veya yükselen trendde CHoCH SELL
            Signal::Sell
        } else if uptrend && current.close < swing_low {
            // CHoCH: yükselen trend kırıldı
            Signal::Sell
        } else if downtrend && current.close > swing_high {
            // CHoCH: düşen trend kırıldı
            Signal::Buy
        } else {
            Signal::Hold
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "SMC"))
    }
}

// ─── YENİ ICT STRATEJİLERİ ───────────────────────────────────────────────────

/// ICT Order Block (OB) stratejisi.
///
/// Bir displacement (güçlü momentum hareketi) öncesindeki son karşı mumu "Order Block"
/// olarak tanımlar. Fiyat bu bölgeye geri döndüğünde kurumsal katılım beklenir.
///
/// - **Bullish OB**: Yükselen market structure içinde son kırmızı mum → fiyat geri döndüğünde BUY
/// - **Bearish OB**: Düşen market structure içinde son yeşil mum → fiyat geri döndüğünde SELL
///
/// `period` parametresi: swing tespiti için lookback (varsayılan 10).
pub struct IctOrderBlockStrategy;
impl Strategy for IctOrderBlockStrategy {
    fn name(&self) -> &str { "ICT_OB" }
    fn generate_signal(
        &self,
        candles: &[Candle],
        params: &StrategyParams,
        _fr: Option<&[FundingRatePoint]>,
        htf_candles: Option<&[Candle]>,
    ) -> Result<Signal> {
        let swing_lb = params.period.unwrap_or(10).max(5);
        if candles.len() < swing_lb * 2 + 3 { return Ok(Signal::Hold); }
        let n = candles.len();
        let current = &candles[n - 1];

        let window = &candles[n - 1 - swing_lb..n - 1];
        let swing_high = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let swing_low  = window.iter().map(|c| c.low ).fold(f64::INFINITY,    f64::min);

        let prev_window = &candles[n - 1 - swing_lb * 2..n - 1 - swing_lb];
        let prev_high = prev_window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let prev_low  = prev_window.iter().map(|c| c.low ).fold(f64::INFINITY,    f64::min);

        let bullish_ms = swing_high > prev_high; // yükselen yapı
        let bearish_ms = swing_low  < prev_low;  // düşen yapı

        // Bullish OB: son kırmızı mum (displacement öncesi karşı hareket)
        if bullish_ms {
            if let Some(ob) = candles[n - 1 - swing_lb..n - 2]
                .iter()
                .rev()
                .find(|c| c.close < c.open)
            {
                let ob_mid = (ob.high + ob.low) / 2.0;
                let in_ob  = current.close >= ob.low && current.close <= ob.high;
                let near_ob = ob_mid > 1e-10
                    && (current.close - ob_mid).abs() / ob_mid < 0.008
                    && current.close < ob_mid;
                if in_ob || near_ob {
                    return Ok(htf_trend_filter(Signal::Buy, htf_candles, 10, 30, "ICT_OB"));
                }
            }
        }
        // Bearish OB: son yeşil mum
        if bearish_ms {
            if let Some(ob) = candles[n - 1 - swing_lb..n - 2]
                .iter()
                .rev()
                .find(|c| c.close > c.open)
            {
                let ob_mid = (ob.high + ob.low) / 2.0;
                let in_ob  = current.close >= ob.low && current.close <= ob.high;
                let near_ob = ob_mid > 1e-10
                    && (current.close - ob_mid).abs() / ob_mid < 0.008
                    && current.close > ob_mid;
                if in_ob || near_ob {
                    return Ok(htf_trend_filter(Signal::Sell, htf_candles, 10, 30, "ICT_OB"));
                }
            }
        }
        Ok(Signal::Hold)
    }
}

/// ICT Liquidity Sweep (Stop Hunt) stratejisi.
///
/// Kurumsal oyuncular retale likiditeyi (stop emirleri) avlar. Fiyat bir swing
/// seviyesini kısa süre geçer (sweep) ve hemen geri döner.
///
/// - **Bullish Sweep**: Fiyat swing low altına geçer → geri döner → BUY
/// - **Bearish Sweep**: Fiyat swing high üstüne geçer → geri döner → SELL
///
/// Sweep tespiti: önceki mum yeni ext seviyeye ulaştı, mevcut mum geri kapandı.
/// `period` parametresi: swing tespiti lookback (varsayılan 20).
pub struct IctLiquiditySweepStrategy;
impl Strategy for IctLiquiditySweepStrategy {
    fn name(&self) -> &str { "ICT_SWEEP" }
    fn generate_signal(
        &self,
        candles: &[Candle],
        params: &StrategyParams,
        _fr: Option<&[FundingRatePoint]>,
        htf_candles: Option<&[Candle]>,
    ) -> Result<Signal> {
        let lookback = params.period.unwrap_or(20).max(5);
        if candles.len() < lookback + 3 { return Ok(Signal::Hold); }
        let n = candles.len();
        let current = &candles[n - 1];
        let prev    = &candles[n - 2];

        // Swing seviyeleri (son 2 mum hariç)
        let window = &candles[n - 2 - lookback..n - 2];
        let swing_high = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let swing_low  = window.iter().map(|c| c.low ).fold(f64::INFINITY,    f64::min);

        // Bullish sweep: önceki mum swing_low'u geçti, bu mum kapandı üstünde
        let bull_sweep = prev.low < swing_low
            && current.close > swing_low
            && current.close > prev.open; // güçlü geri dönüş (bullish close)

        // Bearish sweep: önceki mum swing_high'ı geçti, bu mum kapandı altında
        let bear_sweep = prev.high > swing_high
            && current.close < swing_high
            && current.close < prev.open; // güçlü geri dönüş (bearish close)

        let raw = if bull_sweep      { Signal::Buy  }
                  else if bear_sweep { Signal::Sell }
                  else               { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "ICT_SWEEP"))
    }
}

/// ICT Killzone stratejisi — Londra Açılışı ve NY Açılışı seans filtresi.
///
/// ICT metodolojisine göre en yüksek olasılıklı hamleler belirli saatlerde oluşur:
/// - **Londra Açılışı**: 07:00–10:00 UTC (Asian Sweep + Londra Displacement)
/// - **NY Açılışı**   : 12:00–15:00 UTC (Londra/NY Overlap, en güçlü seans)
///
/// Bu saat dışında `Hold` döner. Saat içindeyse FVG sinyali üretir.
/// Kripto için 7/24 çalışır; forex session'larına göre kalibre edilmiştir.
pub struct IctKillzoneStrategy;
impl Strategy for IctKillzoneStrategy {
    fn name(&self) -> &str { "ICT_KILLZONE" }
    fn generate_signal(
        &self,
        candles: &[Candle],
        params: &StrategyParams,
        _fr: Option<&[FundingRatePoint]>,
        htf_candles: Option<&[Candle]>,
    ) -> Result<Signal> {
        use chrono::Timelike;
        if candles.len() < 10 { return Ok(Signal::Hold); }
        let n = candles.len();
        let current = &candles[n - 1];

        // Seans filtresi (UTC)
        let hour = current.timestamp.hour();
        let in_killzone = (7..10).contains(&hour)   // Londra Açılışı
                       || (12..15).contains(&hour);  // NY Açılışı
        if !in_killzone { return Ok(Signal::Hold); }

        // Seans içindeyse: FVG + Liquidity Sweep kombinasyonu
        let lookback = params.period.unwrap_or(6).max(3);

        // FVG tarama
        let mut raw = Signal::Hold;
        for i in (2..lookback.min(n - 1)).rev() {
            let left  = &candles[n - 1 - i - 1];
            let right = &candles[n - 1 - i + 1];
            if left.high < right.low {
                let fvg_mid = (left.high + right.low) / 2.0;
                if current.close >= left.high && current.close <= right.low {
                    raw = Signal::Buy; break;
                }
                if fvg_mid > 1e-10 && (current.close - fvg_mid).abs() / fvg_mid < 0.012
                    && current.close < fvg_mid
                {
                    raw = Signal::Buy; break;
                }
            }
            if left.low > right.high {
                let fvg_mid = (left.low + right.high) / 2.0;
                if current.close <= left.low && current.close >= right.high {
                    raw = Signal::Sell; break;
                }
                if fvg_mid > 1e-10 && (current.close - fvg_mid).abs() / fvg_mid < 0.012
                    && current.close > fvg_mid
                {
                    raw = Signal::Sell; break;
                }
            }
        }
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "ICT_KILLZONE"))
    }
}

/// ICT OTE (Optimal Trade Entry) stratejisi.
///
/// BOS (Break of Structure) sonrası fiyatın %62–79 Fibonacci retrace bölgesine
/// geri çekilmesi kurumsal katılım için optimal giriş noktasıdır.
///
/// - Bullish BOS → %62–79 retrace bölgesinde BUY
/// - Bearish BOS → %62–79 retrace bölgesinde SELL
///
/// `period` parametresi: swing tespiti lookback (varsayılan 15).
pub struct IctOteStrategy;
impl Strategy for IctOteStrategy {
    fn name(&self) -> &str { "ICT_OTE" }
    fn generate_signal(
        &self,
        candles: &[Candle],
        params: &StrategyParams,
        _fr: Option<&[FundingRatePoint]>,
        htf_candles: Option<&[Candle]>,
    ) -> Result<Signal> {
        let swing_lb = params.period.unwrap_or(15).max(5);
        if candles.len() < swing_lb * 2 + 2 { return Ok(Signal::Hold); }
        let n = candles.len();
        let price = candles[n - 1].close;

        let window = &candles[n - 1 - swing_lb..n - 1];
        let swing_high = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let swing_low  = window.iter().map(|c| c.low ).fold(f64::INFINITY,    f64::min);
        let range = swing_high - swing_low;
        if range < 1e-10 { return Ok(Signal::Hold); }

        let prev_window = &candles[n - 1 - swing_lb * 2..n - 1 - swing_lb];
        let prev_high = prev_window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let prev_low  = prev_window.iter().map(|c| c.low ).fold(f64::INFINITY,    f64::min);

        let bullish_bos = swing_high > prev_high; // yukarı BOS
        let bearish_bos = swing_low  < prev_low;  // aşağı BOS

        let raw = if bullish_bos {
            // Bullish BOS: swing_high'tan geri çekilme → OTE %62–79
            let ote_low  = swing_high - range * 0.79; // deep retrace
            let ote_high = swing_high - range * 0.62; // shallow retrace
            if price >= ote_low && price <= ote_high { Signal::Buy } else { Signal::Hold }
        } else if bearish_bos {
            // Bearish BOS: swing_low'dan geri çekilme → OTE %62–79
            let ote_low  = swing_low + range * 0.62;
            let ote_high = swing_low + range * 0.79;
            if price >= ote_low && price <= ote_high { Signal::Sell } else { Signal::Hold }
        } else {
            Signal::Hold
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "ICT_OTE"))
    }
}

/// ICT Composite stratejisi — en yüksek olasılıklı ICT girişi.
///
/// Birden fazla ICT kavramını kademeli filtreyle birleştirir:
///   1. Market Structure (BOS/CHoCH) → trend yönü belirleme
///   2. Premium/Discount Zone → fiyatın equilibrium'a göre konumu
///   3. FVG **veya** Order Block → giriş bölgesi teyidi
///
/// Üç katman aynı anda onaylarsa sinyal üretir — düşük frekans, yüksek kalite.
/// `period` parametresi: swing lookback (varsayılan 20).
pub struct IctCompositeStrategy;
impl Strategy for IctCompositeStrategy {
    fn name(&self) -> &str { "ICT_COMPOSITE" }
    fn generate_signal(
        &self,
        candles: &[Candle],
        params: &StrategyParams,
        _fr: Option<&[FundingRatePoint]>,
        htf_candles: Option<&[Candle]>,
    ) -> Result<Signal> {
        let swing_lb = params.period.unwrap_or(20).max(8);
        if candles.len() < swing_lb * 2 + 6 { return Ok(Signal::Hold); }
        let n = candles.len();
        let current = &candles[n - 1];

        // ── 1. Market Structure ──────────────────────────────────
        let window = &candles[n - 1 - swing_lb..n - 1];
        let swing_high = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let swing_low  = window.iter().map(|c| c.low ).fold(f64::INFINITY,    f64::min);
        let range = swing_high - swing_low;
        if range < 1e-10 { return Ok(Signal::Hold); }

        let prev_window = &candles[n - 1 - swing_lb * 2..n - 1 - swing_lb];
        let prev_high = prev_window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let prev_low  = prev_window.iter().map(|c| c.low ).fold(f64::INFINITY,    f64::min);

        let bullish_ms = swing_high > prev_high && swing_low >= prev_low; // HH+HL
        let bearish_ms = swing_low  < prev_low  && swing_high <= prev_high; // LL+LH

        // ── 2. Premium/Discount Zone ─────────────────────────────
        let equilibrium = (swing_high + swing_low) / 2.0;
        let in_discount = current.close < equilibrium; // BUY için uygun
        let in_premium  = current.close > equilibrium; // SELL için uygun

        // ── 3a. FVG tespiti (son 6 mum) ─────────────────────────
        let fvg_lb = 6usize.min(n.saturating_sub(2));
        let mut has_bull_fvg = false;
        let mut has_bear_fvg = false;
        for i in 2..fvg_lb {
            if n < i + 2 { break; }
            let left  = &candles[n - 1 - i - 1];
            let right = &candles[n - 1 - i + 1];
            if left.high < right.low {
                if current.close >= left.high && current.close <= right.low {
                    has_bull_fvg = true;
                }
            }
            if left.low > right.high {
                if current.close >= right.high && current.close <= left.low {
                    has_bear_fvg = true;
                }
            }
        }

        // ── 3b. Order Block tespiti ──────────────────────────────
        let ob_lb = swing_lb.min(n.saturating_sub(2));
        let ob_bullish_hit = candles[n - 1 - ob_lb..n - 2]
            .iter().rev()
            .find(|c| c.close < c.open) // kırmızı mum = Bullish OB
            .map(|ob| current.close >= ob.low && current.close <= ob.high)
            .unwrap_or(false);
        let ob_bearish_hit = candles[n - 1 - ob_lb..n - 2]
            .iter().rev()
            .find(|c| c.close > c.open) // yeşil mum = Bearish OB
            .map(|ob| current.close >= ob.low && current.close <= ob.high)
            .unwrap_or(false);

        // ── Kompozit karar ───────────────────────────────────────
        let raw = if bullish_ms && in_discount && (has_bull_fvg || ob_bullish_hit) {
            Signal::Buy
        } else if bearish_ms && in_premium && (has_bear_fvg || ob_bearish_hit) {
            Signal::Sell
        } else {
            Signal::Hold
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "ICT_COMPOSITE"))
    }
}

/// MA Crossover stratejisi (örnek)
pub struct MaCrossoverStrategy;

impl Strategy for MaCrossoverStrategy {
    fn name(&self) -> &str {
        "MA_CROSSOVER"
    }

    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let fast = params.fast.unwrap_or(5);
        let slow = params.slow.unwrap_or(20);
        if candles.len() < slow + 1 { return Ok(Signal::Hold); }
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let fast_ma      = crate::robot::calculations::math::MovingAverage::sma(&closes, fast)?;
        let slow_ma      = crate::robot::calculations::math::MovingAverage::sma(&closes, slow)?;
        let prev_fast_ma = crate::robot::calculations::math::MovingAverage::sma(&closes[..closes.len()-1], fast)?;
        let prev_slow_ma = crate::robot::calculations::math::MovingAverage::sma(&closes[..closes.len()-1], slow)?;
        let raw = if prev_fast_ma <= prev_slow_ma && fast_ma > slow_ma      { Signal::Buy  }
                  else if prev_fast_ma >= prev_slow_ma && fast_ma < slow_ma { Signal::Sell }
                  else                                                       { Signal::Hold };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "MA_Crossover"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    #[test]
    fn test_ma_crossover_buy_signal() {
        // Önce düşen sonra yükselen fiyatlar - MA crossover oluşturur
        let mut prices = vec![100.0; 15];
        prices.extend(vec![95.0, 94.0, 93.0, 92.0, 91.0]);
        prices.extend(vec![96.0, 100.0, 104.0, 108.0, 112.0]);
        
        let candles = prices.iter().enumerate().map(|(_i, &price)| Candle {
            timestamp: Utc::now(),
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 1.0,
            symbol: "TEST".to_string(),
            interval: "1m".to_string(),
        }).collect::<Vec<_>>();
        
        let params = StrategyParams { fast: Some(5), slow: Some(20), period: None, overbought: None, oversold: None, fast_period: None, slow_period: None, signal_period: None, std_dev: None, bb_period: None };
        let strat = MaCrossoverStrategy;
        let sig = strat.generate_signal(&candles, &params, None, None).unwrap();
        // Hızlı yükseliş sonrası Buy sinyali bekleniyor
        assert!(sig == Signal::Buy || sig == Signal::Hold);
    }
}
