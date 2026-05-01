// metrics.rs
// Prometheus metrics endpoint ve exporter entegrasyonu
// Türkçe açıklamalar ile

use prometheus_exporter::prometheus::{register_int_counter, IntCounter, Encoder, TextEncoder};
use once_cell::sync::Lazy;
use prometheus_exporter;
use std::sync::Once;
use axum::{Router, routing::get, response::IntoResponse};

static INIT: Once = Once::new();
pub static REQUESTS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!("requests_total", "Toplam HTTP istek sayısı").unwrap()
});

use std::net::SocketAddr;
pub fn init_metrics() {
    INIT.call_once(|| {
        let addr: SocketAddr = "0.0.0.0:9898".parse().unwrap();
        let _exporter = prometheus_exporter::start(addr).unwrap();
    });
}

pub async fn metrics_handler() -> impl IntoResponse {
    let metric_families = prometheus_exporter::prometheus::gather();
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

pub fn add_metrics_route(router: Router) -> Router {
    router.route("/metrics", get(metrics_handler))
}
