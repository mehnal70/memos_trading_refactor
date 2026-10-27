// src/robot/infra/telemetry.rs - Srivastava ATP Altyapı İzleme ve Telemetri
use crate::core::metrics::PerformanceScorecard;
use crate::robot::robotic_loop::AppState;
use crate::robot::venue::MarketData;
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use memos_venue_api::VenueError;
use prometheus::{
    gather, register_gauge, register_int_counter, Encoder, Gauge, IntCounter, TextEncoder,
};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

// --- STATİK SAYAÇLAR VE GÖSTERGELER ---
pub static REQUESTS_TOTAL: OnceLock<IntCounter> = OnceLock::new();
pub static SHARPE_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static WIN_RATE_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static EQUITY_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static STARTING_CAPITAL_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static REALIZED_PNL_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static OPEN_PNL_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static OPEN_POSITIONS_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static CLOSED_TRADES_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static ANOMALIES_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static LOOP_TICK_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static PAUSED_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static LIVE_DRY_RUN_GAUGE: OnceLock<Gauge> = OnceLock::new();
pub static MT5_HEALTHY_GAUGE: OnceLock<Gauge> = OnceLock::new();

/// Telemetri sistemini başlatır ve Prometheus endpoint'ini hazırlar
pub fn init_telemetry(addr_str: &str) -> anyhow::Result<()> {
    // 1. Prometheus sayaçlarını sisteme mühürle.
    // Headless süreçte init yalnızca bir kez çağrılır; OnceLock tekrar başlatmayı
    // güvenli biçimde no-op yapar.
    let counter = register_int_counter!("memos_http_requests_total", "Toplam işlenen HTTP isteği")?;
    let sharpe = register_gauge!(
        "memos_robot_sharpe_ratio",
        "Anlık risk-adjusted performans skoru"
    )?;
    let wr = register_gauge!("memos_robot_win_rate", "Güncel galibiyet oranı yüzdesi")?;
    let equity = register_gauge!("memos_robot_equity_usd", "Güncel toplam özsermaye")?;
    let starting = register_gauge!("memos_robot_starting_capital_usd", "Başlangıç sermayesi")?;
    let realized_pnl = register_gauge!("memos_robot_realized_pnl_usd", "Gerçekleşmiş net PnL")?;
    let open_pnl = register_gauge!("memos_robot_open_pnl_usd", "Açık pozisyon PnL")?;
    let open_positions = register_gauge!("memos_robot_open_positions", "Açık pozisyon sayısı")?;
    let closed_trades = register_gauge!("memos_robot_closed_trades_total", "Kapanan işlem sayısı")?;
    let anomalies = register_gauge!("memos_robot_anomalies", "Aktif anomali sayısı")?;
    let loop_tick = register_gauge!("memos_robot_loop_tick", "Son ana döngü tick değeri")?;
    let paused = register_gauge!("memos_robot_paused", "Motor duraklatıldı mı (0/1)")?;
    let dry_run = register_gauge!("memos_robot_live_dry_run", "Live dry-run etkin mi (0/1)")?;
    let mt5_healthy = register_gauge!(
        "memos_robot_mt5_bridge_healthy",
        "MT5 bridge sağlıklı mı (0/1)"
    )?;

    let _ = REQUESTS_TOTAL.set(counter);
    let _ = SHARPE_GAUGE.set(sharpe);
    let _ = WIN_RATE_GAUGE.set(wr);
    let _ = EQUITY_GAUGE.set(equity);
    let _ = STARTING_CAPITAL_GAUGE.set(starting);
    let _ = REALIZED_PNL_GAUGE.set(realized_pnl);
    let _ = OPEN_PNL_GAUGE.set(open_pnl);
    let _ = OPEN_POSITIONS_GAUGE.set(open_positions);
    let _ = CLOSED_TRADES_GAUGE.set(closed_trades);
    let _ = ANOMALIES_GAUGE.set(anomalies);
    let _ = LOOP_TICK_GAUGE.set(loop_tick);
    let _ = PAUSED_GAUGE.set(paused);
    let _ = LIVE_DRY_RUN_GAUGE.set(dry_run);
    let _ = MT5_HEALTHY_GAUGE.set(mt5_healthy);

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
    render_metrics()
}

