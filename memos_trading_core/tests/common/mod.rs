//! Integration testleri arası paylaşılan yardımcılar.
//!
//! Kullanım: integration test dosyasının başında `mod common;` ekleyerek erişilir.
//! Cargo `tests/common/mod.rs`'yi test crate'i olarak derlemez, sadece modül olarak içe aktarır.
#![allow(dead_code)] // her test binary'si helper'ların yalnız bir alt-kümesini kullanır

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Bir koşul sağlanana dek periyodik (50ms) yoklar; `ceiling` aşılırsa `false`.
///
/// Sabit `sleep(N).await` + tek `assert!` kalıbının CPU-contention kırılganlığını
/// giderir: koşul ne zaman doğru olursa o an döner (yüksüzde hızlı), yük altında
/// `ceiling`'e kadar bekler. Kapsama kaybı YOK — aynı koşul assert edilir, yalnız
/// _ne zaman_ kontrol edildiği toleranslı. Koşul hiç sağlanmazsa `false` → çağıran
/// yine `assert!` ile düşer. Engine-loop liveness testleri (last_tick/saw_*) için.
pub async fn poll_until<F: FnMut() -> bool>(ceiling: Duration, mut cond: F) -> bool {
    let start = Instant::now();
    let step = Duration::from_millis(50);
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() >= ceiling {
            return false;
        }
        tokio::time::sleep(step).await;
    }
}

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
