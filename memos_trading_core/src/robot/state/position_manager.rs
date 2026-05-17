// robot/state/position_manager.rs - Otonom Pozisyon ve Hayatta Kalma Yönetimi

use crate::core::types::{Market, PositionId, RiskParams};
use chrono::Utc;

fn default_tp1_close_ratio() -> f64 { 0.40 }

/// §12.4: OpenPosition - Açık işlemlerin canlı takibi ve otonom risk yönetimi.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpenPosition {
    pub id: PositionId,
    pub symbol: String,
    pub market: Market,
    pub entry_price: f64,
    pub qty: f64,
    pub is_long: bool,
    pub static_sl: f64,
    pub static_tp: f64,
    pub best_price: f64,
    pub trailing_sl: Option<f64>,
    pub trailing_pct: Option<f64>,
    pub trailing_activation_pct: Option<f64>,
    pub leverage: f64,
    pub liquidation_price: f64,
    pub risk_distance: f64,
    pub breakeven_at_rr: Option<f64>,
    pub breakeven_triggered: bool,
    pub atr_trail_mult: Option<f64>,
    pub partial_tp_ratio: Option<f64>,
    pub partial_tp_triggered: bool,
    pub opened_at: String,
    pub entry_features: Option<[f64; 19]>,
    pub tp1_price: Option<f64>,
    pub tp1_close_ratio: f64,
    pub tp1_triggered: bool,
    pub trade_type: crate::robot::scalp_swing::TradeType,
    pub manual_exit_required: bool,
}

impl OpenPosition {
    pub fn new(
        symbol: String, market: Market, entry_price: f64, qty: f64, is_long: bool,
        risk: &RiskParams, leverage: f64, breakeven_at_rr: Option<f64>,
        atr_trail_mult: Option<f64>, partial_tp_ratio: Option<f64>,
        trailing_activation_pct: Option<f64>,
    ) -> Self {
        let lev = leverage.max(1.0);
        let (static_sl, static_tp) = if is_long {
            (entry_price * (1.0 - risk.stop_loss_pct / 100.0), entry_price * (1.0 + risk.take_profit_pct / 100.0))
        } else {
            (entry_price * (1.0 + risk.stop_loss_pct / 100.0), entry_price * (1.0 - risk.take_profit_pct / 100.0))
        };

        // Srivastava Güvenlik Sınırı: Marjin %90 erirse likidasyon varsayılır.
        let liquidation_price = if is_long { entry_price * (1.0 - 0.9 / lev) } else { entry_price * (1.0 + 0.9 / lev) };
        let risk_distance = (entry_price - static_sl).abs().max(1e-10);
        
        let tp1_price = partial_tp_ratio.map(|_| {
            if is_long { entry_price + 0.5 * (static_tp - entry_price) } else { entry_price - 0.5 * (entry_price - static_tp) }
        });

        Self {
            id: PositionId::new(), symbol, market, entry_price, qty, is_long,
            static_sl, static_tp, best_price: entry_price, trailing_sl: None,
            trailing_pct: risk.trailing_stop_pct, trailing_activation_pct,
            leverage: lev, liquidation_price, risk_distance, breakeven_at_rr,
            breakeven_triggered: false, atr_trail_mult, partial_tp_ratio,
            partial_tp_triggered: false, opened_at: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            entry_features: None, tp1_price, tp1_close_ratio: default_tp1_close_ratio(), 
            tp1_triggered: false, trade_type: crate::robot::scalp_swing::TradeType::Regular, 
            manual_exit_required: false,
        }
    }

    /// Pozisyonu anlık fiyatla otonom günceller ve çıkış sinyali üretir.
    pub fn update(&mut self, current_price: f64) -> Option<&'static str> {
        // En iyi fiyat takibi (Trailing stop referansı)
        if self.is_long { if current_price > self.best_price { self.best_price = current_price; } }
        else { if current_price < self.best_price { self.best_price = current_price; } }

        // 1. Otonom Trailing Stop Güncelleme
        if let Some(tpct) = self.trailing_pct {
            let act = self.trailing_activation_pct.unwrap_or(0.0).max(tpct + 0.5);
            let in_profit = if self.is_long { current_price >= self.entry_price * (1.0 + act / 100.0) }
                            else { current_price <= self.entry_price * (1.0 - act / 100.0) };
            
            if in_profit {
                let profit_rr = (current_price - self.entry_price).abs() / self.risk_distance;
                let effective_tpct = match profit_rr {
                    r if r >= 3.0 => tpct * 0.50,
                    r if r >= 2.0 => tpct * 0.70,
                    _ => tpct,
                };
                let tight_tsl = if self.is_long { self.best_price * (1.0 - effective_tpct / 100.0) }
                                else { self.best_price * (1.0 + effective_tpct / 100.0) };

                self.trailing_sl = Some(if self.is_long { self.trailing_sl.map_or(tight_tsl, |old| old.max(tight_tsl)) }
                                        else { self.trailing_sl.map_or(tight_tsl, |old| old.min(tight_tsl)) });
            }
        }

        // 2. Breakeven (Kafa Kafaya) Tetikleyici
        if !self.breakeven_triggered {
            if let Some(be_rr) = self.breakeven_at_rr {
                let trigger = (current_price - self.entry_price).abs() / self.risk_distance >= be_rr;
                if trigger { self.static_sl = self.entry_price; self.breakeven_triggered = true; }
            }
        }

        // 3. Çıkış Öncelik Matrisi (Functional Check)
        let (p, s, t, sl) = (current_price, self.static_tp, self.trailing_sl, self.static_sl);
        match self.is_long {
            true => {
                if p >= s { return Some("take_profit"); }
                if !self.tp1_triggered && self.tp1_price.map_or(false, |price| p >= price) { return Some("tp1"); }
                if t.map_or(false, |stop| p <= stop) { return Some("trailing_sl"); }
                if p <= sl { return Some("static_sl"); }
            },
            false => {
                if p <= s { return Some("take_profit"); }
                if !self.tp1_triggered && self.tp1_price.map_or(false, |price| p <= price) { return Some("tp1"); }
                if t.map_or(false, |stop| p >= stop) { return Some("trailing_sl"); }
                if p >= sl { return Some("static_sl"); }
            }
        }
        None
    }

    pub fn realized_pnl_with_commission(&self, close_price: f64, commission_pct: f64) -> f64 {
        let gross = if self.is_long { (close_price - self.entry_price) * self.qty } 
                    else { (self.entry_price - close_price) * self.qty };
        let commission = (self.entry_price * self.qty + close_price * self.qty) * commission_pct;
        gross - commission
    }
}
