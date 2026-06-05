// multi_tf_ab — çoklu-TF seed düzeneği için A/B doğrulama (Single vs Multi).
//
// Soru: sembol başına TEK (top-PF) edge mi (Single/A), yoksa TÜM WF-onaylı (TF,strateji)
// edge'lerini tek-pozisyon arbitrasyonuyla koşmak mı (Multi/B) NET daha iyi? `EDGE_SEED_MULTI_TF`'i
// canlıya açmadan önce bunu ölç. Bir edge_scan raporu (EDGE_SEED_REPORT) girdi alır; barı geçen
// >1 izli sembolleri her izin KENDİ TF mumunda backtest eder, Multi kolunu çakışmasız arbitrasyonla
// birleştirir, Single (yalnız anchor) ile kıyaslar. Çekirdek lib'de (robot::backtester::multi_tf_ab).
//
// Kullanım:
//   EDGE_SEED_REPORT=reports/edge_sweep_*.json cargo run --release --example multi_tf_ab
// Env (seed barı — edge_scan/store ile AYNI anlam):
//   EDGE_SEED_REPORT (zorunlu), DB_PATH, TRADE_MARKET (rapor satırları bu markete filtrelenir; "all"=hepsi),
//   EDGE_SEED_MIN_TRADES / EDGE_SEED_MIN_PF / EDGE_SEED_MAX_PF / EDGE_SEED_REQUIRE_WF / EDGE_SEED_MIN_QVOL,
//   EDGE_SEED_MAX_TRACKS (default 3),
//   AB_DIRECTION (long|both|regime; default regime), AB_EDGE_MIN (default 0.20), AB_CANDLE_LIMIT (5000).

use memos_trading_core::robot::backtester::{
    run_multi_tf_ab, AbConfig, SeedRobustness, EdgeScanReport, DirectionMode, ArmMetrics,
};

fn env_f64(k: &str) -> Option<f64> { std::env::var(k).ok().and_then(|s| s.parse().ok()) }
fn env_usize(k: &str, d: usize) -> usize { std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d) }

