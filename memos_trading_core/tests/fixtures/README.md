# Test Fixtures

Integration testlerinde kullanılan sabit veri dosyaları.

## Yapı

- `candles_*.json` — OHLCV mum verileri (sembol/interval bazlı)
- (gelecekte) `funding_rates_*.json`, `orderbook_snapshots_*.json` vb.

## Kullanım

```rust
mod common;
use common::load_fixture_json;

#[test]
fn test_with_fixture() {
    let candles: Vec<Candle> = load_fixture_json("candles_btc_1m_sample.json");
    // testler...
}
```

## Yeni Fixture Ekleme Kuralları

1. **Küçük tut**: 5–50 mum yeterli; integration test hızı kritik
2. **Anlamlı isim**: `<symbol>_<interval>_<senaryo>.json` (örn. `btc_1h_volatile.json`)
3. **Production data sızdırma**: API key, gerçek hesap ID'si yok
4. **JSON format**: serde uyumlu, schema değişikliklerini de versiyonla
