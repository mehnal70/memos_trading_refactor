/// Scalp & Swing Trade Engine
///
/// Mevcut robotic loop'a dokunmadan çakışmasız çalışan kısa-vade fırsat motoru.
///
/// - ScalpEngine  : 3m mumlar + EMA(5/13) + RSI(7) + Bollinger sıkışma; SL ~%0.4, TP ~%0.8
/// - SwingEngine  : 4h mumlar + EMA(21/55) + MACD + ADX; SL ~%2.5, TP ~%5.0
/// - SlotGuard    : sembol başına max 1 scalp + 1 swing pozisyon; çakışma engeli
/// - ModeSelector : ADX + volatiliteye göre otomatik mod seçimi

pub mod scalp_engine;
pub mod swing_engine;
pub mod slot_guard;
pub mod mode_selector;

pub use scalp_engine::ScalpEngine;
pub use swing_engine::SwingEngine;
pub use slot_guard::SlotGuard;
pub use mode_selector::{TradeMode, ModeSelector};

use serde::{Deserialize, Serialize};

/// Skalp/Swing pozisyon türü — OpenPosition'a eklenir
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TradeType {
    #[default]
    Regular,  // Mevcut robotic loop stratejileri
    Scalp,    // ScalpEngine tarafından açıldı (3m/5m)
    Swing,    // SwingEngine tarafından açıldı (4h/1D)
}

impl TradeType {
    pub fn label(&self) -> &'static str {
        match self {
            TradeType::Regular => "REG",
            TradeType::Scalp   => "SCP",
            TradeType::Swing   => "SWG",
        }
    }
}

// ── Otonom ayarlama sınırları ─────────────────────────────────────────────────

/// Bir parametre için otonom ayarlama penceresi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamBounds {
    pub min: f64,
    pub max: f64,
    /// Kaç işlemde bir yeniden değerlendir (0 = devre dışı)
    pub adjust_every_n: usize,
}

impl ParamBounds {
    pub fn clamp(&self, v: f64) -> f64 { v.clamp(self.min, self.max) }
    pub fn enabled(&self) -> bool { self.adjust_every_n > 0 }
}

// ── ScalpSwingConfig ──────────────────────────────────────────────────────────

