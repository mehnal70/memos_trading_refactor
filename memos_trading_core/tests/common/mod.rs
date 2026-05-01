//! Integration testleri arası paylaşılan yardımcılar.
//!
//! Kullanım: integration test dosyasının başında `mod common;` ekleyerek erişilir.
//! Cargo `tests/common/mod.rs`'yi test crate'i olarak derlemez, sadece modül olarak içe aktarır.

use std::path::PathBuf;

/// `tests/fixtures/` dizininin tam yolunu döner.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

/// `fixtures/<name>` dosyasının içeriğini string olarak okur.
pub fn load_fixture_string(name: &str) -> String {
    let path = fixtures_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Fixture '{}' okunamadı: {}", path.display(), e))
}

/// `fixtures/<name>` dosyasını JSON olarak parse eder.
pub fn load_fixture_json<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let raw = load_fixture_string(name);
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("Fixture '{}' JSON parse hatası: {}", name, e))
}
