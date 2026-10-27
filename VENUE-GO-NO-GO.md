# Venue Go/No-Go Raporu

**Değerlendirme tarihi:** 2026-10-01  
**Kapsam:** Paper, demo ve testnet doğrulamaları  
**Canlı para:** Kullanılmadı

## Özet karar

| Venue | Test ortamı | Karar | Kapsam |
|---|---|---|---|
| Binance | Futures Testnet + dry-run | **GO** | Yalnızca testnet/dry-run |
| MT5 | Demo hesap | **GO** | Yalnızca demo |
| IBKR | Paper hesap | **GO** | Yalnızca paper |
| BIST | Paper simulator | **GO** | Paper simülasyon; gerçek execution yok |
| Tüm canlı hesaplar | Live | **NO-GO** | Açık canlı hesap ve operasyon onayı yok |

Bu kararlar canlı ticaret izni anlamına gelmez. Canlıya geçiş için ayrıca hesap, sembol,
limit, alarm ve operatör onayı kabul kapısı gerekir.

## Ortak güvenlik ve operasyon kanıtları

- Risk, R/R, HTF/ATR, trend-only pyramiding ve protection kapıları doğrulandı.
- Reduce-only çıkış, broker-side koruma ve cleanup akışları doğrulandı.
- Binance transient API/DataStall recovery sonrası anomaly temizliği doğrulandı.
- Telegram notifier gönderim hatasında cooldown'u geri alıyor; gerçek test alarmı teslim edildi.
- Watchdog systemd-user servisi aktif ve engine'i denetliyor.
- Engine-down testi:
  - Engine graceful olarak `2026-10-01 14:09:23` tarihinde durduruldu.
  - Watchdog `2026-10-01 14:10:11` tarihinde ölümü tespit etti.
  - Engine `--release` ile yeniden başlatıldı ve son kontrolde çalışıyordu.
- Kill-switch odak testleri: **5/5 geçti**.
  - Risk halt trip sonrası aktif kalıyor.
  - Safe-mode bağımsız mühürleniyor.
  - `cooldown=0` davranışı doğrulanıyor.
  - Live-block recovery risk halt'ı temizlemiyor.
  - Safe-mode allocation çarpanı tahsisatı azaltıyor.

## Venue kanıtları

### Binance Testnet

- Testnet endpoint yönlendirmesi doğrulandı.
- Market entry, Algo SL/TP protection, reduce-only close ve cleanup başarılı.
- Runtime recovery sonrasında heartbeat karar üretmeye devam etti.
- Son kabul koşullarında açık pozisyon ve açık emir bulunmadı.

### MT5 Demo

- EURUSD demo BUY, broker-side SL/TP, ACK ve exact reduce-only SELL ile kapanış doğrulandı.
- Demo login/server allowlist'i sınırlandırıldı.

### IBKR Paper

- Gateway API: `127.0.0.1:4002`
- Bridge: `127.0.0.1:9101`
- Account allowlist aktif ve yanlış/boş eşleşmede fail-closed.
- XLE paper akışı başarıyla tamamlandı:

```text
BUY 1 XLE → OCA SL/TP → reduce-only SELL → cancel_all → reconciliation
```

- BUY fill: `1 @ 61.27`
- Reduce-only SELL fill: `1 @ 61.16`
- Son pozisyon: `0`
- Son açık emir: `0`

### BIST

- Gerçek execution bridge'i kapsam dışı bırakıldı.
- Günlük paper simulator ve `as_of` idempotency doğrulandı.
- Bu nedenle BIST kararı yalnızca paper simülasyon kapsamındadır.

## Açık konular ve canlıya geçiş engelleri

## Otomatik seans kapısı

- Yeni BUY/SELL girişleri merkezi venue/sembol seans kapısından geçer.
- Seans kapalıysa mevcut pozisyon zorla kapatılmaz; koruma ve çıkış yolları çalışmaya devam eder.
- Binance 7/24 kabul edilir; bağlantı/veri tazeliği ve hesap mutabakatı yine ayrı kapılardır.
- MT5 FX/CFD için genel 24/5 pencere kullanılır; broker sembol seansları kesinleşene kadar belirsizlik fail-closed kabul edilir.
- IBKR varsayılanı New York saat diliminde NYSE takvimidir: DST, hafta sonu, resmi tatil ve erken kapanışlar uygulanır.
- IBKR FX/CFD profili gerektiğinde `IBKR_SESSION_PROFILE=fx` veya `cfd` ile seçilebilir.
- BIST execution yoktur; sinyal-only davranışı korunur.
- Bilinmeyen venue veya seans profili giriş açmaz.
- Gerçek canlı girişte beklenen broker-side SL/TP emirlerinden biri kurulamazsa
  pozisyon artık bot-tarafı fallback'e bırakılmaz; koruma emirleri iptal edilir,
  reduce-only kapanış denenir, canlı girişler bloklanır ve kritik alarm üretilir.
- Candle freshness kapısına ek olarak canlı girişte fiyatın son başarılı poll
  zamanı da doğrulanır. Varsayılan `LIVE_PRICE_MAX_AGE_SECS=30` aşılırsa veya
  canlı fiyat timestamp'i yoksa yeni giriş açılmaz.
- Canlı entry sonucu `Filled` veya `PartiallyFilled` değilse pozisyon local
  state'e başarı gibi yazılmaz; `Unknown/New/Canceled/Rejected` sonuçlarında
  otomatik retry yapılmaz, işlem belirsiz kabul edilip canlı girişler bloke edilir.
- IBKR bridge `health` komutu artık bağlantı ile birlikte market-data,
  historical ve security-definition farm durumlarını ayrı raporlar.
- `2103/2105/2157` ilgili farm'ı sağlıksız; `2104/2106/2155` tekrar sağlıklı
  olarak işaretler. Bu durum artık Rust execution gate'ine bağlıdır; bağlantı
  veya gerekli farm'lardan biri sağlıksızsa yeni IBKR girişi engellenir.
  Mevcut profil hâlâ paper/read-only'dir.
- `profiles/paper-ibkr.conf` artık `TRADING_MODE=Paper`, `LIVE_DRY_RUN=true`,
  `IBKR_READ_ONLY=1` ve `IBKR_EXECUTION_ENABLED=0` ile güvenli varsayılandır.

1. Canlı Binance, MT5 ve IBKR hesapları için açık kullanıcı onayı ve ayrı canlı profil gerekir.
2. Canlı sembol/hesap allowlist'leri paper/demo değerlerinden bağımsız doğrulanmalıdır.
3. IBKR pozisyon sorgusunda görülen `entryPrice` ile fill fiyatı arasındaki fark canlı öncesi
   reconciliation raporunda açıklanmalıdır; paper akışının kapanış ve cleanup sonucu düzgündü.
4. Watchdog crash-loop duraklatma ve alarm/operatör müdahalesi ayrıca kontrollü olarak
   gözden geçirilmelidir.
5. BIST gerçek execution kararı ayrıca verilmedikçe paper-only kalmalıdır.
6. Download adayları yalnızca aktif registry'de kayıtlı venue ile eşleşen sembollerden
   oluşturulmalıdır; aksi halde ortak geçmişten kalan başka venue sembolleri pipeline'ı
   gereksiz yere `Failed` gösterebilir. Bu filtre IBKR paper profilinde doğrulandı.

## Sonuç

Sistem paper/demo/testnet doğrulama aşamasında operasyonel olarak **GO** durumundadır.
Canlı para ile işlem açma açısından genel karar **NO-GO** olarak korunmalıdır.

### Test sonrası runtime notu

İlk temiz olmayan restart sonrasında eski profilden kalan
`AUTO_BINANCE_SYMBOLS=BTCUSDT@binance` değeri, yalnızca IBKR venue registry'si aktifken
`ApiError` üretmişti. Watchdog ve engine temiz ortamla yeniden başlatıldıktan sonra bu
anomaly yaklaşık 70 saniye içinde `0` oldu.

Tekrarını önlemek için [lib_launchconf.sh](./scripts/lib_launchconf.sh) güncellendi:
`.launch.conf` içindeki boş değerler artık önceki profil ortam değişkenlerini temizliyor.
Son doğrulama: `open_positions=0`, `anomalies=0`; engine hâlâ paper gözlem modunda.

