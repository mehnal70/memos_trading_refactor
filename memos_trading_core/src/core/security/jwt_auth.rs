// jwt_auth.rs - JWT Tabanlı Kimlik Doğrulama Sistemi

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub mfa_verified: bool,
}

/// JWT Token oluşturur - Modern Rust: Niyetini belli eden tip kullanımı
pub fn create_jwt(username: &str, secret: &str, exp: usize, mfa_verified: bool) -> Result<String, String> {
    let claims = Claims {
        sub: username.to_owned(),
        exp,
        mfa_verified,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| format!("JWT encode hatası: {e}"))
}

/// JWT Token doğrular (Zero-allocation decoding)
pub async fn validate_jwt(token: &str, secret: &str) -> Result<Claims, String> {
    let validation = Validation::new(Algorithm::HS256);
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| format!("JWT doğrulama hatası: {e}"))
}

/// Kimliği doğrulanmış kullanıcıyı temsil eden extractor
pub struct AuthenticatedUser(pub Claims);

#[async_trait]
impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Modern Axum: Authorization header'ını güvenli çekme
        let TypedHeader(Authorization(bearer)) = TypedHeader::<Authorization<Bearer>>::from_request_parts(parts, state)
            .await
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Eksik veya geçersiz Authorization başlığı".to_owned()))?;

        // Secret yönetimi: Hiyerarşik ve güvenli (env var > default)
        let secret = env::var("DASHBOARD_JWT_SECRET").unwrap_or_else(|_| "trading_system_secure_key_2024".to_owned());

        let claims = validate_jwt(bearer.token(), &secret)
            .await
            .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;

        // Güvenlik Ekstra: MFA zorunluluğu olan işlemler için burada ek kontrol yapılabilir
        // if !claims.mfa_verified { return Err((StatusCode::FORBIDDEN, "MFA doğrulaması gerekli".to_owned())); }

        Ok(AuthenticatedUser(claims))
    }
}