fn render_metrics() -> Response {
    let encoder = TextEncoder::new();
    let metric_families = gather();
    let mut buffer = Vec::with_capacity(4096);

    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        eprintln!("🛑 Telemetri encode hatası: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Telemetri hatası").into_response();
    }

    match String::from_utf8(buffer) {
        Ok(body) => ([(header::CONTENT_TYPE, prometheus::TEXT_FORMAT)], body).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Encoding hatası").into_response(),
    }
}

fn sync_state_metrics(state: &AppState) -> Result<(), ()> {
    if let Some(g) = EQUITY_GAUGE.get() {
        g.set(state.finance.equity);
    }
    if let Some(g) = STARTING_CAPITAL_GAUGE.get() {
        g.set(state.finance.starting_capital);
    }
    let realized_pnl = state
        .finance
        .live_closed_trades
        .try_read()
        .map_err(|_| ())?
        .iter()
        .map(|trade| trade.effective_net_pnl())
        .sum();
    if let Some(g) = REALIZED_PNL_GAUGE.get() {
        g.set(realized_pnl);
    }
    let positions = state.finance.live_positions.try_read().map_err(|_| ())?;
    let open_pnl = positions
        .values()
        .map(|position| position.calculate_pnl())
        .sum();
    if let Some(g) = OPEN_PNL_GAUGE.get() {
        g.set(open_pnl);
    }
    if let Some(g) = OPEN_POSITIONS_GAUGE.get() {
        g.set(positions.len() as f64);
    }
    drop(positions);
    if let Some(g) = CLOSED_TRADES_GAUGE.get() {
        g.set(
            state
                .finance
                .closed_trades_total
                .load(std::sync::atomic::Ordering::Relaxed) as f64,
        );
    }
    if let Some(g) = ANOMALIES_GAUGE.get() {
        g.set(state.guardian.anomaly_count as f64);
    }
    if let Some(g) = LOOP_TICK_GAUGE.get() {
        g.set(
            state
                .fleet
                .last_loop_tick
                .load(std::sync::atomic::Ordering::Relaxed) as f64,
        );
    }
    if let Some(g) = PAUSED_GAUGE.get() {
        g.set(
            if state
                .pause_signal
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                1.0
            } else {
                0.0
            },
        );
    }
    if let Some(g) = LIVE_DRY_RUN_GAUGE.get() {
        g.set(if state.live_dry_run { 1.0 } else { 0.0 });
    }
    if let Some(g) = MT5_HEALTHY_GAUGE.get() {
        g.set(
            if state
                .mt5_bridge_healthy
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                1.0
            } else {
                0.0
            },
        );
    }
    Ok(())
}

async fn state_metrics_handler(State(state): State<Arc<Mutex<AppState>>>) -> Response {
    match state.try_lock() {
        Ok(st) => {
            if sync_state_metrics(&st).is_err() {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "AppState data busy; retry metrics scrape",
                )
                    .into_response();
            }
        }
        Err(std::sync::TryLockError::WouldBlock) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "AppState busy; retry metrics scrape",
            )
                .into_response();
        }
        Err(std::sync::TryLockError::Poisoned(_)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "AppState lock poisoned").into_response();
        }
    }
    render_metrics()
}