fn main() {
    let report_path = match std::env::var("EDGE_SEED_REPORT").ok().filter(|s| !s.trim().is_empty()) {
        Some(p) => p,
        None => { eprintln!("❌ EDGE_SEED_REPORT ayarla (edge_sweep raporu JSON)."); std::process::exit(2); }
    };
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".to_string());
    let market = std::env::var("TRADE_MARKET").unwrap_or_else(|_| "futures".to_string());

    // Seed barı — store::from_env ile aynı default'lar (tek kaynak SeedRobustness::default()).
    let d = SeedRobustness::default();
    let r = SeedRobustness {
        min_trades: env_usize("EDGE_SEED_MIN_TRADES", d.min_trades),
        min_pf: env_f64("EDGE_SEED_MIN_PF").unwrap_or(d.min_pf),
        max_pf: env_f64("EDGE_SEED_MAX_PF").unwrap_or(d.max_pf),
        require_wf_robust: !matches!(
            std::env::var("EDGE_SEED_REQUIRE_WF").ok().as_deref(), Some("0")|Some("false")|Some("off")),
        min_daily_quote_volume: env_f64("EDGE_SEED_MIN_QVOL").unwrap_or(d.min_daily_quote_volume),
        wf_max_pvalue: env_f64("EDGE_WF_MAX_PVALUE").unwrap_or(d.wf_max_pvalue),
    };
    let max_tracks = env_usize("EDGE_SEED_MAX_TRACKS",
        memos_trading_core::robot::backtester::SEED_MAX_TRACKS_DEFAULT);

    let direction = match std::env::var("AB_DIRECTION").ok().as_deref() {
        Some("long") => DirectionMode::LongOnly,
        Some("both") => DirectionMode::BothDirections,
        _ => DirectionMode::RegimeDirectional,
    };
    let ab = AbConfig {
        direction,
        edge_min_score: env_f64("AB_EDGE_MIN").or(Some(0.20)),
        candle_limit: env_usize("AB_CANDLE_LIMIT", 5000),
        ..AbConfig::default()
    };

    // Raporu yükle (market filtresi: rapor satırlarını engine market'ına daralt; "all"=hepsi).
    let txt = match std::fs::read_to_string(&report_path) {
        Ok(t) => t,
        Err(e) => { eprintln!("❌ rapor okunamadı ({report_path}): {e}"); std::process::exit(2); }
    };
    let mut report: EdgeScanReport = match serde_json::from_str(&txt) {
        Ok(r) => r,
        Err(e) => { eprintln!("❌ rapor JSON ayrıştırılamadı: {e}"); std::process::exit(2); }
    };
    if market != "all" && !market.is_empty() {
        report.rows.retain(|row| row.market.eq_ignore_ascii_case(&market));
    }

    println!("\n🪢 multi_tf_ab · rapor={report_path} · db={db_path} · market={market}");
    println!("   seed barı: min_trades={} min_pf={:.2} max_pf={:.2} wf={} min_qvol={:.0} · max_tracks={max_tracks}",
        r.min_trades, r.min_pf, r.max_pf, r.require_wf_robust, r.min_daily_quote_volume);
    println!("   A/B knob: direction={:?} edge_min={:?} candle_limit={}",
        ab.direction, ab.edge_min_score, ab.candle_limit);
    println!("   (>1 izli sembol her izin TF mumunda backtest edilir; Multi = çakışmasız arbitrasyon)\n");

    let ab_report = run_multi_tf_ab(&report, r, max_tracks, &db_path, &ab);

    if ab_report.per_symbol.is_empty() {
        println!("  Çoklu-iz (>1 WF-onaylı edge) taşıyan sembol yok — bu raporda düzenek devreye girmez.");
        return;
    }

    // Sembol-başına tablo.
    println!("══════ A/B (sembol başına · Single=top-iz vs Multi=arbitrasyon) ══════");
    println!("  {:<10} {:<5} {:>6} {:>8} {:>6} {:>5}  | {:>6} {:>8} {:>6} {:>5}   izler",
        "symbol", "izsy", "Sişl", "SΣpnl%", "SPF", "Swr", "Mişl", "MΣpnl%", "MPF", "Mwr");
    for s in &ab_report.per_symbol {
        let tracks: String = s.tracks.iter()
            .map(|(iv, st, pf)| format!("{iv}/{st}({pf:.1})")).collect::<Vec<_>>().join(" ");
        println!("  {:<10} {:<5} {:>6} {:>+8.2} {:>6} {:>4.0}%  | {:>6} {:>+8.2} {:>6} {:>4.0}%   {}",
            s.symbol, s.n_tracks,
            s.single.trades, s.single.sum_pnl_pct, pf_fmt(&s.single), s.single.win_rate*100.0,
            s.multi.trades, s.multi.sum_pnl_pct, pf_fmt(&s.multi), s.multi.win_rate*100.0,
            tracks);
    }

    // Portföy toplamı + verdict.
    let (st, mt) = (&ab_report.single_total, &ab_report.multi_total);
    println!("\n══════ PORTFÖY TOPLAMI ══════");
    println!("  Single (A · yalnız top-iz): işlem={:>4} · Σpnl%={:>+8.2} · win={:.0}%",
        st.trades, st.sum_pnl_pct, st.win_rate*100.0);
    println!("  Multi  (B · çoklu-iz arb.) : işlem={:>4} · Σpnl%={:>+8.2} · win={:.0}%",
        mt.trades, mt.sum_pnl_pct, mt.win_rate*100.0);
    let dpnl = mt.sum_pnl_pct - st.sum_pnl_pct;
    let dtr = mt.trades as i64 - st.trades as i64;
    println!("\n  Δ Multi−Single: Σpnl% {dpnl:+.2} · işlem {dtr:+} ({} sembol)", ab_report.per_symbol.len());
    let verdict = if dpnl > 0.0 && mt.win_rate >= st.win_rate - 0.05 {
        "✅ Multi NET KAZANÇ (daha çok fırsat + korunan/iyi win) → EDGE_SEED_MULTI_TF=1 aday"
    } else if dpnl > 0.0 {
        "⚠️ Multi Σpnl artırdı ama win düştü → frekans-kalite dengesini gözden geçir (max_tracks/min_pf)"
    } else {
        "❌ Multi NET KAZANÇ YOK → tek-edge kal (EDGE_SEED_MULTI_TF=0); alt-TF izleri değer katmıyor"
    };
    println!("  VERDICT: {verdict}\n");
}

fn pf_fmt(m: &ArmMetrics) -> String {
    if m.profit_factor.is_infinite() { "∞".to_string() } else { format!("{:.2}", m.profit_factor) }
}
