/// diag.rs — Sistem Sağlık Tanılaması
///
/// Her alt sistemi sırayla kontrol eder, ✅/⚠️/❌ ile gösterir.
/// Trade yapmaz, loop başlatmaz. Yaklaşık 2–5 saniyede tamamlanır.
///
/// Kullanım:
///   cargo run -p memos_trading_core --bin diag
///   cargo run -p memos_trading_core --bin diag -- --full   (Binance ping dahil)

use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ─── Renk sabitleri (ANSI) ──────────────────────────────────────────────────
const GREEN:  &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED:    &str = "\x1b[31m";
const CYAN:   &str = "\x1b[36m";
const BOLD:   &str = "\x1b[1m";
const RESET:  &str = "\x1b[0m";

// ─── Durum sayacı ────────────────────────────────────────────────────────────
struct Report {
    ok:   u32,
    warn: u32,
    fail: u32,
    step: u32,
    total: u32,
}

impl Report {
    fn new(total: u32) -> Self { Self { ok: 0, warn: 0, fail: 0, step: 0, total } }

    fn ok(&mut self, label: &str, detail: &str) {
        self.step += 1; self.ok += 1;
        println!("{GREEN}[{:>2}/{}] ✅ {:<32}{RESET} {}", self.step, self.total, label, detail);
    }
    fn warn(&mut self, label: &str, detail: &str) {
        self.step += 1; self.warn += 1;
        println!("{YELLOW}[{:>2}/{}] ⚠️  {:<32}{RESET} {}", self.step, self.total, label, detail);
    }
    fn fail(&mut self, label: &str, detail: &str) {
        self.step += 1; self.fail += 1;
        println!("{RED}[{:>2}/{}] ❌ {:<32}{RESET} {}", self.step, self.total, label, detail);
    }
    fn info(&mut self, label: &str, detail: &str) {
        self.step += 1;
        println!("{CYAN}[{:>2}/{}] ℹ️  {:<32}{RESET} {}", self.step, self.total, label, detail);
    }
}

// ─── Yardımcı: JSON dosyasını aç ────────────────────────────────────────────
fn load_json(path: &str) -> Option<Value> {
    let txt = fs::read_to_string(path).ok()?;
    serde_json::from_str(&txt).ok()
}

// ─── Yardımcı: dosya yaşı (saniye) ──────────────────────────────────────────
fn file_age_secs(path: &str) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let mtime = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(now.saturating_sub(mtime))
}

fn age_str(secs: u64) -> String {
    if secs < 60        { format!("{}sn önce", secs) }
    else if secs < 3600 { format!("{:.0}dk önce", secs as f64 / 60.0) }
    else if secs < 86400 { format!("{:.1}sa önce", secs as f64 / 3600.0) }
    else                 { format!("{:.1}gün önce", secs as f64 / 86400.0) }
}

fn file_size_str(path: &str) -> String {
    match fs::metadata(path).map(|m| m.len()) {
        Ok(b) if b >= 1_048_576 => format!("{:.1}MB", b as f64 / 1_048_576.0),
        Ok(b) if b >= 1024      => format!("{:.1}KB", b as f64 / 1024.0),
        Ok(b)                   => format!("{}B", b),
        Err(_)                  => "?".into(),
    }
}

