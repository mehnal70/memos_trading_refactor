// live_path.rs - Canlı karar yolunu (process_symbol_cycle) birebir taklit eden
// backtest harness'i. Amaç: edge/risk/exit değişikliklerini (örn. #3 ters-momentum
// cezası, risk-bazlı boyutlandırma) GERÇEK motor fonksiyonlarıyla ölçmek.
//
// Mevcut basit Backtester'dan farkı: bu harness gerçek strateji motorunu
// (make_strategy_pub + generate_signal), HTF sentezini (1m→aggregate), gerçek
// compute_edge_score / dynamic_edge_threshold / check_exit_conditions'ı kullanır.
// Böylece backtest, canlı kararla aynı mekanizmayı paylaşır (long+short).

use crate::core::types::{Candle, Signal, StrategyParams};
use crate::core::model::PositionModel;
use crate::robot::engines::Engine;
use crate::robot::engines::master::ExitReason;
use crate::robot::strategies::Strategy;
use crate::robot::logic::optimizer::make_strategy_pub;
use crate::robot::ml_engine::strategy_selector::StrategySelector;
use crate::robot::data_pipeline::htf_loader::aggregate_1m_to;
use crate::robot::data_pipeline::orchestrator::DataPipeline;

/// Pozisyon boyutlandırma politikası (A/B için).
#[derive(Debug, Clone, Copy)]
pub enum Sizing {
    /// Notional = equity × pct (canlı varsayılan: 0.10).
    NotionalPct(f64),
    /// Risk-bazlı: equity × frac kadar zarar riski; qty = risk / stop_mesafesi.
    RiskFraction(f64),
}

#[derive(Debug, Clone)]
pub struct LivePathConfig {
    pub strategy_name: String,     // "AUTO"/"" → StrategySelector; aksi halde sabit
    pub interval: String,          // base interval (HTF eşlemesi için)
    pub initial_balance: f64,
    pub sizing: Sizing,
    pub edge_reverse_penalty: f64, // compute_edge_score ters-momentum cezası (canlı 0.4)
    pub use_htf: bool,
    pub tp_pct: f64,
    pub sl_pct: f64,
    pub atr_trail_mult: f64,
    pub breakeven_rr: f64,
    pub commission_rate: f64,
    pub min_hold_bars: usize,      // reverse-signal kapanışı için min tutma (bar)
    pub window: usize,             // strateji lookback (canlı: 200)
    /// SR-farkında giriş filtresi (A/B ölçümü için): Some(band%) → long fiyatın band%'i içinde güçlü
    /// direnç altındaysa / short güçlü destek üstündeyse giriş REDDEDİLİR ("dirence alma, desteğe satma").
    /// None → filtre kapalı (mevcut davranış, sıfır regresyon). [[project_sr_display_only]]
    pub sr_filter_band_pct: Option<f64>,
    /// SR filtresi: bu güç eşiğinin altındaki bölgeler yok sayılır (yalnız güçlü S/R engeller).
    pub sr_min_strength: f64,
}

impl Default for LivePathConfig {
    fn default() -> Self {
        Self {
            strategy_name: "AUTO".into(),
            interval: "1m".into(),
            initial_balance: 10_000.0,
            sizing: Sizing::NotionalPct(0.10),
            edge_reverse_penalty: 0.4,
            use_htf: true,
            tp_pct: 3.0,
            sl_pct: 1.5,
            atr_trail_mult: 2.0,
            breakeven_rr: 1.0,
            commission_rate: 0.0004,
            min_hold_bars: 30,
            window: 200,
            sr_filter_band_pct: None, // filtre kapalı (opt-in; A/B ile kanıtlanmadan default-off)
            sr_min_strength: 0.0,
        }
    }
}

