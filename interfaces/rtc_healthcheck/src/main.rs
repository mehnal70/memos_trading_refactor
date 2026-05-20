// rtc_healthcheck — otonom trading sisteminin sağlığını tek komutla doğrular.
//
// İki kol koşturur:
//   1. Paper smoke         — TradingMode::Paper, gerçek market verisi yok.
//   2. Live dry-run smoke  — TradingMode::Live + LIVE_DRY_RUN=true, dummy/testnet key.
//
// Her kolda Engine::run_autonomous_loop spawn edilir, HEALTHCHECK_DURATION_SECS
// (default 60) boyunca koşar, sonra invariant'lar doğrulanır.
//
// Env override'ları:
//   HEALTHCHECK_DURATION_SECS         — her kolun smoke süresi (default 60).
//   HEALTHCHECK_SKIP_LIVE_DRY_RUN     — 1 → live-dry-run kolunu atla.
//   HEALTHCHECK_MAX_DRAWDOWN_PCT      — equity invariant'ı için drawdown limiti (default 50).
//   HEALTHCHECK_KEEP_ARTIFACTS        — 1 → /tmp altındaki workspace'i temizleme.
//
// Çıkış kodu: 0 → tüm invariant'lar pass; 1 → en az bir invariant fail.

mod checks;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use memos_trading_core::core::model::{RoboticLoopConfig, TradingMode};
use memos_trading_core::robot::engines::master::Engine;
use memos_trading_core::robot::robotic_loop::AppState;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> ExitCode {
    let duration: u64 = env_u64("HEALTHCHECK_DURATION_SECS", 60);
    let skip_live = env_flag("HEALTHCHECK_SKIP_LIVE_DRY_RUN");
    let max_dd_pct: f64 = std::env::var("HEALTHCHECK_MAX_DRAWDOWN_PCT")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(50.0);
    let keep_artifacts = env_flag("HEALTHCHECK_KEEP_ARTIFACTS");

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║      Memos RTC — Otonom Sistem Sağlık Denetimi               ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!("  Smoke süresi (her kol)  : {}s", duration);
    println!("  Drawdown eşiği          : %{}", max_dd_pct);
    println!("  Live dry-run atla       : {}", skip_live);
    println!();

    let mut all_pass = true;

    // ─── Kol 1: Paper smoke ─────────────────────────────────────────────
    println!("─── Kol 1: Paper smoke ────────────────────────────────────────");
    let paper_pass = run_smoke_arm(
        "paper",
        TradingMode::Paper,
        false,
        duration,
        max_dd_pct,
        keep_artifacts,
    ).await;
    println!("  → Kol 1 sonuç: {}\n", verdict(paper_pass));
    all_pass &= paper_pass;

    // ─── Kol 2: Live dry-run smoke ─────────────────────────────────────
    if !skip_live {
        println!("─── Kol 2: Live dry-run smoke ─────────────────────────────────");
        let dry_pass = run_smoke_arm(
            "live_dry_run",
            TradingMode::Live,
            true,
            duration,
            max_dd_pct,
            keep_artifacts,
        ).await;
        println!("  → Kol 2 sonuç: {}\n", verdict(dry_pass));
        all_pass &= dry_pass;
    } else {
        println!("⏭️  Kol 2 atlandı (HEALTHCHECK_SKIP_LIVE_DRY_RUN set)\n");
    }

    // ─── Genel özet ─────────────────────────────────────────────────────
    println!("══════════════════════════════════════════════════════════════");
    if all_pass {
        println!("✅ ALL CHECKS PASSED — sistem sağlıklı");
        ExitCode::SUCCESS
    } else {
        println!("❌ SOME CHECKS FAILED — yukarıdaki ✗ satırlarını incele");
        ExitCode::FAILURE
    }
}

