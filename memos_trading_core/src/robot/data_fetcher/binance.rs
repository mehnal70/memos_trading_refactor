// robot/data_fetcher/binance.rs - Binance REST API Veri Çekici (Modernize Edilmiş)

use crate::core::types::Candle;
use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::robot::data_fetcher::websocket::validate_ohlcv; 
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::time::Duration;

pub struct BinanceFetcher {
    client: reqwest::Client,
}

impl Default for BinanceFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl BinanceFetcher {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15)) // Ağ gecikmelerine karşı tolerans artırıldı
                .build()
                .unwrap_or_default(),
        }
    }

    /// Market-farkında klines endpoint'i: futures → fapi.binance.com/fapi/v1,
    /// diğer (spot) → api.binance.com/api/v3. Eskiden fetcher SABİT spot endpoint'ine
    /// vuruyordu → futures botu spot veriyle karar veriyordu (Faz 2 correctness).
    fn klines_base(market: &str) -> &'static str {
        if market.eq_ignore_ascii_case("futures") {
            "https://fapi.binance.com/fapi/v1/klines"
        } else {
            "https://api.binance.com/api/v3/klines"
        }
    }

    /// Market-farkında son N mum. `fetch_latest` bunun spot kısayoludur (geriye-uyum).
    pub async fn fetch_latest_market(
        &self, symbol: &str, interval: &str, market: &str, limit: usize,
    ) -> Result<Vec<Candle>, String> {
        let url = format!(
            "{}?symbol={}&interval={}&limit={}",
            Self::klines_base(market), symbol, interval, limit
        );
        self.fetch_klines(&url, symbol, interval).await
    }

    /// 🕳️ Faz 2 follow-up: `startTime`'dan İLERİ tek-istek (≤1000 bar). `fetch_latest`
    /// son-N çeker (startTime yok) → >1000-bar gap'in dibi asla dolmaz; bu, gap'in
    /// başından (start_ms) başlayıp ileri pagine etmeyi mümkün kılar. Boş yanıt
    /// (start_ms ≥ now veya borsa o aralıkta veri tutmuyor) HATA DEĞİL → Ok(boş).
    pub async fn fetch_range_market(
        &self, symbol: &str, interval: &str, market: &str, start_ms: i64, limit: usize,
    ) -> Result<Vec<Candle>, String> {
        let limit = limit.clamp(1, 1000);
        let url = format!(
            "{}?symbol={}&interval={}&startTime={}&limit={}",
            Self::klines_base(market), symbol, interval, start_ms.max(0), limit
        );
        self.fetch_klines_inner(&url, symbol, interval).await
    }

    /// 🕳️ Derin gap backfill: `start_ms`'ten ŞİMDİYE kadar ileri pagine eder
    /// (her istek ≤1000 bar; imleç son mum + 1 interval ileri). Durma koşulları:
    /// (a) `max_requests` istek tavanı (bir cycle'da sınırlı API yükü → kalan gap
    /// sonraki cycle'larda yakınsar), (b) tam-dolmayan sayfa (= şimdiye yetişildi),
    /// (c) imleç ilerlemiyor (no-progress guard, sonsuz döngü koruması). Birleşik mum
    /// dizisi döner (kronolojik). Hiç veri yoksa Ok(boş) — çağıran başarısızlık saymaz.
    pub async fn fetch_history_market(
        &self, symbol: &str, interval: &str, market: &str,
        start_ms: i64, iv_secs: i64, max_requests: usize,
    ) -> Result<Vec<Candle>, String> {
        let iv_ms = iv_secs.max(1) * 1000;
        let now_ms = crate::core::time::now_epoch_millis() as i64;
        let mut cursor = start_ms.max(0);
        let mut out: Vec<Candle> = Vec::new();
        let mut last_err: Option<String> = None;

        for _ in 0..max_requests.max(1) {
            if cursor >= now_ms { break; }
            match self.fetch_range_market(symbol, interval, market, cursor, 1000).await {
                Ok(batch) => {
                    if batch.is_empty() { break; } // borsa bu aralıkta veri tutmuyor → bitti
                    // İmleci son mumun bir interval ÖTESİNE taşı (üst-üste binmeyi önle).
                    let last_ts = batch.iter().map(|c| c.timestamp.timestamp_millis()).max().unwrap_or(cursor);
                    let n = batch.len();
                    out.extend(batch);
                    let next = last_ts + iv_ms;
                    if next <= cursor { break; } // no-progress guard
                    cursor = next;
                    if n < 1000 { break; } // tam-dolmayan sayfa → şimdiye yetişildi
                }
                // Geçici ağ/parse hatası: ilk hatayı sakla, döngüyü kır (toplanan korunur).
                Err(e) => { last_err = Some(e); break; }
            }
        }

        if out.is_empty() {
            return Err(last_err.unwrap_or_else(|| format!("{} backfill: aralıkta veri yok", symbol)));
        }
        Ok(out)
    }

    /// Ortak klines parse çekirdeği (spot/futures aynı payload formatı).
    /// Boş yanıtta HATA döner (latest-fetch yolu için: sembol delisted/yanlış → görünür sinyal).
    /// Pagination boş-OK ister → [`fetch_klines_inner`] kullanır.
    async fn fetch_klines(&self, url: &str, symbol: &str, interval: &str) -> Result<Vec<Candle>, String> {
        let candles = self.fetch_klines_inner(url, symbol, interval).await?;
        if candles.is_empty() {
            return Err(format!("{} sembolü için geçerli mum verisi alınamadı", symbol));
        }
        Ok(candles)
    }

    /// HTTP + parse çekirdeği — boş yanıtı HATA SAYMAZ (Ok(boş) döner). Pagination
    /// için gerekli (aralık-sonu boş yanıtı normal terminasyon, hata değil).
    async fn fetch_klines_inner(&self, url: &str, symbol: &str, interval: &str) -> Result<Vec<Candle>, String> {
        let resp = self.client.get(url)
            .send()
            .await
            .map_err(|e| format!("Binance Bağlantı Hatası: {}", e))?
            .json::<Vec<Vec<serde_json::Value>>>()
            .await
            .map_err(|e| format!("Binance Veri Format Hatası: {}", e))?;

        let mut candles = Vec::with_capacity(resp.len());

        for k in resp {
            // 1. Zaman Damgası Kontrolü (i64 ms)
            let ts_ms = match k.first().and_then(|v| v.as_i64()) {
                Some(ts) if ts > 0 => ts,
                _ => continue,
            };

            // 2. Sayısal Verilerin Güvenli Parse Edilmesi
            // Binance verileri string döner, bu yüzden as_str() üzerinden parse ediyoruz.
            let parse_f = |idx: usize| {
                k.get(idx)
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
            };

            let open   = parse_f(1).unwrap_or(0.0);
            let high   = parse_f(2).unwrap_or(0.0);
            let low    = parse_f(3).unwrap_or(0.0);
            let close  = parse_f(4).unwrap_or(0.0);
            
            // §12.3: Taker Buy Quote Asset Volume (Index 7) 
            // Bu değer, piyasa alıcılarının (agresif işlemler) gerçek hacmini gösterir.
            let volume = parse_f(7).unwrap_or(0.0);

            // 3. Otonom Veri Doğrulama (validate_ohlcv)
            // Sadece matematiksel olarak tutarlı mumlar boru hattına girebilir.
            if validate_ohlcv(open, high, low, close, volume).is_err() {
                continue;
            }

            if let Some(dt) = DateTime::from_timestamp_millis(ts_ms) {
                candles.push(Candle {
                    timestamp: dt.with_timezone(&Utc),
                    open,
                    high,
                    low,
                    close,
                    volume,
                    symbol: symbol.to_string(),
                    interval: interval.to_string(),
                });
            }
        }

        Ok(candles)
    }
}

