# 📊 Günlük Raporlama ve Arşivleme Sistemi

## Genel Bakış

Memos Trading platformu için kapsamlı günlük raporlama ve arşivleme sistemi. Exchange/Market bazlı analiz ile tüm trading aktivitelerini otomatik olarak raporlar ve arşivler.

## ✨ Özellikler

### 1. Exchange/Market Bazlı Analiz
- **Binance Spot**: Spot piyasa işlemleri
- **Binance Futures**: Vadeli işlem sözleşmeleri  
- **Binance COINM**: Coin-margined futures
- **BIST**: Türkiye borsası entegrasyonu
- Diğer exchange ve market türleri için genişletilebilir yapı

### 2. Kapsam
Günlük raporlar şunları içerir:
- ✅ **Trade History**: Tüm kapanan işlemler (trade_history.jsonl)
- ✅ **Autonomous Cycles**: AI/ML tabanlı otonom trading döngüleri
- ✅ **Robotic Actions**: Otomatik trading sistemi aksiyonları
- ✅ **Performance Metrics**: Win rate, PnL, drawdown, Sharpe ratio
- ✅ **Exchange/Market Stats**: Her pazar için ayrı istatistikler

### 3. Rapor Formatları

#### JSON Format
```json
{
  "date": "2026-01-29",
  "exchange_market_summary": {
    "binance_spot": {
      "exchange": "binance",
      "market": "spot",
      "trade_count": 15,
      "symbols": ["BTCUSDT", "ETHUSDT"],
      "total_volume": 125000.50,
      "pnl": 2345.67,
      "win_rate": 66.7
    }
  },
  "total_trades": 15,
  "total_autonomous_cycles": 8,
  "total_robotic_actions": 42,
  "symbols_traded": ["BTCUSDT", "ETHUSDT", "BNBUSDT"],
  "performance_summary": {
    "total_pnl": 2345.67,
    "total_pnl_pct": 2.35,
    "win_count": 10,
    "loss_count": 5,
    "win_rate": 66.7,
    "largest_win": 450.00,
    "largest_loss": -125.50,
    "avg_trade_duration_mins": 145.3
  },
  "generated_at": "2026-01-29T23:55:00Z"
}
```

#### HTML Format
- Görsel olarak zengin, okunabilir raporlar
- Exchange/Market bazlı tablolar
- Performans metrikleri
- Renk kodlaması (kazanç: yeşil, kayıp: kırmızı)
- Responsive tasarım

#### CSV Format
- Excel/Google Sheets uyumlu
- Exchange, Market, Trade Count, Volume, PnL, Win Rate kolonları
- Performans metrikleri ayrı bölümde
- Kolay veri analizi için

## 📁 Dosya Yapısı

```
logs/
├── trade_history.jsonl          # Trade kayıtları
├── robotic_trader.log            # Robotic trader logları
├── autonomous/                   # Autonomous cycle kayıtları
│   ├── cycle_001_*.json
│   ├── cycle_002_*.json
│   └── ...
└── daily_archives/               # Günlük arşiv raporları
    ├── report_2026-01-29.json   # JSON rapor
    ├── report_2026-01-29.html   # HTML rapor
    ├── report_2026-01-29.csv    # CSV rapor
    └── ...
```

## 🚀 Kullanım

### Dashboard Üzerinden

1. **Manuel Rapor Oluşturma**:
   - Dashboard'da "Günlük Raporlar & Arşiv" kartını bulun
   - "📄 Bugün İçin Rapor Oluştur" butonuna tıklayın
   - Rapor otomatik olarak oluşturulur ve arşivlenir

2. **Arşivleri Listeleme**:
   - "📂 Arşivleri Listele" butonuna tıklayın
   - Tüm eski raporlar listelenir

3. **Eski Raporları Temizleme**:
   - "🗑️ Eski Raporları Temizle" butonuna tıklayın
   - 90 günden eski raporlar otomatik silinir

### Tauri Commands

```typescript
// Günlük rapor oluştur
const report = await invoke('generate_daily_report', { 
  date: "2026-01-29" // Opsiyonel, null ise bugün
})

// Arşivleri listele
const reports = await invoke<string[]>('list_daily_reports')

// Eski raporları temizle (90 günden eski)
const deletedCount = await invoke<number>('cleanup_old_reports', { 
  daysToKeep: 90 
})
```

### Programatik Kullanım (Rust)

```rust
use daily_archiver::DailyArchiver;

let archiver = DailyArchiver::new();

// Bugün için rapor oluştur
let report = archiver.generate_and_archive_daily_report(None)?;

// Belirli bir tarih için
let target_date = Utc::now() - chrono::Duration::days(7);
let report = archiver.generate_and_archive_daily_report(Some(target_date))?;

// Arşivleri listele
let reports = archiver.list_archived_reports()?;

// Eski raporları temizle
let deleted = archiver.cleanup_old_archives(90)?;
```

## ⏰ Otomatik Günlük Raporlama

Sistem her gece **saat 23:55**'te otomatik olarak günlük rapor oluşturur:

- ✅ Otomatik tetikleme (her gün 23:55-23:59 arası)
- ✅ JSON, HTML ve CSV formatlarında kayıt
- ✅ Terminal loglarına başarı/hata mesajları
- ✅ 6 saat bekleme sonrası bir sonraki kontrol
- ✅ 5 dakikalık polling döngüsü

```rust
// main.rs setup hook'unda otomatik çalışır
tauri::async_runtime::spawn(async move {
    loop {
        let now = Utc::now();
        if now.hour() == 23 && now.minute() >= 55 {
            // Günlük rapor oluştur
            archiver.generate_and_archive_daily_report(None)?;
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
});
```

## 📊 Rapor İçeriği

### Exchange/Market Summary
Her exchange/market kombinasyonu için:
- Trade sayısı
- İşlem gören semboller
- Toplam hacim (USD)
- Kar/Zarar (PnL)
- Kazanma oranı (Win Rate %)

### Performance Summary
- **Toplam PnL**: Günlük kar/zarar
- **PnL Yüzdesi**: Başlangıç sermayesine göre %
- **Kazanan İşlem Sayısı**: Kârlı trade'ler
- **Kaybeden İşlem Sayısı**: Zararlı trade'ler
- **Win Rate**: Kazanma oranı %
- **En Büyük Kazanç**: Tek işlemde en yüksek kâr
- **En Büyük Kayıp**: Tek işlemde en yüksek zarar
- **Ortalama İşlem Süresi**: Dakika cinsinden

### Autonomous Trading
- Çalıştırılan otonom döngü sayısı
- Her döngünün detayları (logs/autonomous/)
- Stage bazlı başarı/hata kayıtları

### Robotic Trading
- Otomatik sistem aksiyonları
- BUY/SELL sinyalleri
- Gerçekleştirilen işlemler

## 🔧 Genişletme

### Yeni Exchange Ekleme

```rust
// types.rs içinde
pub enum Exchange {
    Binance,
    Bist,
    YeniExchange,  // ✨ YENİ
}

impl Exchange {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "binance" => Some(Exchange::Binance),
            "bist" => Some(Exchange::Bist),
            "yeni_exchange" => Some(Exchange::YeniExchange),  // ✨ YENİ
            _ => None,
        }
    }
}
```

### Yeni Market Türü Ekleme

```rust
// types.rs içinde
pub enum Market {
    Spot,
    Futures,
    Coinm,
    YeniMarket,  // ✨ YENİ
}

impl Market {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "spot" => Some(Market::Spot),
            "futures" => Some(Market::Futures),
            "coinm" => Some(Market::Coinm),
            "yeni_market" => Some(Market::YeniMarket),  // ✨ YENİ
            _ => None,
        }
    }
}
```

## 📝 Trade Logging için Exchange/Market Ekleme

Trade kayıtlarınızda Exchange ve Market bilgisi olması için:

```rust
// Trade struct'ına ekleme yapın
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: Option<u64>,
    pub symbol: String,
    pub exchange: String,    // ✨ YENİ
    pub market: String,      // ✨ YENİ
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    // ... diğer alanlar
}

// Trade oluştururken exchange/market ekleyin
let trade = Trade {
    symbol: "BTCUSDT".to_string(),
    exchange: "binance".to_string(),
    market: "spot".to_string(),
    // ... diğer değerler
};
```

## 🎯 Avantajlar

1. **Kapsamlı Analiz**: Her exchange ve market için ayrı istatistikler
2. **Otomatik Arşivleme**: Manuel müdahale gerektirmez
3. **Çoklu Format**: JSON (programatik), HTML (görsel), CSV (analiz)
4. **Exchange Agnostic**: Herhangi bir exchange eklenebilir
5. **Performans Odaklı**: Hızlı JSON okuma/yazma
6. **Temiz Arşiv**: Otomatik eski rapor temizleme

## 🛡️ Hata Yönetimi

- Dosya yoksa boş veri döndürür (crash etmez)
- JSON parse hatalarını loglayıp geçer
- Exchange/Market bilgisi yoksa "binance/spot" varsayılanı kullanır
- Arşiv klasörü yoksa otomatik oluşturur

## 📈 Gelecek Geliştirmeler

- [ ] Haftalık/aylık özet raporları
- [ ] E-posta/Telegram bildirimleri
- [ ] Grafik/chart eklentisi (HTML raporlarında)
- [ ] Karşılaştırmalı analiz (önceki günlerle)
- [ ] PDF export desteği
- [ ] Real-time dashboard widget'ı
- [ ] Prometheus/Grafana entegrasyonu

## 📞 Kullanım Örnekleri

### Örnek 1: Son 7 günlük performans
```bash
# logs/daily_archives/ klasöründeki raporları incele
ls -la logs/daily_archives/report_*.json
```

### Örnek 2: CSV'yi Excel'de aç
```bash
# CSV dosyasını Excel/LibreOffice ile aç
xdg-open logs/daily_archives/report_2026-01-29.csv
```

### Örnek 3: HTML raporunu tarayıcıda aç
```bash
# HTML raporunu varsayılan tarayıcıda aç
xdg-open logs/daily_archives/report_2026-01-29.html
```

---

**Oluşturulma Tarihi**: 29 Ocak 2026  
**Versiyon**: 1.0  
**Yazar**: Memos Trading Development Team
