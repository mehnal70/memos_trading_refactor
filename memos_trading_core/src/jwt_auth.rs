// jwt_auth.rs - JWT tabanlı authentication (gerçek uygulama)
// Dashboard ve kritik API'ler için güvenli erişim
// Türkçe açıklamalar ile

use axum::{async_trait, extract::{FromRequestParts, TypedHeader}, http::request::Parts, response::IntoResponse, http::StatusCode};
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm, encode, EncodingKey, Header};
use serde::{Serialize, Deserialize};
use std::env;


#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub mfa_verified: bool,
}


pub fn create_jwt(username: &str, secret: &str, exp: usize, mfa_verified: bool) -> String {
    let claims = Claims { sub: username.to_string(), exp, mfa_verified };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes())).unwrap()
}


pub async fn validate_jwt(token: &str, secret: &str) -> Result<Claims, String> {
    decode::<Claims>(token, &DecodingKey::from_secret(secret.as_bytes()), &Validation::new(Algorithm::HS256))
        .map(|d| d.claims)
        .map_err(|e| format!("JWT doğrulama hatası: {e}"))
}

pub struct AuthenticatedUser(pub Claims);

#[async_trait]
impl<S> FromRequestParts<S> for AuthenticatedUser
where S: Send + Sync
{
    type Rejection = (StatusCode, String);
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let TypedHeader(auth) = TypedHeader::<headers::Authorization<headers::authorization::Bearer>>::from_request_parts(parts, _state).await.map_err(|_| (StatusCode::UNAUTHORIZED, "Eksik Authorization header".to_string()))?;
        let secret = env::var("DASHBOARD_JWT_SECRET").unwrap_or_else(|_| "supersecret".to_string());
        let claims = validate_jwt(auth.token(), &secret).await.map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
        Ok(AuthenticatedUser(claims))
    }
}

// Kullanım örneği (axum route):
// .route("/api/portfolio", get(portfolio_api).layer(require_auth()))
// fn require_auth() -> ...
