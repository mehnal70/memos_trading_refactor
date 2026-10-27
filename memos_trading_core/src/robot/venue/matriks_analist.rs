//! Matriks Analist read-only REST sınırı.
//!
//! Bu modül yalnızca discovery ve metadata sorgularını kapsar. Emir göndermez,
//! JWT'yi dosyaya yazmaz ve MQTT/Protobuf katmanını resmi şemalar gelene kadar
//! bilerek başlatmaz.

use std::env;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_JWT_ENV: &str = "MATRIKS_ANALIST_JWT";

#[derive(Debug, thiserror::Error)]
pub enum AnalistError {
    #[error("Analist konfigürasyonu eksik: {0}")]
    Config(String),
    #[error("Analist HTTP hatası: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Analist JSON çözümleme hatası: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Analist HTTP {status}: {body}")]
    HttpStatus { status: StatusCode, body: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalistConfig {
    pub discovery_url: String,
    pub jwt_env: String,
    pub market_code: Option<String>,
    pub exchange_id: Option<String>,
    pub symbol: Option<String>,
}

impl AnalistConfig {
    pub fn from_env() -> Result<Self, AnalistError> {
        let discovery_url = required_env("MATRIKS_ANALIST_DISCOVERY_URL")?;
        let jwt_env =
            env::var("MATRIKS_ANALIST_JWT_ENV").unwrap_or_else(|_| DEFAULT_JWT_ENV.to_owned());

        Ok(Self {
            discovery_url,
            jwt_env,
            market_code: optional_env("MATRIKS_ANALIST_MARKET_CODE"),
            exchange_id: optional_env("MATRIKS_ANALIST_EXCHANGE_ID"),
            symbol: optional_env("MATRIKS_ANALIST_SYMBOL"),
        })
    }

    pub fn jwt(&self) -> Result<String, AnalistError> {
        let token = env::var(&self.jwt_env).map_err(|_| {
            AnalistError::Config(format!(
                "JWT çevresel değişkeni bulunamadı: {}",
                self.jwt_env
            ))
        })?;
        if token.trim().is_empty() {
            return Err(AnalistError::Config(format!(
                "JWT çevresel değişkeni boş: {}",
                self.jwt_env
            )));
        }
        Ok(token)
    }
}

#[derive(Debug, Clone)]
pub struct AnalistClient {
    http: Client,
    config: AnalistConfig,
}

impl AnalistClient {
    pub fn new(config: AnalistConfig) -> Result<Self, AnalistError> {
        let http = Client::builder()
            .user_agent("memos-trading/matriks-analist")
            .build()?;
        Ok(Self { http, config })
    }

    pub fn config(&self) -> &AnalistConfig {
        &self.config
    }

    pub async fn discovery(&self) -> Result<DiscoveryDocument, AnalistError> {
        let value = self
            .http
            .get(&self.config.discovery_url)
            .send()
            .await?
            .json::<Value>()
            .await?;
        Ok(DiscoveryDocument(value))
    }

    pub async fn get_symbols(&self, url: &str) -> Result<Value, AnalistError> {
        self.authenticated_json(url).await
    }

    pub async fn get_topach(&self, url: &str) -> Result<Value, AnalistError> {
        self.authenticated_json(url).await
    }

    async fn authenticated_json(&self, url: &str) -> Result<Value, AnalistError> {
        let token = self.config.jwt()?;
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("jwt {}", token))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(AnalistError::HttpStatus { status, body });
        }
        Ok(serde_json::from_str(&body)?)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveryDocument(pub Value);

impl DiscoveryDocument {
    pub fn symbols_url(&self) -> Option<&str> {
        self.0
            .pointer("/metaDataV4/symbols/url")
            .and_then(Value::as_str)
    }

    pub fn topach_url(&self) -> Option<&str> {
        self.0
            .pointer("/metaDataV4/topach/url")
            .and_then(Value::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalistSymbol {
    #[serde(rename = "sC")]
    pub symbol_code: String,
    #[serde(rename = "sT", default)]
    pub symbol_type: Option<String>,
}

impl AnalistSymbol {
    pub fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

fn required_env(name: &str) -> Result<String, AnalistError> {
    let value = env::var(name)
        .map_err(|_| AnalistError::Config(format!("çevresel değişken bulunamadı: {}", name)))?;
    if value.trim().is_empty() {
        return Err(AnalistError::Config(format!(
            "çevresel değişken boş: {}",
            name
        )));
    }
    Ok(value)
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovery_urls_are_read_from_document() {
        let document = DiscoveryDocument(json!({
            "metaDataV4": {
                "symbols": { "url": "https://example.test/symbols" },
                "topach": { "url": "https://example.test/topach" }
            }
        }));

        assert_eq!(document.symbols_url(), Some("https://example.test/symbols"));
        assert_eq!(document.topach_url(), Some("https://example.test/topach"));
    }

    #[test]
    fn symbol_metadata_uses_official_short_keys() {
        let symbol = AnalistSymbol::from_value(&json!({
            "sC": "THYAO",
            "sT": "X"
        }))
        .expect("geçerli sembol");

        assert_eq!(symbol.symbol_code, "THYAO");
        assert_eq!(symbol.symbol_type.as_deref(), Some("X"));
    }

    #[test]
    fn missing_discovery_url_is_rejected() {
        let error = required_env("MATRIKS_ANALIST_TEST_MISSING").unwrap_err();
        assert!(error.to_string().contains("MATRIKS_ANALIST_TEST_MISSING"));
    }
}
