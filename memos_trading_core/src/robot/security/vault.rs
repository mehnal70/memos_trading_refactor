// src/robot/security/vault.rs - Maskelenmiş API Sır Kalkanı
// Srivastava ATP - Anahtar Yalıtım Odası

use std::collections::HashMap;
use crate::MemosTradingError;

pub struct ApiKeyManager {
    keys: HashMap<String, String>, // exchange -> masked_key
}

impl ApiKeyManager {
    /// Çevresel değişkenlerden API key'leri izole ederek yükler
    pub fn from_env(exchanges: &[&str]) -> Result<Self, MemosTradingError> {
        let mut keys = HashMap::new();
        
        for exchange in exchanges {
            let key_var = format!("{}_API_KEY", exchange.to_uppercase());
            let secret_var = format!("{}_API_SECRET", exchange.to_uppercase());
            
            let api_key = std::env::var(&key_var)
                .map_err(|_| MemosTradingError::Config(format!("Eksik Çevresel Değişken: {}", key_var)))?;
            
            let _api_secret = std::env::var(&secret_var)
                .map_err(|_| MemosTradingError::Config(format!("Eksik Çevresel Değişken: {}", secret_var)))?;
            
            if api_key.is_empty() {
                return Err(MemosTradingError::Config(format!("{}: API key ve secret boş olamaz", exchange)));
            }
            
            // Log sızıntı koruması: Sırları maskele (Sadece ilk 4 ve son 4 karakteri göster)
            let masked = format!(
                "{}...{}", 
                &api_key[0..4.min(api_key.len())], 
                &api_key[api_key.len().saturating_sub(4)..]
            );
            keys.insert(exchange.to_lowercase(), masked);
        }
        
        Ok(Self { keys })
    }

    /// Raporlama ve loglama için maskelenmiş anahtarı döndürür (Asla ham sır sızmaz)
    pub fn get_masked(&self, exchange: &str) -> Option<String> {
        self.keys.get(&exchange.to_lowercase()).cloned()
    }
    
    pub fn is_initialized(&self) -> bool {
        !self.keys.is_empty()
    }
}
