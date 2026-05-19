// src/robot/infra/reporting/trade_summary.rs
//
// Daily/weekly closed_trades raporu — `AppState.finance.live_closed_trades` üzerinden
// periyodik özet üretir ve `data/reports/` altına JSON olarak yazar.
//
// Üretilen alanlar: total PnL, win/loss, win_rate, profit_factor, avg_rr, best/worst,
// per-symbol breakdown, per-exit-reason breakdown.
//
// Akış:
//   1. spawn_trade_summary task'ı her TRADE_REPORT_EVERY_SECS (default 300) tetiklenir
//   2. O an açık günün ve haftanın özetini hesaplar (geçmiş raporları dokunmaz)
//   3. data/reports/daily_YYYY-MM-DD.json + data/reports/weekly_YYYY-WW.json yazılır
//
// Geçmiş raporlar idempotent: ertesi gün yeni dosya açılır, eskiler dokunulmaz.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::core::model::ClosedTradeModel;
use crate::robot::robotic_loop::AppState;

/// Period tipi: daily veya weekly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportPeriod {
    Daily,
    Weekly,
}

impl ReportPeriod {
    pub fn label(&self, start: DateTime<Utc>) -> String {
        match self {
            ReportPeriod::Daily  => format!("daily {}", start.format("%Y-%m-%d")),
            ReportPeriod::Weekly => format!("weekly {}-W{:02}",
                start.iso_week().year(), start.iso_week().week()),
        }
    }

