// Gerçek zamanlı (anlık) BIST veri çekme testi
// cargo test --test bist_realtime_test ile çalıştırabilirsiniz

use memos_trading_core::robot::infra::exchange::bist;
use chrono::Utc;

// Gerçek HTTP çağrısı (bist::fetch_bist_klines → external API); offline/CI'da
// sahte fail verir. Manuel koşum: `cargo test --test bist_realtime_test -- --ignored`
#[tokio::test]
#[ignore = "external network: BIST realtime klines"]
async fn test_bist_realtime_klines() {
    let symbol = "AKBNK.IS";
    let interval = "1m";
    let now = Utc::now().timestamp_millis();
    let five_min_ago = now - (5 * 60 * 1000); // Son 5 dakika
    let result = bist::fetch_bist_klines(symbol, interval, five_min_ago, now, 10).await;
    match result {
        Ok(data) => {
            println!("Gerçek zamanlı BIST veri çekme başarılı, {} kayıt alındı", data.len());
            for (i, kline) in data.iter().enumerate() {
                println!("{}. kline: {:?}", i + 1, kline);
            }
            assert!(!data.is_empty(), "Gerçek zamanlı BIST veri çekme sonucu boş olmamalı");
        },
        Err(e) => panic!("Gerçek zamanlı BIST veri çekme başarısız: {}", e),
    }
}
