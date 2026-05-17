// src/robot/infra/telemetry.rs - Srivastava ATP Altyapı İzleme ve Telemetri
use axum::{routing::get, Router, response::IntoResponse, http::StatusCode};
use prometheus::{register_int_counter, register_gauge, IntCounter, Gauge, Encoder, TextEncoder, gather};
use std::sync::OnceLock;
use std::net::SocketAddr;
use crate::core::metrics::PerformanceScorecard;

// --- STATİK SAYAÇLAR VE GÖSTERGELER ---
pub static REQUESTS_TOTAL: OnceLock<IntCounter> = OnceLock::new();
pub static SHARPE_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static WIN_RATE_GAUGE: OnceLock<Gauge> = OnceLock::new();

/// Telemetri sistemini başlatır ve Prometheus endpoint'ini hazırlar
pub fn init_telemetry(addr_str: &str) -> anyhow::Result<()> {
    // 1. Prometheus sayaçlarını sisteme mühürle
    let counter = register_int_counter!("memos_http_requests_total", "Toplam işlenen HTTP isteği")?;
    let sharpe = register_gauge!("memos_robot_sharpe_ratio", "Anlık risk-adjusted performans skoru")?;
    let wr = register_gauge!("memos_robot_win_rate", "Güncel galibiyet oranı yüzdesi")?;
    
    let _ = REQUESTS_TOTAL.set(counter);
    let _ = SHARPE_GAUGE.set(sharpe);
    let _ = WIN_RATE_GAUGE.set(wr);

    // 2. Adres çözümleme ve bilgi logu
    let addr: SocketAddr = addr_str.parse()?;
    println!("📊 Srivastava Telemetri İstasyonu aktif: http://{}", addr);
    
    Ok(())
}

/// Robotik beyinden gelen verileri Prometheus'a asenkron olarak aktarır
pub fn sync_performance_metrics(scorecard: &PerformanceScorecard) {
    if let Some(g) = SHARPE_GAUGE.get() {
        g.set(scorecard.sharpe);
    }
    if let Some(g) = WIN_RATE_GAUGE.get() {
        g.set(scorecard.win_rate);
    }
}

/// Axum için yüksek performanslı metrik toplayıcı (Zero-allocation hint)
pub async fn metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = gather();
    let mut buffer = Vec::with_capacity(4096);

    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        eprintln!("🛑 Telemetri encode hatası: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Telemetri hatası").into_response();
    }

    match String::from_utf8(buffer) {
        Ok(body) => body.into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Encoding hatası").into_response(),
    }
}

/// Uygulama Router'ına metrik kanalını enjekte eder
pub fn add_telemetry_route(router: Router) -> Router {
    router.route("/metrics", get(metrics_handler))
}

/// Gelen istek sayacını artırır
pub fn mark_request() {
    if let Some(counter) = REQUESTS_TOTAL.get() {
        counter.inc();
    }
}
