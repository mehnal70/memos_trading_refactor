// multi_tf_ab.rs — Çoklu-TF seed düzeneği için A/B doğrulama harness'i.
//
// Soru: bir sembolün TEK (top-PF) edge'ini koşmak (Single/A) vs TÜM WF-onaylı (TF,strateji)
// edge'lerini tek-pozisyon arbitrasyonuyla koşmak (Multi/B) NET olarak daha mı iyi? Canlı
// `EDGE_SEED_MULTI_TF`'i açmadan önce backtest A/B ile ölçülür ([[feedback_autonomy_first]],
// [[project_autonomy_backlog]]). Model: her iz için backtester'ı KENDİ TF mumunda koş → trade
// listesi (giriş/çıkış zamanı); Multi kolu bu trade'leri ÇAKIŞMASIZ greedy birleştirir
// (Approach A'nın sadık modeli: sembol başına tek pozisyon, flat'ken ilk tetikleyen açar).
// A ve B AYNI per-iz config'leri kullanır → sistematik in-sample optimizmi iki kolda da eşit
// (mekanizma karşılaştırması, mutlak-edge iddiası değil). [[project_edge_scan]].

use chrono::{DateTime, Utc};
use std::collections::HashMap;

use super::backtest_engine::{Backtester, BacktestConfig, DirectionMode, RegimeGate};
use super::edge_scan::{EdgeRow, EdgeScanReport, SeedRobustness, passes_seed_bar};

/// Bir izin tek trade'i — arbitrasyon için zaman penceresi + getiri + iz önceliği.
#[derive(Debug, Clone)]
pub struct TradeSlot {
    pub entry: DateTime<Utc>,
    pub exit: DateTime<Utc>,
    pub pnl_pct: f64,
    /// 0 = en yüksek PF iz (eşit girişte öncelik).
    pub track_idx: usize,
}

/// Bir kolun (Single/Multi) trade kümesi metrikleri.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ArmMetrics {
    pub trades: usize,
    pub wins: usize,
    pub win_rate: f64,
    /// Σ trade getiri% (basit toplam — kollar arası kıyas için yeterli, bileşik değil).
    pub sum_pnl_pct: f64,
    pub avg_pnl_pct: f64,
    /// Σ kazanç / |Σ kayıp|. Kayıp yoksa +∞ (kazanç varsa) ya da 0.
    pub profit_factor: f64,
}

/// TEK-POZİSYON ARBİTRASYONU (saf, testli): tüm izlerin trade'lerini birleştir, GİRİŞ zamanına
/// göre sırala, ÇAKIŞMASIZ greedy kabul et — bir trade yalnız önceki kabul edilenin ÇIKIŞINDAN
/// sonra (≥) başlıyorsa alınır (flat'ken aç). Eşit girişte düşük track_idx (yüksek PF) öncelik.
/// Bu, Approach A'nın (sembol başına tek pozisyon, hızlı TF frekansı artırır) sadık modelidir.
pub fn arbitrate_single_position(mut slots: Vec<TradeSlot>) -> Vec<TradeSlot> {
    slots.sort_by(|a, b| a.entry.cmp(&b.entry).then(a.track_idx.cmp(&b.track_idx)));
    let mut accepted: Vec<TradeSlot> = Vec::new();
    let mut busy_until: Option<DateTime<Utc>> = None;
    for s in slots {
        if busy_until.is_none_or(|bu| s.entry >= bu) {
            busy_until = Some(s.exit);
            accepted.push(s);
        }
    }
    accepted
}

