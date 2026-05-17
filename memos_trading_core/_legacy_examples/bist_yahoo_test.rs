// src/bin/bist_yahoo_test.rs
// BIST 100 sembolleri için Yahoo Finance veri çekme testi

use memos_trading_core::robot::data_fetcher::bist_fetcher::BistFetcher;
use memos_trading_core::robot::data_fetcher::market_fetcher::MarketFetcher;
use std::time::Duration;
use tokio::time::sleep;
use tokio::runtime::Runtime;

// BIST 100 örnek sembolleri (kısa liste, gerekirse uzatılabilir)
const BIST100: &[&str] = &[
    "AKBNK", "ASELS", "BIMAS", "BIZIM", "EKGYO", "EREGL", "FROTO", "GARAN", "ISCTR", "KCHOL",
    "KOZAA", "KOZAL", "ODAS", "PETKM", "PGSUS", "SAHOL", "SISE", "SOKM", "TCELL", "THYAO",
    "TKFEN", "TOASO", "TSKB", "TTKOM", "TUPRS", "VAKBN", "YKBNK"
];

fn main() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let fetcher = BistFetcher;
        for symbol in BIST100 {
            println!("\n🌐 {} için veri çekiliyor...", symbol);
            match fetcher.fetch_latest(symbol, "1d", 10).await {
                Ok(candles) => println!("✅ {}: {} mum çekildi", symbol.to_string(), candles.len()),
                Err(e) => println!("❌ {}: Hata: {}", symbol.to_string(), e),
            }
            sleep(Duration::from_secs(2)).await;
        }
    });
}