## Kalan işler ve kabul kapıları

### Paper/testnet aşamasında tamamlananlar

- [x] Binance Futures Testnet entry/protection/reduce-only close/cleanup
- [x] MT5 demo entry/broker-side protection/reduce-only close
- [x] IBKR paper XLE entry/OCA protection/reduce-only close/reconciliation
- [x] BIST günlük paper simulator ve `as_of` idempotency
- [x] Risk, R/R, HTF/ATR, trend-only pyramiding ve protection kapıları
- [x] Watchdog engine-down recovery
- [x] Risk halt/safe-mode enforcement testleri
- [x] Telegram alarm teslimi ve gönderim hatası retry davranışı
- [x] Profile environment leakage düzeltmesi
- [x] IBKR equity reporting ve `AccountStateUnavailable` anomaly düzeltmesi
- [x] Binance Testnet ↔ IBKR Paper profile isolation geçişi
- [x] Watchdog maintenance pause ve otomatik recovery akışı
- [x] Watchdog crash-loop koruması: 30 saniyede 3 başarısız restart sonrası pause
- [x] `memos up` kurulu systemd headless/TUI hedefiyle hizalandı
- [x] Download adaylarında aktif venue dışına varsayılan route sızıntısı engellendi;
  IBKR profili `1 başarılı / 0 başarısız` olarak doğrulandı

### Sonraki kontrollü recovery testleri

- [x] Belirsiz entry sonucu (`New`/`Unknown`/`Canceled`/`Rejected`) SQLite
  `uncertain_orders` tablosuna profil bazında mühürleniyor.
- [x] Restart recovery sırasında çözülmemiş belirsiz emirler tekrar okunuyor;
  canlı giriş kapısı reconciliation/operator çözümü gelene kadar fail-closed kalıyor.
- [x] Mevcut broker position reconciliation sonucu aynı sembolde pozisyonu
  doğrularsa uncertain kayıt profil bazında `RESOLVED` olarak mühürleniyor ve
  memory gate temizleniyor.
- [x] Açık emir reconciliation artık her kayıtlı venue adapter'ını ayrı ayrı
  sorguluyor; herhangi bir venue sorgusu başarısızsa uncertain çözümleme atlanıyor.
- [x] Broker'da açık emir yokluğu en az iki ardışık sağlıklı reconciliation
  döngüsünde doğrulanmadan uncertain kayıt çözülmüyor; sayaç DB'ye persist ediliyor.
- [x] Fake-venue E2E recovery fixture'i restart, açık emir, venue timeout,
  iki sağlıklı cycle ve broker pozisyonu ile çözümleme senaryolarını doğruluyor.
- [x] Fake adapter, gerçek `MarketData`/`OrderExecution`/`VenueAdapter`
  sözleşmelerini uygulayarak `fetch_open_orders()` ve
  `fetch_account_positions()` çıktılarının typed venue kimliğiyle uyumunu doğruluyor.
- [x] Fake adapter production `VenueRegistry::from_factory` üzerinden kaydedilip
  aynı registry lookup yüzeyinden okunabiliyor.
- [x] Fake registry, production `Engine` reconciliation yollarından geçirilerek
  açık emir, healthy sorgu ve hesap pozisyonu çıktılarının gerçek state'e işlendiği
  integration testiyle doğrulanıyor.
- [x] Venue readiness kararları merkezi `READY / DEGRADED / BLOCKED / UNKNOWN /
  SIGNAL_ONLY` modeliyle üretiliyor; mevcut raw health alanları korunuyor.
- [x] IBKR historical bridge istekleri bounded timeout, ardışık hata eşiği ve
  geçici circuit breaker ile izole ediliyor; başarılı istek hata sayacını sıfırlıyor.
- [x] IBKR, Binance Testnet ve MT5 paper profillerini ayrı DB/PID/heartbeat/log
  alanlarında çalıştıran güvenli soak matrisi hazırlandı:
  `scripts/soak_matrix.sh start|stop|status`.
- [x] BIST, otonom headless runtime içine ayrı `paper-bist-signal` profiliyle
  dahil edildi; yalnız `SIGNAL_ONLY`/data-only çalışır, ayrı DB ve metrics portu
  (`9114`) kullanır.
- [ ] Broker açık emirleri ve pozisyonlarıyla belirsiz kayıtların otomatik,
  güvenli çözümlemesi; bu tamamlanana kadar kayıtlar otomatik silinmeyecek.

- [ ] Açık pozisyon varken engine restart: broker pozisyonu yeniden yüklenmeli,
  protection doğrulanmalı ve reconciliation `MATCHED` olmadan yeni giriş açılmamalı.
- [ ] Pending close varken restart: aynı kapanış emri ikinci kez gönderilmemeli.
- [ ] Protection emri varken restart: broker-side SL/TP yeniden keşfedilmeli;
  eksik protection varsa yeni giriş kapanmalı ve kritik alarm üretilmeli.
- [ ] Network/bridge timeout sonrası restart: belirsiz entry sonucu retry edilmemeli;
  broker pozisyonu ve açık emirler doğrulanmadan işlem sürdürülmemeli.
- [ ] Broker'da lokal state'te olmayan pozisyon: güvenli adopt ve protection kontrolü.
- [ ] Lokal state'te olup broker'da olmayan pozisyon: stale silme yerine kapanış/
  reconciliation kaydı ve alarm.

### Paper gözleminde izlenecekler

- [x] Engine'in `phase=Recovering` durumunun nedeni açıklanıp `Scanning` geçişi
  doğrulandı
- [x] Report-only trend-following/pyramiding karşılaştırma runner'ı eklendi ve
  `XLE@ibkr` için ilk geçmiş veri raporu üretildi
- [ ] Uzun bir gözlem penceresinde heartbeat sürekliliğini ve `anomalies=0` durumunu
  korumak
- [x] Binance Testnet ↔ IBKR Paper profile geçişini kontrollü olarak doğrulamak
- [x] Watchdog crash-loop duraklatma politikasını ayrıca doğrulamak
- [x] `.watchdog.pause` bakım duraklatma ve otomatik recovery akışını doğrulamak
- [x] `memos up` ile kurulu systemd watchdog hedefinin headless/TUI davranışını
  netleştirmek

### İlk report-only trend/pyramiding sonucu

`XLE@ibkr`, `SUPERTREND`, aynı çıkış/risk modeliyle:

- `15m` / 2448 mum: tek giriş `-407.13`, pyramiding `-230.29`; fark `+176.83`
  USD, max DD değişimi `-1.97` puan
- `1h` / 1072 mum: tek giriş `-371.62`, pyramiding `-198.22`; fark `+173.40`
  USD, max DD değişimi `-1.91` puan
- `4h` / 1847 mum: tek giriş `+1005.62`, pyramiding `+662.58`; fark `-343.04`
  USD, max DD değişimi `-3.54` puan

Sizing, venue fiyatından bağımsız olarak yaklaşık `%1` sermaye riski hedefleyecek
şekilde ATR/SL tabanlıdır; sabit BTC miktarı kullanılmaz.

İlk Binance karşılaştırması (`BTCUSDT@binance`, SUPERTREND):

- `1h` / 2511 mum: tek giriş `+475.86`, pyramiding `+271.92`; max DD
  `%7.59` → `%4.34`
- `4h` / 1378 mum: tek giriş `-2465.45`, pyramiding `-1408.83`; max DD
  `%26.03` → `%14.99`

İlk MT5 karşılaştırması (`EURUSD@mt5`, SUPERTREND):

- `1h` / 3202 mum: tek giriş `-631.08`, pyramiding `-360.62`; max DD
  `%7.46` → `%4.27`
- `4h` / 1263 mum: tek giriş `-474.93`, pyramiding `-271.39`; max DD
  `%4.75` → `%2.71`

### Strateji matrisi ilk taraması

Aynı sizing ve çıkış modeliyle temsilî timeframe'lerde:

- IBKR `XLE@ibkr / 4h`: `SUPERTREND` tek giriş `+1005.62`, `EMA_CROSSOVER`
  `+478.70`, `DONCHIAN` `-645.64` USD.