/// Bir trade kümesinin kol-metriklerini hesaplar (saf, testli).
pub fn arm_metrics(slots: &[TradeSlot]) -> ArmMetrics {
    let trades = slots.len();
    let wins = slots.iter().filter(|s| s.pnl_pct > 0.0).count();
    let gross_win: f64 = slots.iter().filter(|s| s.pnl_pct > 0.0).map(|s| s.pnl_pct).sum();
    let gross_loss: f64 = slots.iter().filter(|s| s.pnl_pct < 0.0).map(|s| -s.pnl_pct).sum();
    let sum: f64 = slots.iter().map(|s| s.pnl_pct).sum();
    ArmMetrics {
        trades,
        wins,
        win_rate: if trades > 0 { wins as f64 / trades as f64 } else { 0.0 },
        sum_pnl_pct: sum,
        avg_pnl_pct: if trades > 0 { sum / trades as f64 } else { 0.0 },
        profit_factor: if gross_loss > 0.0 { gross_win / gross_loss }
            else if gross_win > 0.0 { f64::INFINITY } else { 0.0 },
    }
}

/// A/B koşum ayarları (per-iz config'lere uygulanır; A ve B'de AYNI → kıyas adil).
#[derive(Debug, Clone)]
pub struct AbConfig {
    pub initial_balance: f64,
    pub commission_pct: f64,
    pub direction: DirectionMode,
    pub edge_min_score: Option<f64>,
    /// Seri başına yüklenecek azami mum (read_candles_market limiti).
    pub candle_limit: usize,
    pub breakeven_at_rr: Option<f64>,
    pub atr_trail_mult: Option<f64>,
}

impl Default for AbConfig {
    fn default() -> Self {
        Self {
            initial_balance: 10_000.0,
            commission_pct: 0.0004,
            direction: DirectionMode::RegimeDirectional, // canlı varsayılanına yakın
            edge_min_score: Some(0.20),                  // canlı cold-start huni eşiği
            candle_limit: 5000,
            breakeven_at_rr: Some(1.0),
            atr_trail_mult: Some(2.0),
        }
    }
}

/// Bir EdgeRow (iz) için backtester config'i — satırın optimize TP/SL/PS + strateji/interval'i,
/// ortak A/B knob'ları. A ve B aynı config'i kullanır → fark yalnız tek-iz vs çoklu-iz arbitrasyonu.
fn track_config(symbol: &str, row: &EdgeRow, ab: &AbConfig) -> BacktestConfig {
    BacktestConfig {
        symbol: symbol.to_string(),
        interval: row.interval.clone(),
        initial_balance: ab.initial_balance,
        max_position_size: row.max_position_size,
        take_profit_pct: row.take_profit_pct,
        stop_loss_pct: row.stop_loss_pct,
        strategy_name: row.best_strategy.clone(),
        strategy_params: None,
        commission_pct: ab.commission_pct,
        breakeven_at_rr: ab.breakeven_at_rr,
        atr_trail_mult: ab.atr_trail_mult,
        partial_tp_ratio: None,
        position_profile: None,
        security_profile: None,
        use_htf: false,
        edge_min_score: ab.edge_min_score,
        orderbook_sim: None,
        regime_gate: RegimeGate::Off,
        direction: ab.direction,
        atr_sl_mult: None,
        atr_tp_mult: None,
        vol_target_pct: None,
    }
}

/// Bir sembolün izlerini (rows: PF-azalan, ilk = top-PF anchor) backtest edip Single/Multi
/// kollarını üretir. `load`: (symbol, interval, market) → mumlar (DB'den ayrıştırılması test
/// edilebilsin diye closure). track_idx = rows sırası (0 = anchor).
pub fn run_symbol_ab<F>(symbol: &str, rows: &[EdgeRow], ab: &AbConfig, load: &F) -> SymbolAb
where
    F: Fn(&str, &str, &str) -> Vec<crate::core::types::Candle>,
{
    let mut per_track: Vec<Vec<TradeSlot>> = Vec::with_capacity(rows.len());
    for (idx, row) in rows.iter().enumerate() {
        let candles = load(symbol, &row.interval, &row.market);
        let slots = if candles.is_empty() {
            Vec::new()
        } else {
            match Backtester::new(track_config(symbol, row, ab)).run(&candles) {
                Ok(res) => res.trades.iter().filter_map(|t| {
                    let entry = DateTime::parse_from_rfc3339(&t.entry_time).ok()?.with_timezone(&Utc);
                    let exit = DateTime::parse_from_rfc3339(&t.exit_time).ok()?.with_timezone(&Utc);
                    Some(TradeSlot { entry, exit, pnl_pct: t.pnl_pct, track_idx: idx })
                }).collect(),
                Err(_) => Vec::new(),
            }
        };
        per_track.push(slots);
    }
    // Single (A): yalnız anchor izi (top-PF). Multi (B): tüm izlerin arbitrasyonu.
    let single_slots = per_track.first().cloned().unwrap_or_default();
    let all: Vec<TradeSlot> = per_track.into_iter().flatten().collect();
    let multi_slots = arbitrate_single_position(all);
    SymbolAb {
        symbol: symbol.to_string(),
        n_tracks: rows.len(),
        tracks: rows.iter().map(|r| (r.interval.clone(), r.best_strategy.clone(), r.profit_factor)).collect(),
        single: arm_metrics(&single_slots),
        multi: arm_metrics(&multi_slots),
    }
}

