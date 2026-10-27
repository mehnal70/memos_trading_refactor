//! Veri-only BIST trend veri güncelleme ve alarm pipeline'ı.
//!
//! Downloader doğrudan [`BistFetcher`] kullanır; daemon `cargo` veya shell
//! çağırmaz. BIST venue'si execution sağlamadığı için bu modül yalnızca
//! candle upsert eder ve karar/alarm raporu üretir.

use chrono::{DateTime, Datelike, Timelike, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock,
    },
    time::{Duration, Instant},
};

use crate::core::types::Candle;
use crate::persistence::{open_db, reader::read_candles_market, writer::save_candle};

pub const LOOKBACK: usize = 150;
pub const EMA_FAST: usize = 50;
pub const EMA_SLOW: usize = 100;
pub const DONCHIAN: usize = 20;
pub const ATR_PERIOD: usize = 14;

pub static LAST_SHEET_REJECTED_ROWS: AtomicUsize = AtomicUsize::new(0);
static SHEET_ROWS_CACHE: LazyLock<
    tokio::sync::Mutex<Option<(Instant, ParsedSheetRows)>>,
> = LazyLock::new(|| tokio::sync::Mutex::new(None));

#[derive(Debug, Clone, Serialize)]
pub struct PyramidPlan {
    pub max_tranches: u8,
    pub weights: [f64; 3],
    pub initial_entry: f64,
    pub add_after_atr: [f64; 2],
    pub protective_stop: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendDecision {
    pub symbol: String,
    pub interval: String,
    pub as_of: DateTime<Utc>,
    pub previous_close: Option<f64>,
    pub gap_pct: Option<f64>,
    pub close: f64,
    pub ema50: f64,
    pub ema100: f64,
    pub atr14: f64,
    pub donchian20_previous_high: f64,
    pub trend: &'static str,
    pub action: &'static str,
    #[serde(default = "default_opportunity_status")]
    pub opportunity_status: String,
    pub reason: String,
    pub pyramid: Option<PyramidPlan>,
    pub suggested_entry: Option<f64>,
    pub exit_reference: Option<f64>,
    pub exit_condition: Option<String>,
    #[serde(default = "default_risk_multiplier")]
    pub risk_multiplier: f64,
    pub manual_note: &'static str,
}

fn default_risk_multiplier() -> f64 {
    1.0
}

fn default_opportunity_status() -> String {
    "NONE".into()
}

#[derive(Debug, Clone)]
pub struct BistTrendConfig {
    pub db_path: String,
    pub symbols_path: String,
    pub report_path: String,
    pub status_path: String,
    pub interval: String,
    pub intervals: Vec<String>,
    pub candle_limit: usize,
    pub include_db_symbols: bool,
    pub sheets: BistSheetsConfig,
}

#[derive(Debug, Clone)]
pub struct BistIntradayConfig {
    pub db_path: String,
    pub symbols_path: String,
    pub report_path: String,
    pub intervals: Vec<String>,
    pub range: String,
    pub candle_limit: usize,
    pub higher_timeframe: String,
    pub sheets: BistSheetsConfig,
}

#[derive(Debug, Clone)]
pub struct BistSheetsConfig {
    pub enabled: bool,
    pub service_account_path: Option<String>,
    pub spreadsheet_id: Option<String>,
    pub api_url: Option<String>,
    pub range: String,
    pub inbox_ranges: Vec<(String, String)>,
    pub latest_range: String,
    pub import_log_range: String,
    pub publish_latest: bool,
    pub auto_clear: bool,
    pub csv_url: Option<String>,
    pub cache_path: String,
    pub ohlc_cache_path: String,
    pub timeout_secs: u64,
    pub require_match: bool,
}

#[derive(Debug, Clone)]
struct BistControlRow {
    enabled: bool,
    manual_veto: bool,
    risk_multiplier: f64,
}

#[derive(Debug, Clone)]
struct BistSheetCandle {
    symbol: String,
    interval: String,
    candle: Candle,
    risk_multiplier: f64,
    enabled: bool,
    manual_veto: bool,
}

#[derive(Debug, Clone)]
struct ParsedSheetRows {
    rows: Vec<BistSheetCandle>,
    rejected_rows: usize,
}

impl std::ops::Deref for ParsedSheetRows {
    type Target = [BistSheetCandle];

    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}

#[derive(Debug, Deserialize)]
struct GoogleValuesResponse {
    values: Option<Vec<Vec<serde_json::Value>>>,
}

#[derive(Debug, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: String,
}

