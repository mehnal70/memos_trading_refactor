// dashboard_server.rs - Web Tabanlı Dashboard ve Raporlama API'si

use axum::{
    extract::{State, Json},
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use serde_json::{json, Value};
use crate::portfolio::Portfolio;
use crate::audit_trail::AuditTrail;
use crate::ml_anomaly::MlAnomalyDetector;
use crate::gdpr::GdprManager;
use crate::mfa::{MfaManager, Claims as MfaClaims};
use crate::jwt_auth::AuthenticatedUser;

// Paylaşılan portföy durumu (Modernize edilmiş State yönetimi)
pub type SharedPortfolio = Arc<Mutex<Portfolio>>;

/// Dashboard HTML içeriği - Statik metin kopyalama maliyeti engellendi
async fn dashboard_html() -> Html<&'static str> {
    const HTML: &str = r#"
    <html><head><title>Trading Dashboard</title></head>
    <body>
    <h1>Trading Dashboard</h1>
    <div id='content'>Loading...</div>
    <script>
    fetch('/api/portfolio').then(r=>r.json()).then(d=>{
      document.getElementById('content').innerHTML = '<pre>'+JSON.stringify(d,null,2)+'</pre>';
    });
    </script>
    </body></html>"#;
    Html(HTML)
}

// --- PORTFÖY API ---
async fn portfolio_api(
    State(state): State<SharedPortfolio>,
    _auth: AuthenticatedUser
) -> Json<Value> {
    let pf = state.lock().await;
    Json(json!({
        "balance": pf.balance,
        "positions": pf.positions,
        "trade_history": pf.trade_history,
    }))
}

// --- MFA ENDPOINTS ---
async fn mfa_enroll(Json(payload): Json<Value>) -> Json<Value> {
    let username = payload["username"].as_str().unwrap_or_default();
    let issuer = "MemosTrading";
    let (secret, qr) = MfaManager::enroll_user(username, issuer);
    Json(json!({"secret": secret, "qr": qr}))
}

async fn mfa_verify(Json(payload): Json<Value>) -> Json<Value> {
    let username = payload["username"].as_str().unwrap_or_default();
    let code = payload["code"].as_str().unwrap_or_default();
    let mfa_verified = MfaManager::verify(username, code);
    
    let claims = MfaClaims {
        sub: username.to_owned(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        mfa_verified,
    };
    Json(json!({"success": mfa_verified, "mfa_verified": mfa_verified, "claims": claims}))
}

// --- GDPR ENDPOINTS ---
async fn gdpr_mask_user(Json(payload): Json<Value>) -> Json<Value> {
    let username = payload["username"].as_str().unwrap_or_default();
    let masked = GdprManager::get_user(username, true);
    Json(json!({"masked": masked}))
}

async fn gdpr_delete_user(Json(payload): Json<Value>) -> Json<Value> {
    let username = payload["username"].as_str().unwrap_or_default();
    GdprManager::delete_user(username);
    Json(json!({"deleted": true}))
}

async fn gdpr_access_logs() -> Json<Value> {
    Json(json!({"logs": GdprManager::get_access_logs()}))
}

// --- AUDIT & ANOMALY ENDPOINTS ---
async fn ml_anomaly_detect(Json(payload): Json<Value>) -> Json<Value> {
    let is_anomaly = MlAnomalyDetector::analyze_event(&payload);
    Json(json!({"anomaly": is_anomaly}))
}

async fn audit_all() -> Json<Value> {
    Json(json!({"audit": AuditTrail::all()}))
}

async fn audit_search(Json(payload): Json<Value>) -> Json<Value> {
    let user = payload.get("user").and_then(|v| v.as_str());
    let action = payload.get("action").and_then(|v| v.as_str());
    let result = payload.get("result").and_then(|v| v.as_str());
    let logs = AuditTrail::search(user, action, result);
    Json(json!({"audit": logs}))
}

// --- SERVER RUNNER ---
pub async fn run_dashboard_server(portfolio: SharedPortfolio) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(dashboard_html))
        .route("/api/portfolio", get(portfolio_api))
        .route("/api/mfa/enroll", post(mfa_enroll))
        .route("/api/mfa/verify", post(mfa_verify))
        .route("/api/gdpr/mask_user", post(gdpr_mask_user))
        .route("/api/gdpr/delete_user", post(gdpr_delete_user))
        .route("/api/gdpr/access_logs", get(gdpr_access_logs))
        .route("/api/ml/anomaly_detect", post(ml_anomaly_detect))
        .route("/api/audit/all", get(audit_all))
        .route("/api/audit/search", post(audit_search))
        .with_state(portfolio);

    let addr = "127.0.0.1:8080".parse()?;
    println!("🚀 Dashboard aktif: http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}
