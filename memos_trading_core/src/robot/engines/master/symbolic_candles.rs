use crate::core::types::Candle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SymbolicBias {
    Bullish,
    Bearish,
    Neutral,
}

impl SymbolicBias {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Bullish => "bullish",
            Self::Bearish => "bearish",
            Self::Neutral => "neutral",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SymbolicDiagnosis {
    pub(crate) bias: SymbolicBias,
    pub(crate) confidence: f64,
    pub(crate) score: f64,
    pub(crate) patterns: Vec<&'static str>,
}

#[inline]
fn body(c: &Candle) -> f64 {
    (c.close - c.open).abs()
}

#[inline]
fn range(c: &Candle) -> f64 {
    (c.high - c.low).abs().max(1e-9)
}

#[inline]
fn upper_wick(c: &Candle) -> f64 {
    c.high - c.open.max(c.close)
}

#[inline]
fn lower_wick(c: &Candle) -> f64 {
    c.open.min(c.close) - c.low
}

#[inline]
fn is_bull(c: &Candle) -> bool {
    c.close > c.open
}

#[inline]
fn is_bear(c: &Candle) -> bool {
    c.close < c.open
}

// Son mumlardan sembolik pattern çıkartır. Bu fonksiyon karar üretmez;
// yalnız otonom gözlem katmanına bias+güven bilgisi sağlar.
pub(crate) fn analyze_recent_patterns(candles: &[Candle]) -> Option<SymbolicDiagnosis> {
    if candles.len() < 3 {
        return None;
    }

    let c0 = &candles[candles.len() - 3];
    let c1 = &candles[candles.len() - 2];
    let c2 = &candles[candles.len() - 1];

    let mut score: f64 = 0.0;
    let mut patterns: Vec<&'static str> = Vec::new();

    // İki mumlu: engulfing
    if is_bear(c1) && is_bull(c2) && c2.open <= c1.close && c2.close >= c1.open {
        score += 1.20;
        patterns.push("bullish_engulfing");
    }
    if is_bull(c1) && is_bear(c2) && c2.open >= c1.close && c2.close <= c1.open {
        score -= 1.20;
        patterns.push("bearish_engulfing");
    }

    // Tek mumlu: hammer / shooting star
    let c2_range = range(c2);
    let c2_body = body(c2);
    let c2_lw = lower_wick(c2).max(0.0);
    let c2_uw = upper_wick(c2).max(0.0);
    if c2_lw > c2_body * 2.0 && c2_uw < c2_body && c2_body / c2_range < 0.45 {
        score += 0.80;
        patterns.push("hammer");
    }
    if c2_uw > c2_body * 2.0 && c2_lw < c2_body && c2_body / c2_range < 0.45 {
        score -= 0.80;
        patterns.push("shooting_star");
    }

    // Üç mumlu: morning/evening star (hafif form)
    if is_bear(c0)
        && body(c0) / range(c0) > 0.45
        && body(c1) / range(c1) < 0.30
        && is_bull(c2)
        && c2.close >= (c0.open + c0.close) * 0.5
    {
        score += 1.10;
        patterns.push("morning_star");
    }
    if is_bull(c0)
        && body(c0) / range(c0) > 0.45
        && body(c1) / range(c1) < 0.30
        && is_bear(c2)
        && c2.close <= (c0.open + c0.close) * 0.5
    {
        score -= 1.10;
        patterns.push("evening_star");
    }

    // Doji: yönü nötrleştirici zayıf sinyal
    if c2_body / c2_range < 0.10 {
        patterns.push("doji");
        score *= 0.8;
    }

    let bias = if score > 0.25 {
        SymbolicBias::Bullish
    } else if score < -0.25 {
        SymbolicBias::Bearish
    } else {
        SymbolicBias::Neutral
    };

    let confidence = (score.abs() / 3.0).clamp(0.0, 1.0);
    Some(SymbolicDiagnosis {
        bias,
        confidence,
        score,
        patterns,
    })
}

#[cfg(test)]
mod tests {
    use super::{analyze_recent_patterns, SymbolicBias};
    use crate::core::types::Candle;
    use chrono::{TimeZone, Utc};

    fn mk(ts: i64, o: f64, h: f64, l: f64, c: f64) -> Candle {
        Candle {
            timestamp: Utc.timestamp_opt(ts, 0).unwrap(),
            open: o,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
            symbol: "T".to_string(),
            interval: "1m".to_string(),
        }
    }

    #[test]
    fn bullish_engulfing_yields_bullish_bias() {
        let candles = vec![
            mk(1_700_000_000, 101.0, 102.0, 100.0, 100.2),
            mk(1_700_000_060, 100.8, 101.0, 99.5, 99.8),
            mk(1_700_000_120, 99.7, 101.5, 99.6, 101.2),
        ];

        let diag = analyze_recent_patterns(&candles).expect("diag");
        assert_eq!(diag.bias, SymbolicBias::Bullish);
        assert!(diag.patterns.contains(&"bullish_engulfing"));
    }

    #[test]
    fn doji_softens_signal_to_neutral_or_low_conf() {
        let candles = vec![
            mk(1_700_000_000, 100.0, 100.8, 99.5, 100.7),
            mk(1_700_000_060, 100.7, 101.1, 100.4, 100.9),
            mk(1_700_000_120, 100.9, 101.8, 99.8, 100.92),
        ];

        let diag = analyze_recent_patterns(&candles).expect("diag");
        assert!(diag.patterns.contains(&"doji"));
        assert!(matches!(
            diag.bias,
            SymbolicBias::Neutral | SymbolicBias::Bullish
        ));
        assert!(diag.confidence <= 0.35);
    }
}