- Binance `BTCUSDT@binance / 1h`: `DONCHIAN` tek giriş `+732.85`,
  `SUPERTREND` `+475.86`, `EMA_CROSSOVER` `-162.98` USD.
- MT5 `EURUSD@mt5 / 1h`: üç strateji de negatif; en düşük kayıp `DONCHIAN`
  `-169.24`, ardından `EMA_CROSSOVER` `-191.80`, `SUPERTREND` `-631.08` USD.

Bu sıralama henüz OOS/WF kabulü değildir; yalnızca aday strateji havuzunu daraltan
ilk teşhis taramasıdır. Pyramiding her matriste drawdown'u azaltmış, fakat pozitif
getiriyi de düşürebilmiştir.

### İlk IS/OOS holdout doğrulaması

Report-only holdout `%70 IS / %30 OOS` olarak çalıştırıldı:

- IBKR `XLE@ibkr / 4h / SUPERTREND`: OOS yalnızca 3 işlem; tek giriş
  `-229.30 USD`, pyramiding `-128.69 USD`. Risk azaldı, fakat pozitif edge
  doğrulanmadı.
- Binance `BTCUSDT@binance / 1h / DONCHIAN`: OOS 9 işlem; tek giriş
  `+721.03 USD`, pyramiding `+412.02 USD`, profit factor `3.59`. Şimdilik
  en güçlü aday; örneklem hâlâ küçük.
- MT5 `EURUSD@mt5 / 1h / DONCHIAN`: OOS yalnızca 3 işlem; tek giriş
  `-61.65 USD`, pyramiding `-35.23 USD`. Edge doğrulanmadı.

Sonuçlar karar için henüz yeterli işlem sayısına sahip değildir. Binance adayı
ek sembol ve farklı OOS pencereleriyle doğrulanmadan paper stratejisi olarak
otomatik etkinleştirilmeyecektir.

### Nested Walk-Forward doğrulaması

[nested_walk_forward_report.rs](./memos_trading_core/examples/nested_walk_forward_report.rs)
ile her kayan pencerede strateji, TP, SL ve pyramiding yalnızca IS bölümünde
seçildi; seçilen kombinasyon OOS bölümünde kilitli çalıştırıldı. Bu çalışma
report-only'dir ve aktif engine/paper ayarlarını değiştirmez.

| Venue | Pencereler | OOS işlem | Pozitif pencere | Toplam OOS PnL | Değerlendirme |
|---|---:|---:|---:|---:|---|
| Binance `BTCUSDT / 1h` | 13 | 29 | `%23.1` | `-105.70 USD` | Edge doğrulanmadı |
| IBKR `XLE / 4h` | 15 | 26 | `%20.0` | `+160.23 USD` | Tek büyük işlemin etkisi, tutarsız |
| MT5 `EURUSD / 1h` | 18 | 18 | `%0.0` | `-214.82 USD` | Edge doğrulanmadı |

Sonuçlar, önceki Binance tek holdout pozitifliğinin rolling nested seçimde
tekrarlanmadığını gösteriyor. IBKR toplamı da çoğunlukla tek büyük OOS işlemine
dayanıyor; pencere tutarlılığı kabul eşiğinin altında. Bu nedenle üç venue için
de paper stratejisi otomatik olarak değiştirilmedi ve canlı geçiş kararı
**NO-GO** olarak kaldı.

### Sembol evreni nested taraması

[nested_walk_forward_universe_report.rs](./memos_trading_core/examples/nested_walk_forward_universe_report.rs)
ile Binance futures `1h` üzerinde temsili 10 sembol ayrı ayrı test edildi.
Bu sonuçlar ortak portföy PnL'i değildir; her sembolün kendi backtest hesabında
ölçülen toplam OOS değerleridir.

| Sembol | OOS işlem | OOS PnL | Pozitif pencere | Sınıf |
|---|---:|---:|---:|---|
| `LINKUSDT` | 15 | `+633.69` | `%36.4` | Insufficient data |
| `BNBUSDT` | 21 | `+153.22` | `%27.3` | Insufficient data |
| `DOGEUSDT` | 10 | `-20.29` | `%25.0` | Insufficient data |
| `BTCUSDT` | 29 | `-105.70` | `%23.1` | Insufficient data |
| `ETHUSDT` | 46 | `-495.85` | `%30.8` | Rejected |
| `SOLUSDT` | 33 | `-839.00` | `%16.7` | Rejected |
| `ADAUSDT` | 52 | `-1213.06` | `%18.2` | Rejected |
| `DOTUSDT` | 67 | `-1361.02` | `%25.0` | Rejected |
| `XRPUSDT` | 45 | `-2029.95` | `%27.3` | Rejected |
| `AVAXUSDT` | 8 | `-381.63` | `%16.7` | Insufficient data |

Bu ilk tarama tek sembolde görülen pozitifliğin genellenebilir olmadığını
gösteriyor. `LINKUSDT` ve `BNBUSDT` pozitif görünse de işlem/pencere sayısı
kabul eşiğine ulaşmadığı için paper adayı değildir. Daha fazla OOS işlemi olan
sembollerde mevcut strateji ailesi çoğunlukla reddedildi. Çalışma report-only'dir;
aktif paper profili değiştirilmemiştir.

### Diğer venue evrenleri

Aynı runner ile IBKR `4h` ve MT5 `1h` sepetleri de tarandı:

| Venue | Test edilen sembol | OOS işlem | OOS PnL | Sonuç |
|---|---|---:|---:|---|
| IBKR | `XLE` | 26 | `+160.23` | Yetersiz ve %20 pozitif pencere |
| IBKR | `TLT` | 7 | `-127.09` | Yetersiz veri |
| IBKR | `XLF` | 21 | `-657.59` | Yetersiz ve %0 pozitif pencere |
| MT5 | `USDJPY` | 21 | `-142.27` | Yetersiz ve %0 pozitif pencere |
| MT5 | `EURUSD` | 18 | `-214.82` | Yetersiz ve %0 pozitif pencere |
| MT5 | `AUDUSD` | 20 | `-290.05` | Yetersiz ve %5.6 pozitif pencere |
| MT5 | `GBPUSD` | 26 | `-391.60` | Yetersiz ve %0 pozitif pencere |
| MT5 | `XAUUSD` | 29 | `-404.03` | Yetersiz ve %17.6 pozitif pencere |
| MT5 | `USDCHF` | 36 | `-445.35` | Reddedildi |

IBKR veri havuzunda istenen sekiz sembolden yalnızca üçü yeterli veriyle
çalıştırılabildi; bu nedenle IBKR sonucu tam evren kararı değildir. Buna rağmen
mevcut örneklemde hiçbir venue için birden fazla sembolde istikrarlı,
pozitif ve yeterli işlem sayısına sahip strateji ailesi bulunamadı.

### Fırsat teşhisi: MFE/MAE

[trade_opportunity_report.rs](./memos_trading_core/examples/trade_opportunity_report.rs)
ile mevcut çıkış modeline dokunmadan işlemlerin işlem süresince gördüğü MFE
(maksimum lehte hareket) ve MAE (maksimum aleyhte hareket) ölçüldü. Bu ilk
rapor long-only akış içindir:

| Seri | İşlem | Pozitif | Ort. MFE | Ort. MAE | Ort. çıkış yakalama |
|---|---:|---:|---:|---:|---:|
| BTCUSDT / DONCHIAN / 1h | 29 | 9 | `%2.52` | `%1.25` | `-%22.20` |
| XLE / SUPERTREND / 4h | 75 | 32 | `%2.85` | `%1.61` | `-%15.67` |
| EURUSD / DONCHIAN / 1h | 8 | 2 | `%0.58` | `%0.48` | `-%14.55` |

BTC raporunda pozitif sonuç birkaç kümelenmiş trend işlemi tarafından taşınıyor;
bu, önceki OOS pozitifliğinin neden rolling testte kalıcı olmadığını açıklayan
önemli bir yanlış-pozitif sinyalidir. XLE'de fırsat hareketi mevcut görünmesine
rağmen çıkış yakalama zayıf; EURUSD'de ise fırsat hareketi zaten sınırlı.
Bunlar henüz TP/SL değişikliği için yeterli kanıt değildir; sıradaki araştırma
çıkış modelini değil, işlemlerin pencere ve rejim bazında kümelenmesini
ölçmelidir.