/// Scalp & Swing motorun tüm parametreleri — `config/rtc_config.json` içindeki
/// `scalp_swing` bloğundan yüklenir.  Her alan `#[serde(default)]` ile geriye uyumludur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalpSwingConfig {

    // ── Aktif/pasif bayrakları ─────────────────────────────────────────────────
    #[serde(default = "default_true")]
    pub scalp_enabled: bool,
    #[serde(default = "default_true")]
    pub swing_enabled: bool,

    // ── Zaman dilimleri ────────────────────────────────────────────────────────
    #[serde(default = "default_scalp_interval")]
    pub scalp_interval: String,   // "3m" | "5m"
    #[serde(default = "default_swing_interval")]
    pub swing_interval: String,   // "4h" | "1d"

    // ── SL / TP yüzdeleri ─────────────────────────────────────────────────────
    #[serde(default = "default_scalp_sl")]
    pub scalp_sl_pct: f64,        // varsayılan %0.40
    #[serde(default = "default_scalp_tp")]
    pub scalp_tp_pct: f64,        // varsayılan %0.85
    #[serde(default = "default_swing_sl")]
    pub swing_sl_pct: f64,        // varsayılan %2.50
    #[serde(default = "default_swing_tp")]
    pub swing_tp_pct: f64,        // varsayılan %5.00

    // ── Kaldıraç (Futures; Spot'ta her zaman 1.0x) ────────────────────────────
    #[serde(default = "default_scalp_leverage")]
    pub scalp_leverage: f64,      // varsayılan 2.0x
    #[serde(default = "default_swing_leverage")]
    pub swing_leverage: f64,      // varsayılan 3.0x

    // ── Bütçe (toplam sermayenin yüzdesi) ─────────────────────────────────────
    /// Scalp işlemleri için ayrılan sermaye yüzdesi.
    /// Örn. 0.20 → toplam capital'in %20'si scalp'e ayrılır.
    /// None = genel trade_amount/capital kullanılır (ayrım yok).
    #[serde(default)]
    pub scalp_budget_pct: Option<f64>,   // None | 0.05–0.50
    /// Swing işlemleri için ayrılan sermaye yüzdesi.
    #[serde(default)]
    pub swing_budget_pct: Option<f64>,   // None | 0.05–0.50

    // ── Komisyon & maliyet ────────────────────────────────────────────────────
    /// Tek taraf komisyon oranı — None ise loop'un commission_pct'si kullanılır.
    /// Binance Futures taker VIP0: 0.0004 (%0.04)
    #[serde(default)]
    pub commission_pct: Option<f64>,

    /// Spread yüzdesi — None ise loop'un execution_cost_config'i kullanılır.
    #[serde(default)]
    pub spread_pct: Option<f64>,

    /// Slippage yüzdesi — None ise loop'un execution_cost_config'i kullanılır.
    #[serde(default)]
    pub slippage_pct: Option<f64>,

    // ── Risk limitleri ────────────────────────────────────────────────────────
    /// Günlük maksimum kayıp yüzdesi — bu aşılınca scalp/swing işlem engellenir.
    /// None = kontrol yok.
    #[serde(default = "default_max_daily_loss")]
    pub max_daily_loss_pct: f64,  // varsayılan %3.0

    /// Tek pozisyon maksimum notional (USDT) — None = sınır yok.
    #[serde(default)]
    pub max_notional_usd: Option<f64>,

    // ── Sinyal kalite eşikleri ────────────────────────────────────────────────
    #[serde(default = "default_scalp_min_score")]
    pub scalp_min_score: f64,     // varsayılan 0.60
    #[serde(default = "default_swing_min_adx")]
    pub swing_min_adx: f64,       // varsayılan 20.0
    #[serde(default = "default_swing_min_score")]
    pub swing_min_score: f64,     // varsayılan 0.55

    // ── Slot limitleri ────────────────────────────────────────────────────────
    #[serde(default = "default_one")]
    pub max_scalp_per_symbol: usize,
    #[serde(default = "default_one")]
    pub max_swing_per_symbol: usize,

    // ── Saat kısıtı ───────────────────────────────────────────────────────────
    #[serde(default = "default_scalp_hours")]
    pub scalp_active_hours: [u32; 2],  // UTC [start, end]

    // ── Otonom ayarlama ───────────────────────────────────────────────────────
    /// true = performansa göre SL/TP/leverage/min_score otomatik ayarlanır.
    #[serde(default)]
    pub autonomous_tuning: bool,

    /// Otonom ayarlama sınırları — scalp SL
    #[serde(default = "default_scalp_sl_bounds")]
    pub scalp_sl_bounds: ParamBounds,
    /// Otonom ayarlama sınırları — scalp TP
    #[serde(default = "default_scalp_tp_bounds")]
    pub scalp_tp_bounds: ParamBounds,
    /// Otonom ayarlama sınırları — swing SL
    #[serde(default = "default_swing_sl_bounds")]
    pub swing_sl_bounds: ParamBounds,
    /// Otonom ayarlama sınırları — swing TP
    #[serde(default = "default_swing_tp_bounds")]
    pub swing_tp_bounds: ParamBounds,
    /// Otonom ayarlama sınırları — scalp kaldıraç
    #[serde(default = "default_scalp_lev_bounds")]
    pub scalp_lev_bounds: ParamBounds,
    /// Otonom ayarlama sınırları — swing kaldıraç
    #[serde(default = "default_swing_lev_bounds")]
    pub swing_lev_bounds: ParamBounds,
    /// Otonom ayarlama sınırları — scalp min_score
    #[serde(default = "default_score_bounds")]
    pub scalp_score_bounds: ParamBounds,
    /// Otonom ayarlama sınırları — swing min_score
    #[serde(default = "default_score_bounds")]
    pub swing_score_bounds: ParamBounds,
}

