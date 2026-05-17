// src/bin/finnhub_test.rs
// Finnhub API ile BIST veri çekme testi

use memos_trading_core::robot::data_fetcher::finnhub::FinnhubFetcher;
use memos_trading_core::robot::data_fetcher::market_fetcher::MarketFetcher;
use std::env;
use tokio::runtime::Runtime;

const BIST100: &[&str] = &[
    "AKBNK.IS", "ASELS.IS", "BIMAS.IS", "BIZIM.IS", "EKGYO.IS"
    // ...devamı eklenebilir
];

fn main() {
    let api_key = env::var("FINNHUB_API_KEY").expect("FINNHUB_API_KEY ortam değişkeni gerekli!");
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let fetcher = FinnhubFetcher::new(api_key);
        for symbol in BIST100 {
            println!("\n🌐 {} için veri çekiliyor...", symbol);
            match fetcher.fetch_latest(symbol, "D", 10).await {
                Ok(candles) => println!("✅ {}: {} mum çekildi", symbol, candles.len()),
                Err(e) => println!("❌ {}: Hata: {}", symbol, e),
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    });
}
