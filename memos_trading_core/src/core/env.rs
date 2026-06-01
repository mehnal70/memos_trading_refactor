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

/// `env_parse` + default: tanımsız/geçersizse `default` döner. `.unwrap_or(d)`
/// boilerplate'ini tek noktaya toplar (üst-katman `master::env_parse` muadili).
pub fn env_parse_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    env_parse(key).unwrap_or(default)
}

/// Dar "açık mı?" bayrağı: yalnızca `"1"` veya `"true"` (case-insensitive) → true;
/// tanımsız ya da başka her değer → false. Default-false toggle'lar
/// (`*_DISABLE`, `*_ENABLED`, `ALLOW_*`) için. Daha esnek küme isteyen `env_bool`.
pub fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Esnek bool okuyucu — kabul edilenler (trim + case-insensitive):
/// `"1"/"true"/"yes"/"on"` → Some(true), `"0"/"false"/"no"/"off"` → Some(false).
/// Tanımsız veya tanınmayan değer → None (çağıran kendi default'unu korur).
pub fn env_bool(key: &str) -> Option<bool> {
    let v = std::env::var(key).ok()?;
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
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

    #[test]
    fn env_parse_or_falls_back_on_unset_and_garbage() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CORE_ENV_TEST_P");
        assert_eq!(env_parse_or::<u64>("CORE_ENV_TEST_P", 7), 7);
        std::env::set_var("CORE_ENV_TEST_P", "bad");
        assert_eq!(env_parse_or::<u64>("CORE_ENV_TEST_P", 7), 7);
        std::env::set_var("CORE_ENV_TEST_P", "9");
        assert_eq!(env_parse_or::<u64>("CORE_ENV_TEST_P", 7), 9);
        std::env::remove_var("CORE_ENV_TEST_P");
    }

    #[test]
    fn env_truthy_only_accepts_one_and_true() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CORE_ENV_TEST_T");
        assert!(!env_truthy("CORE_ENV_TEST_T"));
        for v in ["1", "true", "TRUE", "True"] {
            std::env::set_var("CORE_ENV_TEST_T", v);
            assert!(env_truthy("CORE_ENV_TEST_T"), "{v} → true beklendi");
        }
        for v in ["0", "yes", "on", "false", "garbage"] {
            std::env::set_var("CORE_ENV_TEST_T", v);
            assert!(!env_truthy("CORE_ENV_TEST_T"), "{v} → false beklendi");
        }
        std::env::remove_var("CORE_ENV_TEST_T");
    }

    #[test]
    fn env_bool_accepts_extended_set() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CORE_ENV_TEST_B");
        assert_eq!(env_bool("CORE_ENV_TEST_B"), None);
        for v in ["1", "true", "yes", "on", " ON "] {
            std::env::set_var("CORE_ENV_TEST_B", v);
            assert_eq!(env_bool("CORE_ENV_TEST_B"), Some(true), "{v}");
        }
        for v in ["0", "false", "no", "off"] {
            std::env::set_var("CORE_ENV_TEST_B", v);
            assert_eq!(env_bool("CORE_ENV_TEST_B"), Some(false), "{v}");
        }
        std::env::set_var("CORE_ENV_TEST_B", "maybe");
        assert_eq!(env_bool("CORE_ENV_TEST_B"), None);
        std::env::remove_var("CORE_ENV_TEST_B");
    }
}