/// Sembol-başına A/B sonucu.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SymbolAb {
    pub symbol: String,
    pub n_tracks: usize,
    /// (interval, strateji, PF) izler — PF azalan.
    pub tracks: Vec<(String, String, f64)>,
    pub single: ArmMetrics,
    pub multi: ArmMetrics,
}

/// Tüm A/B raporu (sembol-başına + portföy toplamı).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AbReport {
    pub per_symbol: Vec<SymbolAb>,
    pub single_total: ArmMetrics,
    pub multi_total: ArmMetrics,
}

/// Bir edge_scan raporundan ÇOKLU-İZ A/B'sini koşar: robustluk barını geçen, >1 izli sembolleri
/// seçer (tek-izli sembol A/B'ye katkısız), her izi DB mumunda backtest eder, Single vs Multi
/// portföy metriklerini toplar. `max_tracks` ile bounded. DB okuma `read_candles_market` ile.
pub fn run_multi_tf_ab(
    report: &EdgeScanReport,
    r: SeedRobustness,
    max_tracks: usize,
    db_path: &str,
    ab: &AbConfig,
) -> AbReport {
    // Sembol → barı geçen satırlar (PF azalan, max_tracks bounded).
    let mut by_symbol: HashMap<String, Vec<EdgeRow>> = HashMap::new();
    for row in &report.rows {
        if !passes_seed_bar(row, &r) { continue; }
        by_symbol.entry(row.symbol.clone()).or_default().push(row.clone());
    }
    let limit = ab.candle_limit;
    let load = |sym: &str, iv: &str, mk: &str| -> Vec<crate::core::types::Candle> {
        crate::persistence::reader::read_candles_market(db_path, sym, iv, mk, limit).unwrap_or_default()
    };

    let mut per_symbol: Vec<SymbolAb> = Vec::new();
    for (sym, mut rows) in by_symbol {
        rows.sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap_or(std::cmp::Ordering::Equal));
        // (market,interval,strategy) dedup — aynı izin birden çok serisi olmasın.
        rows.dedup_by(|a, b| a.market == b.market && a.interval == b.interval && a.best_strategy == b.best_strategy);
        rows.truncate(max_tracks.max(1));
        if rows.len() < 2 { continue; } // tek-iz → çoklu-TF düzeneğine katkısız
        per_symbol.push(run_symbol_ab(&sym, &rows, ab, &load));
    }
    per_symbol.sort_by(|a, b| a.symbol.cmp(&b.symbol));

    // Portföy toplamı = sembollerin metriklerinin (trade-ağırlıklı) toplamı.
    let single_total = sum_arms(per_symbol.iter().map(|s| &s.single));
    let multi_total = sum_arms(per_symbol.iter().map(|s| &s.multi));
    AbReport { per_symbol, single_total, multi_total }
}

