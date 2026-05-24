// Daily/Weekly Trade Summary Testleri
//
// TradeSummary::aggregate'in saf hesaplama mantığını ve dosya yazımının
// idempotent davranışını doğrular. Network/IO bağımlılığı yok.

use chrono::{Datelike, TimeZone, Utc};

use memos_trading_core::core::model::ClosedTradeModel;
use memos_trading_core::robot::infra::reporting::trade_summary::{
    current_day_window, current_week_window, write_summary_to_file,
    ReportPeriod, TradeSummary,
};

fn trade(symbol: &str, pnl: f64, exit_reason: &str, closed_at: &str) -> ClosedTradeModel {
    ClosedTradeModel {
        symbol: symbol.into(),
        is_long: true,
        pnl,
        pnl_pct: 0.0,
        exit_reason: exit_reason.into(),
        closed_at: closed_at.into(),
        opened_at: String::new(),
        leverage: 1.0,
    }
}

fn day_window(year: i32, month: u32, day: u32) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    let start = Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap();
    let end = start + chrono::Duration::days(1);
    (start, end)
}

#[test]
fn empty_trades_zero_stats() {
    let (start, end) = day_window(2026, 5, 19);
    let s = TradeSummary::aggregate(&[], ReportPeriod::Daily, start, end);
    assert_eq!(s.total_trades, 0);
    assert_eq!(s.wins, 0);
    assert_eq!(s.losses, 0);
    assert_eq!(s.win_rate, 0.0);
    assert_eq!(s.total_pnl_usd, 0.0);
    assert_eq!(s.profit_factor, Some(0.0));
    assert_eq!(s.avg_rr, Some(0.0));
    assert_eq!(s.best_trade_pnl, 0.0);
    assert_eq!(s.worst_trade_pnl, 0.0);
    assert!(s.by_symbol.is_empty());
    assert!(s.by_exit_reason.is_empty());
}

#[test]
fn all_wins_winrate_one() {
    let (start, end) = day_window(2026, 5, 19);
    let trades = vec![
        trade("BTCUSDT", 10.0, "TAKE_PROFIT", "2026-05-19T10:00:00Z"),
        trade("ETHUSDT", 5.0,  "TAKE_PROFIT", "2026-05-19T12:00:00Z"),
    ];
    let s = TradeSummary::aggregate(&trades, ReportPeriod::Daily, start, end);
    assert_eq!(s.total_trades, 2);
    assert_eq!(s.wins, 2);
    assert_eq!(s.losses, 0);
    assert!((s.win_rate - 1.0).abs() < 1e-9);
    assert!((s.total_pnl_usd - 15.0).abs() < 1e-9);
    assert!((s.avg_win_usd - 7.5).abs() < 1e-9);
    assert_eq!(s.avg_loss_usd, 0.0);
    assert_eq!(s.profit_factor, None, "no losses → PF tanımsız (None)");
    assert_eq!(s.avg_rr, None);
}

#[test]
fn all_losses_winrate_zero() {
    let (start, end) = day_window(2026, 5, 19);
    let trades = vec![
        trade("BTCUSDT", -3.0, "STOP_LOSS", "2026-05-19T09:00:00Z"),
        trade("BTCUSDT", -7.0, "STOP_LOSS", "2026-05-19T11:00:00Z"),
    ];
    let s = TradeSummary::aggregate(&trades, ReportPeriod::Daily, start, end);
    assert_eq!(s.wins, 0);
    assert_eq!(s.losses, 2);
    assert_eq!(s.win_rate, 0.0);
    assert!((s.total_pnl_usd - (-10.0)).abs() < 1e-9);
    assert!((s.avg_loss_usd - 5.0).abs() < 1e-9, "loss ortalaması mutlak değer 5.0");
    assert_eq!(s.profit_factor, Some(0.0), "Σwin=0 ve Σloss>0 → PF=0");
}