#[derive(Debug, Serialize)]
struct ServiceAccountClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Serialize)]
struct ValuesUpdate<'a> {
    range: &'a str,
    major_dimension: &'static str,
    values: Vec<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct ImportLogRow {
    run_id: String,
    finished_at: String,
    rows_read: usize,
    rows_imported: usize,
    rows_rejected: usize,
    source_tab: String,
    latest_candle: String,
    data_freshness: String,
    error_summary: String,
    status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntradayConfirmation {
    pub symbol: String,
    pub interval: String,
    pub as_of: DateTime<Utc>,
    pub close: f64,
    pub ema20: f64,
    pub ema50: f64,
    pub higher_timeframe_uptrend: bool,
    pub status: &'static str,
    pub reason: String,
}

impl BistIntradayConfig {
    pub fn from_env() -> Result<Self, String> {
        let raw = std::env::var("BIST_INTRADAY_INTERVALS").unwrap_or_else(|_| "15m,30m,1h".into());
        let intervals = parse_intraday_intervals(&raw)?;
        let range = std::env::var("BIST_INTRADAY_RANGE").unwrap_or_else(|_| "5d".into());
        validate_intraday_range(&intervals, &range)?;
        Ok(Self {
            db_path: std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into()),
            symbols_path: std::env::var("BIST_INTRADAY_SYMBOLS_FILE")
                .or_else(|_| std::env::var("BIST_SYMBOLS_FILE"))
                .unwrap_or_else(|_| "config/bist_intraday_symbols.txt".into()),
            report_path: std::env::var("BIST_INTRADAY_REPORT")
                .unwrap_or_else(|_| "data/bist_intraday_confirmation.json".into()),
            intervals,
            range,
            candle_limit: std::env::var("BIST_INTRADAY_CANDLE_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120)
                .max(EMA_FAST + 1),
            higher_timeframe: std::env::var("BIST_INTRADAY_HTF_INTERVAL")
                .unwrap_or_else(|_| "1d".into()),
            sheets: BistSheetsConfig::from_env(),
        })
    }
}

impl BistTrendConfig {
    pub fn from_env() -> Result<Self, String> {
        let fallback = std::env::var("BIST_INTERVAL").unwrap_or_else(|_| "1d".into());
        let raw_intervals = std::env::var("BIST_INTERVALS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| fallback.clone());
        let intervals = parse_intervals(&raw_intervals)?;
        let interval = fallback;
        Ok(Self {
            db_path: std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into()),
            symbols_path: std::env::var("BIST_SYMBOLS_FILE")
                .unwrap_or_else(|_| "config/bist100_symbols.txt".into()),
            report_path: std::env::var("BIST_TREND_REPORT")
                .unwrap_or_else(|_| "data/bist_trend.json".into()),
            status_path: std::env::var("BIST_TREND_STATUS")
                .unwrap_or_else(|_| "data/bist_trend_status.json".into()),
            interval,
            intervals,
            candle_limit: std::env::var("BIST_CANDLE_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(LOOKBACK)
                .max(LOOKBACK),
            include_db_symbols: std::env::var("BIST_INCLUDE_DB_SYMBOLS")
                .map(|v| {
                    !matches!(
                        v.trim().to_ascii_lowercase().as_str(),
                        "0" | "false" | "off" | "no"
                    )
                })
                .unwrap_or(true),
            sheets: BistSheetsConfig::from_env(),
        })
    }
}

impl BistSheetsConfig {
    fn from_env() -> Self {
        let enabled = std::env::var("BIST_SHEETS_ENABLED")
            .map(|value| {
                !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            })
            .unwrap_or(false);
        Self {
            enabled,
            service_account_path: std::env::var("BIST_SHEETS_SERVICE_ACCOUNT_JSON")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            spreadsheet_id: std::env::var("BIST_SHEETS_SPREADSHEET_ID")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            api_url: std::env::var("BIST_SHEETS_API_URL")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            range: std::env::var("BIST_SHEETS_RANGE").unwrap_or_else(|_| "OHCL_INBOX_1d".into()),
            inbox_ranges: std::env::var("BIST_SHEETS_INBOX_RANGES")
                .ok()
                .map(|value| {
                    value
                        .split(',')
                        .filter_map(|item| {
                            let (range, interval) = item.split_once(':')?;
                            Some((range.trim().to_owned(), interval.trim().to_owned()))
                        })
                        .filter(|(range, interval)| !range.is_empty() && !interval.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            latest_range: std::env::var("BIST_SHEETS_LATEST_RANGE")
                .unwrap_or_else(|_| "Latest".into()),
            import_log_range: std::env::var("BIST_SHEETS_IMPORT_LOG_RANGE")
                .unwrap_or_else(|_| "ImportLog".into()),
            publish_latest: std::env::var("BIST_SHEETS_PUBLISH_LATEST")
                .map(|value| {
                    !matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "0" | "false" | "off" | "no"
                    )
                })
                .unwrap_or(true),
            auto_clear: std::env::var("BIST_SHEETS_AUTO_CLEAR")
                .map(|value| {
                    !matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "0" | "false" | "off" | "no"
                    )
                })
                .unwrap_or(false),
            csv_url: std::env::var("BIST_SHEETS_CSV_URL")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            cache_path: std::env::var("BIST_SHEETS_CACHE_PATH")
                .unwrap_or_else(|_| "data/bist_sheets_control.csv".into()),
            ohlc_cache_path: std::env::var("BIST_SHEETS_OHLC_CACHE_PATH")
                .unwrap_or_else(|_| "data/bist_sheets_ohlc.json".into()),
            timeout_secs: std::env::var("BIST_SHEETS_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(15)
                .clamp(3, 60),
            require_match: std::env::var("BIST_SHEETS_REQUIRE_MATCH")
                .map(|value| {
                    !matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "0" | "false" | "off" | "no"
                    )
                })
                .unwrap_or(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BistHealthStatus {
    pub expected_symbol_count: usize,
    pub universe_symbol_count: usize,
    pub expected_download_count: usize,
    pub successful_download_count: usize,
    pub skipped_error_count: usize,
    pub errors: Vec<String>,
    pub latest_candle_timestamp: Option<DateTime<Utc>>,
    pub run_started: DateTime<Utc>,
    pub run_finished: DateTime<Utc>,
    pub configured_interval: String,
    pub configured_intervals: Vec<String>,
    pub status: String,
    #[serde(default)]
    pub source_tab: String,
    #[serde(default)]
    pub data_freshness: String,
}

struct BistRunLock {
    path: std::path::PathBuf,
}

impl BistRunLock {
    fn acquire() -> Result<Self, String> {
        let path =
            std::env::var("BIST_TREND_LOCK").unwrap_or_else(|_| "data/bist_trend.lock".into());
        let path = std::path::PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("BIST lock klasörü açılamadı: {e}"))?;
        }
        match fs::create_dir(&path) {
            Ok(()) => Ok(Self { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .map(|age| age > std::time::Duration::from_secs(6 * 60 * 60))
                    .unwrap_or(false);
                if stale {
                    let _ = fs::remove_dir_all(&path);
                    fs::create_dir(&path)
                        .map(|()| Self { path })
                        .map_err(|e| format!("stale BIST lock temizlenemedi: {e}"))
                } else {
                    Err("BIST trend pipeline zaten çalışıyor".into())
                }
            }
            Err(error) => Err(format!("BIST trend lock alınamadı: {error}")),
        }
    }
}

impl Drop for BistRunLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_health_status(path: &str, status: &BistHealthStatus) -> Result<(), String> {
    let target = std::path::Path::new(path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("BIST status klasörü açılamadı: {e}"))?;
    }
    let temporary = target.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(status)
            .map_err(|e| format!("BIST status serileştirilemedi: {e}"))?,
    )
    .map_err(|e| format!("BIST status yazılamadı: {e}"))?;
    fs::rename(&temporary, target).map_err(|e| format!("BIST status atomik değiştirilemedi: {e}"))
}

pub fn read_health_status(path: &str) -> Result<BistHealthStatus, String> {
    let body = fs::read(path).map_err(|e| format!("BIST status okunamadı: {e}"))?;
    serde_json::from_slice(&body).map_err(|e| format!("BIST status geçersiz JSON: {e}"))
}

fn freshness_limit(interval: &str) -> chrono::Duration {
    match interval {
        "1wk" => chrono::Duration::days(14),
        "1mo" | "3mo" => chrono::Duration::days(45),
        // Cuma kapanışı pazartesi seansında hâlâ en güncel günlük BIST barıdır.
        _ => chrono::Duration::days(5),
    }
}

fn freshness_label(timestamp: DateTime<Utc>) -> String {
    let age = Utc::now().signed_duration_since(timestamp);
    if age <= freshness_limit("1d") {
        "FRESH".into()
    } else {
        format!("STALE_{}H", age.num_hours().max(0))
    }
}

fn next_bist_session_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let turkey = now + chrono::Duration::hours(3);
    let mut date = turkey.date_naive();
    let minutes = turkey.hour() * 60 + turkey.minute();
    if minutes >= 10 * 60 {
        date = date.succ_opt().expect("valid next calendar date");
    }
    while matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
        date = date.succ_opt().expect("valid next calendar date");
    }
    chrono::NaiveDateTime::new(
        date,
        chrono::NaiveTime::from_hms_opt(10, 0, 0).expect("valid session start"),
    )
    .and_utc()
        - chrono::Duration::hours(3)
}

fn health_status(
    latest: Option<DateTime<Utc>>,
    successful: usize,
    expected: usize,
    errors: usize,
    interval: &str,
) -> &'static str {
    if latest.is_none() {
        return "UNAVAILABLE";
    }
    let stale = latest
        .map(|ts| Utc::now() - ts > freshness_limit(interval))
        .unwrap_or(true);
    if stale {
        return "STALE";
    }
    if successful == 0 || errors > 0 || successful < expected {
        return "DEGRADED";
    }
    "HEALTHY"
}

pub fn parse_intervals(raw: &str) -> Result<Vec<String>, String> {
    let mut intervals = Vec::new();
    for value in raw.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        let interval = value.to_ascii_lowercase();
        validate_interval(&interval)?;
        if !intervals.contains(&interval) {
            intervals.push(interval);
        }
    }
    if intervals.is_empty() {
        return Err("BIST_INTERVALS boş; en az bir interval belirtin".into());
    }
    Ok(intervals)
}

/// Yahoo chart'ın desteklediği ve trend hesaplarının güvenle yorumlayabildiği
/// aralıklar. Keyfi değerleri kabul etmemek yanlış candle serisi üretimini önler.
pub fn validate_interval(interval: &str) -> Result<(), String> {
    match interval.trim().to_ascii_lowercase().as_str() {
        "1d" | "5d" | "1wk" | "1mo" | "3mo" => Ok(()),
        other => Err(format!(
            "BIST interval desteklenmiyor: {other}; 1d, 5d, 1wk, 1mo veya 3mo kullanın"
        )),
    }
}

