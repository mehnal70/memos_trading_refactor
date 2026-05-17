use reqwest::Client;
use serde_json::Value;
use chrono::Utc;

#[tokio::main]
async fn main() {
    let symbol = "AKBNK.IS";
    let interval = "1d";
    let now = Utc::now().timestamp_millis();
    let thirty_days_ago = now - (30 * 24 * 60 * 60 * 1000);
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&period1={}&period2={}",
        symbol, interval, thirty_days_ago / 1000, now / 1000
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
                println!("Yanıt: {:?}", data);
            } else {
                println!("HTTP hata: {}", resp.status());
            }
        },
        Err(e) => {
            println!("İstek hatası: {}", e);
        }
    }
}
