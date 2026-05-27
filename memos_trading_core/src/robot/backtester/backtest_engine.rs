// backtest_engine.rs - Yüksek Performanslı Otonom Simülasyon Motoru

use crate::core::types::{Candle, StrategyParams, Signal};
use crate::robot::order_management::{OrderBookSimulator, SyntheticBookConfig};
use crate::robot::strategies::base::Strategy;
use crate::Result;
use crate::MemosTradingError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// --- 1. YAPILANDIRMA VE VERİ MODELLERİ ---

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BacktestConfig {
    pub symbol: String,
    pub interval: String,
    pub initial_balance: f64,
    pub max_position_size: f64,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub strategy_name: String,
    pub strategy_params: Option<StrategyParams>,
    #[serde(default = "default_commission")]
    pub commission_pct: f64,
    // Pozisyon Yönetimi (B1/B2/B3)
    pub breakeven_at_rr: Option<f64>,
    pub atr_trail_mult: Option<f64>,
    pub partial_tp_ratio: Option<f64>,
    pub position_profile: Option<String>,
    pub security_profile: Option<String>,
    /// Multi-TF hizalama: true ise backtest, base mumları HTF'e (get_htf_interval)
    /// toplayıp generate_signal'a YALNIZ tamamlanmış (look-ahead'siz) HTF dilimini
    /// verir → canlı motorun htf_trend_filter davranışıyla aynı. Default false
    /// (backward-compat: screener/A-B tek-TF kalır). Backtest job multi_tf.enabled
    /// ile açar. [[project_param_modularity]]
    #[serde(default)]
    pub use_htf: bool,
    /// Giriş kalitesi filtresi (#4): `Signal::Buy` üretilse bile pozisyon yalnız
    /// edge skoru bu eşiği aşarsa açılır → canlı `process_symbol_cycle`'ın
    /// `compute_edge_score >= edge_threshold` hunisini AYNALAR (tek-kaynak). Eskiden
    /// backtester her Buy'da açıyordu; canlı ise zayıf/ters-momentum girişleri
    /// reddediyordu → 1m'de backtest aşırı-işlem + komisyon erozyonu gösterip param
    /// aramasını yanıltıyordu. `None` → filtre yok (legacy, backward-compat: screener/
    /// A-B/eski testler). `Some(t)` → `Engine::compute_edge_score_with(window, Buy,
    /// ml=0, penalty=0.4) >= t` şartı. Canlı cold-start eşiği `dynamic_edge_threshold(0)
    /// = 0.20`. Backtest job env `BACKTEST_EDGE_FILTER` ile doldurur.
    #[serde(default)]
    pub edge_min_score: Option<f64>,
    /// Opt-in orderbook icrası (#c): `Some(profil)` ise giriş/çıkış emirleri
    /// `candle.close` etrafında üretilen sentetik L2 deftere karşı doldurulur →
    /// slippage gerçekçiliği (canlı paper yolu `OrderBookSimulator`'la aynı motor).
    /// Profil: `"illiquid"` (geniş spread/sığ derinlik) · diğer (`"liquid"`) likit.
    /// `None` → fill = close (legacy, slippage'sız). Backtest job env
    /// `BACKTEST_ORDERBOOK` ile doldurur. [[regime_context]]
    #[serde(default)]
    pub orderbook_sim: Option<String>,
}

fn default_commission() -> f64 { 0.001 }