pub fn parse_intraday_intervals(raw: &str) -> Result<Vec<String>, String> {
    let mut intervals = Vec::new();
    for value in raw.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        let interval = value.to_ascii_lowercase();
        if !matches!(interval.as_str(), "15m" | "30m" | "1h") {
            return Err(format!(
                "BIST intraday interval desteklenmiyor: {interval}; 15m, 30m veya 1h kullanın"
            ));
        }
        if !intervals.contains(&interval) {
            intervals.push(interval);
        }
    }
    if intervals.is_empty() {
        return Err("BIST_INTRADAY_INTERVALS boş; en az bir interval belirtin".into());
    }
    Ok(intervals)
}

pub fn validate_intraday_range(intervals: &[String], range: &str) -> Result<(), String> {
    let allowed = match range.trim().to_ascii_lowercase().as_str() {
        "1d" | "5d" | "7d" | "1mo" | "60d" => true,
        "3mo" | "6mo" => intervals.iter().all(|i| i == "1h"),
        "1y" | "2y" | "730d" => intervals.iter().all(|i| i == "1h"),
        other => {
            return Err(format!(
                "BIST intraday range desteklenmiyor: {other}; kısa Yahoo range kullanın"
            ))
        }
    };
    if !allowed {
        return Err(format!(
            "BIST intraday range {range} interval kombinasyonuyla geçersiz (15m/30m: en fazla 60d; 1h: en fazla 730d)"
        ));
    }
    Ok(())
}
pub fn ema(values: &[f64], period: usize) -> Option<f64> {
    if values.len() < period || period == 0 {
        return None;
    }
    let multiplier = 2.0 / (period as f64 + 1.0);
    let mut current = values[..period].iter().sum::<f64>() / period as f64;
    for value in &values[period..] {
        current = (value - current) * multiplier + current;
    }
    Some(current)
}

pub fn atr(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period + 1 || period == 0 {
        return None;
    }
    let start = candles.len() - period;
    let total = (start..candles.len())
        .map(|index| {
            let previous_close = candles[index - 1].close;
            (candles[index].high - candles[index].low)
                .max((candles[index].high - previous_close).abs())
                .max((candles[index].low - previous_close).abs())
        })
        .sum::<f64>();
    Some(total / period as f64)
}

pub fn decision(symbol: &str, candles: &[Candle]) -> Option<TrendDecision> {
    decision_for_interval(symbol, "1d", candles)
}

pub fn decision_for_interval(
    symbol: &str,
    interval: &str,
    candles: &[Candle],
) -> Option<TrendDecision> {
    if candles.len() < EMA_SLOW + DONCHIAN + 1 {
        return None;
    }
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let close = *closes.last()?;
    let ema50 = ema(&closes, EMA_FAST)?;
    let ema100 = ema(&closes, EMA_SLOW)?;
    let atr14 = atr(candles, ATR_PERIOD)?;
    let previous = &candles[candles.len() - DONCHIAN - 1..candles.len() - 1];
    let donchian_high = previous.iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let uptrend = ema50 > ema100;
    let breakout = close > donchian_high;
    let trend = if uptrend { "UPTREND" } else { "NO_UPTREND" };
    let (action, reason, pyramid, suggested_entry, exit_reference, exit_condition) =
        if uptrend && breakout {
            (
                "ENTRY_CANDIDATE",
                format!("EMA50>EMA100 ve close Donchian20 üstünde ({close:.2}>{donchian_high:.2})"),
                Some(PyramidPlan {
                    max_tranches: 3,
                    weights: [1.0, 0.5, 0.25],
                    initial_entry: close,
                    add_after_atr: [close + atr14, close + 2.0 * atr14],
                    protective_stop: close - 2.0 * atr14,
                }),
                Some(donchian_high),
                None,
                None,
            )
        } else if !uptrend && close < ema50 {
            (
                "EXIT_WARNING",
                format!("trend filtresi zayıf ve close EMA50 altında ({close:.2}<{ema50:.2})"),
                None,
                None,
                Some(ema50),
                Some("close EMA50 altında; kesin satış fiyatı değildir".into()),
            )
        } else if uptrend {
            (
                "WATCH",
                "yukarı trend korunuyor, yeni Donchian kırılımı bekleniyor".to_string(),
                None,
                None,
                None,
                None,
            )
        } else {
            (
                "HOLD",
                "yukarı trend teyidi yok".to_string(),
                None,
                None,
                None,
                None,
            )
        };
    Some(TrendDecision {
        symbol: symbol.to_owned(),
        interval: interval.to_owned(),
        as_of: candles.last()?.timestamp,
        previous_close: candles
            .get(candles.len().saturating_sub(2))
            .map(|candle| candle.close),
        gap_pct: None,
        close,
        ema50,
        ema100,
        atr14,
        donchian20_previous_high: donchian_high,
        trend,
        action,
        opportunity_status: if matches!(action, "ENTRY_CANDIDATE" | "EXIT_WARNING") {
            "NEW".into()
        } else {
            "NONE".into()
        },
        reason,
        pyramid,
        suggested_entry,
        exit_reference,
        exit_condition,
        risk_multiplier: 1.0,
        manual_note: "Manuel Ziraat karar desteğidir; emir gönderilmez.",
    })
}

pub fn load_symbols(path: &str) -> Result<Vec<String>, String> {
    let symbols = fs::read_to_string(path)
        .map_err(|e| format!("BIST sembol manifestosu okunamadı ({path}): {e}"))?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if symbols.is_empty() {
        return Err(format!("BIST sembol manifestosu boş: {path}"));
    }

    Ok(symbols)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpportunityState {
    signal_key: String,
    action: String,
    status: String,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
}

pub fn update_opportunity_lifecycle(
    path: &str,
    decisions: &mut [TrendDecision],
) -> Result<Vec<TrendDecision>, String> {
    let mut states: HashMap<String, OpportunityState> = if std::path::Path::new(path).exists() {
        let body = fs::read(path).map_err(|e| format!("BIST opportunity state okunamadı: {e}"))?;
        serde_json::from_slice(&body)
            .map_err(|e| format!("BIST opportunity state geçersiz JSON: {e}"))?
    } else {
        HashMap::new()
    };
    let now = Utc::now();
    let mut actionable = Vec::new();
    for decision in decisions.iter_mut() {
        let key = format!("{}|{}", decision.symbol, decision.interval);
        let is_actionable = matches!(decision.action, "ENTRY_CANDIDATE" | "EXIT_WARNING");
        let signal_key = format!("{}|{}", decision.action, decision.as_of.to_rfc3339());
        let previous = states.get(&key).cloned();
        let status = if !is_actionable {
            "EXPIRED"
        } else if previous
            .as_ref()
            .map(|state| state.signal_key == signal_key)
            .unwrap_or(false)
        {
            "ACTIVE"
        } else {
            "NEW"
        };
        decision.opportunity_status = status.into();
        states.insert(
            key,
            OpportunityState {
                signal_key: signal_key.clone(),
                action: decision.action.into(),
                status: status.into(),
                first_seen: previous.map(|state| state.first_seen).unwrap_or(now),
                last_seen: now,
            },
        );
        if is_actionable && decision.opportunity_status == "NEW" {
            actionable.push(decision.clone());
        }
    }
    if let Some(parent) = std::path::Path::new(path).parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("BIST opportunity state klasörü açılamadı: {e}"))?;
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(&states)
            .map_err(|e| format!("BIST opportunity state serileştirilemedi: {e}"))?,
    )
    .map_err(|e| format!("BIST opportunity state yazılamadı: {e}"))?;
    Ok(actionable)
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(field.trim().to_owned());
                field.clear();
            }
            _ => field.push(ch),
        }
    }
    fields.push(field.trim().to_owned());
    fields
}

fn truthy(value: Option<&str>, default: bool) -> bool {
    value
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no" | "n"
            )
        })
        .unwrap_or(default)
}