/// Kol-metriklerini toplar (trade sayıları + Σpnl ekle; oranları toplamdan yeniden hesapla).
fn sum_arms<'a>(arms: impl Iterator<Item = &'a ArmMetrics>) -> ArmMetrics {
    let mut trades = 0usize;
    let mut wins = 0usize;
    let mut sum = 0.0f64;
    // PF'yi yeniden hesaplamak için kazanç/kayıp ayrımı gerek; ArmMetrics'te ayrı tutmadığımız
    // için avg×trades ile sum elde edip win_rate'i wins/trades'ten türetiyoruz. PF toplamda
    // sembol-PF'lerinin trade-ağırlıklı temsili değil → portföy PF'i için sum_pnl + win_rate
    // yeterli sinyal; PF alanını burada NaN-güvenli 0 bırakıyoruz (per-symbol PF'ler raporda).
    for a in arms {
        trades += a.trades;
        wins += a.wins;
        sum += a.sum_pnl_pct;
    }
    ArmMetrics {
        trades,
        wins,
        win_rate: if trades > 0 { wins as f64 / trades as f64 } else { 0.0 },
        sum_pnl_pct: sum,
        avg_pnl_pct: if trades > 0 { sum / trades as f64 } else { 0.0 },
        profit_factor: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn slot(track: usize, e: i64, x: i64, pnl: f64) -> TradeSlot {
        TradeSlot {
            entry: Utc.timestamp_opt(e, 0).single().unwrap(),
            exit: Utc.timestamp_opt(x, 0).single().unwrap(),
            pnl_pct: pnl, track_idx: track,
        }
    }

    #[test]
    fn arbitrate_rejects_overlap_and_keeps_more_frequent_nonoverlapping() {
        // İz0 (1d): 1 uzun trade [0,100]. İz1 (1h): 3 kısa trade — [10,20] (çakışır, RED),
        // [110,120] (boşta, KABUL), [125,130] (boşta, KABUL).
        let slots = vec![
            slot(0, 0, 100, 5.0),
            slot(1, 10, 20, 1.0),
            slot(1, 110, 120, 2.0),
            slot(1, 125, 130, -1.0),
        ];
        let acc = arbitrate_single_position(slots);
        // [0,100] kabul → [10,20] RED (çakışır) → [110,120] kabul → [125,130] kabul = 3 trade.
        assert_eq!(acc.len(), 3);
        let entries: Vec<i64> = acc.iter().map(|s| s.entry.timestamp()).collect();
        assert_eq!(entries, vec![0, 110, 125], "çakışan elenir, çakışmayan hızlı izler eklenir");
    }

    #[test]
    fn arbitrate_tie_prefers_higher_pf_track() {
        // Aynı girişte iki iz: düşük track_idx (yüksek PF) öncelik; diğeri çakışır → RED.
        let acc = arbitrate_single_position(vec![
            slot(1, 0, 50, 1.0),
            slot(0, 0, 50, 9.0),
        ]);
        assert_eq!(acc.len(), 1);
        assert_eq!(acc[0].track_idx, 0, "eşit girişte yüksek-PF (track 0) kazanır");
    }

    #[test]
    fn arm_metrics_pf_and_winrate() {
        let m = arm_metrics(&[slot(0,0,1,3.0), slot(0,2,3,-1.0), slot(0,4,5,1.0)]);
        assert_eq!(m.trades, 3);
        assert_eq!(m.wins, 2);
        assert!((m.win_rate - 2.0/3.0).abs() < 1e-9);
        assert!((m.sum_pnl_pct - 3.0).abs() < 1e-9);
        assert!((m.profit_factor - 4.0).abs() < 1e-9, "kazanç 4 / kayıp 1 = 4");
    }

    #[test]
    fn arm_metrics_no_loss_is_infinite_pf() {
        let m = arm_metrics(&[slot(0,0,1,2.0)]);
        assert!(m.profit_factor.is_infinite());
        let empty = arm_metrics(&[]);
        assert_eq!(empty.profit_factor, 0.0);
        assert_eq!(empty.trades, 0);
    }
}