impl Default for ScalpSwingConfig {
    fn default() -> Self {
        Self {
            scalp_enabled:       true,
            swing_enabled:       true,
            scalp_interval:      "3m".to_string(),
            swing_interval:      "4h".to_string(),
            scalp_sl_pct:        0.40,
            scalp_tp_pct:        0.85,
            swing_sl_pct:        2.50,
            swing_tp_pct:        5.00,
            scalp_leverage:      2.0,
            swing_leverage:      3.0,
            scalp_budget_pct:    None,
            swing_budget_pct:    None,
            commission_pct:      None,
            spread_pct:          None,
            slippage_pct:        None,
            max_daily_loss_pct:  3.0,
            max_notional_usd:    None,
            scalp_min_score:     0.60,
            swing_min_adx:       20.0,
            swing_min_score:     0.55,
            max_scalp_per_symbol: 1,
            max_swing_per_symbol: 1,
            scalp_active_hours:  [6, 22],
            autonomous_tuning:   false,
            scalp_sl_bounds:     default_scalp_sl_bounds(),
            scalp_tp_bounds:     default_scalp_tp_bounds(),
            swing_sl_bounds:     default_swing_sl_bounds(),
            swing_tp_bounds:     default_swing_tp_bounds(),
            scalp_lev_bounds:    default_scalp_lev_bounds(),
            swing_lev_bounds:    default_swing_lev_bounds(),
            scalp_score_bounds:  default_score_bounds(),
            swing_score_bounds:  default_score_bounds(),
        }
    }
}

// ── Serde default fonksiyonları ───────────────────────────────────────────────
fn default_true()              -> bool      { true }
fn default_scalp_interval()    -> String    { "3m".to_string() }
fn default_swing_interval()    -> String    { "4h".to_string() }
fn default_scalp_sl()          -> f64       { 0.40 }
fn default_scalp_tp()          -> f64       { 0.85 }
fn default_swing_sl()          -> f64       { 2.50 }
fn default_swing_tp()          -> f64       { 5.00 }
fn default_scalp_leverage()    -> f64       { 2.0 }
fn default_swing_leverage()    -> f64       { 3.0 }
fn default_max_daily_loss()    -> f64       { 3.0 }
fn default_one()               -> usize     { 1 }
fn default_scalp_min_score()   -> f64       { 0.60 }
fn default_swing_min_adx()     -> f64       { 20.0 }
fn default_swing_min_score()   -> f64       { 0.55 }
fn default_scalp_hours()       -> [u32; 2]  { [6, 22] }

fn default_scalp_sl_bounds() -> ParamBounds {
    ParamBounds { min: 0.20, max: 1.00, adjust_every_n: 10 }
}
fn default_scalp_tp_bounds() -> ParamBounds {
    ParamBounds { min: 0.40, max: 2.00, adjust_every_n: 10 }
}
fn default_swing_sl_bounds() -> ParamBounds {
    ParamBounds { min: 1.00, max: 5.00, adjust_every_n: 5 }
}
fn default_swing_tp_bounds() -> ParamBounds {
    ParamBounds { min: 2.00, max: 10.0, adjust_every_n: 5 }
}
fn default_scalp_lev_bounds() -> ParamBounds {
    ParamBounds { min: 1.0, max: 5.0, adjust_every_n: 20 }
}
fn default_swing_lev_bounds() -> ParamBounds {
    ParamBounds { min: 1.0, max: 7.0, adjust_every_n: 10 }
}
fn default_score_bounds() -> ParamBounds {
    ParamBounds { min: 0.40, max: 0.90, adjust_every_n: 15 }
}

// ── Performans istatistikleri (otonom ayarlama için) ──────────────────────────

/// Scalp veya swing için oturum geneli istatistikler.
/// LoopState içinde tutulur; close_position_and_log tetiklenince güncellenir.
#[derive(Debug, Clone, Default)]
pub struct ScalpSwingStats {
    pub total_closed:   usize,
    pub wins:           usize,
    pub total_pnl:      f64,
    pub total_win_pnl:  f64,
    pub total_loss_pnl: f64,
    pub loss_streak:    usize,
    pub max_loss_streak: usize,
    /// Son ayarlama yapıldığındaki closed sayısı
    pub last_tune_at:   usize,
}