/// SAF: SR-farkında giriş uygun mu? Long → fiyatın `band_pct`'i (%) ÜSTünde `min_strength`'i aşan bir
/// DİRENÇ bölgesi varsa REDDET (sınırlı yukarı alan / ret riski). Short → band içinde ALTında güçlü
/// DESTEK varsa reddet. Engelleyici bölge yoksa true. price<=0 / band<=0 → true (koruma). Testli.
pub fn sr_entry_ok(
    zones: &[crate::robot::sr_detector::SrZone], is_long: bool, price: f64, band_pct: f64, min_strength: f64,
) -> bool {
    use crate::robot::sr_detector::ZoneType;
    if price <= 0.0 || band_pct <= 0.0 {
        return true;
    }
    let band = price * band_pct / 100.0;
    for z in zones {
        if z.strength < min_strength {
            continue;
        }
        if is_long && matches!(z.zone_type, ZoneType::Resistance)
            && z.midpoint > price && z.midpoint <= price + band {
            return false; // hemen üstte güçlü direnç → long alma
        }
        if !is_long && matches!(z.zone_type, ZoneType::Support)
            && z.midpoint < price && z.midpoint >= price - band {
            return false; // hemen altta güçlü destek → short satma
        }
    }
    true
}

#[derive(Debug, Clone, Default)]
pub struct LivePathResult {
    pub total_trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub total_pnl_pct: f64,
    pub sum_trade_pnl_pct: f64, // Σ per-trade % (qty-bağımsız getiri proxy'si)
    pub max_drawdown_pct: f64,
    pub profit_factor: f64,
    pub sharpe: f64,
    pub final_equity: f64,
}

struct OpenPos {
    pos: PositionModel,
    entry_bar: usize,
    entry_commission: f64,
}

fn gen_signal(cfg: &LivePathConfig, window: &[Candle], symbol: &str) -> Signal {
    let params = StrategyParams::default();
    let name = if cfg.strategy_name.eq_ignore_ascii_case("auto")
        || cfg.strategy_name.eq_ignore_ascii_case("default")
        || cfg.strategy_name.is_empty()
    {
        StrategySelector::new().select_best(window, &params).to_string()
    } else {
        cfg.strategy_name.clone()
    };
    let strat = make_strategy_pub(&name);
    let htf_vec = if cfg.use_htf {
        aggregate_1m_to(window, DataPipeline::get_htf_interval(&cfg.interval), symbol)
    } else {
        Vec::new()
    };
    let htf = if htf_vec.is_empty() { None } else { Some(htf_vec.as_slice()) };
    strat.generate_signal(window, &params, None, htf).unwrap_or(Signal::Hold)
}

