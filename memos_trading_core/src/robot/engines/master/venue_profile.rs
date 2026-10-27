use crate::core::indicators::calculate_atr;
use crate::core::types::{Candle, Exchange};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VenueProfile {
    pub exchange: Exchange,
    pub max_pyramid_tranches: usize,
    pub max_atr_pct: f64,
}

fn exchange_for_symbol(symbol: &str) -> Exchange {
    let (bare, tag) = crate::robot::venue::registry::split_venue_tag(symbol);
    tag.unwrap_or_else(|| Exchange::classify(bare))
}

use crate::robot::venue::registry::VenueRegistry;

fn exchange_usize_override(key: &str, exchange: Exchange) -> Option<usize> {
    std::env::var(key).ok()?.split([',', ';']).find_map(|item| {
        let (name, value) = item.split_once(':')?;
        (Exchange::from_token(name.trim()) == Some(exchange))
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
            .filter(|value| *value > 0)
    })
}

fn exchange_f64_override(key: &str, exchange: Exchange) -> Option<f64> {
    std::env::var(key).ok()?.split([',', ';']).find_map(|item| {
        let (name, value) = item.split_once(':')?;
        (Exchange::from_token(name.trim()) == Some(exchange))
            .then(|| value.trim().parse::<f64>().ok())
            .flatten()
            .filter(|value| value.is_finite() && *value > 0.0)
    })
}

impl VenueProfile {
    /// Sembolün venue profilini çıkar: TEK KAYNAK `VenueAdapter::trading_profile()`.
    ///
    /// ESKİ HAL: default'lar burada `match exchange { Binance => (4, 5.0), ... }` ile
    /// motorda gömülüydü — yeni venue eklemek bu dosyayı düzenlemek demekti ve değerler
    /// venue tanımından kopuktu. Artık registry'den route edilip venue'nun kendi
    /// profili alınır; route edilemeyen sembol muhafazakar `TradingProfile::default()`
    /// tabanına düşer. `PYRAMID_*_BY_EXCHANGE` env override'ları (profillerde aktif
    /// kullanımda) davranış-nötr korunur ve her zaman venue default'unu ezer.
    pub(crate) fn for_symbol(symbol: &str, registry: &VenueRegistry) -> Self {
        let exchange = exchange_for_symbol(symbol);
        let base = registry
            .route(symbol)
            .map(|(v, _)| v.trading_profile())
            .unwrap_or_default();
        Self {
            exchange,
            max_pyramid_tranches: exchange_usize_override(
                "PYRAMID_MAX_TRANCHES_BY_EXCHANGE",
                exchange,
            )
            .unwrap_or(base.max_pyramid_tranches),
            max_atr_pct: exchange_f64_override("PYRAMID_MAX_ATR_PCT_BY_EXCHANGE", exchange)
                .unwrap_or(base.max_atr_pct),
        }
    }

    pub(crate) fn allows_pyramid(&self, candles: &[Candle]) -> bool {
        let Some(last) = candles.last() else {
            return false;
        };
        if last.close <= 0.0 {
            return false;
        }
        let atr = calculate_atr(candles, 14).last().copied().unwrap_or(0.0);
        atr <= 0.0 || (atr / last.close * 100.0) <= self.max_atr_pct
    }
}

fn strategy_directive(raw: &str, exchange: Exchange) -> Option<String> {
    raw.split([',', ';']).find_map(|item| {
        let (name, strategy) = item.split_once(':')?;
        (Exchange::from_token(name.trim()) == Some(exchange))
            .then(|| strategy.trim().to_ascii_uppercase())
            .filter(|strategy| !strategy.is_empty())
    })
}

pub(crate) fn strategy_override_for_symbol(symbol: &str) -> Option<String> {
    let directive = strategy_directive(
        &std::env::var("STRATEGY_BY_EXCHANGE").ok()?,
        exchange_for_symbol(symbol),
    )?;
    (!matches!(directive.as_str(), "AUTO" | "DEFAULT")).then_some(directive)
}