### Raw signal audit

[raw_signal_audit.rs](./memos_trading_core/examples/raw_signal_audit.rs)
ile strateji katmanının HTF/rejim/edge kapılarından önce ürettiği ham aksiyon
sinyalleri, gerçekleşen işlemlerle karşılaştırıldı:

| Seri | Ham aksiyon | Gerçekleşen işlem | Ham/işlem oranı | Backtest PnL |
|---|---:|---:|---:|---:|
| BTCUSDT / DONCHIAN / 1h | 242 | 29 | `%12.0` | `+732.85` |
| XLE / SUPERTREND / 4h | 1828 | 75 | `%4.1` | `+1005.62` |
| EURUSD / DONCHIAN / 1h | 322 | 8 | `%2.5` | `-169.24` |

Ham/işlem oranı tek başına filtre reddi değildir; aynı yöndeki ardışık
sinyaller, açık pozisyon ve backtest'in long-only davranışı bu oranı etkiler.
Yine de XLE'de çok yoğun ham sinyal akışı, EURUSD'de ise az sayıda işlem ve
negatif sonuç görülüyor. Bu nedenle bir sonraki filtreleme çalışması
`signal_id` bazında ardışık sinyal kümelerini tek fırsat olarak gruplayıp,
HTF/rejim/edge kapılarının her birinin reddettiği fırsatı ayrı saymalıdır.

### Fırsat kümeleri

[opportunity_cluster_report.rs](./memos_trading_core/examples/opportunity_cluster_report.rs)
ile ardışık ham Buy sinyalleri, en fazla iki bar boşlukla tek fırsat kümesi
olarak birleştirildi. Her küme için 24 bar ileriye dönük, sabit `1.5% SL /
3.0% TP` referansıyla ölçüm yapıldı. Bu sonuçlar canlı ayar değildir; yalnızca
fırsatın yapısını teşhis eder.

| Seri | Ham Buy | Fırsat | 1R erişim | 2R erişim | STOP | TARGET | Nötr | Ort. MFE | Ort. MAE |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| BTCUSDT / DONCHIAN / 1h | 133 | 80 | `%45.0` | `%13.8` | `%27.5` | `%13.8` | `%58.8` | `%2.09` | `%1.35` |
| XLE / SUPERTREND / 4h | 1026 | 17 | `%52.9` | `%41.2` | `%58.8` | `%17.6` | `%23.5` | `%2.82` | `%5.78` |
| EURUSD / DONCHIAN / 1h | 132 | 92 | `%0.0` | `%0.0` | `%0.0` | `%0.0` | `%100.0` | `%0.27` | `%0.30` |

Bu teşhis daha net bir ayrım sağladı:

- BTC'de fırsat var, fakat çoğunluğu nötr; 2R'ye ulaşan kümeler az.
- XLE'de trend fırsatı mevcut, ancak MAE çok yüksek ve kümelerin çoğu SL'ye
  ulaşıyor. Sorun yalnızca sinyal değil; giriş gecikmesi ve risk/çıkış
  modelinin trend içi dalgalanmaya uyumsuzluğu muhtemel.
- EURUSD'de 24 bar içinde anlamlı 1R hareketi oluşmuyor. Parametre gevşetmek
  yerine bu sembol/timeframe'i işlem dışı bırakmak daha doğru.

### Long/short ve futures yönlü fırsat kontrolü

[opportunity_cluster_report.rs](./memos_trading_core/examples/opportunity_cluster_report.rs)
artık `OPPORTUNITY_DIRECTIONS=both` ile long ve short sinyalleri ayrı kümeler
olarak değerlendiriyor. Bu, özellikle Binance futures için iki yönlü fırsat
potansiyelini görmek içindir; mevcut paper engine'in long-only politikasını
değiştirmez ve komisyon/funding/short borrow maliyeti içermez.

| Seri | Long/Short küme | Fırsat | 1R | 2R | Stop | Target | Nötr | Ort. MFE | Ort. MAE |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| BTCUSDT / DONCHIAN / 1h | 80 / 63 | 143 | `%36.4` | `%9.8` | `%33.6` | `%9.8` | `%56.6` | `%1.66` | `%1.48` |
| XLE / SUPERTREND / 4h | 18 / 17 | 35 | `%45.7` | `%31.4` | `%62.9` | `%11.4` | `%25.7` | `%2.53` | `%9.91` |
| EURUSD / DONCHIAN / 1h | 92 / 115 | 207 | `%0.0` | `%0.0` | `%0.0` | `%0.0` | `%100.0` | `%0.24` | `%0.29` |

İki yönlü bakış sonucu kararı değiştirmedi. BTC'de short tarafı da 2R
fırsatlarını anlamlı biçimde artırmıyor. XLE'de iki yönlü fırsat görünse de
MAE aşırı yüksek ve stop oranı kabul edilemez. EURUSD'de iki yönde de anlamlı
hareket yok. Futures için gerçek karar vermeden önce aynı yönlü backtest'e
funding, short yönü komisyonu, slippage ve ortak equity curve eklenmelidir.

### Gerçek yönlü backtest ve Binance funding düzeltmesi

[futures_direction_report.rs](./memos_trading_core/examples/futures_direction_report.rs)
ile aynı backtest konfigürasyonu `LONG_ONLY` ve `BOTH_DIRECTIONS` olarak
çalıştırıldı. Binance futures funding kayıtları trade giriş-çıkış aralığında
millisecond timestamp ile eşleştirilip long için maliyet, short için gelir
olarak net PnL'e uygulandı.

| Seri | Mod | İşlem | Funding | Net PnL | PF | Max DD |
|---|---|---:|---:|---:|---:|---:|
| BTCUSDT / DONCHIAN / 1h | Long-only | 53 | `-19.36` | `-508.91` | `0.83` | `%10.17` |
| BTCUSDT / DONCHIAN / 1h | Both-directions | 95 | `-4.10` | `-1387.46` | `0.74` | `%17.47` |
| XLE / SUPERTREND / 4h | Long-only | 75 | `0.00` | `+1005.62` | `1.25` | `%8.25` |
| XLE / SUPERTREND / 4h | Both-directions | 102 | `0.00` | `+704.61` | `1.12` | `%11.87` |
| EURUSD / DONCHIAN / 1h | Long-only | 8 | `0.00` | `-169.24` | `0.25` | `%2.85` |
| EURUSD / DONCHIAN / 1h | Both-directions | 24 | `0.00` | `-591.37` | `0.14` | `%6.58` |

Güncel, funding kapsamı doğrulanmış BTC penceresinde hem long-only hem de iki
yönlü sonuç negatiftir. Bu nedenle önceki `+725.69 USD` sonucu geçerli karar
verisi olarak geri çekilmiştir; sonuç, eksik/yanlış tarih penceresi ve funding
kapsamı nedeniyle yeniden üretilememiştir. Short tarafı mevcut strateji ailesi
için kurtarıcı görünmemektedir.
XLE'de short eklemek PnL'i düşürüp drawdown'u artırdı. MT5'te iki yönlü mod kaybı
belirgin biçimde büyüttü. Bu nedenle futures için short otomatik etkinleştirme
kararı **NO-GO** olarak kaldı.

### Binance majörleri: güncel funding-düzeltilmiş karşılaştırma

Veri kalite kapısından geçen altı Binance futures `1h` majörü aynı
DONCHIAN/TP/SL/risk konfigürasyonuyla yeniden çalıştırıldı. Sonuçlar
report-only'dir; sembol başına 5.000 candle penceresi kullanılmıştır.

