// trading_cli mantığına uygun, async/await ve User-Agent ile Yahoo Finance üzerinden BIST veri çekme testi
// Bu dosya bağımsız çalışır, cargo test --test bist_async_yahoo_test ile çalıştırabilirsiniz

use reqwest::Client;
use serde_json::Value;
use chrono::Utc;

#[tokio::test]
async fn test_bist_async_yahoo_style() {
    let symbol = "AKBNK.IS";
    let interval = "1d";
    let now = Utc::now().timestamp_millis();
    let thirty_days_ago = now - (30 * 24 * 60 * 60 * 1000);
    let yf_interval = match interval {
        "1m" => "1m",
        "2m" => "2m",
        "5m" => "5m",
        "15m" => "15m",
        "30m" => "30m",
        "60m" | "1h" => "1h",
        "90m" => "90m",
        "1d" => "1d",
        "5d" => "5d",
        "1wk" | "1w" => "1wk",
        "1mo" | "1M" => "1mo",
        "3mo" => "3mo",
        _ => "1d",
    };
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&period1={}&period2={}",
        symbol, yf_interval, thirty_days_ago / 1000, now / 1000
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
                let result = data["chart"]["result"].as_array().and_then(|results| results.first());
                if let Some(result) = result {
                    let timestamps = result["timestamp"].as_array();
                    let quote = result["indicators"]["quote"].as_array().and_then(|quotes| quotes.first());
                    if let (Some(timestamps), Some(_quote)) = (timestamps, quote) {
                        println!("Yahoo Finance'dan {} kayıt alındı", timestamps.len());
                        assert!(!timestamps.is_empty(), "Yahoo Finance'dan veri alınamadı");
                    } else {
                        println!("Yahoo Finance veri formatı beklenmedik: {:?}", data);
                        assert!(false, "Yahoo Finance veri formatı hatalı");
                    }
                } else {
                    println!("Yahoo Finance result array yok: {:?}", data);
                    assert!(false, "Yahoo Finance result array yok");
                }
            } else {
                println!("Yahoo Finance HTTP hata: {}", resp.status());
                assert!(false, "Yahoo Finance HTTP hata");
            }
        },
        Err(e) => {
            println!("Yahoo Finance istek hatası: {}", e);
            assert!(false, "Yahoo Finance istek hatası");
        }
    }
}