pub(crate) fn strategy_auto_for_symbol(symbol: &str) -> bool {
    std::env::var("STRATEGY_BY_EXCHANGE")
        .ok()
        .and_then(|raw| strategy_directive(&raw, exchange_for_symbol(symbol)))
        .is_some_and(|directive| matches!(directive.as_str(), "AUTO" | "DEFAULT"))
}

pub(crate) fn strategy_pool_for_symbol(symbol: &str) -> Vec<String> {
    let exchange = exchange_for_symbol(symbol);
    let configured = std::env::var("STRATEGY_POOL_BY_EXCHANGE")
        .ok()
        .and_then(|raw| {
            raw.split(';').find_map(|item| {
                let (name, pool) = item.split_once(':')?;
                (Exchange::from_token(name.trim()) == Some(exchange)).then(|| {
                    pool.split('|')
                        .map(|strategy| strategy.trim().to_ascii_uppercase())
                        .filter(|strategy| !strategy.is_empty())
                        .collect::<Vec<_>>()
                })
            })
        })
        .unwrap_or_default();
    if configured.is_empty() {
        match exchange {
            Exchange::Binance => vec!["EMA_CROSSOVER", "DONCHIAN", "SUPERTREND"],
            Exchange::Mt5 => vec!["EMA_CROSSOVER", "MA_CROSSOVER", "RSI"],
            Exchange::Ibkr => vec!["EMA_CROSSOVER", "MA_CROSSOVER", "DONCHIAN"],
            _ => vec!["EMA_CROSSOVER"],
        }
        .into_iter()
        .map(str::to_string)
        .collect()
    } else {
        configured
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default'lar artık motor match'inden değil venue impl'lerinden gelir
    /// (`VenueAdapter::trading_profile`) — registry üzerinden uçtan uca doğrula.
    /// Route edilemeyen sembol muhafazakar tabana düşmeli.
    #[test]
    fn defaults_come_from_venue_trading_profile() {
        use crate::core::types::{Market, VenueSpec};
        let specs = [
            VenueSpec::new(Exchange::Binance, Market::Futures),
            VenueSpec::new(Exchange::Mt5, Market::Cfd),
            VenueSpec::new(Exchange::Ibkr, Market::Cfd),
        ];
        let reg = crate::robot::venue::registry::VenueRegistry::from_specs(&specs, None);
        assert_eq!(
            VenueProfile::for_symbol("BTCUSDT@binance", &reg).max_pyramid_tranches,
            4
        );
        assert_eq!(
            VenueProfile::for_symbol("EURUSD@mt5", &reg).max_pyramid_tranches,
            3
        );
        assert_eq!(
            VenueProfile::for_symbol("XLE@ibkr", &reg).max_pyramid_tranches,
            2
        );
        assert!((VenueProfile::for_symbol("EURUSD@mt5", &reg).max_atr_pct - 2.5).abs() < 1e-9);

        // Boş registry → route yok → muhafazakar taban (2, 2.0)
        let empty = crate::robot::venue::registry::VenueRegistry::from_specs(&[], None);
        let p = VenueProfile::for_symbol("BTCUSDT@binance", &empty);
        assert_eq!(p.max_pyramid_tranches, 2);
        assert!((p.max_atr_pct - 2.0).abs() < 1e-9);
    }

    #[test]
    fn default_strategy_pools_are_venue_specific() {
        assert!(strategy_pool_for_symbol("BTCUSDT@binance").contains(&"SUPERTREND".to_string()));
        assert!(strategy_pool_for_symbol("EURUSD@mt5").contains(&"RSI".to_string()));
        assert!(strategy_pool_for_symbol("XLE@ibkr").contains(&"DONCHIAN".to_string()));
    }

    #[test]
    fn strategy_directive_distinguishes_auto_from_fixed_override() {
        let raw = "binance:AUTO,mt5:MA_CROSSOVER,ibkr:DEFAULT";
        assert_eq!(
            strategy_directive(raw, Exchange::Binance).as_deref(),
            Some("AUTO")
        );
        assert_eq!(
            strategy_directive(raw, Exchange::Mt5).as_deref(),
            Some("MA_CROSSOVER")
        );
        assert_eq!(
            strategy_directive(raw, Exchange::Ibkr).as_deref(),
            Some("DEFAULT")
        );
    }
}
