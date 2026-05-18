// Güvenilir BIST veri çekme testi (AKBNK.IS, 1d, son 30 gün)
use memos_trading_core::robot::infra::exchange::bist;
use chrono::Utc;

#[tokio::test]
async fn test_bist_akbnk_1d_last_30days() {
    let symbol = "AKBNK.IS";
    let interval = "1d";
    let now = Utc::now().timestamp_millis();
    let thirty_days_ago = now - (30 * 24 * 60 * 60 * 1000);
    let result = bist::fetch_bist_klines(symbol, interval, thirty_days_ago, now, 30).await;
    match result {
        Ok(data) => {
            println!("AKBNK.IS 1d son 30 gün veri çekme: {} kayıt", data.len());
            for (i, kline) in data.iter().enumerate() {
                println!("{}. kline: {:?}", i + 1, kline);
            }
            assert!(!data.is_empty(), "AKBNK.IS 1d son 30 gün veri çekme sonucu boş olmamalı");
        },
        Err(e) => panic!("AKBNK.IS 1d son 30 gün veri çekme başarısız: {}", e),
    }
}
