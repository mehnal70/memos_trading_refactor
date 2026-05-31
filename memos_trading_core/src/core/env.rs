//! Alt-katman (core) ortam değişkeni okuyucuları — tek-nokta.
//!
//! `robot::engines::master::env_parse` üst-katmanda yaşıyor ve core ondan import
//! EDEMEZ (katman yönü: robot → core). Bu modül core ve altındaki katmanların
//! `std::env::var(...).unwrap_or_else/ok/parse` boilerplate'ini DRY'lar
//! ([[project_modernization_roadmap]] Faz 1 cross-layer env maddesi;
//! [[feedback_config_externalization]]).

/// String env değeri; tanımsızsa `default` (sahiplenilmiş String döner).
pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

/// Opsiyonel String env değeri (tanımsız → None). Boş string yine Some("") döner;
/// boşu reddetmek isteyen çağıran `.filter(|s| !s.is_empty())` ekler.
pub fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// `T: FromStr` olarak parse edilen env değeri; tanımsız VEYA geçersiz → None.
/// (env_parse üst-katman muadiliyle aynı semantik.)
pub fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|v| v.parse::<T>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env testleri process-global state'e dokunur → seri çalışsın.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn env_or_returns_default_when_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CORE_ENV_TEST_X");
        assert_eq!(env_or("CORE_ENV_TEST_X", "def"), "def");
    }

    #[test]
    fn env_or_and_parse_read_set_value() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("CORE_ENV_TEST_Y", "42");
        assert_eq!(env_or("CORE_ENV_TEST_Y", "def"), "42");
        assert_eq!(env_parse::<i64>("CORE_ENV_TEST_Y"), Some(42));
        assert_eq!(env_opt("CORE_ENV_TEST_Y"), Some("42".to_string()));
        std::env::remove_var("CORE_ENV_TEST_Y");
    }

    #[test]
    fn env_parse_none_on_garbage() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("CORE_ENV_TEST_Z", "not-a-number");
        assert_eq!(env_parse::<f64>("CORE_ENV_TEST_Z"), None);
        std::env::remove_var("CORE_ENV_TEST_Z");
    }
}