fn parse_control_csv(body: &str) -> Result<HashMap<String, BistControlRow>, String> {
    let mut lines = body.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().ok_or("Google Sheets CSV başlığı boş")?;
    let headers = parse_csv_line(header)
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let column = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| headers.iter().position(|header| header == name))
    };
    let symbol_column = column(&["symbol", "ticker", "sembol"])
        .ok_or("Google Sheets CSV içinde symbol/ticker/sembol sütunu yok")?;
    let risk_column = column(&["risk_multiplier", "risk", "risk_carpani"]);
    let enabled_column = column(&["enabled", "active", "aktif"]);
    let veto_column = column(&["manual_veto", "veto", "blocked", "disabled"]);
    let mut rows = HashMap::new();
    for (line_number, line) in lines.enumerate() {
        let fields = parse_csv_line(line);
        let symbol = fields
            .get(symbol_column)
            .map(|value| value.trim().trim_end_matches(".IS").to_ascii_uppercase())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "Google Sheets CSV satır {} sembol içermiyor",
                    line_number + 2
                )
            })?;
        let enabled = enabled_column
            .and_then(|index| fields.get(index).map(String::as_str))
            .map(|value| truthy(Some(value), true))
            .unwrap_or(true);
        let manual_veto = veto_column
            .and_then(|index| fields.get(index).map(String::as_str))
            .map(|value| truthy(Some(value), false))
            .unwrap_or(false);
        let risk_multiplier = risk_column
            .and_then(|index| fields.get(index))
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(1.0);
        rows.insert(
            symbol,
            BistControlRow {
                enabled,
                manual_veto,
                risk_multiplier,
            },
        );
    }
    if rows.is_empty() {
        return Err("Google Sheets CSV kontrol satırı içermiyor".into());
    }
    Ok(rows)
}

async fn load_control_rows(
    config: &BistSheetsConfig,
) -> Result<Option<HashMap<String, BistControlRow>>, String> {
    if !config.enabled {
        return Ok(None);
    }
    let body = if let Some(url) = config.csv_url.as_deref() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .user_agent("memos-trading-bist-control/1.0")
            .build()
            .map_err(|error| format!("Google Sheets HTTP istemcisi oluşturulamadı: {error}"))?;
        match client.get(url).send().await {
            Ok(response) if response.status().is_success() => {
                let body = response
                    .text()
                    .await
                    .map_err(|error| format!("Google Sheets CSV gövdesi okunamadı: {error}"))?;
                if let Some(parent) = std::path::Path::new(&config.cache_path).parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("Google Sheets cache klasörü açılamadı: {error}")
                    })?;
                }
                let temporary = format!("{}.tmp", config.cache_path);
                fs::write(&temporary, &body).map_err(|error| {
                    format!("Google Sheets cache geçici dosyası yazılamadı: {error}")
                })?;
                fs::rename(&temporary, &config.cache_path).map_err(|error| {
                    format!("Google Sheets cache atomik değiştirilemedi: {error}")
                })?;
                body
            }
            Ok(response) => {
                let status = response.status();
                fs::read_to_string(&config.cache_path).map_err(|error| {
                    format!("Google Sheets HTTP {status}; lokal cache okunamadı: {error}")
                })?
            }
            Err(error) => fs::read_to_string(&config.cache_path).map_err(|cache_error| {
                format!("Google Sheets erişilemedi ({error}); lokal cache okunamadı: {cache_error}")
            })?,
        }
    } else {
        fs::read_to_string(&config.cache_path)
            .map_err(|error| format!("BIST Sheets etkin ama URL/cache yok: {error}"))?
    };
    parse_control_csv(&body).map(Some)
}

fn apply_control_rows(
    symbols: Vec<String>,
    rows: Option<&HashMap<String, BistControlRow>>,
    require_match: bool,
) -> Result<Vec<String>, String> {
    let Some(rows) = rows else {
        return Ok(symbols);
    };
    let mut matched = 0usize;
    let filtered = symbols
        .into_iter()
        .filter(|symbol| {
            let Some(row) = rows.get(symbol) else {
                return !require_match;
            };
            matched += 1;
            row.enabled
                && !row.manual_veto
                && row.risk_multiplier.is_finite()
                && row.risk_multiplier > 0.0
        })
        .collect::<Vec<_>>();
    if require_match && matched == 0 {
        return Err("Google Sheets kontrol evreni ile BIST manifestosu kesişmiyor".into());
    }
    if filtered.is_empty() {
        return Err("Google Sheets kontrolü tüm BIST sembollerini devre dışı bıraktı".into());
    }
    Ok(filtered)
}

fn parse_sheet_timestamp(value: &str) -> Result<DateTime<Utc>, String> {
    let value = value.trim();
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Ok(timestamp.with_timezone(&Utc));
    }
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|date| date.and_hms_opt(0, 0, 0).expect("valid midnight").and_utc())
        .map_err(|error| format!("Google Sheets tarih geçersiz ({value}): {error}"))
}

fn parse_sheet_rows(body: &str) -> Result<ParsedSheetRows, String> {
    let response: GoogleValuesResponse = serde_json::from_str(body)
        .map_err(|error| format!("Google Sheets API JSON geçersiz: {error}"))?;
    let rows = response
        .values
        .ok_or("Google Sheets API values alanı yok")?;
    let header = rows.first().ok_or("Google Sheets API tablosu boş")?;
    let headers = header
        .iter()
        .map(sheet_value_to_string)
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let column = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| headers.iter().position(|header| header == name))
    };
    let symbol_column = column(&["symbol", "ticker", "sembol"])
        .ok_or("Google Sheets OHLC tablosunda symbol sütunu yok")?;
    let date_column = column(&["date", "timestamp", "tarih"])
        .ok_or("Google Sheets OHLC tablosunda date sütunu yok")?;
    let open_column =
        column(&["open", "acilis"]).ok_or("Google Sheets OHLC tablosunda open sütunu yok")?;
    let high_column =
        column(&["high", "yuksek"]).ok_or("Google Sheets OHLC tablosunda high sütunu yok")?;
    let low_column =
        column(&["low", "dusuk"]).ok_or("Google Sheets OHLC tablosunda low sütunu yok")?;
    let close_column =
        column(&["close", "kapanis"]).ok_or("Google Sheets OHLC tablosunda close sütunu yok")?;
    let volume_column = column(&["volume", "hacim"]);
    let interval_column = column(&["interval", "timeframe", "periyot"]);
    let risk_column = column(&["risk_multiplier", "risk", "risk_carpani"]);
    let enabled_column = column(&["enabled", "active", "aktif"]);
    let veto_column = column(&["manual_veto", "veto", "blocked", "disabled"]);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut rejected_rows = 0usize;
    for (row_number, row) in rows.iter().enumerate().skip(1) {
        let field = |index: usize| {
            row.get(index)
                .map(sheet_value_to_string)
                .unwrap_or_default()
        };
        let symbol = field(symbol_column)
            .trim()
            .trim_end_matches(".IS")
            .to_ascii_uppercase();
        if symbol.is_empty() {
            rejected_rows += 1;
            log::warn!(
                "Google Sheets OHLC satırı atlandı: sembol boş (satır {})",
                row_number + 1
            );
            continue;
        }
        let interval = interval_column
            .map(field)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "1d".into());
        let parse_price = |index: usize, name: &str| {
            field(index)
                .trim()
                .replace(',', ".")
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or_else(|| {
                    format!(
                        "Google Sheets {name} değeri geçersiz: {symbol} satır {}",
                        row_number + 1
                    )
                })
        };
        let candle = match (|| -> Result<Candle, String> {
            Ok(Candle {
                timestamp: parse_sheet_timestamp(&field(date_column))?,
                open: parse_price(open_column, "open")?,
                high: parse_price(high_column, "high")?,
                low: parse_price(low_column, "low")?,
                close: parse_price(close_column, "close")?,
                volume: volume_column
                    .map(field)
                    .and_then(|value| value.trim().replace(',', ".").parse::<f64>().ok())
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .unwrap_or(0.0),
                symbol: symbol.clone(),
                interval: interval.clone(),
            })
        })() {
            Ok(candle) => candle,
            Err(error) => {
                rejected_rows += 1;
                log::warn!(
                    "Google Sheets OHLC satırı atlandı: {} (satır {})",
                    error,
                    row_number + 1
                );
                continue;
            }
        };
        let risk_multiplier = risk_column
            .map(field)
            .and_then(|value| value.trim().replace(',', ".").parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(1.0);
        let enabled = enabled_column
            .map(field)
            .map(|value| truthy(Some(value.trim()), true))
            .unwrap_or(true);
        let manual_veto = veto_column
            .map(field)
            .map(|value| truthy(Some(value.trim()), false))
            .unwrap_or(false);
        let dedup_key = format!("{}|{}|{}", symbol, interval, candle.timestamp.timestamp());
        if !seen.insert(dedup_key) {
            continue;
        }

        out.push(BistSheetCandle {
            symbol,
            interval,
            candle,
            risk_multiplier,
            enabled,
            manual_veto,
        });
    }
    if out.is_empty() {
        return Err("Google Sheets OHLC tablosunda veri satırı yok".into());
    }
    Ok(ParsedSheetRows {
        rows: out,
        rejected_rows,
    })
}