/// Canlı-yol backtest'i tek sembol 1m mum dizisi üzerinde çalıştırır.
pub fn run(candles_1m: &[Candle], cfg: &LivePathConfig) -> LivePathResult {
    let n = candles_1m.len();
    if n <= cfg.window + 2 { return LivePathResult::default(); }
    let symbol = candles_1m[0].symbol.clone();

    let mut equity = cfg.initial_balance;
    let mut peak = equity;
    let mut max_dd: f64 = 0.0;
    let mut open: Option<OpenPos> = None;
    let mut trade_pnls: Vec<f64> = Vec::new();      // mutlak net PnL
    let mut trade_pnl_pcts: Vec<f64> = Vec::new();  // % net

    let threshold = Engine::dynamic_edge_threshold(0.0); // cold-start (harness'te GBT yok)

    for i in cfg.window..n {
        let price = candles_1m[i].close;
        if price <= 0.0 { continue; }
        let window = &candles_1m[i + 1 - cfg.window..=i];
        let atr = Engine::calc_atr(window, 14);

        // --- Açık pozisyon: exit denetimi ---
        if let Some(op) = open.as_mut() {
            op.pos.current_price = price;
            let exit = Engine::check_exit_conditions(
                &mut op.pos, price, atr, cfg.atr_trail_mult, cfg.breakeven_rr,
            );
            let reason = exit.or_else(|| {
                // Reverse-signal kapanışı (min-hold sonrası), canlı StrategySignal yolu.
                if i - op.entry_bar >= cfg.min_hold_bars {
                    let sig = gen_signal(cfg, window, &symbol);
                    let rev = (op.pos.is_long && matches!(sig, Signal::Sell))
                        || (!op.pos.is_long && matches!(sig, Signal::Buy));
                    if rev { return Some(ExitReason::StrategySignal); }
                }
                None
            });
            if let Some(reason) = reason {
                let exit_price = match reason {
                    ExitReason::StopLoss | ExitReason::Breakeven => op.pos.stop_loss,
                    ExitReason::TakeProfit => op.pos.take_profit,
                    ExitReason::TrailingStop => op.pos.trailing_stop,
                    ExitReason::StrategySignal => price,
                };
                let exit_price = if exit_price > 0.0 { exit_price } else { price };
                let gross = if op.pos.is_long {
                    (exit_price - op.pos.entry_price) * op.pos.qty
                } else {
                    (op.pos.entry_price - exit_price) * op.pos.qty
                };
                let exit_comm = exit_price * op.pos.qty * cfg.commission_rate;
                let net = gross - op.entry_commission - exit_comm;
                equity += gross - exit_comm; // entry_comm açılışta zaten düşüldü
                let notional = op.pos.entry_price * op.pos.qty;
                trade_pnls.push(net);
                trade_pnl_pcts.push(if notional > 0.0 { net / notional * 100.0 } else { 0.0 });
                open = None;
            }
        }

        // --- Pozisyon yok: giriş denetimi ---
        if open.is_none() {
            let sig = gen_signal(cfg, window, &symbol);
            if matches!(sig, Signal::Buy | Signal::Sell) {
                let edge = Engine::compute_edge_score_with(window, &sig, 0.0, cfg.edge_reverse_penalty);
                // SR-farkında giriş filtresi (opt-in): aday giriş güçlü S/R'a sıkışıyorsa reddet.
                // Yalnız edge geçen adayda detect → ucuz (her barda değil). [[project_sr_display_only]]
                let sr_ok = match cfg.sr_filter_band_pct {
                    Some(band) => {
                        let zones = crate::robot::sr_detector::SrDetector::new(
                            crate::robot::sr_detector::SrDetectorConfig::default()).detect(window);
                        sr_entry_ok(&zones, matches!(sig, Signal::Buy), price, band, cfg.sr_min_strength)
                    }
                    None => true,
                };
                if edge >= threshold && sr_ok {
                    let is_long = matches!(sig, Signal::Buy);
                    let entry = price;
                    let (stop_loss, take_profit) = if is_long {
                        (entry * (1.0 - cfg.sl_pct / 100.0), entry * (1.0 + cfg.tp_pct / 100.0))
                    } else {
                        (entry * (1.0 + cfg.sl_pct / 100.0), entry * (1.0 - cfg.tp_pct / 100.0))
                    };
                    let qty = match cfg.sizing {
                        Sizing::NotionalPct(p) => (equity * p) / entry,
                        Sizing::RiskFraction(r) => {
                            let risk_dist = (entry - stop_loss).abs().max(f64::EPSILON);
                            (equity * r) / risk_dist
                        }
                    };
                    if qty > 0.0 {
                        let trailing = if is_long { entry - atr * cfg.atr_trail_mult }
                                       else       { entry + atr * cfg.atr_trail_mult };
                        let entry_commission = entry * qty * cfg.commission_rate;
                        equity -= entry_commission;
                        open = Some(OpenPos {
                            pos: PositionModel {
                                pos_id: String::new(),
                                symbol: symbol.clone(),
                                entry_price: entry,
                                current_price: entry,
                                qty,
                                leverage: 1.0,
                                market: "spot".into(),
                                interval: cfg.interval.clone(),
                                is_long,
                                trade_type: "harness".into(),
                                opened_at: candles_1m[i].timestamp.to_rfc3339(),
                                stop_loss,
                                take_profit,
                                trailing_stop: trailing,
                                max_favorable_price: entry,
                                breakeven_activated: false,
                                kind: None,
                            },
                            entry_bar: i,
                            entry_commission,
                        });
                    }
                }
            }
        }

        peak = peak.max(equity);
        if peak > 0.0 { max_dd = max_dd.max((peak - equity) / peak * 100.0); }
    }

    finalize(cfg, equity, &trade_pnls, &trade_pnl_pcts, max_dd)
}