impl ScalpSwingStats {
    pub fn win_rate(&self) -> f64 {
        if self.total_closed == 0 { return 0.5; }
        self.wins as f64 / self.total_closed as f64
    }

    pub fn profit_factor(&self) -> f64 {
        let loss = self.total_loss_pnl.abs();
        if loss < f64::EPSILON { return 3.0; }
        self.total_win_pnl / loss
    }

    pub fn avg_win(&self) -> f64 {
        if self.wins == 0 { return 0.0; }
        self.total_win_pnl / self.wins as f64
    }

    pub fn avg_loss(&self) -> f64 {
        let losses = self.total_closed - self.wins;
        if losses == 0 { return 0.0; }
        self.total_loss_pnl.abs() / losses as f64
    }

    /// Yeni kapanan işlemi kaydet
    pub fn record(&mut self, pnl: f64) {
        self.total_closed += 1;
        self.total_pnl    += pnl;
        if pnl > 0.0 {
            self.wins         += 1;
            self.total_win_pnl += pnl;
            self.loss_streak   = 0;
        } else {
            self.total_loss_pnl += pnl;
            self.loss_streak    += 1;
            if self.loss_streak > self.max_loss_streak {
                self.max_loss_streak = self.loss_streak;
            }
        }
    }
}

/// Otonom ayarlama — her N işlemde çağrılır, config parametrelerini günceller.
///
/// Kurallar:
///   WinRate < 40%  → SL'i daralt (%10), min_score'u yükselt (+0.03)
///   WinRate > 65%  → TP'yi genişlet (+%10), kaldıracı artır (+0.5 — sınır dahilinde)
///   PF < 1.0       → TP/SL oranını iyileştir, kaldıracı düşür
///   LossStreak ≥ 3 → kaldıracı 1 adım azalt, min_score'u yükselt
pub fn auto_tune(
    stats:       &ScalpSwingStats,
    trade_type:  TradeType,
    cfg:         &mut ScalpSwingConfig,
) -> Vec<String> {
    let mut changes: Vec<String> = Vec::new();
    let wr  = stats.win_rate();
    let pf  = stats.profit_factor();

    match trade_type {
        TradeType::Scalp => {
            let b_sl    = &cfg.scalp_sl_bounds.clone();
            let b_tp    = &cfg.scalp_tp_bounds.clone();
            let b_lev   = &cfg.scalp_lev_bounds.clone();
            let b_score = &cfg.scalp_score_bounds.clone();

            // ── WinRate çok düşük ─────────────────────────────────────────────
            if wr < 0.40 {
                let new_sl = b_sl.clamp(cfg.scalp_sl_pct * 0.90);
                if (new_sl - cfg.scalp_sl_pct).abs() > 0.001 {
                    changes.push(format!("SCP SL {:.2}%→{:.2}% (WR={:.0}%)", cfg.scalp_sl_pct, new_sl, wr * 100.0));
                    cfg.scalp_sl_pct = new_sl;
                }
                let new_sc = b_score.clamp(cfg.scalp_min_score + 0.03);
                if new_sc > cfg.scalp_min_score {
                    changes.push(format!("SCP min_score {:.2}→{:.2}", cfg.scalp_min_score, new_sc));
                    cfg.scalp_min_score = new_sc;
                }
            }

            // ── WinRate yüksek ────────────────────────────────────────────────
            if wr > 0.65 && pf > 1.5 {
                let new_tp = b_tp.clamp(cfg.scalp_tp_pct * 1.10);
                if new_tp > cfg.scalp_tp_pct + 0.01 {
                    changes.push(format!("SCP TP {:.2}%→{:.2}% (WR={:.0}%)", cfg.scalp_tp_pct, new_tp, wr * 100.0));
                    cfg.scalp_tp_pct = new_tp;
                }
                let new_lev = b_lev.clamp(cfg.scalp_leverage + 0.5);
                if new_lev > cfg.scalp_leverage + 0.1 {
                    changes.push(format!("SCP lev {:.1}x→{:.1}x", cfg.scalp_leverage, new_lev));
                    cfg.scalp_leverage = new_lev;
                }
            }

            // ── Profit factor zayıf ───────────────────────────────────────────
            if pf < 1.0 {
                let new_lev = b_lev.clamp(cfg.scalp_leverage - 0.5);
                if new_lev < cfg.scalp_leverage - 0.1 {
                    changes.push(format!("SCP lev {:.1}x→{:.1}x (PF={:.2})", cfg.scalp_leverage, new_lev, pf));
                    cfg.scalp_leverage = new_lev;
                }
            }

            // ── Ardışık kayıp ─────────────────────────────────────────────────
            if stats.loss_streak >= 3 {
                let new_lev = b_lev.clamp(cfg.scalp_leverage - 0.5);
                if new_lev < cfg.scalp_leverage - 0.1 {
                    changes.push(format!("SCP lev {:.1}x→{:.1}x (streak={})", cfg.scalp_leverage, new_lev, stats.loss_streak));
                    cfg.scalp_leverage = new_lev;
                }
                let new_sc = b_score.clamp(cfg.scalp_min_score + 0.02);
                if new_sc > cfg.scalp_min_score {
                    changes.push(format!("SCP min_score {:.2}→{:.2} (streak)", cfg.scalp_min_score, new_sc));
                    cfg.scalp_min_score = new_sc;
                }
            }
        }

        TradeType::Swing => {
            let b_sl    = &cfg.swing_sl_bounds.clone();
            let b_tp    = &cfg.swing_tp_bounds.clone();
            let b_lev   = &cfg.swing_lev_bounds.clone();
            let b_score = &cfg.swing_score_bounds.clone();

            if wr < 0.40 {
                let new_sl = b_sl.clamp(cfg.swing_sl_pct * 0.90);
                if (new_sl - cfg.swing_sl_pct).abs() > 0.01 {
                    changes.push(format!("SWG SL {:.2}%→{:.2}%", cfg.swing_sl_pct, new_sl));
                    cfg.swing_sl_pct = new_sl;
                }
                let new_sc = b_score.clamp(cfg.swing_min_score + 0.03);
                if new_sc > cfg.swing_min_score {
                    changes.push(format!("SWG min_score {:.2}→{:.2}", cfg.swing_min_score, new_sc));
                    cfg.swing_min_score = new_sc;
                }
            }

            if wr > 0.60 && pf > 1.5 {
                let new_tp = b_tp.clamp(cfg.swing_tp_pct * 1.10);
                if new_tp > cfg.swing_tp_pct + 0.05 {
                    changes.push(format!("SWG TP {:.2}%→{:.2}%", cfg.swing_tp_pct, new_tp));
                    cfg.swing_tp_pct = new_tp;
                }
                let new_lev = b_lev.clamp(cfg.swing_leverage + 0.5);
                if new_lev > cfg.swing_leverage + 0.1 {
                    changes.push(format!("SWG lev {:.1}x→{:.1}x", cfg.swing_leverage, new_lev));
                    cfg.swing_leverage = new_lev;
                }
            }

            if pf < 1.0 {
                let new_lev = b_lev.clamp(cfg.swing_leverage - 0.5);
                if new_lev < cfg.swing_leverage - 0.1 {
                    changes.push(format!("SWG lev {:.1}x→{:.1}x (PF={:.2})", cfg.swing_leverage, new_lev, pf));
                    cfg.swing_leverage = new_lev;
                }
            }

            if stats.loss_streak >= 3 {
                let new_lev = b_lev.clamp(cfg.swing_leverage - 0.5);
                if new_lev < cfg.swing_leverage - 0.1 {
                    changes.push(format!("SWG lev {:.1}x→{:.1}x (streak={})", cfg.swing_leverage, new_lev, stats.loss_streak));
                    cfg.swing_leverage = new_lev;
                }
            }
        }

        TradeType::Regular => {}
    }

    changes
}

/// Bir scalp/swing fırsatının tam açıklaması — robotic_loop'a döner
#[derive(Debug, Clone)]
pub struct TradeOpportunity {
    pub trade_type:  TradeType,
    pub is_long:     bool,
    pub score:       f64,      // 0.0–1.0 güven skoru
    pub sl_pct:      f64,
    pub tp_pct:      f64,
    pub reason:      String,   // log metni
}