fn sheet_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn is_public_apps_script_web_app(url: &str) -> bool {
    url.starts_with("https://script.google.com/macros/")
        || url.starts_with("https://script.googleusercontent.com/")
}

async fn load_sheet_candles(config: &BistSheetsConfig) -> Result<ParsedSheetRows, String> {
    if !config.enabled {
        return Err("Google Sheets BIST kaynağı etkin değil".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_secs))
        .user_agent("memos-trading-bist-sheets/1.0")
        .build()
        .map_err(|error| format!("Google Sheets HTTP istemcisi oluşturulamadı: {error}"))?;
    let api_url = config
        .api_url
        .as_deref()
        .ok_or("Google Sheets API URL eksik; BIST_SHEETS_API_URL yapılandırılmalı")?;
    let access_token = if !is_public_apps_script_web_app(api_url) {
        if let Some(path) = config.service_account_path.as_deref() {
        let key: ServiceAccountKey = serde_json::from_slice(
            &fs::read(path).map_err(|error| format!("service account JSON okunamadı: {error}"))?,
        )
        .map_err(|error| format!("service account JSON geçersiz: {error}"))?;
        let now = Utc::now().timestamp();
        let claims = ServiceAccountClaims {
            iss: &key.client_email,
            scope: "https://www.googleapis.com/auth/spreadsheets",
            aud: &key.token_uri,
            iat: now,
            exp: now + 3600,
        };
        let assertion = encode(
            &Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &EncodingKey::from_rsa_pem(key.private_key.as_bytes())
                .map_err(|error| format!("service account private key geçersiz: {error}"))?,
        )
        .map_err(|error| format!("service account JWT üretilemedi: {error}"))?;
        let token_response = client
            .post(&key.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|error| format!("Google OAuth token isteği başarısız: {error}"))?;
        if !token_response.status().is_success() {
            return Err(format!(
                "Google OAuth token HTTP {}",
                token_response.status()
            ));
        }
        Some(
            token_response
                .json::<TokenResponse>()
                .await
                .map_err(|error| format!("Google OAuth token yanıtı geçersiz: {error}"))?
                .access_token,
        )
        } else {
            None
        }
    } else {
        None
    };
    let mut body = None;
    let mut last_error = String::new();
    for attempt in 0..3u32 {
        let mut request = client
            .get(api_url)
            .query(&[("range", config.range.as_str())]);
        if let Some(access_token) = access_token.as_deref() {
            request = request.bearer_auth(access_token);
        }
        match request.send().await {
            Ok(candidate) if candidate.status().is_success() => match candidate.text().await {
                Ok(candidate_body) => {
                    body = Some(candidate_body);
                    break;
                }
                Err(error) => {
                    last_error = format!("Google Sheets API gövdesi okunamadı: {error}");
                }
            },
            Ok(candidate) if matches!(candidate.status().as_u16(), 429 | 500 | 502 | 503 | 504) => {
                last_error = format!("Google Sheets API HTTP {}", candidate.status());
            }
            Ok(candidate) => {
                return Err(format!("Google Sheets API HTTP {}", candidate.status()));
            }
            Err(error) => {
                last_error = format!("Google Sheets API erişilemedi: {error}");
            }
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
        }
    }
    let body = if let Some(body) = body {
        if let Some(parent) = std::path::Path::new(&config.ohlc_cache_path).parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Sheets OHLC cache klasörü açılamadı: {error}"))?;
        }
        fs::write(&config.ohlc_cache_path, &body)
            .map_err(|error| format!("Google Sheets OHLC cache yazılamadı: {error}"))?;
        body
    } else {
        fs::read_to_string(&config.ohlc_cache_path).map_err(|cache_error| {
            format!("{last_error}; Google Sheets OHLC cache okunamadı: {cache_error}")
        })?
    };
    let parsed = parse_sheet_rows(&body)?;
    LAST_SHEET_REJECTED_ROWS.store(parsed.rejected_rows, Ordering::Relaxed);
    if parsed.rejected_rows > 0 {
        log::warn!(
            "Google Sheets OHLC: {} bozuk satir atlandi",
            parsed.rejected_rows
        );
    }
    Ok(parsed)
}

/// Google Sheets OHLC kaynağından belirli BIST sembolü için doğrulanmış mumları getirir.
/// Trend taraması ve headless venue aynı parser/cache yolunu kullanır.
pub async fn fetch_google_sheet_candles(
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Result<Vec<Candle>, String> {
    let config = BistSheetsConfig::from_env();
    let requested_symbol = symbol.trim_end_matches(".IS").to_ascii_uppercase();
    let requested_interval = interval.trim().to_ascii_lowercase();
    let cache_ttl_secs = std::env::var("BIST_SHEETS_CACHE_TTL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(900)
        .clamp(30, 86_400);
    // Aynı Sheets snapshot'ını sembol başına tekrar indirme; TTL dolunca yenile.
    let mut cache = SHEET_ROWS_CACHE.lock().await;
    let refresh_cache = cache
        .as_ref()
        .map(|(loaded_at, _)| loaded_at.elapsed() >= Duration::from_secs(cache_ttl_secs))
        .unwrap_or(true);
    if refresh_cache {
        let rows = load_sheet_candles(&config).await?;
        *cache = Some((Instant::now(), rows));
    }
    let (_, rows) = cache
        .as_ref()
        .ok_or("Google Sheets snapshot cache boş")?
        .clone();
    drop(cache);
    let mut candles = rows
        .rows
        .into_iter()
        .filter(|row| {
            row.symbol == requested_symbol
                && row.interval.to_ascii_lowercase() == requested_interval
                && row.enabled
                && !row.manual_veto
        })
        .map(|row| row.candle)
        .collect::<Vec<_>>();
    candles.sort_by_key(|candle| candle.timestamp);
    if candles.is_empty() {
        return Err(format!(
            "Google Sheets OHLC içinde {requested_symbol} {requested_interval} verisi yok"
        ));
    }
    if candles.len() > limit {
        candles = candles.split_off(candles.len() - limit);
    }
    Ok(candles)
}

async fn sheet_access_token(
    config: &BistSheetsConfig,
    client: &reqwest::Client,
) -> Result<String, String> {
    let path = config
        .service_account_path
        .as_deref()
        .ok_or("Google Sheets yazma için service account JSON eksik")?;
    let key: ServiceAccountKey = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("service account JSON okunamadı: {error}"))?,
    )
    .map_err(|error| format!("service account JSON geçersiz: {error}"))?;
    let now = Utc::now().timestamp();
    let claims = ServiceAccountClaims {
        iss: &key.client_email,
        scope: "https://www.googleapis.com/auth/spreadsheets",
        aud: &key.token_uri,
        iat: now,
        exp: now + 3600,
    };
    let assertion = encode(
        &Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &EncodingKey::from_rsa_pem(key.private_key.as_bytes())
            .map_err(|error| format!("service account private key geçersiz: {error}"))?,
    )
    .map_err(|error| format!("service account JWT üretilemedi: {error}"))?;
    let response = client
        .post(&key.token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ])
        .send()
        .await
        .map_err(|error| format!("Google OAuth token isteği başarısız: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("Google OAuth token HTTP {}", response.status()));
    }
    response
        .json::<TokenResponse>()
        .await
        .map(|token| token.access_token)
        .map_err(|error| format!("Google OAuth token yanıtı geçersiz: {error}"))
}