#[test]
fn mixed_trades_metric_math() {
    let (start, end) = day_window(2026, 5, 19);
    let trades = vec![
        trade("BTCUSDT",  10.0, "TAKE_PROFIT", "2026-05-19T10:00:00Z"),
        trade("BTCUSDT",  20.0, "TAKE_PROFIT", "2026-05-19T11:00:00Z"),
        trade("ETHUSDT",  -5.0, "STOP_LOSS",   "2026-05-19T12:00:00Z"),
        trade("ETHUSDT", -10.0, "STOP_LOSS",   "2026-05-19T13:00:00Z"),
    ];
    let s = TradeSummary::aggregate(&trades, ReportPeriod::Daily, start, end);
    assert_eq!(s.total_trades, 4);
    assert_eq!(s.wins, 2);
    assert_eq!(s.losses, 2);
    assert!((s.win_rate - 0.5).abs() < 1e-9);
    assert!((s.total_pnl_usd - 15.0).abs() < 1e-9);
    // sum_win=30, sum_loss_abs=15 → PF=2.0
    assert!((s.profit_factor.unwrap() - 2.0).abs() < 1e-9);
    // avg_win=15, avg_loss=7.5 → avg_rr=2.0
    assert!((s.avg_rr.unwrap() - 2.0).abs() < 1e-9);
    assert!((s.best_trade_pnl - 20.0).abs() < 1e-9);
    assert!((s.worst_trade_pnl - (-10.0)).abs() < 1e-9);
}

#[test]
fn date_filter_excludes_out_of_window() {
    let (start, end) = day_window(2026, 5, 19);
    let trades = vec![
        trade("BTCUSDT", 10.0, "TP", "2026-05-18T23:59:59Z"), // dışarıda (önce)
        trade("BTCUSDT", 20.0, "TP", "2026-05-19T00:00:00Z"), // içinde (sınır kapsayıcı)
        trade("BTCUSDT", 30.0, "TP", "2026-05-19T23:59:59Z"), // içinde
        trade("BTCUSDT", 40.0, "TP", "2026-05-20T00:00:00Z"), // dışarıda (sonra, dışlayıcı)
    ];
    let s = TradeSummary::aggregate(&trades, ReportPeriod::Daily, start, end);
    assert_eq!(s.total_trades, 2, "sadece 19 Mayıs içindeki 2 trade sayılmalı");
    assert!((s.total_pnl_usd - 50.0).abs() < 1e-9);
}

#[test]
fn malformed_closed_at_is_skipped() {
    let (start, end) = day_window(2026, 5, 19);
    let trades = vec![
        trade("BTC", 10.0, "TP", "2026-05-19T10:00:00Z"),
        trade("BTC", 20.0, "TP", "garbage-date"),
    ];
    let s = TradeSummary::aggregate(&trades, ReportPeriod::Daily, start, end);
    assert_eq!(s.total_trades, 1, "bozuk closed_at skiplenmeli");
    assert!((s.total_pnl_usd - 10.0).abs() < 1e-9);
}

#[test]
fn by_symbol_breakdown_sorted_by_pnl_desc() {
    let (start, end) = day_window(2026, 5, 19);
    let trades = vec![
        trade("ETHUSDT", -5.0, "SL", "2026-05-19T10:00:00Z"),
        trade("BTCUSDT", 20.0, "TP", "2026-05-19T11:00:00Z"),
        trade("BTCUSDT", -3.0, "SL", "2026-05-19T12:00:00Z"),
        trade("SOLUSDT",  8.0, "TP", "2026-05-19T13:00:00Z"),
    ];
    let s = TradeSummary::aggregate(&trades, ReportPeriod::Daily, start, end);
    assert_eq!(s.by_symbol.len(), 3);
    // PnL: BTC=17, SOL=8, ETH=-5
    assert_eq!(s.by_symbol[0].symbol, "BTCUSDT");
    assert!((s.by_symbol[0].pnl_usd - 17.0).abs() < 1e-9);
    assert_eq!(s.by_symbol[0].trades, 2);
    assert_eq!(s.by_symbol[0].wins, 1);
    assert!((s.by_symbol[0].win_rate - 0.5).abs() < 1e-9);
    assert_eq!(s.by_symbol[1].symbol, "SOLUSDT");
    assert_eq!(s.by_symbol[2].symbol, "ETHUSDT");
}