/// HTF interval → bucket saniyesi (look-ahead'siz HTF dilimleme için). Bilinmeyen → 0.
fn htf_bucket_secs(interval: &str) -> i64 {
    match interval {
        "1m" => 60, "5m" => 300, "15m" => 900, "30m" => 1800,
        "1h" => 3600, "4h" => 14_400, "1d" => 86_400,
        _ => 0,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedTrade {
    pub symbol: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub entry_time: String,
    pub exit_time: String,
    pub amount: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub duration_minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub symbol: String,
    pub strategy: String,
    pub total_trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub total_pnl_pct: f64,
    pub max_drawdown_pct: f64,
    pub profit_factor: f64,
    pub sharpe_ratio: f64,
    pub trades: Vec<SimulatedTrade>,
}

// --- 2. ANA SİMÜLASYON MOTORU ---

struct BacktestPos {
    entry_price: f64,
    entry_idx: usize,
    entry_ts: DateTime<Utc>,
    qty: f64,
    sl_price: f64,
    tp_price: f64,
    risk_distance: f64,
    best_price: f64,
    trailing_pct: Option<f64>,
    trailing_sl: Option<f64>,
    breakeven_triggered: bool,
    partial_tp_triggered: bool,
}

pub struct Backtester {
    config: BacktestConfig,
    trades: Vec<SimulatedTrade>,
    balance_history: Vec<(DateTime<Utc>, f64)>,
}

impl Backtester {
    pub fn new(config: BacktestConfig) -> Self {
        Self {
            config,
            trades: Vec::with_capacity(100),
            balance_history: Vec::with_capacity(1000),
        }
    }

    pub fn run(&mut self, candles: &[Candle]) -> Result<BacktestResult> {
        if candles.is_empty() {
            return Err(MemosTradingError::Strategy("Mum verisi yok".to_owned()));
        }

        let mut balance = self.config.initial_balance;
        let mut pos: Option<BacktestPos> = None;
        let mut max_balance = balance;
        let mut max_drawdown: f64 = 0.0;

        // Mumları zaman sırasına sok (Emanet kopyalama yerine referans kullanımı)
        let mut sorted = candles.to_vec();
        sorted.sort_by_key(|c| c.timestamp);

        // Sinyal kaynağı: canlı motorla AYNI Strategy trait'i (tek-kaynak). Eskiden
        // should_open hardcoded basit bir reimplementasyondu ve cfg.strategy_params'ı
        // YOK SAYIYORDU → param_spec araması düz zeminde etkisizdi. Strateji bir kez
        // kurulur (per-bar alloc yok); generate_signal cfg.strategy_params ile çağrılır.
        let entry_strat = crate::robot::strategies::default_registry()
            .make(&self.config.strategy_name);
        let entry_params = self.config.strategy_params.unwrap_or_default();

        // Multi-TF hizalama (use_htf): base seriyi HTF'e bir kez topla. Her bara
        // YALNIZ tamamlanmış HTF mumları verilir (forming bucket dışlanır) → canlı
        // load_htf_candles davranışıyla look-ahead'siz hizalı. bucket_secs slicing için.
        let (htf_full, htf_bucket_secs) = if self.config.use_htf {
            let htf_int = crate::robot::data_pipeline::orchestrator::DataPipeline
                ::get_htf_interval(&self.config.interval);
            let agg = crate::robot::data_pipeline::aggregate_to(&sorted, htf_int, &self.config.symbol);
            let bs = htf_bucket_secs(htf_int);
            (agg, bs)
        } else {
            (Vec::new(), 0)
        };

        for (idx, candle) in sorted.iter().enumerate() {
            let mut close_signal = false;
            let mut trade_net = 0.0;

            if let Some(ref mut p) = pos {
                // B2: Trailing Stop Güncelleme
                if let Some(trail_pct) = p.trailing_pct {
                    p.best_price = p.best_price.max(candle.close);
                    let new_trail = p.best_price * (1.0 - trail_pct / 100.0);
                    p.trailing_sl = Some(p.trailing_sl.unwrap_or(0.0).max(new_trail));
                }

                let eff_sl = p.trailing_sl.unwrap_or(p.sl_price).max(p.sl_price);

                // B1: Breakeven (Başabaş) Kontrolü
                if !p.breakeven_triggered {
                    if let Some(be_rr) = self.config.breakeven_at_rr {
                        if candle.close - p.entry_price >= be_rr * p.risk_distance {
                            p.sl_price = p.entry_price;
                            p.breakeven_triggered = true;
                        }
                    }
                }

                // B3: Kısmi Kar Al (Partial TP)
                if !p.partial_tp_triggered {
                    if let Some(ratio) = self.config.partial_tp_ratio {
                        let partial_threshold = p.entry_price + (p.tp_price - p.entry_price) * 0.5;
                        if candle.close >= partial_threshold {
                            let p_qty = p.qty * ratio;
                            // Çıkış = market SELL → orderbook açıksa slippage'li avg fill.
                            let exit_fill = self.sim_fill_price(false, candle.close, p_qty);
                            let fee = p_qty * (p.entry_price + exit_fill) * self.config.commission_pct;
                            let net = (exit_fill - p.entry_price) * p_qty - fee;

                            self.trades.push(self.create_sim_trade(p, candle, exit_fill, p_qty, net));
                            balance += net;
                            p.qty -= p_qty;
                            p.partial_tp_triggered = true;
                        }
                    }
                }

                // Tam Çıkış Kontrolü (SL veya TP)
                if candle.close >= p.tp_price || candle.close <= eff_sl {
                    // Tetik candle.close'da; gerçekleşen çıkış market SELL fill'i (slippage).
                    let exit_fill = self.sim_fill_price(false, candle.close, p.qty);
                    let fee = p.qty * (p.entry_price + exit_fill) * self.config.commission_pct;
                    trade_net = (exit_fill - p.entry_price) * p.qty - fee;
                    self.trades.push(self.create_sim_trade(p, candle, exit_fill, p.qty, trade_net));
                    close_signal = true;
                }
            }

            if close_signal {
                balance += trade_net;
                pos = None;
            }

            // Stratejik Giriş Kontrolü
            if pos.is_none() && Self::entry_long_signal(
                entry_strat.as_ref(), &entry_params, &sorted, idx, &htf_full, htf_bucket_secs,
                self.config.edge_min_score,
            ) {
                // Giriş = market BUY → orderbook açıksa slippage'li avg fill (yoksa close).
                let entry = self.sim_fill_price(true, candle.close, self.config.max_position_size);
                let sl = entry * (1.0 - self.config.stop_loss_pct / 100.0);
                let trail_pct = self.config.atr_trail_mult.map(|m| Self::calc_atr_pct(&sorted[..=idx]) * m);
                
                pos = Some(BacktestPos {
                    entry_price: entry,
                    entry_idx: idx,
                    entry_ts: candle.timestamp,
                    qty: self.config.max_position_size,
                    sl_price: sl,
                    tp_price: entry * (1.0 + self.config.take_profit_pct / 100.0),
                    risk_distance: (entry - sl).abs().max(f64::EPSILON),
                    best_price: entry,
                    trailing_pct: trail_pct,
                    trailing_sl: None,
                    breakeven_triggered: false,
                    partial_tp_triggered: false,
                });
            }

            // Risk & Bakiye Takibi
            max_balance = max_balance.max(balance);
            let current_val = balance + pos.as_ref().map_or(0.0, |p| (candle.close - p.entry_price) * p.qty);
            max_drawdown = max_drawdown.max((max_balance - current_val) / max_balance * 100.0);
            self.balance_history.push((candle.timestamp, balance));
        }

        self.finalize_result(balance, max_drawdown)
    }

    /// `exit_price`: gerçekleşen çıkış fiyatı (orderbook açıkken slippage'li avg fill,
    /// kapalıyken candle.close). PnL ve raporlama bu fiyattan hesaplanır.
    fn create_sim_trade(&self, p: &BacktestPos, c: &Candle, exit_price: f64, qty: f64, net: f64) -> SimulatedTrade {
        SimulatedTrade {
            symbol: c.symbol.clone(),
            entry_price: p.entry_price,
            exit_price,
            entry_time: p.entry_ts.to_rfc3339(),
            exit_time: c.timestamp.to_rfc3339(),
            amount: qty,
            pnl: net,
            pnl_pct: (net / (p.entry_price * qty + f64::EPSILON)) * 100.0,
            duration_minutes: (c.timestamp - p.entry_ts).num_minutes(),
        }
    }

    /// Opt-in orderbook icra fiyatı (#c). `orderbook_sim` `Some(profil)` ise emir,
    /// `mid` (=candle.close) etrafında üretilen sentetik L2 deftere karşı doldurulur;
    /// dönen `avg_fill_price` slippage içerir (BUY ≥ mid, SELL ≤ mid). `None` → `mid`
    /// (legacy, slippage'sız). Kısmi-doldurma olsa bile avg fill kullanılır (backtester
    /// sabit-qty varsayar — basitleştirme). Geçersiz mid/qty → `mid`.
    fn sim_fill_price(&self, is_buy: bool, mid: f64, qty: f64) -> f64 {
        match self.config.orderbook_sim.as_deref() {
            None => mid,
            Some(profile) => {
                if mid <= 0.0 || qty <= 0.0 { return mid; }
                let cfg = if profile.eq_ignore_ascii_case("illiquid") {
                    SyntheticBookConfig::illiquid(mid)
                } else {
                    SyntheticBookConfig::liquid(mid)
                };
                let sim = OrderBookSimulator::new(cfg);
                let fr = if is_buy { sim.simulate_buy(qty) } else { sim.simulate_sell(qty) };
                if fr.filled_qty > 0.0 && fr.avg_fill_price > 0.0 { fr.avg_fill_price } else { mid }
            }
        }
    }

    fn finalize_result(&self, balance: f64, max_dd: f64) -> Result<BacktestResult> {
        let total_pnl = balance - self.config.initial_balance;
        let win_count = self.trades.iter().filter(|t| t.pnl > 0.0).count();

        // Profit factor = brüt kâr / brüt zarar (gerçek hesap; eski hardcode 1.5 idi).
        // Zarar yokken kâr varsa anlamlı bir tavan (999) — INF JSON'da sorun çıkarır.
        let gross_profit: f64 = self.trades.iter().filter(|t| t.pnl > 0.0).map(|t| t.pnl).sum();
        let gross_loss: f64   = self.trades.iter().filter(|t| t.pnl < 0.0).map(|t| -t.pnl).sum();
        let profit_factor = if gross_loss > f64::EPSILON { gross_profit / gross_loss }
            else if gross_profit > 0.0 { 999.0 } else { 0.0 };

        // Per-trade Sharpe = ortalama getiri / getiri std (gerçek hesap; eski hardcode 2.0).
        // sqrt(n) ölçeklemesi YOK — A/B karşılaştırmasında trade sayısı farklı olabilir,
        // bu yüzden trade-başına risk-ayarlı getiri daha adil bir kıyas metriğidir.
        let rets: Vec<f64> = self.trades.iter().map(|t| t.pnl_pct).collect();
        let n = rets.len();
        let sharpe_ratio = if n >= 2 {
            let mean = rets.iter().sum::<f64>() / n as f64;
            let var = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
            let sd = var.sqrt();
            if sd > f64::EPSILON { mean / sd } else { 0.0 }
        } else { 0.0 };

        Ok(BacktestResult {
            symbol: self.config.symbol.clone(),
            strategy: self.config.strategy_name.clone(),
            total_trades: self.trades.len(),
            win_rate: (win_count as f64 / self.trades.len().max(1) as f64) * 100.0,
            total_pnl,
            total_pnl_pct: (total_pnl / self.config.initial_balance) * 100.0,
            max_drawdown_pct: max_dd,
            profit_factor,
            sharpe_ratio,
            trades: self.trades.clone(),
        })
    }

    // --- 3. TEKNİK ANALİZ VE STRATEJİ MATRİSİ ---

    /// Gerçek strateji sinyali (canlı motorla tek-kaynak): `Signal::Buy` → long aç.
    /// Backtester long-only olduğundan Sell/Hold → giriş yok. `params` =
    /// cfg.strategy_params (param_spec araması bunu doldurur) → indikatör periyot/
    /// eşikleri artık backtest'i GERÇEKTEN etkiler.
    ///
    /// Pencere son `W` bara sınırlanır: stratejilerin azami lookback'i (ICT_COMPOSITE
    /// ~66 bar) çok altında kalır, böylece per-bar maliyet O(W) sabit → derin seride
    /// (BACKTEST_CANDLE_LIMIT yüksek) bile backtest O(n·W), O(n²) değil.
    ///
    /// `edge_min` (#4 giriş kalitesi): `Some(t)` ise Buy sinyali ek olarak edge skoru
    /// eşiğini geçmeli — canlı `process_symbol_cycle`'daki `edge < edge_threshold ⇒
    /// REDDEDİLDİ` kapısının BİREBİR aynısı (`compute_edge_score_with`, ml_confidence=0
    /// çünkü harness'te GBT yok, ters-momentum cezası 0.4 = canlı default). Böylece
    /// param araması zayıf/ters-momentum girişlerini canlıyla aynı şekilde eler.
    /// `None` → filtre yok (legacy).
    fn entry_long_signal(
        strat: &dyn Strategy,
        params: &StrategyParams,
        candles: &[Candle],
        idx: usize,
        htf_full: &[Candle],
        htf_bucket_secs: i64,
        edge_min: Option<f64>,
    ) -> bool {
        const W: usize = 200;
        if idx < 20 { return false; }
        let start = (idx + 1).saturating_sub(W);
        let window = &candles[start..=idx];
        // HTF dilimi: yalnız o anki bara göre TAMAMLANMIŞ bucket'lar (forming hariç →
        // look-ahead yok). htf_full sıralı; forming bucket başlangıcından öncekiler alınır.
        let htf: Option<&[Candle]> = if htf_bucket_secs > 0 && !htf_full.is_empty() {
            let cur_bucket_start = candles[idx].timestamp.timestamp()
                .div_euclid(htf_bucket_secs) * htf_bucket_secs;
            let n = htf_full.partition_point(|c| c.timestamp.timestamp() < cur_bucket_start);
            if n > 0 { Some(&htf_full[..n]) } else { None }
        } else {
            None
        };
        if !matches!(strat.generate_signal(window, params, None, htf), Ok(Signal::Buy)) {
            return false;
        }
        // Giriş kalitesi kapısı: canlı motorla tek-kaynak edge hunisi.
        match edge_min {
            Some(t) => {
                let edge = crate::robot::engines::Engine::compute_edge_score_with(
                    window, &Signal::Buy, 0.0, 0.4,
                );
                edge >= t
            }
            None => true,
        }
    }

    fn calc_atr_pct(candles: &[Candle]) -> f64 {
        let n = candles.len();
        if n < 2 { return 1.0; }
        let tr = (candles[n-1].high - candles[n-1].low)
            .max((candles[n-1].high - candles[n-2].close).abs());
        (tr / candles[n-1].close) * 100.0
    }
}

#[cfg(test)]
mod edge_filter_tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    /// Sürüklenme + salınımlı sentetik seri → MA_CROSSOVER (DEFAULT) tekrar tekrar
    /// Buy üretir, fiyat yükseldiğinden TP devreye girip pozisyonlar döner.
    fn synthetic_uptrend(n: usize) -> Vec<Candle> {
        (0..n).map(|i| {
            let f = i as f64;
            let close = 100.0 + 0.05 * f + 6.0 * (f * 0.3).sin();
            Candle {
                timestamp: Utc.timestamp_opt(1_700_000_000 + (i as i64) * 3600, 0).unwrap(),
                open: close,
                high: close * 1.004,
                low: close * 0.996,
                close,
                volume: 1_000.0,
                symbol: "TEST".into(),
                interval: "1h".into(),
            }
        }).collect()
    }

    fn cfg(edge_min: Option<f64>) -> BacktestConfig {
        BacktestConfig {
            symbol: "TEST".into(),
            interval: "1h".into(),
            initial_balance: 10_000.0,
            max_position_size: 1.0,
            take_profit_pct: 3.0,
            stop_loss_pct: 1.5,
            strategy_name: "DEFAULT".into(),
            strategy_params: None,
            commission_pct: 0.0004,
            breakeven_at_rr: Some(1.0),
            atr_trail_mult: Some(2.0),
            partial_tp_ratio: None,
            position_profile: None,
            security_profile: None,
            use_htf: false,
            edge_min_score: edge_min,
            orderbook_sim: None,
        }
    }

    fn n_trades(edge_min: Option<f64>, candles: &[Candle]) -> usize {
        Backtester::new(cfg(edge_min)).run(candles).unwrap().total_trades
    }

    #[test]
    fn sim_fill_price_none_is_mid_some_adds_slippage() {
        // orderbook_sim=None → fill=mid (legacy). Some → BUY≥mid, SELL≤mid; illiquid
        // likitten daha kötü (geniş spread). sim_fill_price private → bu modülden erişilir.
        let mid = 100.0;
        let bt_off = Backtester::new(cfg(None));
        assert_eq!(bt_off.sim_fill_price(true, mid, 1.0), mid, "None → mid (slippage yok)");

        let mut c_liq = cfg(None); c_liq.orderbook_sim = Some("liquid".into());
        let bt_liq = Backtester::new(c_liq);
        let buy_liq  = bt_liq.sim_fill_price(true,  mid, 1.0);
        let sell_liq = bt_liq.sim_fill_price(false, mid, 1.0);
        assert!(buy_liq  >= mid, "BUY fill ≥ mid (slippage yukarı): {buy_liq}");
        assert!(sell_liq <= mid, "SELL fill ≤ mid (slippage aşağı): {sell_liq}");

        let mut c_illq = cfg(None); c_illq.orderbook_sim = Some("illiquid".into());
        let bt_illq = Backtester::new(c_illq);
        let buy_illq = bt_illq.sim_fill_price(true, mid, 1.0);
        assert!(buy_illq >= buy_liq, "illiquid BUY slippage'i ≥ liquid: {buy_illq} vs {buy_liq}");

        // Geçersiz mid → mid (guard).
        assert_eq!(bt_illq.sim_fill_price(true, 0.0, 1.0), 0.0);
    }

    #[test]
    fn none_means_legacy_with_trades() {
        // edge_min_score=None → eski davranış: filtre yok, trend serisinde işlem üretir.
        let candles = synthetic_uptrend(400);
        assert!(n_trades(None, &candles) > 0, "filtre kapalı baz işlem üretmeli");
    }

    #[test]
    fn threshold_above_max_blocks_all_entries() {
        // Edge skoru [0,1]'e clamp'li → eşik 2.0 hiçbir zaman aşılmaz → 0 işlem.
        // Kapının gerçekten Buy'ı kestiğini kanıtlar (yalnız wiring değil semantik).
        let candles = synthetic_uptrend(400);
        assert_eq!(n_trades(Some(2.0), &candles), 0, "eşik>1.0 tüm girişleri engellemeli");
    }

    #[test]
    fn filter_changes_behavior_and_is_deterministic() {
        // Canlı cold-start eşiği (0.20) işlem sayısını filtresizden FARKLI kılmalı
        // (filtre fiilen devrede) ve iki koşu birebir aynı olmalı (determinizm).
        let candles = synthetic_uptrend(400);
        let none = n_trades(None, &candles);
        let f1 = n_trades(Some(0.20), &candles);
        let f2 = n_trades(Some(0.20), &candles);
        assert_eq!(f1, f2, "backtest deterministik olmalı");
        assert_ne!(f1, none, "0.20 eşiği filtresizden farklı bir giriş seti vermeli");
    }
}