#[async_trait]
impl MarketFetcher for BinanceFetcher {
    fn name(&self) -> &'static str { "binance" }

    /// Trait yolu spot kısayolu (geriye-uyum). Market-farkında çağrılar
    /// `fetch_latest_market` kullanmalı (download job Faz 2'de geçti).
    async fn fetch_latest(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>, String> {
        self.fetch_latest_market(symbol, interval, "spot", limit).await
    }
}

#[cfg(test)]
mod backfill_net_tests {
    use super::*;

    // Ağ testi — #[ignore] (test-hijyeni: ağ testleri CI'de koşmaz). Elle:
    // `cargo test -p memos_trading_core fetch_history_market_paginates_forward -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn fetch_history_market_paginates_forward() {
        let f = BinanceFetcher::new();
        // ~2500 bar (1m) önce başla → ≥1000 = pagination şart; 3 istek tavanı.
        let now_ms = crate::core::time::now_epoch_millis() as i64;
        let start = now_ms - 2500 * 60_000;
        let candles = f.fetch_history_market("BTCUSDT", "1m", "spot", start, 60, 3).await.unwrap();
        // 3×1000 tavanı → >2000 bar gelmeli (tek-istek 1000'i aşar = gap kapanır).
        assert!(candles.len() > 2000, "pagination >2000 bar getirmeli, geldi: {}", candles.len());
        // Kronolojik + tekil (cursor doğru ilerledi, üst-üste binme yok).
        for w in candles.windows(2) {
            assert!(w[1].timestamp > w[0].timestamp, "mumlar artan-zaman + tekil olmalı");
        }
    }
}