| Sembol | Long-only işlem | Long-only net PnL | Long-only PF | Long-only Max DD | Both net PnL | Both PF |
|---|---:|---:|---:|---:|---:|---:|
| BTCUSDT | 53 | `-508.91` | `0.83` | `%10.17` | `-1387.46` | `0.74` |
| ETHUSDT | 55 | `-604.53` | `0.83` | `%10.65` | `-2015.28` | `0.66` |
| BNBUSDT | 67 | `-2065.26` | `0.48` | `%22.24` | `-3979.26` | `0.40` |
| SOLUSDT | 67 | `-1276.27` | `0.72` | `%18.63` | `-3662.74` | `0.58` |
| XRPUSDT | 58 | `+212.08` | `1.06` | `%7.55` | `-2388.26` | `0.69` |
| LINKUSDT | 72 | `-2530.13` | `0.52` | `%25.34` | `-4648.58` | `0.47` |

XRPUSDT sonucu yalnızca zayıf bir `research-only` sinyalidir: tek pencere,
komisyon/funding sonrası düşük pozitiflik ve PF `1.06` paper adaylığı için
yeterli değildir. Bu nedenle altı sembolün hiçbiri `PAPER_CANDIDATE` olarak
sınıflandırılmadı. Hem Binance majör sepeti hem de short/both-directions modu
şimdilik **NO-GO** olarak kaldı.

### Portfolio-level nested Walk-Forward

[portfolio_nested_walk_forward.rs](./memos_trading_core/examples/portfolio_nested_walk_forward.rs)
ile altı veri-kalite-kapılı Binance majörü tek ortak equity üzerinde, 30
çakışmayan OOS penceresinde test edildi. Her sembolün strateji/TP/SL seçimi
yalnızca kendi IS bölümünde yapıldı; OOS işlemleri kronolojik olarak birleştirildi,
funding uygulandı ve aynı anda en fazla üç açık pozisyon kabul edildi.

| Mod | Başlangıç equity | Bitiş equity | Net PnL | İşlem | Limit reddi | Kapanış DD | Global MTM DD |
|---|---:|---:|---:|---:|---:|---:|---:|
| Long-only | `10,000.00` | `8,597.80` | `-1,402.20` | 376 | 66 | `%14.04` | `%14.32` |
| Both-directions | `10,000.00` | `7,722.25` | `-2,277.75` | 457 | 85 | `%22.78` | `%22.78` |

Portföy bileşik doğrulaması da tek-sembol raporlarını doğruladı: Binance majör
sepetinde mevcut strateji ailesi için paper adaylığı oluşmadı; both-directions
modu drawdown'u belirgin biçimde artırdı. Runner report-only'dir. Ek olarak
MTM artık tüm OOS pencereleri boyunca taşınan tek global equity eğrisi üzerinden
hesaplanır; pencere-içi teşhis alanı da ayrıca korunur. Gerçek venue emir kuyruğu
ve bar-içi high/low mark-to-market henüz bu rapora dahil değildir.

### Veri kalite denetimi

### Canlı karar yolu sinyal audit'i

[live_path_signal_audit.rs](./memos_trading_core/examples/live_path_signal_audit.rs)
ile aynı canlı karar yolu harness'i kullanılarak altı Binance majörünün 5.000
adet `1h` candle penceresi denetlendi. Sonuç, mevcut edge/S/R kapılarının
pratikte hiçbir ham sinyali elemediğini gösterdi:

| Sembol | Ham sinyal | Edge reddi | S/R reddi | Kabul edilen giriş | Gerçekleşen işlem | PnL |
|---|---:|---:|---:|---:|---:|---:|
| BTCUSDT | 213 | 0 | 0 | 213 | 213 | `-2214.91` |
| ETHUSDT | 216 | 0 | 0 | 216 | 215 | `-2545.33` |
| BNBUSDT | 211 | 0 | 0 | 211 | 210 | `-1611.24` |
| SOLUSDT | 238 | 0 | 0 | 238 | 238 | `-986.87` |
| XRPUSDT | 226 | 0 | 0 | 226 | 225 | `-495.69` |
| LINKUSDT | 230 | 0 | 0 | 230 | 230 | `-3189.28` |

Bu nedenle kaybın ana nedeni mevcut edge veya S/R filtrelerinin aşırı
reddedici olması değildir; tersine ham sinyallerin neredeyse tamamı girişe
dönüşmektedir. Bir sonraki araştırma giriş filtrelerinin eşiklerini rastgele
artırmak değil, ham sinyal yönünün ileri getirisi ile stop/TP ve trailing
çıkışlarının hangi aşamada negatif beklenti ürettiğini ayırmaktır. Bu audit
report-only'dir ve aktif profilleri değiştirmemiştir.

### Ham sinyal ileri kalite denetimi

[signal_forward_quality_report.rs](./memos_trading_core/examples/signal_forward_quality_report.rs)
ile her ham sinyalin girişten sonraki `1/3/6/12` bar davranışı ölçüldü.
Altı bar görünümünde sonuçlar özellikle önemlidir:

| Sembol/yön | Sinyal | Ort. kapanış getirisi | Ort. MFE | Ort. MAE | 2R teması | Stop teması |
|---|---:|---:|---:|---:|---:|---:|
| BTC long | 214 | `%0.00` | `%0.91` | `-%0.82` | `%4.67` | `%13.08` |
| BTC short | 198 | `-%0.08` | `%0.92` | `-%0.87` | `%4.04` | `%16.16` |
| ETH long | 175 | `%0.21` | `%1.45` | `-%1.04` | `%12.57` | `%22.86` |
| SOL long | 220 | `%0.14` | `%1.56` | `-%1.29` | `%15.91` | `%29.09` |
| XRP long | 215 | `%0.06` | `%1.90` | `-%1.51` | `%16.28` | `%31.16` |
| LINK long | 239 | `-%0.08` | `%1.46` | `-%1.43` | `%11.72` | `%33.05` |

Ham sinyallerin ortalama ileri kapanış getirisi çoğunlukla sıfıra yakınken
MAE, MFE'den yüksek veya ona çok yakındır. Bu, mevcut negatif sonucun yalnızca
exit modelinden kaynaklanmadığını; sinyal girişlerinin baştan güçlü bir
asimetrik avantaj üretmediğini gösterir. 2R'ye ulaşma oranı düşük, stop teması
özellikle XRP/LINK/SOL long tarafında yüksektir. Rapor:
[signal_forward_quality_binance_majors.json](./logs/paper-ibkr/signal_forward_quality_binance_majors.json).

[data_quality_report.rs](./memos_trading_core/examples/data_quality_report.rs)
ile mevcut SQLite veri tabanı için 909 sembol/timeframe serisi denetlendi:

- OHLC tutarsızlığı: `0`
- Duplicate timestamp: `0`
- Binance futures `1h` serisi: 256 sembol; her temsilî majör seride iç gap `0`
- Binance futures funding kapsamı: temsilî 6 majör `1h` serisinde coverage geçer
- IBKR `4h`: 15 seri, yalnızca 9'u takvimsel olarak kesintisiz
- MT5 `1h`: 13 seri, yalnızca 2'si takvimsel olarak kesintisiz

Binance backfill sırasında BTCUSDT'te `2025-04-28 21:00 UTC` ile
`2026-06-18 22:00 UTC` arasındaki `9.985` saatlik iç candle boşluğu tespit edilip
dolduruldu. Altı majör `1h` serisinde artık duplicate, OHLC hatası veya internal
gap yoktur; funding kayıtları `2024-10-02`–`2026-10-02` aralığını kapsar.
Son tamamlanmamış 8 saatlik funding periyodu için kalite ve runner guard'larında
tek candle aralığı toleransı kullanılır. Eski `futures_direction_*` çıktıları
bu backfill öncesi üretildiği için geçersizdir; güncel BTC çıktısı negatiftir.

IBKR/MT5 takvimsel gap'leri büyük ölçüde seans/hafta sonu kaynaklı olabilir;
henüz session-aware doğrulama yapılmadığından bu seriler otomatik olarak
“temiz” kabul edilmemelidir. Bu kalite raporu report-only'dir; veri tabanı,
paper engine ve canlı ayarlar değiştirilmemiştir.

Session-aware ilk teşhis [data_quality_report_session_aware.json](./logs/paper-ibkr/data_quality_report_session_aware.json)
ile üretildi. Rapor, her serinin gözlenen hafta-günü/saat/dakika slotlarını
öğrenerek yalnızca bu slotlara düşen beklenen barları ayrıca sayar; bu bir
borsa takvimi değil, takvimsel gap'leri azaltan konservatif bir teşhistir.
Toplam sonuçlar:

| Venue | Seri | Takvimsel eksik bar | Gözlenen seans slotlarına göre eksik |
|---|---:|---:|---:|
| IBKR | 87 | `231,016` | `76,738` |
| MT5 | 71 | `191,216` | `22,510` |

Bu düzeltme IBKR/MT5 boşluklarının önemli bölümünün seans/hafta sonu kaynaklı
olabileceğini destekliyor; ancak öğrenilmiş slot yaklaşımı tam exchange
takvimi yerine geçmez. Gerçek veri kaybını kesinleştirmek için sembol ve
venue bazlı resmi trading-hours/session takvimi hâlâ sonraki doğrulama kapısıdır.

Takvim kuralı henüz otomatik olarak genellenmedi; çünkü katalog tek bir varlık
ailesinden oluşmuyor. IBKR tarafında ETF/equity (`XLE`, `XLF`, `XLV`, `TLT`,
`DBA`) ile FX/CFD sembolleri, MT5 tarafında ise FX ve metal CFD'leri birlikte
bulunuyor. Bu nedenle tüm IBKR veya tüm MT5 serilerine tek bir UTC seansı
uygulamak yanlış pozitif gap üretebilir. Bir sonraki veri kalite adımı,
sembol ailesi + broker timezone bilgisi doğrulanmış küçük takvim profilleri
eklemek olmalıdır; metadata doğrulanmadan seri “temiz” ilan edilmeyecektir.

İlk IBKR örneklemesinde ayrıca bir cadence uyarısı görüldü: `XLE` için `15m`
olarak etiketlenen ilk kayıtlar saat başı timestamp aralığı gösteriyor. Bu,
seans takvimi eklenmeden önce interval etiketinin ve bridge/download dönüşümünün
ayrıca doğrulanmasını gerektirir. Timestamp cadence'i kesinleşmeden ABD seansı
kuralı eklemek, gerçek eksikleri gizleyebilir; bu nedenle equity takvimi şimdilik
uygulanmadı.

Cadence ölçümü rapora eklendikten sonra 909 serinin dağılımında `15` seri
`CADENCE_MISMATCH` olarak ayrıldı. IBKR'de `14` seri etkileniyor: özellikle
`XLE`, `XLF`, `XLU`, `XLV`, `TLT` ve `DBA` için `4h`/`1d` etiketli serilerde
modal gözlenen aralık `15m` veya `4h` ile uyuşmuyor. MT5'te yalnızca `XAGUSD/5m`
serisi modal cadence açısından uyuşmaz görünüyor. Bu sonuç, seans takvimi
uygulamadan önce candle downloader/bridge'in interval parametresinin ve mevcut
DB namespace'inin temizlenmesi gerektiğini gösteriyor. `CADENCE_MISMATCH`
serileri strateji/OOS kanıtına dahil edilmemeli; rapor yalnızca teşhis amaçlıdır.

IBKR bridge'in mevcut timeframe eşlemesi incelendi ve `1m`–`1d` aralığı için
IBKR `barSizeSetting` değerlerinin doğru gönderildiği doğrulandı. Sorunun
mevcut bridge mapping'inden çok, daha önce yazılmış bazı serilerin yanlış
interval etiketiyle aynı namespace'te kalması olduğu değerlendirildi.
Gelecekteki IBKR candle yanıtları için [protocol.rs](./memos_market_ibkr/src/protocol.rs)
içine cadence guard eklendi: dönen barların modal timestamp aralığı istenen
interval ile uyuşmazsa veri DB'ye yazılmadan açık hata döner. Eski kirli kayıtlar
otomatik silinmedi; yeniden backfill/rebuild kararı verilene kadar kanıt dışıdır.

Aynı koruma MT5 parser'ına da eklendi:
[memos_market_mt5/src/protocol.rs](./memos_market_mt5/src/protocol.rs) artık
gelen candle dizisinin modal cadence'ini istenen interval ile karşılaştırıyor;
uyuşmazlık varsa veri DB yazma aşamasına ulaşmadan açık hata dönüyor. Bu
değişiklik mevcut MT5/IBKR kayıtlarını silmez, taşımaz veya yeniden yazmaz.

Ek olarak [reader.rs](./memos_trading_core/src/persistence/reader.rs) içindeki
venue/profile okuma yolu IBKR ve MT5 serilerinde cadence kontrolünü tekrar
uyguluyor. Böylece eski yanlış etiketli seri backtest veya karar yoluna sessizce
giremez; okuyucu açık `Database` hatası döndürür. Binance ve diğer venue'ların
mevcut okuma davranışı değiştirilmedi. Bu kapı yalnız interval cadence'ini
kontrol eder; seans/hafta sonu kaynaklı normal boşlukları otomatik veri kaybı
saymaz. Eski kayıtların silinmemesi ve rapor dışı bırakılması korunuyor.

### Hedefli tarihsel veri hazırlığı

IBKR bridge hazır olduğunda `XLE`, `TLT` ve `DBA` için `1h` hedefli backfill
çalıştırıldı; her seri `1.342` candle'a ulaştı ve cadence `1h` olarak doğrulandı.
Serilerde kalan `48` seans-aware eksik bar nedeniyle henüz OOS'a alınmıyor.

MT5 EA probe'u yeniden başarılı oldu ve `EURUSD`, `GBPUSD`, `USDJPY`, `XAUUSD`
için ayrı `exchange=mt5, market=mt5` namespace'ine toplam `8.000` adet `1h`
candle yazıldı. Yeni MT5 serilerinin cadence'i doğru:

| Sembol | Yeni namespace barı | Cadence | Seans-aware eksik |
|---|---:|---:|---:|
| EURUSD | 2.000 | 1h | 0 |
| GBPUSD | 2.000 | 1h | 1 |
| USDJPY | 2.000 | 1h | 1 |
| XAUUSD | 2.000 | 1h | 11 |

Bu yeni seriler cadence açısından geçerli, ancak seans/takvim doğrulaması
tamamlanana kadar araştırma girdisi olarak yalnızca `DATA_READY`, paper adayı
olarak değil değerlendirilecek. Eski `market=cfd` serileri silinmedi veya
üzerine yazılmadı.

İlk nested OOS teşhisi aynı yeni `market=mt5` namespace'i üzerinde, sembol
başına 500 IS / 150 OOS / 150 adım ve 2.000 candle ile çalıştırıldı:

| Sembol | OOS işlem | OOS PnL | Kârlı pencere |
|---|---:|---:|---:|
| EURUSD | 11 | `-71.70` | `%0` |
| GBPUSD | 11 | `-97.21` | `%0` |
| USDJPY | 17 | `-104.73` | `%0` |
| XAUUSD | 19 | `-65.41` | `%30` |

Bu erken ve sınırlı gözlemde hiçbir MT5 sembolü pozitif veya paper adayı
değildir. Seans doğrulaması ve daha uzun tarihsel kapsam tamamlanmadan bu
sonuçlar nihai venue kararı olarak kullanılmayacaktır. Raporlar:
[MT5 OOS rapor klasöründe](./logs/paper-ibkr).

IBKR tarihsel veri kapsamını genişletmek için downloader'a `end_timestamp_ms`
tabanlı pagination eklendi. Küçük testte bridge yanıtı cadence açısından
geçerli kaldı; ancak XLE/TLT/DBA serileri hâlâ `1.342` candle ve `48`
session-aware boşluk ile sınırlı. Bu nedenle pagination'ın tarihsel kapsamı
gerçekten genişlettiği henüz kanıtlanmış değildir. Sayfa bazlı teşhis çıktısı
(`requested_end`, candle sayısı, oldest/newest ve overlap) eklendi. Yeniden
doğrulama için IB Gateway API (`127.0.0.1:4002`) ve bridge
(`127.0.0.1:9101`) birlikte erişilebilir olmalıdır; API kapalıyken downloader
çalıştırılmayacak ve seri OOS'a alınmayacaktır. Downloader artık mevcut
`ibkr/cfd/symbol/interval` serisinin en eski timestamp'ini anchor alıp daha
eski sayfaları ister; seri yoksa yalnızca o zaman güncel zamandan başlar.