/// AppState bağlantılı Prometheus router'ı.
pub fn metrics_router(state: Arc<Mutex<AppState>>) -> Router {
    Router::new()
        .route("/metrics", get(state_metrics_handler))
        .route("/venue-detail", get(venue_detail_handler))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct VenueDetailQuery {
    venue: String,
    symbol: String,
    #[serde(default = "default_interval")]
    interval: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_interval() -> String {
    "1h".to_owned()
}

fn default_limit() -> usize {
    50
}

fn venue_detail_error(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(json!({"error": error.into()}))).into_response()
}

async fn venue_detail_handler(
    State(state): State<Arc<Mutex<AppState>>>,
    Query(query): Query<VenueDetailQuery>,
) -> Response {
    if query.venue.len() > 32
        || query.symbol.len() > 64
        || query.interval.len() > 16
        || query.venue.is_empty()
        || query.symbol.is_empty()
        || query.interval.is_empty()
        || !query
            .symbol
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_/@".contains(&byte))
        || !query
            .venue
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b":-_".contains(&byte))
        || !query
            .interval
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
    {
        return venue_detail_error(
            StatusCode::BAD_REQUEST,
            "gecersiz venue, sembol veya interval",
        );
    }

    let limit = query.limit.clamp(1, 200);
    let routed_symbol = if query.symbol.contains('@') {
        query.symbol.clone()
    } else {
        let exchange = query.venue.split(':').next().unwrap_or_default();
        format!("{}@{}", query.symbol, exchange)
    };

    let (venue, bare_symbol) = match state.try_lock() {
        Ok(st) => match st.venue_registry.route(&routed_symbol) {
            Some((venue, bare)) => (Arc::clone(venue), bare.to_owned()),
            None => {
                return venue_detail_error(
                    StatusCode::NOT_FOUND,
                    "istenen venue kayıtlı veya aktif değil",
                )
            }
        },
        Err(std::sync::TryLockError::WouldBlock) => {
            return venue_detail_error(StatusCode::SERVICE_UNAVAILABLE, "motor durumu meşgul")
        }
        Err(std::sync::TryLockError::Poisoned(_)) => {
            return venue_detail_error(StatusCode::INTERNAL_SERVER_ERROR, "motor durumu bozuldu")
        }
    };

    let timeout = Duration::from_secs(8);
    let ticker = match tokio::time::timeout(timeout, venue.book_ticker(&bare_symbol)).await {
        Ok(Ok((bid, ask))) => {
            Some(json!({"bid": bid, "ask": ask, "last": (bid + ask) / 2.0, "source": "quote"}))
        }
        Ok(Err(VenueError::Unsupported(_))) => None,
        Ok(Err(_error)) if venue.execution_engine().is_none() => None,
        Ok(Err(error)) => {
            return venue_detail_error(
                StatusCode::BAD_GATEWAY,
                format!("ticker alınamadı: {error}"),
            )
        }
        Err(_) => {
            return venue_detail_error(StatusCode::GATEWAY_TIMEOUT, "ticker zaman aşımına uğradı")
        }
    };
    let candles = match tokio::time::timeout(
        timeout,
        venue.fetch_candles(&bare_symbol, &query.interval, limit),
    )
    .await
    {
        Ok(Ok(candles)) => candles,
        Ok(Err(error)) => {
            return venue_detail_error(
                StatusCode::BAD_GATEWAY,
                format!("mumlar alınamadı: {error}"),
            )
        }
        Err(_) => {
            return venue_detail_error(
                StatusCode::GATEWAY_TIMEOUT,
                "mum sorgusu zaman aşımına uğradı",
            )
        }
    };
    let ticker = ticker.or_else(|| {
        candles.last().map(|candle| {
            json!({
                "bid": null,
                "ask": null,
                "last": candle.close,
                "source": "candle_close",
            })
        })
    });

    Json(json!({
        "venue": query.venue,
        "symbol": query.symbol,
        "interval": query.interval,
        "source": "live_venue_adapter",
        "queried_at": chrono::Utc::now().to_rfc3339(),
        "ticker": ticker,
        "candles": candles,
    }))
    .into_response()
}

/// Headless süreç için loopback Prometheus HTTP sunucusunu başlatır.
pub async fn start_metrics_server(
    state: Arc<Mutex<AppState>>,
    addr_str: &str,
) -> anyhow::Result<()> {
    init_telemetry(addr_str)?;
    let addr: SocketAddr = addr_str.parse()?;
    let listener = std::net::TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    let router = metrics_router(state);
    println!("📊 Prometheus metrikleri aktif: http://{}/metrics", addr);
    std::thread::Builder::new()
        .name("memos-metrics".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    eprintln!("🛑 Prometheus runtime başlatılamadı: {}", err);
                    return;
                }
            };
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(err) => {
                        eprintln!("🛑 Prometheus listener hazırlanamadı: {}", err);
                        return;
                    }
                };
                if let Err(err) = axum::serve(listener, router).await {
                    eprintln!("🛑 Prometheus sunucusu durdu: {}", err);
                }
            });
        })?;
    Ok(())
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
