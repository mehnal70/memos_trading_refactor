# MatriksIQ + Ziraat bridge hazırlığı

Bu dizin, Ubuntu'daki Rust motoru ile MatriksIQ/Ziraat'ın bulunduğu Windows
VM arasında resmi bir dış entegrasyon sağlandığında kullanılacak hazırlık
alanıdır. MatriksIQ masaüstü penceresini otomatik tıklatarak emir göndermek
desteklenen bir entegrasyon değildir.

## Güvenli çalışma sırası

1. Tiny10 VM'de MatriksIQ + Ziraat oturumunu doğrula.
2. Matriks veya Ziraat'tan resmi API/SDK/Stream/XML/FIX protokolünü al.
3. Windows bridge'i yalnız Tailscale IP'sinde dinlet.
4. Önce `health`, `quote`, `candles` ve `session_status` sorgularını doğrula.
5. Sonra yalnız `balance`, `positions`, `open_orders` read-only sorgularını doğrula.
6. Paper emir akışını test et.
7. `MATRIKS_EXECUTION_ENABLED=0` kapısı kaldırılmadan gerçek emir gönderme.

## Beklenen bridge sözleşmesi

Sözleşme, Matriks'in resmi protokolü öğrenilmeden broker API'si gibi
uygulanmamalıdır. Hazırlık profili, Ubuntu ↔ Windows iletişimi için
satır-sınırlı JSON (NDJSON) taşıma biçimini hedefler:

```text
health
session_status
symbol_info
quote
candles
balance
positions
open_orders
order_status
```

Emir komutları (`submit_order`, `cancel_order`, `replace_order`) gerçek broker
protokolü ve yetkisi yazılı olarak doğrulanana kadar kapalıdır.

## Profil

`matriks-ziraat-paper.conf`, Binance, MT5, IBKR ve BIST'i aynı venue
registry'sinde tutar. Matriks/Ziraat yalnız `bist:spot` yolunun veri/read-only
hazırlık katmanıdır; diğer venue'lerin bridge ve execution ayarları
değiştirilmez.

```bash
RTC_LAUNCH_CONF=scripts/matriks-ziraat-paper.conf \
  bash scripts/engine.sh start --release
```

Profilin `MATRIKS_READ_ONLY=1` ve `MATRIKS_EXECUTION_ENABLED=0` ayarları
bilinçli olarak korunur. BIST kapalı günlerinde snapshot'ın eski veya boş
olması, bridge bağlantısının başarısız olduğu anlamına tek başına gelmez;
`source`, `last_bar` ve `session_status` ayrıca raporlanmalıdır.

## Maliyetsiz BIST tarihsel veri yolu

Matriks lisansı olmadan sinyal üretimi için mevcut BIST Yahoo adapter'ı
`.IS` sembol dönüşümünü ve SQLite candle upsert'ini zaten destekler:

`yfinance` güncel olmalıdır; eski sürümlerde Yahoo'nun timezone/crumb yanıtı
bozulabildiği için `No timezone found` hatası görülebilir:

```bash
python3 -m pip install --upgrade "yfinance>=1.0"
```

```bash
BIST_SYMBOLS=THYAO,GARAN,AKBNK,EREGL,ASELS \
DB_PATH=data/trader.db \
  bash scripts/bist/update_history.sh
```

Kalabalık bir evren için sembolleri dosyada tutabilirsiniz:

```text
# config/bist100_symbols.txt
THYAO
GARAN
AKBNK
EREGL
ASELS
```

```bash
BIST_SYMBOLS_FILE=config/bist100_symbols.txt \
DB_PATH=data/trader.db \
  bash scripts/bist/update_history.sh
```

BIST 100 listesini Borsa İstanbul'un resmi bileşen dosyasından otomatik
yenileyip aynı çalıştırmada verileri çekebilirsiniz:

```bash
BIST_AUTO_REFRESH=1 \
DB_PATH=data/trader.db \
  bash scripts/bist/update_history.sh
```

Liste yenileme scripti [refresh_bist100_symbols.py](../bist/refresh_bist100_symbols.py)
tarafından yapılır. Kaynak CSV'de `INDEX CODE=XU100` satırları seçilir; `.E`
uzantısı kaldırılır, semboller doğrulanır ve atomik olarak manifestoya yazılır.

CSV ile çalışılan akışta DB importundan önce kalite kontrolü çalıştırın:

```bash
python3 scripts/bist/validate_ohlcv_csv.py ./yahoo_bist_csv
```

Doğrulayıcı; `Date/Open/High/Low/Close` kolonlarını, sayısal formatı, pozitif
fiyatları, `High/Low` OHLC ilişkisini, negatif hacmi, duplicate tarihleri ve
tarih sırasını denetler. Hafta sonu ve resmi tatiller normal kabul edilir;
10 günden uzun boşluklar uyarı olarak raporlanır. Tekil satırdaki OHLC
tutarsızlıkları uyarıdır; importer aynı satırı ayrıca atlar. Eksik başlık,
boş dosya, bozuk tarih/sayı veya duplicate/sıra hatası gibi yapısal sorunlar
ise import öncesi düzeltilmelidir.