Bu mekanizma ile XLE, TLT ve DBA için sekiz sayfalık derin backfill
overlap olmadan tamamlandı. Kapsam Ekim 2025'e kadar genişledi. Son kalite
raporu:

| Sembol | Candle | İlk tarih | Son tarih | Duplicate | OHLC hatası | Modal cadence | Session missing | Verdict |
|---|---:|---|---|---:|---:|---|---:|---|
| XLE | 4.549 | 2025-10-02 | 2026-10-02 | 0 | 0 | 1h | 427 | `GAPPY` |
| TLT | 4.024 | 2025-10-02 | 2026-10-02 | 0 | 0 | 1h | 427 | `GAPPY` |
| DBA | 4.024 | 2025-10-02 | 2026-10-02 | 0 | 0 | 1h | 427 | `GAPPY` |

Rapor: [deep IBKR backfill kalite raporu](./logs/paper-ibkr/data_quality_report_after_ibkr_deep_backfill.json).
Cadence ve OHLC bütünlüğü sağlanmış olsa da `GAPPY` sonucu nedeniyle
IBKR serileri henüz OOS veya paper adayı değildir. Session missing değeri
resmî tatil/seans takvimiyle ayrıştırılmadan veri kaybı olarak
değerlendirilmemiştir.

Ek timestamp incelemesinde üç seride de yeni backfill'in baskın profili
UTC `08:00-23:00` hafta içi saatleri olarak görülürken, eski kayıtlar içinde
UTC `00:00` saatinde `83` ve Cumartesi üzerinde `17` gözlem bulundu. Bu
nadiren görülen slotlar mevcut "gözlenen slotları öğren" hesabını
şişirebildiği için `session_missing=427` doğrudan gerçek veri kaybı olarak
yorumlanmayacaktır. Eski kayıtların timestamp/IBKR timezone kökeni ayrıca
incelenmeden bu satırlar silinmeyecek veya yeniden yazılmayacaktır.

Bu ayrımı ölçmek için [profiled IBKR kalite raporu](./logs/paper-ibkr/data_quality_report_profiled_ibkr.json)
üretildi. Baskın seans profili hafta içi 80 saatlik slot olarak ayrıştı:
DBA/TLT'de `83` satır, XLE'de `606` satır profil dışı kaldı. Bu sonuç,
özellikle XLE'de eski kayıtların farklı tarih/saat kapsamı taşıdığını
gösteriyor. DST-aware raporla DBA/TLT'nin alt-interval satırı `0`, XLE'nin
alt-interval satırı `688` bulundu. XLE bu nedenle artık yalnızca `GAPPY`
değil, açık `CADENCE_MISMATCH` olarak işaretleniyor; satırlar otomatik
düzeltilmedi ve bu namespace henüz OOS'a alınmadı.

TLT ve DBA için `session_missing=166` değeri gün bazında ayrıca ayrıştırıldı:
10 tam gün kapanışı (2025-11-27, 2025-12-25, 2026-01-01, 2026-01-19,
2026-02-16, 2026-04-03, 2026-05-25, 2026-06-19, 2026-07-03, 2026-09-07)
ve iki erken kapanış günü (2025-11-28, 2025-12-24) gözlendi. Son tarih olan
2026-10-02 de tamamlanmamış boundary olarak görünüyor. Bu desen normal NYSE
takvimiyle uyumludur; TLT/DBA için kalan boşluklar şu aşamada gerçek veri
kaybı sayılmıyor. XLE'nin alt-interval karışımı bu kararı değiştirmiyor.

NYSE takvim profili kalite raporuna eklendi. `IBKR_NEW_YORK_SESSION_SYMBOLS`
ile çalıştırılan son raporda TLT ve DBA için `calendar_missing=0`,
`sub_interval_rows=0` ve verdict `STRUCTURALLY_CLEAN` çıktı. XLE için
`calendar_missing=0` olsa da `sub_interval_rows=688` nedeniyle verdict
`CADENCE_MISMATCH` kaldı. Rapor:
[IBKR New York calendar kalite raporu](./logs/paper-ibkr/data_quality_report_ibkr_new_york_calendar.json).

Takvim kapısından geçen TLT ve DBA ile report-only nested OOS tekrarlandı
(IS 500 / OOS 150 / step 150; 23 pencere):

| Sembol | OOS işlem | OOS PnL | Kârlı pencere | En yüksek pencere DD |
|---|---:|---:|---:|---:|
| TLT | 32 | `-402.70` | `%4.35` | `%1.71` |
| DBA | 37 | `+1,077.34` | `%39.13` | `%2.69` |

DBA'nın toplamı pozitif olsa da kârlı pencere oranı düşük ve sonuç tek başına
istikrarlı edge kanıtı değil; TLT açıkça negatiftir. Bu nedenle iki sembol de
paper profile adayı yapılmadı. Raporlar:
[TLT nested OOS](./logs/paper-ibkr/nested_wf_ibkr_tlt_1h.json) ve
[DBA nested OOS](./logs/paper-ibkr/nested_wf_ibkr_dba_1h.json).

Backtester'ın MTM/drawdown yolu intrabar high/low ile daha muhafazakâr hale
getirildikten sonra DBA OOS tekrarlandı. Yeni sonuç `38` işlem, `+1,069.95`
PnL, `%34.78` kârlı pencere ve `%2.97` en yüksek pencere drawdown oldu.
Önceki kapanış-bazlı sonuç `37` işlem, `+1,077.34` PnL, `%39.13` kârlı
pencere ve `%2.69` drawdown idi. Risk ölçümü sıkılaştırılınca sonuç
zayıfladı; DBA yine paper adayı değildir. Rapor:
[DBA intrabar MTM nested OOS](./logs/paper-ibkr/nested_wf_ibkr_dba_1h_intrabar_mtm.json).

DBA için rolling OOS sağlamlık kontrolü iki farklı pencere yapısıyla da
çalıştırıldı. IS=750/OOS=150 düzeninde `44` işlem, `+353.40` PnL,
`%38.10` kârlı pencere ve `%2.68` en yüksek pencere DD; IS=500/OOS=300
düzeninde `70` işlem, `+905.81` PnL, `%40.91` kârlı pencere ve `%4.47`
en yüksek pencere DD görüldü. Pencere oranı yaklaşık `%38–41` bandında
kalsa da daha uzun OOS penceresinde drawdown arttı; bu sonuç istikrarlı
edge veya paper geçişi için yeterli değildir. Raporlar:
[DBA IS750 rolling OOS](./logs/paper-ibkr/nested_wf_ibkr_dba_1h_intrabar_is750.json)
ve [DBA OOS300 rolling OOS](./logs/paper-ibkr/nested_wf_ibkr_dba_1h_intrabar_oos300.json).

IBKR historical timeout teşhisi için bridge'e `errorEvent` ve structured
request logları eklendi. Kontrollü yeniden başlatma sırasında gerçek IBKR
mesajları alındı: `2103 usfarm`, `2105 ushmds` ve `2157 secdefil` data-farm
bağlantılarının koptuğunu bildiriyor. API socket handshake başarılı olsa da
market, HMDS ve security-definition farm'ları sağlıklı değildi; bu nedenle
historical candle isteği timeout oldu. Gateway/TWS durdurulmadı. Diagnostic
bridge, takılı historical çağrının headless akışını bloke etmemesi için
durduruldu; yeni historical backfill bu farm mesajları düzelmeden
çalıştırılmayacak.

### Strateji ailesi ileri kalite karşılaştırması

Aynı altı Binance majörü ve aynı 6-bar ileri kalite ölçümü
`DONCHIAN`, `SUPERTREND` ve `EMA_CROSSOVER` için tekrarlandı. `SUPERTREND`
altı sembolün tamamında küçük pozitif ortalama ileri kapanış getirisi gösterse
de çok sayıda tekrarlayan sinyal üretti (sembol başına yaklaşık `2.100–2.600`
long sinyali); bu ham sinyal sayısı gerçek bağımsız fırsat sayısı değildir.

