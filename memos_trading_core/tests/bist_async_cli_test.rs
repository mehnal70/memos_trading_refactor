// trading_cli mantığına uygun, async/await ve User-Agent ile BIST veri çekme testi
// Bu dosya bağımsız çalışır, cargo test --test bist_async_cli_test ile çalıştırabilirsiniz

use reqwest::Client;
use serde_json::Value;
use chrono::Utc;

// Gerçek HTTP çağrısı (api.borsaistanbul.com) gerektirir; offline/CI'da
// sahte fail verir. Manuel koşum: `cargo test --test bist_async_cli_test -- --ignored`
#[tokio::test]
#[ignore = "external network: api.borsaistanbul.com"]
async fn test_bist_async_cli_style() {
    let symbol = "AKBNK.IS";
    let interval = "1d";
    let now = Utc::now().timestamp_millis();
    let thirty_days_ago = now - (30 * 24 * 60 * 60 * 1000);
    let bist_interval = match interval {
        "1m" => "1min",
        "3m" => "3min",
        "5m" => "5min",
        "15m" => "15min",
        "30m" => "30min",
        "1h" => "1hour",
        "2h" => "2hour",
        "4h" => "4hour",
        "6h" => "6hour",
        "8h" => "8hour",
        "12h" => "12hour",
        "1d" => "1day",
        "3d" => "3day",
        "1w" => "1week",
        "1M" => "1month",
        _ => interval,
    };
    let url = format!(
        "https://api.borsaistanbul.com/v1/stocks/{}/klines?interval={}&startTime={}&endTime={}&limit=30",
        symbol.replace(".IS", ""), bist_interval, thirty_days_ago, now
    );
    let client = Client::new();
    let resp = client.get(&url)
        .header("User-Agent", "Mozilla/5.0 (compatible; trading-bot/1.0)")
        .send()
        .await;
    match resp {
        Ok(resp) => {
            if resp.status().is_success() {
                let data: Value = resp.json().await.unwrap();
                if let Some(klines_array) = data.as_array() {
                    println!("BIST API'den {} kayıt alındı", klines_array.len());
                    assert!(!klines_array.is_empty(), "BIST API'den veri alınamadı");
                } else {
                    println!("BIST API'den veri formatı beklenmedik: {:?}", data);
                    assert!(false, "BIST API veri formatı hatalı");
                }
            } else {
                println!("BIST API HTTP hata: {}", resp.status());
                assert!(false, "BIST API HTTP hata");
            }
        },
        Err(e) => {
            println!("BIST API istek hatası: {}", e);
            assert!(false, "BIST API istek hatası");
        }
    }
}