Bu komut [download_yahoo.rs](../../memos_trading_core/examples/download_yahoo.rs)
üzerinden günlük OHLC geçmişini `exchange=bist`, `market=bist` etiketiyle
mevcut `candles` tablosuna yazar. Aynı mum tekrar alınırsa canonical unique key
nedeniyle kopya üretmez.

TradingView'den elle dışa aktarılan CSV'ler de doğrudan içeri alınabilir:

```bash
DB_PATH=data/trader.db \
  cargo run --release --manifest-path Cargo.toml \
  -p memos_trading_core --example import_csv -- \
  bist 1d ./tradingview_csv
```

CSV dosya adı sembol olmalıdır (`THYAO.csv`). Bu yol canlı veri veya emir
üretmez; veri yalnızca tarihsel/sinyal/backtest amacıyla kullanılmalıdır.
TradingView web arayüzünü otomatik kazımaya dayalı bir downloader eklemiyoruz;
resmi dışa aktarma veya mevcut Yahoo yolu tercih edilmelidir.

## Günlük trend karar taraması

Veriler içe aktarıldıktan sonra BIST100 evrenini EMA50/EMA100 trend filtresi,
Donchian20 kırılımı ve ATR14 seviyeleriyle tarayın:

```bash
BIST_AUTO_REFRESH=1 \
DB_PATH=data/trader.db \
  bash scripts/bist/run_daily_trend.sh
```

Akış resmi BIST100 listesini yeniler (istenirse), Yahoo günlük CSV verisini
çeker, kalite kontrolünü çalıştırır, `candles` tablosuna upsert eder ve
`data/bist_trend_daily.json` raporunu üretir. `ENTRY_CANDIDATE` için üç
kademeli örnek piramit seviyesi ve koruyucu stop rapora eklenir.
`EXIT_WARNING` ve `ENTRY_CANDIDATE` kararları Telegram ortam değişkenleri
tanımlıysa bildirilir. Bu rapor karar desteğidir; manuel Ziraat işlemlerinin
yerine geçmez ve hiçbir emir göndermez.

## Analist REST hazırlığı

Analist market-data istemcisi Rust çekirdeğinde read-only olarak hazırlanmıştır.
Discovery adresi ve JWT yalnızca çalışma ortamından verilir; JWT'yi profile veya
repository dosyalarına yazmayın:

```bash
export MATRIKS_ANALIST_DISCOVERY_URL="https://<discovery-endpoint>"
export MATRIKS_ANALIST_JWT="<jwt>"
export MATRIKS_ANALIST_MARKET_CODE="BIST"
export MATRIKS_ANALIST_EXCHANGE_ID="4"
export MATRIKS_ANALIST_SYMBOL="THYAO"
```

`MATRIKS_ANALIST_JWT_ENV` kullanılırsa JWT'nin okunacağı değişken adı
`MATRIKS_ANALIST_JWT` yerine değiştirilebilir. REST istemcisi discovery,
Symbols ve Topach sorgularını destekler; canlı MQTT akışı ve emir gönderimi bu
katmandan başlatılmaz. Symbols/Topach URL'leri discovery yanıtından alınmalı,
hardcode edilmemelidir.

## Ağ

Bridge'i public IP'de açmayın. Ubuntu ile Windows VM arasında Tailscale veya
WireGuard kullanın ve bridge'i yalnız özel IP'de bind edin. API anahtarlarını
bu repository'deki config dosyalarına yazmayın.

## Transport probe

Windows tarafında resmi bridge çalışırken Ubuntu'dan yalnız transport ve
oturum durumunu test edin:

```bash
RTC_LAUNCH_CONF=scripts/matriks-ziraat-paper.conf \
  bash scripts/matriks/probe_bridge.sh
```

`UNAVAILABLE` sonucu bridge portunda dinleyen bir servis olmadığını gösterir;
bu, MatriksIQ oturumunun veya Ziraat hesabının durumunu tek başına kanıtlamaz.
Probe hiçbir emir komutu göndermez.

## Raw TCP listener probe

Kullanıcının C# listener'ı `sembol;islem;Limit;fiyat;adet` formatını
bekliyorsa bu, NDJSON bridge ile aynı protokol değildir. İlk testte yalnızca
portun dinlediğini kontrol edin:

```bash
MATRIKS_ORDER_BRIDGE_ADDR=192.168.122.186:9000 \
  bash scripts/matriks/probe_tcp.sh
```

Bu komut veri göndermez ve emir oluşturmaz. `TCP-READY` yalnızca Windows'ta
bir listener olduğunu gösterir; MatriksIQ/Ziraat oturumunun veya demo emrinin
başarılı olduğunu kanıtlamaz.

Raw listener'ı otonom engine'e doğrudan bağlamayın. Engine entegrasyonu için
ACK, broker order id, reject/error, fill ve reconciliation mesajları olan
belgelenmiş bir adapter gerekir. Ayrıca gerçek emir gönderimi şu kapılar
kapalı kalırken test edilmelidir:

```text
MATRIKS_READ_ONLY=1
MATRIKS_EXECUTION_ENABLED=0
```