async fn publish_latest_and_log(
    config: &BistSheetsConfig,
    decisions: &[TrendDecision],
    rows_read: usize,
    rows_rejected: usize,
    latest_candle: Option<DateTime<Utc>>,
    error_summary: &str,
) -> Result<(), String> {
    if !config.enabled || !config.publish_latest {
        return Ok(());
    }
    let spreadsheet_id = config
        .spreadsheet_id
        .as_deref()
        .ok_or("Google Sheets yazma için spreadsheet ID eksik")?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_secs))
        .user_agent("memos-trading-bist-sheets/1.0")
        .build()
        .map_err(|error| format!("Google Sheets HTTP istemcisi oluşturulamadı: {error}"))?;
    let token = sheet_access_token(config, &client).await?;
    let base = format!(
        "https://sheets.googleapis.com/v4/spreadsheets/{}/values",
        spreadsheet_id
    );
    let mut latest = vec![vec![
        "symbol".into(),
        "interval".into(),
        "as_of".into(),
        "close".into(),
        "trend".into(),
        "action".into(),
        "risk_multiplier".into(),
        "previous_close".into(),
        "gap_pct".into(),
        "next_session_start".into(),
        "opening_confirmation".into(),
        "updated_at".into(),
    ]];
    let now = Utc::now().to_rfc3339();
    let next_session = next_bist_session_start(Utc::now());
    latest.extend(decisions.iter().map(|decision| {
        vec![
            decision.symbol.clone(),
            decision.interval.clone(),
            decision.as_of.to_rfc3339(),
            decision.close.to_string(),
            decision.trend.to_string(),
            decision.action.to_string(),
            decision.risk_multiplier.to_string(),
            decision
                .previous_close
                .map(|value| value.to_string())
                .unwrap_or_default(),
            decision
                .gap_pct
                .map(|value| value.to_string())
                .unwrap_or_default(),
            next_session.to_rfc3339(),
            "PENDING".into(),
            now.clone(),
        ]
    }));
    let update_url = format!("{}/{}", base, config.latest_range);
    let update = client
        .put(update_url)
        .bearer_auth(&token)
        .query(&[("valueInputOption", "RAW")])
        .json(&ValuesUpdate {
            range: &config.latest_range,
            major_dimension: "ROWS",
            values: latest,
        })
        .send()
        .await
        .map_err(|error| format!("Google Sheets Latest yazılamadı: {error}"))?;
    if !update.status().is_success() {
        return Err(format!("Google Sheets Latest HTTP {}", update.status()));
    }
    let log = ImportLogRow {
        run_id: uuid::Uuid::new_v4().to_string(),
        finished_at: now,
        rows_read,
        rows_imported: decisions.len(),
        rows_rejected,
        source_tab: config.range.clone(),
        latest_candle: latest_candle
            .map(|value| value.to_rfc3339())
            .unwrap_or_default(),
        data_freshness: latest_candle
            .map(|value| freshness_label(value))
            .unwrap_or_else(|| "UNAVAILABLE".into()),
        error_summary: error_summary.to_owned(),
        status: "IMPORTED",
    };
    let log_url = format!("{}/{}:append", base, config.import_log_range);
    let log_response = client
        .post(log_url)
        .bearer_auth(&token)
        .query(&[
            ("valueInputOption", "RAW"),
            ("insertDataOption", "INSERT_ROWS"),
        ])
        .json(&ValuesUpdate {
            range: &config.import_log_range,
            major_dimension: "ROWS",
            values: vec![vec![
                log.run_id,
                log.finished_at,
                log.rows_read.to_string(),
                log.rows_imported.to_string(),
                log.rows_rejected.to_string(),
                log.source_tab,
                log.latest_candle,
                log.data_freshness,
                log.error_summary,
                log.status.into(),
            ]],
        })
        .send()
        .await
        .map_err(|error| format!("Google Sheets ImportLog yazılamadı: {error}"))?;
    if !log_response.status().is_success() {
        return Err(format!(
            "Google Sheets ImportLog HTTP {}",
            log_response.status()
        ));
    }
    if config.auto_clear {
        let clear_url = format!("{}/{}:clear", base, config.range);
        let clear_response = client
            .post(clear_url)
            .bearer_auth(&token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|error| format!("Google Sheets inbox temizlenemedi: {error}"))?;
        if !clear_response.status().is_success() {
            return Err(format!(
                "Google Sheets inbox temizleme HTTP {}",
                clear_response.status()
            ));
        }
    }
    Ok(())
}