// ─── Ana diagnostic ─────────────────────────────────────────────────────────
fn main() {
    let full_mode = std::env::args().any(|a| a == "--full");
    let total: u32 = if full_mode { 12 } else { 11 };

    println!();
    println!("{BOLD}══════════════════════════════════════════════════════{RESET}");
    println!("{BOLD}  Memos Trading — Sistem Sağlık Tanılaması            {RESET}");
    println!("{BOLD}══════════════════════════════════════════════════════{RESET}");
    println!();

    let mut r = Report::new(total);

    // ── [1] Config Dosyaları ────────────────────────────────────────────────
    let config_files = [
        "config/rtc_config.json",
        "config/trade_quality.json",
        "config/robotic_profiles.json",
        "config/adaptive_params.json",
    ];
    let mut cfg_ok = vec![];
    let mut cfg_fail = vec![];
    for f in &config_files {
        if load_json(f).is_some() {
            cfg_ok.push(Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string());
        } else {
            cfg_fail.push(Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string());
        }
    }
    if cfg_fail.is_empty() {
        r.ok("Config Dosyaları", &format!("{} dosya geçerli JSON", cfg_ok.len()));
    } else {
        r.fail("Config Dosyaları", &format!("EKSIK/BOZUK: {}", cfg_fail.join(", ")));
    }

    // ── [2] Ortam Değişkenleri ─────────────────────────────────────────────
    let api_key    = std::env::var("BINANCE_API_KEY").unwrap_or_default();
    let api_secret = std::env::var("BINANCE_API_SECRET").unwrap_or_default();
    let paper_mode = std::env::var("BINANCE_PAPER_MODE").unwrap_or_else(|_| "true".into());
    let symbol     = std::env::var("TRADE_SYMBOL").unwrap_or_else(|_| "BTCUSDT".into());
    let market     = std::env::var("TRADE_MARKET").unwrap_or_else(|_| "spot".into());

    if api_key.is_empty() || api_secret.is_empty() {
        r.warn(
            "Ortam Değişkenleri",
            &format!("API key YOK → paper={} sym={} market={}", paper_mode, symbol, market),
        );
    } else {
        r.ok(
            "Ortam Değişkenleri",
            &format!("API key SET, paper={}, sym={} {}", paper_mode, symbol, market),
        );
    }

    // ── [3] SQLite Veritabanı ──────────────────────────────────────────────
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    match Connection::open(&db_path) {
        Err(e) => r.fail("SQLite Veritabanı", &format!("{}: {}", db_path, e)),
        Ok(conn) => {
            // Tablo listesi
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table'")
                .and_then(|mut s| {
                    s.query_map([], |row| row.get(0))
                        .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();

            // candles tablosu var mı?
            if tables.iter().any(|t| t == "candles") {
                let count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM candles", [], |r| r.get(0))
                    .unwrap_or(0);
                let symbols: i64 = conn
                    .query_row("SELECT COUNT(DISTINCT symbol) FROM candles", [], |r| r.get(0))
                    .unwrap_or(0);
                if count > 0 {
                    r.ok(
                        "SQLite Veritabanı",
                        &format!("{} mum kaydı, {} sembol  ({})", count, symbols, file_size_str(&db_path)),
                    );
                } else {
                    r.warn("SQLite Veritabanı", "candles tablosu boş — download bekleniyor");
                }
            } else {
                r.warn(
                    "SQLite Veritabanı",
                    &format!("candles tablosu yok. Tablolar: {}", tables.join(", ")),
                );
            }
        }
    }

    // ── [4] GNB Classifier (ML Giriş Filtresi) ────────────────────────────
    let clf_path = "config/classifier_state.json";
    match load_json(clf_path) {
        None => r.warn("GNB Classifier", "classifier_state.json yok — cold-start modu"),
        Some(v) => {
            let trained   = v["classifier"]["is_trained"].as_bool().unwrap_or(false);
            let n_win     = v["classifier"]["n_win"].as_u64().unwrap_or(0);
            let n_loss    = v["classifier"]["n_loss"].as_u64().unwrap_or(0);
            let buf_len   = v["buffer"].as_array().map(|a| a.len()).unwrap_or(0);
            let total_clf = n_win + n_loss;

            if trained {
                r.ok(
                    "GNB Classifier",
                    &format!("Eğitilmiş — {} kazanç / {} kayıp (toplam {})", n_win, n_loss, total_clf),
                );
            } else if buf_len > 0 {
                r.warn(
                    "GNB Classifier",
                    &format!("Henüz eğitilmedi — buffer: {}/20 örnek", buf_len),
                );
            } else {
                r.warn("GNB Classifier", "Cold-start — buffer boş, min 20 trade bekleniyor");
            }
        }
    }

    // ── [5] UCB1 Strategy Scorer ───────────────────────────────────────────
    let scorer_path = "config/strategy_scorer_state.json";
    match load_json(scorer_path) {
        None => r.warn("UCB1 Strategy Scorer", "strategy_scorer_state.json yok — cold-start"),
        Some(v) => {
            let total_n       = v["total_n"].as_u64().unwrap_or(0);
            let scalp_dis     = v["scalp_disabled"].as_bool().unwrap_or(false);
            let swing_dis     = v["swing_disabled"].as_bool().unwrap_or(false);
            let reg_dis       = v["reg_disabled"].as_bool().unwrap_or(false);
            let last_reason   = v["last_reason"].as_str().unwrap_or("").to_string();

            // Kaç kol dolu?
            let empty_vec = vec![];
            let populated: usize = v["arms"]
                .as_array()
                .unwrap_or(&empty_vec)
                .iter()
                .flat_map(|row| {
                    let ev2: Vec<Value> = row.as_array()
                        .unwrap_or(&empty_vec)
                        .iter()
                        .cloned()
                        .collect();
                    ev2.into_iter()
                })
                .filter(|arm| arm["n"].as_u64().unwrap_or(0) > 0)
                .count();

            let disabled: Vec<&str> = [
                if scalp_dis { Some("Scalp") } else { None },
                if swing_dis { Some("Swing") } else { None },
                if reg_dis   { Some("Regular") } else { None },
            ]
            .iter()
            .flatten()
            .copied()
            .collect();

            if total_n == 0 {
                r.warn("UCB1 Strategy Scorer", "Henüz trade kaydı yok — tüm kollar eşit başlıyor");
            } else if !disabled.is_empty() {
                r.warn(
                    "UCB1 Strategy Scorer",
                    &format!(
                        "{} trade, {}/{} kol dolu. DEVRE DIŞI: {}  ({})",
                        total_n, populated, 12,
                        disabled.join("+"),
                        if last_reason.is_empty() { "sebep yok".to_string() } else { last_reason.chars().take(60).collect() }
                    ),
                );
            } else {
                r.ok(
                    "UCB1 Strategy Scorer",
                    &format!("{} trade, {}/12 kol dolu", total_n, populated),
                );
            }
        }
    }

    // ── [6] Equity Snapshot ────────────────────────────────────────────────
    let eq_path = "config/equity_snapshot.json";
    match load_json(eq_path) {
        None => r.info("Equity Snapshot", "Henüz oluşturulmadı — ilk WAL kontrol noktasında yazılır"),
        Some(v) => {
            let capital   = v["capital"].as_f64().unwrap_or(0.0);
            let cum_pnl   = v["cumulative_pnl"].as_f64().unwrap_or(0.0);
            let peak_eq   = v["peak_equity"].as_f64().unwrap_or(capital);
            let current   = capital + cum_pnl;
            let dd_pct    = if peak_eq > 0.0 { (peak_eq - current) / peak_eq * 100.0 } else { 0.0 };
            let age       = file_age_secs(eq_path).unwrap_or(0);

            if dd_pct > 15.0 {
                r.warn(
                    "Equity Snapshot",
                    &format!(
                        "cumPnL={:+.2}$  peak={:.0}$  DD={:.1}%  ({})  ⚠️ Yüksek DD",
                        cum_pnl, peak_eq, dd_pct, age_str(age)
                    ),
                );
            } else {
                r.ok(
                    "Equity Snapshot",
                    &format!(
                        "cumPnL={:+.2}$  peak={:.0}$  DD={:.1}%  ({})",
                        cum_pnl, peak_eq, dd_pct, age_str(age)
                    ),
                );
            }
        }
    }

    // ── [7] Evolution / FSM Snapshot ──────────────────────────────────────
    let evo_path = "config/evolution_state.json";
    let fsm_path = "config/fsm_state.json";
    let evo_ok   = load_json(evo_path).is_some();
    let fsm_ok   = load_json(fsm_path).is_some();

    if evo_ok && fsm_ok {
        let evo_age = file_age_secs(evo_path).map(age_str).unwrap_or_default();
        let fsm_state = load_json(fsm_path)
            .and_then(|v| v["state"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "?".into());
        let cycle_id = load_json(fsm_path)
            .and_then(|v| v["cycle_id"].as_u64())
            .unwrap_or(0);
        r.ok(
            "Evolution/FSM Snapshot",
            &format!("FSM state={} cycle_id={}  (evo: {})", fsm_state, cycle_id, evo_age),
        );
    } else {
        let missing: Vec<&str> = [
            if !evo_ok { Some("evolution_state.json") } else { None },
            if !fsm_ok { Some("fsm_state.json") } else { None },
        ]
        .iter().flatten().copied().collect();
        r.warn("Evolution/FSM Snapshot", &format!("Eksik: {}  — cold-start modu", missing.join(", ")));
    }

    // ── [8] Adaptive Params ────────────────────────────────────────────────
    let ap_path = "config/adaptive_params.json";
    match load_json(ap_path) {
        None => r.warn("Adaptive Params", "adaptive_params.json yok — varsayılanlar kullanılır"),
        Some(v) => {
            let sl_m   = v["sl_atr_multiplier"].as_f64().unwrap_or(0.0);
            let tp_m   = v["tp_atr_multiplier"].as_f64().unwrap_or(0.0);
            let tsl    = v["trailing_sl_activation_pct"].as_f64().unwrap_or(0.0);
            let consec = v["max_consecutive_losses"].as_u64().unwrap_or(0);
            let daily  = v["max_daily_sl_per_symbol"].as_u64().unwrap_or(0);
            let age    = file_age_secs(ap_path).map(age_str).unwrap_or_default();

            r.ok(
                "Adaptive Params",
                &format!(
                    "SL={:.2}×ATR  TP={:.2}×ATR  TSL={:.1}%  maxConsecLoss={}  dailySL={}  ({})",
                    sl_m, tp_m, tsl, consec, daily, age
                ),
            );
        }
    }

    // ── [9] Log Dosyaları ──────────────────────────────────────────────────
    let log_files = [
        "logs/robotic_trader.log",
        "logs/trade_history.jsonl",
    ];
    let mut log_details = vec![];
    let mut log_stale   = false;

    for f in &log_files {
        if !Path::new(f).exists() {
            log_details.push(format!("{}: YOK", Path::new(f).file_name().unwrap_or_default().to_string_lossy()));
            continue;
        }
        let size = file_size_str(f);
        let age  = file_age_secs(f).unwrap_or(u64::MAX);
        if age > 3600 { log_stale = true; }
        log_details.push(format!(
            "{}: {} ({})",
            Path::new(f).file_name().unwrap_or_default().to_string_lossy(),
            size,
            age_str(age)
        ));
    }

    if log_stale {
        r.warn("Log Dosyaları", &log_details.join("  |  "));
    } else {
        r.ok("Log Dosyaları", &log_details.join("  |  "));
    }

    // ── [10] App Snapshot (özet) ───────────────────────────────────────────
    let snap_path = "config/app_snapshot.json";
    match load_json(snap_path) {
        None => r.warn("App Snapshot", "app_snapshot.json yok — ilk çalışmada oluşturulur"),
        Some(v) => {
            let total_trades   = v["total_trades"].as_u64().unwrap_or(0);
            let ml_confidence  = v["ml_confidence"].as_f64().unwrap_or(0.0);
            let best_strategy  = v["best_strategy_name"].as_str().unwrap_or("?");
            let symbol_snap    = v["active_symbol"]["symbol"].as_str().unwrap_or("?");
            let interval_snap  = v["active_symbol"]["interval"].as_str().unwrap_or("?");
            let age            = file_age_secs(snap_path).map(age_str).unwrap_or_default();

            r.ok(
                "App Snapshot",
                &format!(
                    "trades={} conf={:.2} strat={} sym={} {} ({})",
                    total_trades, ml_confidence, best_strategy, symbol_snap, interval_snap, age
                ),
            );
        }
    }

    // ── [11] Binance Bağlantısı (--full gerekli değil, basit ping) ─────────
    {
        // Public REST endpoint — auth gerektirmez
        let url = "https://api.binance.com/api/v3/ping";
        match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .and_then(|c| c.get(url).send())
        {
            Ok(resp) if resp.status().is_success() => {
                r.ok("Binance Bağlantısı", "api.binance.com/api/v3/ping → 200 OK");
            }
            Ok(resp) => {
                r.warn("Binance Bağlantısı", &format!("HTTP {}", resp.status()));
            }
            Err(e) => {
                r.fail("Binance Bağlantısı", &format!("Ulaşılamıyor: {}", e));
            }
        }
    }

    // ── [12] Fiyat kontrolü (--full ile) ──────────────────────────────────
    if full_mode {
        let sym    = std::env::var("TRADE_SYMBOL").unwrap_or_else(|_| "BTCUSDT".into());
        let market = std::env::var("TRADE_MARKET").unwrap_or_else(|_| "spot".into());
        let url = if market.to_lowercase() == "futures" {
            format!("https://fapi.binance.com/fapi/v1/ticker/price?symbol={}", sym)
        } else {
            format!("https://api.binance.com/api/v3/ticker/price?symbol={}", sym)
        };
        match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .and_then(|c| c.get(&url).send())
            .and_then(|r| r.json::<Value>())
        {
            Ok(v) => {
                let price = v["price"].as_str().unwrap_or("?");
                r.ok("Fiyat Kontrolü", &format!("{} {} = {} USDT", market, sym, price));
            }
            Err(e) => {
                r.warn("Fiyat Kontrolü", &format!("{} fiyat alınamadı: {}", sym, e));
            }
        }
    }

    // ── Özet ──────────────────────────────────────────────────────────────
    println!();
    println!("{BOLD}══════════════════════════════════════════════════════{RESET}");
    println!(
        "  {GREEN}✅ {}{RESET}   {YELLOW}⚠️  {}{RESET}   {RED}❌ {}{RESET}",
        r.ok, r.warn, r.fail
    );
    println!("{BOLD}══════════════════════════════════════════════════════{RESET}");
    println!();

    if r.fail > 0 {
        println!("{RED}Kritik sorun var — yukarıdaki ❌ satırlarını incele.{RESET}");
        std::process::exit(2);
    } else if r.warn > 0 {
        println!("{YELLOW}Uyarılar var — ⚠️  satırları gözden geçir.{RESET}");
        std::process::exit(1);
    } else {
        println!("{GREEN}Sistem sağlıklı görünüyor.{RESET}");
        std::process::exit(0);
    }
}