| Aile | Gözlem | Karar |
|---|---|---|
| DONCHIAN | Daha seyrek; BTC/LINK negatif, ETH/SOL/XRP sınırlı pozitif | Araştırma |
| SUPERTREND | Tüm sembollerde küçük pozitif 6-bar ortalama; tekrarlı sinyal yoğunluğu çok yüksek | Fırsat kümelemesi gerekli |
| EMA_CROSSOVER | Karışık; BTC/BNB/SOL/XRP sınırlı, ETH/LINK daha iyi | Araştırma |

Bu nedenle SUPERTREND sonucu doğrudan paper adayı sayılmadı. Önce aynı yöndeki
ardışık SUPERTREND sinyalleri tek fırsat kümesine indirilmeli, sonra bağımsız
fırsat başına ileri kalite ve OOS portföy sonucu yeniden ölçülmelidir. Aile
karşılaştırma raporları [paper-ibkr rapor klasöründe](./logs/paper-ibkr) tutulur.

Kümeleme yapıldıktan sonraki altı sembol toplamı:

| Aile | Bağımsız fırsat | 1R teması | 2R teması | Stop | Nötr | Ort. MFE | Ort. MAE |
|---|---:|---:|---:|---:|---:|---:|---:|
| DONCHIAN | 1.613 | `%52.70` | `%25.73` | `%46.87` | `%31.74` | `%2.33` | `%2.29` |
| SUPERTREND | 706 | `%53.40` | `%24.36` | `%43.06` | `%35.84` | `%2.17` | `%2.21` |
| EMA_CROSSOVER | 1.809 | `%47.48` | `%19.90` | `%46.16` | `%37.15` | `%1.95` | `%2.00` |

SUPERTREND tekrarlayan sinyal sorununu azaltınca bağımsız fırsat sayısı
DONCHIAN'ın yarısından azına indi; ancak 2R oranı DONCHIAN'dan daha iyi
olmadı. DONCHIAN en yüksek 2R oranını üretse de stop oranı hâlâ yüksektir.
Bu üç aileden hiçbiri bu aşamada paper adayı değildir.

### Rejim filtresi + fırsat kümesi kontrollü testi

Rejim filtresi kümeleme sonrasında, kümenin ilk sinyalindeki rejime uygulandı;
aksi halde tekrar eden sinyaller yapay biçimde ayrı fırsatlara bölünüyordu.
`OPPORTUNITY_REGIME=trending` ile altı Binance majörü yeniden ölçüldü:

| Aile | Fırsat | Rejimde reddedilen ham sinyal | 1R | 2R | Stop | Ort. MFE | Ort. MAE |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| DONCHIAN | 1.299 | 472 | `%53.19` | `%27.10` | `%47.34` | `%2.39` | `%2.34` |
| SUPERTREND | 447 | 9.773 | `%55.26` | `%27.96` | `%40.94` | `%2.34` | `%2.19` |
| EMA_CROSSOVER | 611 | 1.198 | `%48.94` | `%20.62` | `%47.46` | `%1.98` | `%2.16` |

SUPERTREND rejim filtresiyle 2R oranını artırıp stop oranını düşürdü; bu,
araştırmaya devam etmeye değer ilk kontrollü iyileşmedir. Ancak `%40.94` stop
oranı hâlâ yüksektir ve bu yalnızca in-sample/diagnostic fırsat ölçümüdür.
SUPERTREND bu nedenle henüz `PAPER_CANDIDATE` değildir; bir sonraki zorunlu
adım aynı rejim kuralının nested OOS ve ortak equity portfolio testidir.

### TrendingOnly nested OOS ve ortak equity doğrulaması

`RegimeGate::TrendingOnly` backtest motoruna opt-in olarak eklendi. Mevcut
`Off`, `Absolute` ve `Adaptive` davranışları korunurken, her girişte son
lookback penceresinin `AdxRegime::Trending` olması zorunlu tutuldu. Portfolio
runner'da `PORTFOLIO_REGIME_GATE=trending` ile etkinleştirildi. Test aynı altı
Binance futures majörü, `1h`, 500 bar IS / 150 bar OOS / 150 bar adım, 30
çakışmayan pencere ve ortak 10.000 USD equity ile çalıştırıldı.

| Deney | Son equity | Net PnL | İşlem | Limit reddi | Global MTM DD |
|---|---:|---:|---:|---:|---:|
| Baseline, long-only | `8,597.80` | `-1,402.20` | `376` | `66` | `%14.32` |
| `TrendingOnly`, long-only | `8,579.80` | `-1,420.20` | `363` | `57` | `%14.81` |
| `TrendingOnly`, both-directions | `8,145.22` | `-1,854.78` | `409` | `70` | `%18.80` |

Rejim kapısı işlem sayısını ve limit reddini azalttı, ancak nested OOS
performansını iyileştirmedi; long-only sonuçta net PnL `18.01 USD` daha kötü,
global MTM drawdown `0.49` puan daha yüksek oldu. Both-directions sonucu daha
zayıf kaldı. Bu nedenle rejim filtresi teşhis seviyesinde yararlı olsa da
`PAPER_CANDIDATE` veya aktif profil adayı değildir. Raporlar:
[trending long](./logs/paper-ibkr/portfolio_nested_wf_binance_majors_trending_long.json),
[trending both](./logs/paper-ibkr/portfolio_nested_wf_binance_majors_trending_both.json).

### Tam canlı-yol karar zinciri audit'i

[live_path.rs](./memos_trading_core/src/robot/backtester/live_path.rs) içine
pozisyon meşgulken gelen sinyaller için ayrı `position_busy_rejected` sayacı
eklendi. Böylece ham sinyal, edge, S/R, pozisyon meşgul, kabul ve gerçekleşen
işlem ayrımı aynı raporda görülebiliyor. Altı Binance majörü üzerinde önceki
5.000 mumluk audit tekrarlandı:

| Aşama | Sayı |
|---|---:|
| Ham BUY/SELL sinyali | `2,325` |
| Edge reddi | `0` |
| S/R reddi | `0` |
| Pozisyon meşgul reddi | `991` |
| Kabul edilen giriş | `1,334` |
| Gerçekleşen işlem | `1,331` |

Bu bulgu önceki teşhisi netleştiriyor: filtrelerin çoğu sinyali reddetmiyor;
karar yolundaki esas azaltma tek-pozisyon meşgul kapısında gerçekleşiyor.
Aradaki üç giriş/işlem farkı, pozisyon boyutu veya veri sonu gibi execution
durumlarının ayrıca ayrıştırılması gereken küçük bir kalıntıdır. Rapor:
[live_path_signal_audit_binance_majors_busy.json](./logs/paper-ibkr/live_path_signal_audit_binance_majors_busy.json).

Bu üç fark için reason audit sayaçları eklendi. Yeni çalıştırmada toplam
`non_positive_qty_rejected=0`, `open_position_at_end=3` bulundu; farkın tamamı
ETHUSDT, BNBUSDT ve XRPUSDT'de rapor sonunda pozisyonun hâlâ açık olmasından
kaynaklanıyor. Bu bir emir gönderme reddi değil, gözlem penceresi sınırı.
Yeni rapor:
[reasoned live-path audit](./logs/paper-ibkr/live_path_signal_audit_binance_majors_reasoned.json).

### Canlıya geçiş öncesi zorunlu kapılar

- [ ] Canlı hesaplar için ayrı profile, hesap/server allowlist ve sembol allowlist
- [ ] Canlı API anahtarlarının yetki kapsamı ve para çekme yetkisinin kapalı olduğunun
  doğrulanması
- [ ] Venue bazında notional, pozisyon, günlük zarar ve emir hızı limitleri
- [ ] Son reconciliation: broker pozisyonu, local snapshot ve açık emirler
- [ ] Panic/close-all, Telegram alarmı ve watchdog recovery operatör runbook'u
- [ ] Canlı öncesi yıkıcı olmayan dress rehearsal ve açık pozisyon bırakmama kanıtı
- [ ] Kullanıcı tarafından açık ve venue-bazlı go kararı

### Kapsam dışında bırakılanlar

- [ ] BIST gerçek execution bridge'i: ayrıca onaylanmadıkça yapılmayacak
- [ ] Canlı IBKR/Binance/MT5 profili: hesap bilgileri ve açık onay olmadan hazırlanmayacak