fn candles_for_sheet(
    rows: &[BistSheetCandle],
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Vec<Candle> {
    let mut candles = rows
        .iter()
        .filter(|row| row.symbol == symbol && row.interval.eq_ignore_ascii_case(interval))
        .map(|row| row.candle.clone())
        .collect::<Vec<_>>();
    candles.sort_by_key(|candle| candle.timestamp);
    if candles.len() > limit {
        candles.split_off(candles.len() - limit)
    } else {
        candles
    }
}

fn control_rows_from_sheet(rows: &[BistSheetCandle]) -> HashMap<String, BistControlRow> {
    rows.iter()
        .map(|row| {
            (
                row.symbol.clone(),
                BistControlRow {
                    enabled: row.enabled,
                    manual_veto: row.manual_veto,
                    risk_multiplier: row.risk_multiplier,
                },
            )
        })
        .collect()
}

/// Manifestoyu SQLite'daki BIST günlük serilerinin sembolleriyle deterministik birleştirir.
pub fn merge_universe(
    manifest_symbols: &[String],
    conn: &rusqlite::Connection,
    include_db_symbols: bool,
) -> Result<Vec<String>, String> {
    let mut symbols = manifest_symbols
        .iter()
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    if include_db_symbols {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT symbol FROM candles \
                 WHERE exchange = 'bist' AND market = 'bist' AND interval = '1d' \
                 AND TRIM(symbol) <> '' ORDER BY symbol",
            )
            .map_err(|e| format!("BIST candle evreni sorgusu hazırlanamadı: {e}"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("BIST candle evreni okunamadı: {e}"))?;
        for row in rows {
            let symbol = row.map_err(|e| format!("BIST candle sembolü okunamadı: {e}"))?;
            symbols.insert(symbol.trim().to_ascii_uppercase());
        }
    }
    Ok(symbols.into_iter().collect())
}

pub fn intraday_confirmation(
    symbol: &str,
    interval: &str,
    candles: &[Candle],
    higher_timeframe_uptrend: bool,
) -> Option<IntradayConfirmation> {
    if candles.len() < EMA_FAST + 1 {
        return None;
    }
    let closes = candles.iter().map(|c| c.close).collect::<Vec<_>>();
    let close = *closes.last()?;
    let ema20 = ema(&closes, 20)?;
    let ema50 = ema(&closes, EMA_FAST)?;
    let confirmed = higher_timeframe_uptrend && ema20 > ema50 && close > ema20;
    let status = if confirmed { "CONFIRMED" } else { "WARNING" };
    let reason = if !higher_timeframe_uptrend {
        "yüksek zaman diliminde yukarı trend teyidi yok; bağımsız giriş üretilmedi".into()
    } else if ema20 <= ema50 || close <= ema20 {
        format!("intraday teyidi zayıf: EMA20={ema20:.2}, EMA50={ema50:.2}, close={close:.2}")
    } else {
        format!("HTF yukarı trend + EMA20>EMA50 ve close EMA20 üstünde ({close:.2})")
    };
    Some(IntradayConfirmation {
        symbol: symbol.into(),
        interval: interval.into(),
        as_of: candles.last()?.timestamp,
        close,
        ema20,
        ema50,
        higher_timeframe_uptrend,
        status,
        reason,
    })
}

pub async fn update_and_confirm_intraday(
    config: &BistIntradayConfig,
) -> Result<Vec<IntradayConfirmation>, String> {
    let manifest_symbols = load_symbols(&config.symbols_path)?;
    let sheet_rows = load_sheet_candles(&config.sheets).await?;
    let control_rows = control_rows_from_sheet(&sheet_rows);
    let symbols = apply_control_rows(
        manifest_symbols,
        Some(&control_rows),
        config.sheets.require_match,
    )?;
    let conn = open_db(&config.db_path).map_err(|e| format!("BIST DB açılamadı: {e}"))?;
    let mut skipped = HashSet::new();
    let mut results = Vec::new();
    for interval in &config.intervals {
        for symbol in &symbols {
            let candles = candles_for_sheet(&sheet_rows, symbol, interval, config.candle_limit);
            if candles.is_empty() {
                // Bir sembolün eksik Sheets serisi kalan sembolleri durdurmamalı.
                skipped.insert((symbol.clone(), interval.clone()));
                results.push(IntradayConfirmation {
                    symbol: symbol.clone(),
                    interval: interval.clone(),
                    as_of: Utc::now(),
                    close: 0.0,
                    ema20: 0.0,
                    ema50: 0.0,
                    higher_timeframe_uptrend: false,
                    status: "SKIPPED",
                    reason: "Google Sheets OHLC verisi yok; sembol atlandı".into(),
                });
                continue;
            }
            for candle in &candles {
                save_candle(&conn, "bist", "bist", candle)
                    .map_err(|e| format!("{symbol} {interval}: candle import hatası: {e}"))?;
            }
        }
    }
    for interval in &config.intervals {
        for symbol in &symbols {
            if skipped.contains(&(symbol.clone(), interval.clone())) {
                continue;
            }
            let candles = read_candles_market(
                &config.db_path,
                symbol,
                interval,
                "bist",
                config.candle_limit,
            )
            .map_err(|e| format!("{symbol} {interval}: intraday candle okuma hatası: {e}"))?;
            let htf = read_candles_market(
                &config.db_path,
                symbol,
                &config.higher_timeframe,
                "bist",
                EMA_SLOW + DONCHIAN + 1,
            )
            .map_err(|e| format!("{symbol} HTF candle okuma hatası: {e}"))?;
            let htf_uptrend = decision_for_interval(symbol, &config.higher_timeframe, &htf)
                .map(|d| d.trend == "UPTREND")
                .unwrap_or(false);
            if let Some(result) = intraday_confirmation(symbol, interval, &candles, htf_uptrend) {
                results.push(result);
            }
        }
    }
    if let Some(parent) = std::path::Path::new(&config.report_path).parent() {
        fs::create_dir_all(parent).map_err(|e| format!("intraday rapor klasörü açılamadı: {e}"))?;
    }
    fs::write(
        &config.report_path,
        serde_json::to_vec_pretty(&results).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("intraday raporu yazılamadı: {e}"))?;
    Ok(results)
}

/// Veriyi SQLite'a upsert eder, aynı güncel seriden trend kararlarını üretir.
pub async fn update_and_scan(config: &BistTrendConfig) -> Result<Vec<TrendDecision>, String> {
    let _run_lock = BistRunLock::acquire()?;
    let run_started = Utc::now();
    let manifest_symbols = load_symbols(&config.symbols_path)?;
    let sheet_rows = load_sheet_candles(&config.sheets).await?;
    let control_rows = control_rows_from_sheet(&sheet_rows);
    let conn = match open_db(&config.db_path) {
        Ok(conn) => conn,
        Err(error) => {
            let status = BistHealthStatus {
                expected_symbol_count: 0,
                universe_symbol_count: 0,
                expected_download_count: 0,
                successful_download_count: 0,
                skipped_error_count: 1,
                errors: vec![format!("BIST DB açılamadı: {error}")],
                latest_candle_timestamp: None,
                run_started,
                run_finished: Utc::now(),
                configured_interval: config.interval.clone(),
                configured_intervals: config.intervals.clone(),
                status: "UNAVAILABLE".into(),
                source_tab: config.sheets.range.clone(),
                data_freshness: "UNAVAILABLE".into(),
            };
            let _ = write_health_status(&config.status_path, &status);
            return Err(format!("BIST DB açılamadı: {error}"));
        }
    };
    let symbols = apply_control_rows(
        merge_universe(&manifest_symbols, &conn, config.include_db_symbols)?,
        Some(&control_rows),
        config.sheets.require_match,
    )?;
    let expected = symbols.len() * config.intervals.len();
    let mut successful = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();
    let mut latest_candle_timestamp: Option<DateTime<Utc>> = None;
    for interval in &config.intervals {
        for symbol in &symbols {
            let candles = candles_for_sheet(&sheet_rows, symbol, interval, config.candle_limit);
            if candles.is_empty() {
                skipped += 1;
                errors.push(format!(
                    "{symbol} {interval}: Google Sheets OHLC verisi yok"
                ));
                continue;
            }
            successful += 1;
            for candle in &candles {
                latest_candle_timestamp = Some(
                    latest_candle_timestamp
                        .map(|current| current.max(candle.timestamp))
                        .unwrap_or(candle.timestamp),
                );
            }
            for candle in &candles {
                if let Err(error) = save_candle(&conn, "bist", "bist", candle) {
                    skipped += 1;
                    errors.push(format!("{symbol} {interval}: candle import: {error}"));
                    break;
                }
            }
        }
    }

    let mut decisions = Vec::new();
    for interval in &config.intervals {
        for symbol in &symbols {
            let candles = match read_candles_market(
                &config.db_path,
                symbol,
                interval,
                "bist",
                config.candle_limit,
            ) {
                Ok(candles) => candles,
                Err(error) => {
                    skipped += 1;
                    errors.push(format!("{symbol} {interval}: DB okuma: {error}"));
                    continue;
                }
            };
            for candle in &candles {
                latest_candle_timestamp = Some(
                    latest_candle_timestamp
                        .map(|current| current.max(candle.timestamp))
                        .unwrap_or(candle.timestamp),
                );
            }
            if let Some(mut result) = decision_for_interval(symbol, interval, &candles) {
                result.gap_pct = result.previous_close.map(|previous| {
                    if previous == 0.0 {
                        0.0
                    } else {
                        (result.close - previous) / previous * 100.0
                    }
                });
                result.risk_multiplier = control_rows
                    .get(symbol)
                    .map(|row| row.risk_multiplier)
                    .unwrap_or(1.0);
                decisions.push(result);
            }
        }
    }
    let freshness = health_status(
        latest_candle_timestamp,
        successful,
        expected,
        skipped,
        &config.interval,
    );
    if freshness == "STALE" {
        errors.push(format!(
            "freshness gate: {} verisi bayat; karar üretimi engellendi",
            config.interval
        ));
        decisions.clear();
    }
    decisions.sort_by(|a, b| a.symbol.cmp(&b.symbol).then(a.interval.cmp(&b.interval)));
    if let Some(parent) = std::path::Path::new(&config.report_path).parent() {
        fs::create_dir_all(parent).map_err(|e| format!("trend rapor klasörü açılamadı: {e}"))?;
    }
    fs::write(
        &config.report_path,
        serde_json::to_vec_pretty(&decisions)
            .map_err(|e| format!("trend raporu serileştirilemedi: {e}"))?,
    )
    .map_err(|e| format!("trend raporu yazılamadı: {e}"))?;
    errors.truncate(50);
    let status = BistHealthStatus {
        expected_symbol_count: symbols.len(),
        universe_symbol_count: symbols.len(),
        expected_download_count: expected,
        successful_download_count: successful,
        skipped_error_count: skipped,
        errors,
        latest_candle_timestamp,
        run_started,
        run_finished: Utc::now(),
        configured_interval: config.interval.clone(),
        configured_intervals: config.intervals.clone(),
        status: freshness.into(),
        source_tab: config.sheets.range.clone(),
        data_freshness: latest_candle_timestamp
            .map(freshness_label)
            .unwrap_or_else(|| "UNAVAILABLE".into()),
    };
    write_health_status(&config.status_path, &status)?;
    publish_latest_and_log(
        &config.sheets,
        &decisions,
        sheet_rows.len().saturating_sub(1),
        skipped,
        latest_candle_timestamp,
        &status.errors.join(" | "),
    )
    .await
    .map_err(|error| format!("Google Sheets yayın/temizleme başarısız: {error}"))?;
    Ok(decisions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn candles(values: &[f64]) -> Vec<Candle> {
        values
            .iter()
            .enumerate()
            .map(|(index, close)| Candle {
                timestamp: Utc::now() + chrono::Duration::days(index as i64),
                open: *close,
                high: close + 1.0,
                low: close - 1.0,
                close: *close,
                volume: 1.0,
                symbol: "TEST".into(),
                interval: "1d".into(),
            })
            .collect()
    }

    #[test]
    fn interval_validation_rejects_unsafe_values() {
        assert!(validate_interval("1d").is_ok());
        assert!(validate_interval("1wk").is_ok());
        assert!(validate_interval("15m").is_err());
        assert_eq!(parse_intervals("1d, 1wk,1d").unwrap(), ["1d", "1wk"]);
        assert!(parse_intervals(" ").is_err());
        assert_eq!(
            parse_intraday_intervals("15m, 30m,15m").unwrap(),
            ["15m", "30m"]
        );
        assert!(validate_intraday_range(&["15m".into()], "1y").is_err());
        assert!(validate_intraday_range(&["1h".into()], "730d").is_ok());
    }

    #[test]
    fn sheets_control_filters_disabled_and_vetoed_symbols() {
        let rows = parse_control_csv(
            "symbol,enabled,manual_veto,risk_multiplier,note\n\
             THYAO,1,0,1.0,ok\n\
             ASELS,0,0,1.0,disabled\n\
             EREGL,1,1,1.0,veto\n",
        )
        .unwrap();
        let symbols = apply_control_rows(
            vec!["THYAO".into(), "ASELS".into(), "EREGL".into()],
            Some(&rows),
            true,
        )
        .unwrap();
        assert_eq!(symbols, vec!["THYAO"]);
    }

    #[test]
    fn sheets_control_requires_intersection_when_configured() {
        let rows = parse_control_csv("symbol,enabled\nGARAN,1\n").unwrap();
        let error = apply_control_rows(vec!["THYAO".into()], Some(&rows), true).unwrap_err();
        assert!(error.contains("kesişmiyor"));
    }

    #[test]
    fn sheets_api_rows_parse_ohlc_and_controls() {
        let rows = parse_sheet_rows(
            r#"{"values":[
                ["symbol","date","open","high","low","close","volume","interval","enabled","manual_veto","risk_multiplier"],
                ["THYAO","2026-09-25","300,0","310,0","295,0","305,0","1200","1d","1","0","0.75"]
            ]}"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "THYAO");
        assert_eq!(rows[0].candle.close, 305.0);
        assert_eq!(rows[0].risk_multiplier, 0.75);
        assert!(rows[0].enabled);
        assert!(!rows[0].manual_veto);
    }

    #[test]
    fn sheets_api_rows_deduplicate_symbol_interval_timestamp() {
        let rows = parse_sheet_rows(
            r#"{"values":[
                ["symbol","date","open","high","low","close","volume","interval"],
                ["THYAO","2026-09-25","300","310","295","305","1200","1d"],
                ["THYAO","2026-09-25","300","310","295","305","1200","1d"]
            ]}"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn health_status_round_trips_safe_json() {
        let status = BistHealthStatus {
            expected_symbol_count: 2,
            universe_symbol_count: 2,
            expected_download_count: 2,
            successful_download_count: 1,
            skipped_error_count: 1,
            errors: vec!["THYAO: timeout".into()],
            latest_candle_timestamp: Some(Utc::now()),
            run_started: Utc::now(),
            run_finished: Utc::now(),
            configured_interval: "1d".into(),
            configured_intervals: vec!["1d".into()],
            status: "DEGRADED".into(),
            source_tab: "OHCL_INBOX_1d".into(),
            data_freshness: "STALE_24H".into(),
        };
        let encoded = serde_json::to_vec(&status).unwrap();
        let decoded: BistHealthStatus = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.status, "DEGRADED");
        assert_eq!(decoded.errors, vec!["THYAO: timeout"]);
    }

    #[test]
    fn intraday_requires_htf_confirmation() {
        let candles = (1..=60)
            .map(|i| Candle {
                timestamp: Utc::now(),
                open: i as f64,
                high: i as f64 + 1.0,
                low: i as f64 - 1.0,
                close: i as f64,
                volume: 1.0,
                symbol: "TEST".into(),
                interval: "15m".into(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            intraday_confirmation("TEST", "15m", &candles, false)
                .unwrap()
                .status,
            "WARNING"
        );
        assert_eq!(
            intraday_confirmation("TEST", "15m", &candles, true)
                .unwrap()
                .status,
            "CONFIRMED"
        );
    }

    #[test]
    fn universe_merges_manifest_and_bist_daily_db_symbols_deterministically() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE candles (exchange TEXT, market TEXT, symbol TEXT, interval TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO candles VALUES ('bist','bist','THYAO','1d')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO candles VALUES ('bist','bist',' ASELS ','1d')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO candles VALUES ('binance','spot','BTCUSDT','1d')",
            [],
        )
        .unwrap();
        let result = merge_universe(&["KCHOL".into(), "thyao".into()], &conn, true).unwrap();
        assert_eq!(result, ["ASELS", "KCHOL", "THYAO"]);
        assert_eq!(
            merge_universe(&["KCHOL".into()], &conn, false).unwrap(),
            ["KCHOL"]
        );
    }

    #[test]
    fn actionable_decision_exposes_reference_prices_and_manual_note() {
        let mut values = (1..=179).map(|v| v as f64).collect::<Vec<_>>();
        values.push(300.0);
        let result = decision("TEST", &candles(&values)).unwrap();
        assert_eq!(result.suggested_entry, Some(180.0));
        let plan = result.pyramid.unwrap();
        assert!(plan.add_after_atr[0] > plan.initial_entry);
        assert!(plan.protective_stop < plan.initial_entry);
        assert!(result.manual_note.contains("emir gönderilmez"));
    }

    #[test]
    fn opportunity_lifecycle_suppresses_duplicate_alerts() {
        let path = std::env::temp_dir().join(format!(
            "memos-bist-opportunity-{}.json",
            std::process::id()
        ));
        let mut values = (1..=179).map(|v| v as f64).collect::<Vec<_>>();
        values.push(300.0);
        let mut first = vec![decision("TEST", &candles(&values)).unwrap()];
        let mut second = first.clone();
        let first_alerts =
            update_opportunity_lifecycle(path.to_str().unwrap(), &mut first).unwrap();
        let second_alerts =
            update_opportunity_lifecycle(path.to_str().unwrap(), &mut second).unwrap();
        assert_eq!(first_alerts.len(), 1);
        assert_eq!(second_alerts.len(), 0);
        assert_eq!(second[0].opportunity_status, "ACTIVE");
        let _ = std::fs::remove_file(path);
    }
}