fn finalize(
    cfg: &LivePathConfig, equity: f64, pnls: &[f64], pnl_pcts: &[f64], max_dd: f64,
) -> LivePathResult {
    let n = pnls.len();
    let wins = pnls.iter().filter(|p| **p > 0.0).count();
    let gross_profit: f64 = pnls.iter().filter(|p| **p > 0.0).sum();
    let gross_loss: f64 = pnls.iter().filter(|p| **p < 0.0).map(|p| -p).sum();
    let profit_factor = if gross_loss > f64::EPSILON { gross_profit / gross_loss }
        else if gross_profit > 0.0 { 999.0 } else { 0.0 };
    let sharpe = if n >= 2 {
        let mean = pnl_pcts.iter().sum::<f64>() / n as f64;
        let var = pnl_pcts.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
        let sd = var.sqrt();
        if sd > f64::EPSILON { mean / sd } else { 0.0 }
    } else { 0.0 };
    let total_pnl = equity - cfg.initial_balance;
    LivePathResult {
        total_trades: n,
        win_rate: if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 },
        total_pnl,
        total_pnl_pct: total_pnl / cfg.initial_balance * 100.0,
        sum_trade_pnl_pct: pnl_pcts.iter().sum(),
        max_drawdown_pct: max_dd,
        profit_factor,
        sharpe,
        final_equity: equity,
    }
}

#[cfg(test)]
mod sr_filter_tests {
    use super::*;
    use crate::robot::sr_detector::{SrZone, ZoneType};

    fn zone(zt: ZoneType, mid: f64, strength: f64) -> SrZone {
        SrZone { price_low: mid * 0.999, price_high: mid * 1.001, midpoint: mid,
                 zone_type: zt, strength, touch_count: 3, vol_weight: 1.0 }
    }

    #[test]
    fn sr_blocks_long_under_resistance() {
        // Fiyat 100, %1 band → 100..101 aralığında güçlü direnç (100.5) → long REDDEDİLİR.
        let z = vec![zone(ZoneType::Resistance, 100.5, 5.0)];
        assert!(!sr_entry_ok(&z, true, 100.0, 1.0, 1.0), "dirence sıkışan long reddedilmeli");
        // Direnç band dışında (102 > 101) → long uygun.
        let z2 = vec![zone(ZoneType::Resistance, 102.0, 5.0)];
        assert!(sr_entry_ok(&z2, true, 100.0, 1.0, 1.0), "uzak direnç long'u engellemez");
    }

    #[test]
    fn sr_blocks_short_above_support() {
        // Fiyat 100, %1 band → 99..100 aralığında güçlü destek (99.5) → short REDDEDİLİR.
        let z = vec![zone(ZoneType::Support, 99.5, 5.0)];
        assert!(!sr_entry_ok(&z, false, 100.0, 1.0, 1.0), "desteğe sıkışan short reddedilmeli");
    }

    #[test]
    fn sr_ignores_weak_zones_and_disabled() {
        // Zayıf direnç (strength 0.5 < min 1.0) → engellemez.
        let z = vec![zone(ZoneType::Resistance, 100.5, 0.5)];
        assert!(sr_entry_ok(&z, true, 100.0, 1.0, 1.0), "zayıf bölge yok sayılır");
        // band=0 → filtre kapalı → daima uygun.
        let z2 = vec![zone(ZoneType::Resistance, 100.5, 5.0)];
        assert!(sr_entry_ok(&z2, true, 100.0, 0.0, 1.0), "band 0 → filtre devre dışı");
    }
}
