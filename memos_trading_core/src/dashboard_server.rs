use crate::audit_trail::AuditTrail;
// Audit trail: tüm logları getir
async fn audit_all() -> Json<serde_json::Value> {
    let logs = AuditTrail::all();
    Json(serde_json::json!({"audit": logs}))
}

// Audit trail: filtreli arama
async fn audit_search(Json(payload): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let user = payload.get("user").and_then(|v| v.as_str());
    let action = payload.get("action").and_then(|v| v.as_str());
    let result = payload.get("result").and_then(|v| v.as_str());
    let logs = AuditTrail::search(user, action, result);
    Json(serde_json::json!({"audit": logs}))
}
use crate::ml_anomaly::MlAnomalyDetector;
// ML tabanlı anomali tespiti endpoint'i
async fn ml_anomaly_detect(Json(payload): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let is_anomaly = MlAnomalyDetector::analyze_event(&payload);
    Json(serde_json::json!({"anomaly": is_anomaly}))
}
use crate::gdpr::{GdprManager, UserData};
// GDPR: Kullanıcı verisini maskeleme endpoint'i
async fn gdpr_mask_user(Json(payload): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let username = payload["username"].as_str().unwrap_or("");
    let masked = GdprManager::get_user(username, true);
    Json(serde_json::json!({"masked": masked}))
}

// GDPR: Kullanıcı verisini silme endpoint'i
async fn gdpr_delete_user(Json(payload): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let username = payload["username"].as_str().unwrap_or("");
    GdprManager::delete_user(username);
    Json(serde_json::json!({"deleted": true}))
}

// GDPR: Erişim loglarını getirme endpoint'i
async fn gdpr_access_logs() -> Json<serde_json::Value> {
    let logs = GdprManager::get_access_logs();
    Json(serde_json::json!({"logs": logs}))
}
// dashboard_server.rs - Web Tabanlı Dashboard ve Raporlama API'si
// Canlı performans, risk ve portföy görselleştirme için temel HTTP sunucu
// Türkçe açıklamalar ile

use std::sync::Arc;
use tokio::sync::Mutex;
use axum::{Router, routing::{get, post}, response::Html, Json};
use axum::extract::State;
use crate::mfa::{MfaManager, Claims as MfaClaims};
use crate::jwt_auth::AuthenticatedUser;
use serde_json::json;
use crate::portfolio::Portfolio;

// Paylaşılan portföy durumu (örnek)
pub type SharedPortfolio = Arc<Mutex<Portfolio>>;

async fn dashboard_html() -> Html<String> {
    // MFA kayıt endpoint'i: kullanıcıya secret ve QR kodu döner
    async fn mfa_enroll(Json(payload): Json<serde_json::Value>) -> Json<serde_json::Value> {
        let username = payload["username"].as_str().unwrap_or("");
        let issuer = "MemosTrading";
        let (secret, qr) = MfaManager::enroll_user(username, issuer);
        Json(serde_json::json!({"secret": secret, "qr": qr}))
    }

    // MFA doğrulama endpoint'i: kodu doğrular, JWT'ye mfa_verified ekler
    async fn mfa_verify(Json(payload): Json<serde_json::Value>) -> Json<serde_json::Value> {
        let username = payload["username"].as_str().unwrap_or("");
        let code = payload["code"].as_str().unwrap_or("");
        let ok = MfaManager::verify(username, code);
        let mfa_verified = ok;
        // JWT claim örneği (gerçekte: oturumda tutulur)
        let claims = MfaClaims {
            sub: username.to_string(),
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
            mfa_verified,
        };
        Json(serde_json::json!({"success": ok, "mfa_verified": mfa_verified, "claims": claims}))
    }
    // Basit HTML dashboard (geliştirilebilir)
    let html = r#"
    <html><head><title>Trading Dashboard</title></head>
    <body>
    <h1>Trading Dashboard</h1>
    <div id='content'>Loading...</div>
    <script>
    fetch('/api/portfolio').then(r=>r.json()).then(d=>{
      document.getElementById('content').innerHTML = '<pre>'+JSON.stringify(d,null,2)+'</pre>';
    });
    </script>
    </body></html>
    "#;
    Html(html.to_string())
}

async fn portfolio_api(
    state: axum::extract::State<SharedPortfolio>,
    _auth: AuthenticatedUser
) -> axum::Json<serde_json::Value> {
    let pf = state.lock().await;
    axum::Json(json!({
        "balance": pf.balance,
        "positions": pf.positions,
        "trade_history": pf.trade_history,
    }))
}

pub async fn run_dashboard_server(portfolio: SharedPortfolio) {
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
    let addr = "127.0.0.1:8080".parse().unwrap();
    println!("Dashboard http://{}", addr);
    axum::Server::bind(&addr).serve(app.into_make_service()).await.unwrap();
}

// Kullanım örneği (main fonksiyonunda async olarak):
// let portfolio = Arc::new(Mutex::new(Portfolio::default()));
// tokio::spawn(run_dashboard_server(portfolio.clone()));