/// Tek bir smoke kolu: tmp workspace kur, Engine spawn et, duration boyunca koş,
/// invariant'ları doğrula, engine'i kapat.
async fn run_smoke_arm(
    label: &str,
    mode: TradingMode,
    dry_run: bool,
    duration_secs: u64,
    max_dd_pct: f64,
    keep_artifacts: bool,
) -> bool {
    let workspace = setup_workspace(label);
    apply_env_for_arm(&workspace, dry_run);

    let cfg = build_config(&workspace, mode, dry_run);
    let state = Arc::new(Mutex::new(AppState::new(cfg)));
    let start_tick = state.lock().unwrap().fleet.last_loop_tick.load(Ordering::Relaxed);

    // Engine'i spawn et
    let engine_state = Arc::clone(&state);
    let handle = tokio::spawn(async move {
        Engine::run_autonomous_loop(engine_state).await;
    });

    println!("  ⏳ Engine boot edildi, {}s smoke koşuyor...", duration_secs);
    tokio::time::sleep(Duration::from_secs(duration_secs)).await;

    // İnvariant'lar
    println!("  🔍 İnvariant'lar doğrulanıyor:");
    let mut pass = true;
    pass &= checks::check_phase_advanced(&state);
    pass &= checks::check_loop_tick_advanced(&state, start_tick);
    pass &= checks::check_snapshot_fresh(&workspace.snapshot_path, 30);
    pass &= checks::check_heartbeat_fresh(&workspace.heartbeat_path, 90);
    pass &= checks::check_equity_sane(&state, max_dd_pct);
    pass &= checks::check_no_critical_alerts(&state);
    if dry_run {
        // Live dry-run'da balance-sync ve WS user-data task'leri dormant log atmalı.
        pass &= checks::check_log_contains(
            &state,
            "Balance sync: Paper/DryRun",
            "balance_sync_dormant",
        );
        pass &= checks::check_log_contains(
            &state,
            "WS userDataStream: Paper/DryRun",
            "ws_user_data_dormant",
        );
    }

    // Engine'i durdur
    if let Ok(st) = state.lock() {
        st.app_stop_signal.store(true, Ordering::SeqCst);
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    cleanup_env_for_arm();
    if !keep_artifacts {
        let _ = std::fs::remove_dir_all(&workspace.root);
    } else {
        println!("  📁 Artefaktlar korundu: {}", workspace.root.display());
    }

    pass
}

struct ArmWorkspace {
    root: PathBuf,
    snapshot_path: PathBuf,
    heartbeat_path: PathBuf,
    db_path: String,
}

fn setup_workspace(label: &str) -> ArmWorkspace {
    let root = std::env::temp_dir().join(format!(
        "rtc_healthcheck_{}_{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("workspace klasörü yaratılamadı");
    let logs = root.join("logs");
    let data = root.join("data");
    std::fs::create_dir_all(&logs).expect("logs klasörü");
    std::fs::create_dir_all(&data).expect("data klasörü");
    let db_path = data.join("trader.db").to_string_lossy().into_owned();
    let snapshot_path = data.join("mission_control.json");
    let heartbeat_path = logs.join("heartbeat.jsonl");
    ArmWorkspace { root, snapshot_path, heartbeat_path, db_path }
}

fn apply_env_for_arm(ws: &ArmWorkspace, dry_run: bool) {
    // Snapshot + heartbeat hızlandırılır: invariant penceresine sığsın.
    std::env::set_var("MISSION_CONTROL_SNAPSHOT_PATH", ws.snapshot_path.as_os_str());
    std::env::set_var("MISSION_CONTROL_SNAPSHOT_SECS", "2");
    std::env::set_var("HEARTBEAT_PATH", ws.heartbeat_path.as_os_str());
    std::env::set_var("HEARTBEAT_SECS", "5");
    // TradingLogger ve trade summary'yi disable et: smoke'ta gerek yok, dosya çakışması olmasın.
    std::env::set_var("TRADING_LOGGER_DISABLE", "1");
    // Live dry-run sadece bu kolda set'lensin
    if dry_run {
        std::env::set_var("LIVE_DRY_RUN", "true");
    } else {
        std::env::remove_var("LIVE_DRY_RUN");
    }
}

fn cleanup_env_for_arm() {
    std::env::remove_var("MISSION_CONTROL_SNAPSHOT_PATH");
    std::env::remove_var("MISSION_CONTROL_SNAPSHOT_SECS");
    std::env::remove_var("HEARTBEAT_PATH");
    std::env::remove_var("HEARTBEAT_SECS");
    std::env::remove_var("TRADING_LOGGER_DISABLE");
    std::env::remove_var("LIVE_DRY_RUN");
}

fn build_config(ws: &ArmWorkspace, mode: TradingMode, dry_run: bool) -> RoboticLoopConfig {
    let mut cfg = RoboticLoopConfig::default();
    cfg.symbol = "BTCUSDT".into();
    cfg.db_path = ws.db_path.clone();
    cfg.pinned_symbols = vec![];
    cfg.download_enabled = false;
    cfg.pipeline_enabled = false;
    cfg.trading_mode = mode;
    if dry_run {
        cfg.api_key = Some("dummy_testnet_key".into());
        cfg.secret_key = Some("dummy_testnet_secret".into());
    }
    cfg
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_flag(key: &str) -> bool {
    matches!(std::env::var(key).ok().as_deref(), Some("1") | Some("true") | Some("TRUE"))
}

fn verdict(pass: bool) -> &'static str {
    if pass { "✅ PASS" } else { "❌ FAIL" }
}