#[test]
fn by_exit_reason_breakdown_sorted_by_count_desc() {
    let (start, end) = day_window(2026, 5, 19);
    let trades = vec![
        trade("BTC", 10.0, "TAKE_PROFIT", "2026-05-19T10:00:00Z"),
        trade("BTC", -3.0, "STOP_LOSS",   "2026-05-19T11:00:00Z"),
        trade("BTC", -2.0, "STOP_LOSS",   "2026-05-19T12:00:00Z"),
        trade("BTC", -1.0, "STOP_LOSS",   "2026-05-19T13:00:00Z"),
    ];
    let s = TradeSummary::aggregate(&trades, ReportPeriod::Daily, start, end);
    assert_eq!(s.by_exit_reason[0].reason, "STOP_LOSS");
    assert_eq!(s.by_exit_reason[0].count, 3);
    assert!((s.by_exit_reason[0].pnl_usd - (-6.0)).abs() < 1e-9);
    assert_eq!(s.by_exit_reason[1].reason, "TAKE_PROFIT");
    assert_eq!(s.by_exit_reason[1].count, 1);
}

#[test]
fn period_label_daily_format() {
    let start = Utc.with_ymd_and_hms(2026, 5, 19, 0, 0, 0).unwrap();
    assert_eq!(ReportPeriod::Daily.label(start), "daily 2026-05-19");
    assert_eq!(ReportPeriod::Daily.file_stem(start), "daily_2026-05-19");
}

#[test]
fn period_label_weekly_iso_format() {
    // 2026-05-19 Salı; ISO hafta = 21
    let start = Utc.with_ymd_and_hms(2026, 5, 18, 0, 0, 0).unwrap(); // Pazartesi
    let lbl = ReportPeriod::Weekly.label(start);
    // ISO yıl ve hafta numarasını kontrol edelim (chrono kendi hesaplar)
    assert!(lbl.starts_with("weekly 2026-W"), "weekly label formatı: {}", lbl);
    let week = start.iso_week().week();
    assert!(lbl.contains(&format!("W{:02}", week)));
}

#[test]
fn current_day_window_starts_at_midnight() {
    let now = Utc.with_ymd_and_hms(2026, 5, 19, 14, 35, 22).unwrap();
    let (s, e) = current_day_window(now);
    assert_eq!(s.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-05-19 00:00:00");
    assert_eq!(e.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-05-20 00:00:00");
}

#[test]
fn current_week_window_starts_monday() {
    // 2026-05-19 Salı → haftanın Pazartesi'si 2026-05-18
    let now = Utc.with_ymd_and_hms(2026, 5, 19, 14, 35, 22).unwrap();
    let (s, e) = current_week_window(now);
    assert_eq!(s.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-05-18 00:00:00");
    assert_eq!(e.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-05-25 00:00:00");
}

#[test]
fn write_summary_to_file_creates_json() {
    let (start, end) = day_window(2026, 5, 19);
    let s = TradeSummary::aggregate(
        &vec![trade("BTC", 1.0, "TP", "2026-05-19T10:00:00Z")],
        ReportPeriod::Daily, start, end,
    );
    let dir = std::env::temp_dir().join(format!("memos_summary_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = write_summary_to_file(&s, &dir, ReportPeriod::Daily, start).expect("write OK");
    assert!(path.exists(), "dosya yazılmalı: {:?}", path);
    let raw = std::fs::read_to_string(&path).expect("oku");
    let round: TradeSummary = serde_json::from_str(&raw).expect("JSON parse");
    assert_eq!(round, s, "yazılan/okunan özet aynı olmalı");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_write_is_idempotent_overwrite() {
    let (start, _end) = day_window(2026, 5, 19);
    let s1 = TradeSummary::aggregate(&[], ReportPeriod::Daily, start, start);
    let s2 = TradeSummary::aggregate(
        &vec![trade("BTC", 1.0, "TP", "2026-05-19T10:00:00Z")],
        ReportPeriod::Daily, start, start + chrono::Duration::days(1),
    );
    let dir = std::env::temp_dir().join(format!("memos_summary_idem_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p1 = write_summary_to_file(&s1, &dir, ReportPeriod::Daily, start).unwrap();
    let p2 = write_summary_to_file(&s2, &dir, ReportPeriod::Daily, start).unwrap();
    assert_eq!(p1, p2, "aynı period için aynı dosya");
    let last: TradeSummary = serde_json::from_str(&std::fs::read_to_string(&p2).unwrap()).unwrap();
    assert_eq!(last.total_trades, 1, "ikinci yazma birinciyi ezmeli");
    let _ = std::fs::remove_dir_all(&dir);
}
