// BIST veri indirme fonksiyonunun bağımsız testi
// cargo test --test bist_download_test ile çalıştırabilirsiniz

use memos_trading_core::robot::infra::exchange::bist;
use serde_json::Value;

#[tokio::test]
async fn test_bist_klines_download() {
    // Örnek sembol ve kısa zaman aralığı (son 1 gün)
    let symbol = "AKBNK.IS";
    let interval = "1d";
    let now = chrono::Utc::now().timestamp_millis();
    let one_day_ago = now - (24 * 60 * 60 * 1000);
    let result = bist::fetch_bist_klines(symbol, interval, one_day_ago, now, 10).await;
    match result {
        Ok(data) => {
            println!("BIST veri indirme başarılı, {} kayıt alındı", data.len());
            assert!(!data.is_empty(), "BIST veri indirme sonucu boş olmamalı");
            // Kline formatı kontrolü
            let first = &data[0];
            assert!(first.len() >= 6, "Kline dizisi en az 6 eleman içermeli");
            assert!(matches!(first[0], Value::Number(_)), "Timestamp number olmalı");
        },
        Err(e) => panic!("BIST veri indirme başarısız: {}", e),
    }
}