    /// Period için dosya adı tabanı (uzantısız).
    pub fn file_stem(&self, start: DateTime<Utc>) -> String {
        match self {
            ReportPeriod::Daily  => format!("daily_{}", start.format("%Y-%m-%d")),
            ReportPeriod::Weekly => format!("weekly_{}-W{:02}",
                start.iso_week().year(), start.iso_week().week()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SymbolSummary {
    pub symbol: String,
    pub trades: usize,
    pub wins: usize,
    pub pnl_usd: f64,
    pub win_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExitSummary {
    pub reason: String,
    pub count: usize,
    pub pnl_usd: f64,
}

/// Bir döneme ait özet kapanış istatistikleri.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TradeSummary {
    /// "daily 2026-05-19" veya "weekly 2026-W21".
    pub period: String,
    /// Dönemin başlangıcı (kapsayıcı), RFC3339 UTC.
    pub start: String,
    /// Dönemin bitişi (dışlayıcı), RFC3339 UTC.
    pub end: String,
    pub total_trades: usize,
    pub wins: usize,
    pub losses: usize,
    /// Win sayısı / toplam (0..1). Toplam 0 ise 0.0.
    pub win_rate: f64,
    pub total_pnl_usd: f64,
    /// Kazanan işlemlerin ortalama PnL'i (USD). Win yoksa 0.
    pub avg_win_usd: f64,
    /// Kaybeden işlemlerin ortalama PnL'i (mutlak değer, USD). Loss yoksa 0.
    pub avg_loss_usd: f64,
    /// Σwin / Σ|loss|. Loss yoksa None (tanımsız); win+loss yoksa Some(0.0).
    pub profit_factor: Option<f64>,
    /// avg_win / avg_loss. Loss yoksa None; ikisi de 0 ise Some(0.0).
    pub avg_rr: Option<f64>,
    pub best_trade_pnl: f64,
    pub worst_trade_pnl: f64,
    pub by_symbol: Vec<SymbolSummary>,
    pub by_exit_reason: Vec<ExitSummary>,
    /// Bu raporun yazıldığı zaman (RFC3339 UTC).
    pub written_at: String,
}

impl TradeSummary {
    /// Verilen kapanış listesini [start, end) penceresine göre filtreleyip özetler.
    /// `start` ve `end` UTC. closed_at parse edilemeyen kayıtlar atlanır.
    pub fn aggregate(
        trades: &[ClosedTradeModel],
        period: ReportPeriod,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Self {
        // 1. Pencereye düşen kapanışları topla
        let in_window: Vec<&ClosedTradeModel> = trades.iter().filter(|t| {
            DateTime::parse_from_rfc3339(&t.closed_at)
                .ok()
                .map(|dt| {
                    let utc: DateTime<Utc> = dt.with_timezone(&Utc);
                    utc >= start && utc < end
                })
                .unwrap_or(false)
        }).collect();

        let total_trades = in_window.len();
        let mut sum_win: f64 = 0.0;
        let mut sum_loss_abs: f64 = 0.0;
        let mut wins: usize = 0;
        let mut losses: usize = 0;
        let mut best: f64 = f64::NEG_INFINITY;
        let mut worst: f64 = f64::INFINITY;

        // 2. Per-symbol ve per-exit-reason kovaları
        let mut sym_bucket: HashMap<String, (usize, usize, f64)> = HashMap::new(); // (trades, wins, pnl)
        let mut exit_bucket: HashMap<String, (usize, f64)> = HashMap::new();       // (count, pnl)

        for t in &in_window {
            if t.pnl > 0.0 { wins += 1; sum_win += t.pnl; }
            else if t.pnl < 0.0 { losses += 1; sum_loss_abs += -t.pnl; }
            if t.pnl > best  { best  = t.pnl; }
            if t.pnl < worst { worst = t.pnl; }

            let s = sym_bucket.entry(t.symbol.clone()).or_insert((0, 0, 0.0));
            s.0 += 1; if t.pnl > 0.0 { s.1 += 1; } s.2 += t.pnl;

            let e = exit_bucket.entry(t.exit_reason.clone()).or_insert((0, 0.0));
            e.0 += 1; e.1 += t.pnl;
        }

        // 3. Türetilmiş metrikler
        let total_pnl_usd: f64 = sum_win - sum_loss_abs;
        let win_rate = if total_trades > 0 { wins as f64 / total_trades as f64 } else { 0.0 };
        let avg_win_usd  = if wins > 0   { sum_win / wins as f64 } else { 0.0 };
        let avg_loss_usd = if losses > 0 { sum_loss_abs / losses as f64 } else { 0.0 };
        let profit_factor: Option<f64> = if sum_loss_abs > 0.0 {
            Some(sum_win / sum_loss_abs)
        } else if sum_win > 0.0 { None } else { Some(0.0) };
        let avg_rr: Option<f64> = if avg_loss_usd > 0.0 {
            Some(avg_win_usd / avg_loss_usd)
        } else if avg_win_usd > 0.0 { None } else { Some(0.0) };
        let best_trade_pnl  = if total_trades > 0 { best  } else { 0.0 };
        let worst_trade_pnl = if total_trades > 0 { worst } else { 0.0 };

        // 4. Kovaları sıralı Vec'e dök (deterministik output)
        let mut by_symbol: Vec<SymbolSummary> = sym_bucket.into_iter().map(|(symbol, (n, w, pnl))| {
            SymbolSummary {
                symbol,
                trades: n,
                wins: w,
                pnl_usd: pnl,
                win_rate: if n > 0 { w as f64 / n as f64 } else { 0.0 },
            }
        }).collect();
        by_symbol.sort_by(|a, b| b.pnl_usd.partial_cmp(&a.pnl_usd).unwrap_or(std::cmp::Ordering::Equal));

        let mut by_exit_reason: Vec<ExitSummary> = exit_bucket.into_iter().map(|(reason, (n, pnl))| {
            ExitSummary { reason, count: n, pnl_usd: pnl }
        }).collect();
        by_exit_reason.sort_by(|a, b| b.count.cmp(&a.count));

        TradeSummary {
            period: period.label(start),
            start: start.to_rfc3339(),
            end: end.to_rfc3339(),
            total_trades, wins, losses, win_rate,
            total_pnl_usd, avg_win_usd, avg_loss_usd,
            profit_factor, avg_rr,
            best_trade_pnl, worst_trade_pnl,
            by_symbol, by_exit_reason,
            written_at: Utc::now().to_rfc3339(),
        }
    }
}

/// O an içinde bulunulan günün [start, end) UTC penceresi.
pub fn current_day_window(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let date = now.date_naive();
    let start = utc_at_midnight(date);
    let end   = start + ChronoDuration::days(1);
    (start, end)
}

/// O an içinde bulunulan ISO haftasının [start, end) UTC penceresi (Pazartesi başlar).
pub fn current_week_window(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let date = now.date_naive();
    // ISO hafta: Pazartesi = weekday().num_days_from_monday() == 0
    let days_since_monday = date.weekday().num_days_from_monday() as i64;
    let monday = date - ChronoDuration::days(days_since_monday);
    let start = utc_at_midnight(monday);
    let end   = start + ChronoDuration::days(7);
    (start, end)
}

fn utc_at_midnight(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("00:00:00 geçerli"))
}

/// Bir özet'i dosyaya atomik yazar (.tmp → rename).
pub fn write_summary_to_file(summary: &TradeSummary, dir: &Path, period: ReportPeriod, start: DateTime<Utc>) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let file_name = format!("{}.json", period.file_stem(start));
    let target = dir.join(&file_name);
    let tmp = dir.join(format!("{}.tmp", &file_name));
    let json = serde_json::to_string_pretty(summary)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &target)?;
    Ok(target)
}

/// 📊 Periyodik closed_trades raporu task'ı.
/// `reports_dir` örn. "data/reports". `interval_secs` default 300 (5 dk).
/// Hem daily hem weekly özetini her tick'te idempotent olarak yeniden yazar.
pub fn spawn_trade_summary(
    state: Arc<Mutex<AppState>>,
    reports_dir: String,
    interval_secs: u64,
) {
    tokio::spawn(async move {
        let interval = Duration::from_secs(interval_secs.max(60));
        let dir = PathBuf::from(&reports_dir);
        let mut tick: u64 = 0;
        loop {
            let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
            if stop { break; }

            // Kapanış listesinin clone'unu kısa kilit altında al
            let trades: Vec<ClosedTradeModel> = {
                let st = match state.lock() { Ok(s) => s, Err(_) => break };
                st.finance.live_closed_trades.read()
                    .map(|v| v.clone())
                    .unwrap_or_default()
            };

            let now = Utc::now();
            let (d_start, d_end) = current_day_window(now);
            let (w_start, w_end) = current_week_window(now);
            let daily  = TradeSummary::aggregate(&trades, ReportPeriod::Daily,  d_start, d_end);
            let weekly = TradeSummary::aggregate(&trades, ReportPeriod::Weekly, w_start, w_end);

            let mut errors: Vec<String> = Vec::new();
            if let Err(e) = write_summary_to_file(&daily, &dir, ReportPeriod::Daily, d_start) {
                errors.push(format!("daily: {:?}", e));
            }
            if let Err(e) = write_summary_to_file(&weekly, &dir, ReportPeriod::Weekly, w_start) {
                errors.push(format!("weekly: {:?}", e));
            }

            // İlk tick'te başlama log'u; sonra sadece hata varsa (Telegram'a da yolla)
            if tick == 0 {
                if let Ok(mut st) = state.lock() {
                    if errors.is_empty() {
                        st.push_log(format!(
                            "📊 Trade summary yazıcı aktif: {} (her {}s) — daily+weekly",
                            reports_dir, interval_secs,
                        ));
                    } else {
                        st.push_alert(
                            "TRADE-SUMMARY-IO",
                            crate::robot::infra::telegram_notifier::Severity::Warning,
                            format!("[TRADE-SUMMARY-IO] yazma hatası: {}", errors.join(" · ")),
                        );
                    }
                }
            } else if !errors.is_empty() {
                if let Ok(mut st) = state.lock() {
                    st.push_alert(
                        "TRADE-SUMMARY-IO",
                        crate::robot::infra::telegram_notifier::Severity::Warning,
                        format!("[TRADE-SUMMARY-IO] {}", errors.join(" · ")),
                    );
                }
            }

            tick = tick.wrapping_add(1);
            tokio::time::sleep(interval).await;
        }
    });
}
