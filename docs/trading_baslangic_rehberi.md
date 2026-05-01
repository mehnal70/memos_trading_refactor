# Ticaret Dünyasına Giriş: Sıfırdan Anlama Rehberi

*Bu kitapçık, memos_trading sistemini ve genel ticaret kavramlarını hiç bilmeyenler için yazılmıştır.
Her terim, neden var olduğu ve ne işe yaradığıyla birlikte açıklanmıştır.*

---

## Birinci Bölüm: Piyasa Nedir, Nasıl Çalışır?

### Piyasa

Bir şeyi alıp satmak isteyenlerin buluştuğu yerdir. Pazarda domates alıp satmak gibi — ama burada alınıp satılan şey para birimleri, hisse senetleri veya kripto paralardır.

**Neden var?** Bir şeyin fiyatı alıcı ve satıcı arasındaki anlaşmayla oluşur. Piyasa, bu anlaşmaların sürekli ve hızlı şekilde yapılabildiği bir ortamdır.

---

### Fiyat: Bir Şeyin Değeri Nasıl Belirlenir?

Bir bitcoin'in fiyatı "gerçek" bir değer değildir. Şu anda birisinin almaya razı olduğu fiyattır. Çok insan almak isterse fiyat yükselir, çok insan satmak isterse düşer.

Bu yüzden fiyat her saniye değişir.

---

### Mum (Candle)

Bir mum, belirli bir zaman dilimindeki fiyat hareketini özetler.

```
     │  ← Yüksek (High): O sürede ulaşılan en yüksek fiyat
   ┌─┴─┐
   │   │ ← Gövde: Açılış ile kapanış arası
   └─┬─┘
     │  ← Düşük (Low): O sürede ulaşılan en düşük fiyat
```

**Dört temel değer:**

| Terim | İngilizce | Ne demek |
|-------|-----------|----------|
| Açılış | Open (O) | O zaman dilimi başında fiyat neydi |
| Kapanış | Close (C) | O zaman dilimi sonunda fiyat neydi |
| Yüksek | High (H) | O sürede en yüksek fiyat neydi |
| Düşük | Low (L) | O sürede en düşük fiyat neydi |

**Neden kullanılır?** Ham fiyat verisi saniyede yüzlerce kez değişir ve takip edilemez. Mum, "5 dakika boyunca neler oldu?" sorusunu tek bir şekle sıkıştırır.

**1m mum:** 1 dakikalık özet. **1h mum:** 1 saatlik özet. **1d mum:** 1 günlük özet.

---

### Hacim (Volume)

O zaman diliminde ne kadar alım-satım yapıldığı.

**Neden önemli?** Fiyat yükselse bile az kişi işlem yaptıysa bu hareket zayıf sayılır. Fiyat yükselirken hacim de yüksekse hareket güçlüdür, devam etmesi olasıdır.

---

## İkinci Bölüm: İki Temel İşlem Tipi

### LONG (Uzun Pozisyon) — "Fiyat yükselecek" bahsi

Bir şeyi ucuzken alıp pahalıyken satmak.

**Örnek:**
- Bitcoin 60.000 $'dan satın aldın.
- Bitcoin 65.000 $'a yükseldi.
- Sattın → 5.000 $ kazandın.

**Neden "long" denir?** Fiyatın uzun vadede yukarı gittiğine inandığını gösterir. Geleneksel yatırımın çoğu "long"dur — hisse alıp değer kazanmasını beklemek.

---

### SHORT (Kısa Pozisyon) — "Fiyat düşecek" bahsi

Bir şeyi pahalıyken (ödünç alıp) satmak, ucuzladıktan sonra geri almak.

**Örnek:**
- Bitcoin 65.000 $'dan "ödünç alıp" sattın.
- Bitcoin 60.000 $'a düştü.
- 60.000 $'dan geri aldın → 5.000 $ kazandın.

**Neden zor kavranır?** Sezgiyle çelişir: "Elimde olmayan bir şeyi nasıl satarım?" Borsa bunu teknik olarak mümkün kılar.

**Riski:** LONG'da en fazla yatırdığın kadar kaybedebilirsin (bitcoin sıfıra inerse). SHORT'ta teorik olarak sınırsız kayıp var — fiyat sonsuza çıkabilir.

---

### Spot vs Futures — İki Farklı Oyun Alanı

**Spot piyasası:** "Şimdi al, şimdi teslim al." Bir bitcoin alıyorsun, o bitcoin senin oluyor.

**Futures (Vadeli) piyasası:** "Gelecekte belirli bir fiyattan alım/satım yapacağız" sözleşmesi. Gerçek varlığı almıyorsun, sadece fiyat hareketine katılıyorsun. Kaldıraç kullanmak mümkün.

**Neden futures?** Yatırımın olmadan bile fiyat hareketinden para kazanabilirsin. Daha az sermayeyle daha büyük pozisyon alabilirsin (kaldıraç sayesinde).

---

## Üçüncü Bölüm: Kâr, Zarar ve Risk

### PnL (Profit and Loss) — Kâr/Zarar

Bir işlemden elde edilen net kazanç veya kayıp.

**Gerçekleşmemiş PnL (Unrealized):** Pozisyon hâlâ açıkken kâğıt üzerindeki kâr/zarar. Henüz "cebinde" değil.

**Gerçekleşmiş PnL (Realized):** Pozisyon kapandıktan sonra eline geçen gerçek kâr/zarar.

---

### Kaldıraç (Leverage) — Risk ve Kazancı Büyütme Aracı

"Elimde 1.000 $ var ama 10.000 $'lık işlem yapmak istiyorum" durumunu mümkün kılar.

**10x kaldıraç:** Borsanın geri kalan 9.000 $'ı sana ödünç verdiği anlamına gelir.

**Kazancı büyütür:** Fiyat %1 yükselirse kaldıraçsız 10 $ kazanırsın; 10x kaldıraçla 100 $ kazanırsın.

**Kaybı da büyütür:** Fiyat %1 düşerse kaldıraçsız 10 $ kaybedersin; 10x kaldıraçla 100 $ kaybedersin. Yani sermayenin %10'unu yitirirsin.

**×7.0** gibi ifadeler sistemde kaldıraç oranını gösterir.

---

### Marjin (Margin) — "Teminat"

Kaldıraçlı işlemde borsaya "garanti" olarak bıraktığın para.

**Formül:** `Marjin = Pozisyon Büyüklüğü / Kaldıraç`

Örneğin 1.000 $'lık pozisyon için 10x kaldıraçla 100 $ marjin yatırırsın.

**Neden önemli?** Marjin biterse borsa pozisyonunu zorla kapatır (likidasyon).

---

### Likidasyon (Liquidation) — Zorla Kapanış

Kaldıraçlı pozisyonun zarar etmesi ve marjinin erimesi durumunda borsa pozisyonu otomatik kapatır.

**Neden var?** Borsa sana ödünç para vermiştir. Sen o parayı kaybetmemesi için pozisyonu kapatır.

Sistemde `liq:` olarak gösterilen fiyat, "Bu fiyata ulaşırsa pozisyon otomatik kapanır" noktasıdır.

---

### Stop Loss (SL) — Otomatik Dur Noktası

"Fiyat bu noktaya düşerse zararı kabul et ve çık" emridir.

**Neden kullanılır?** Piyasa her zaman tahmin ettiğin yönde gitmez. SL olmadan küçük bir kayıp kontrolden çıkıp büyük bir felakete dönüşebilir.

**Örnek:** Bitcoin'i 60.000 $'dan aldın. SL'yi 58.500 $'a koydun. Bitcoin 58.500 $'a düşerse sistem otomatik satar, 1.500 $ zarar etmiş olursun — ama daha fazla kaybetmezsin.

---

### Take Profit (TP) — Otomatik Kâr Al

"Fiyat bu noktaya ulaşırsa kârı realize et ve çık" emridir.

**Neden kullanılır?** Piyasa senin yönünde gidebilir, sonra geri dönebilir. TP olmadan kazanç kaçabilir. "Açgözlülük" kayıpların önemli bir nedenidir.

---

### Trailing Stop Loss (TSL) — Takip Eden Dur

SL sabit değil, fiyat senin yönünde hareket ettikçe onu takip eder.

**Nasıl çalışır?**
- Bitcoin'i 60.000 $'dan aldın, TSL'yi %2 mesafeye koydun.
- Bitcoin 65.000 $'a çıktı → TSL 63.700 $'a yükseldi.
- Bitcoin 70.000 $'a çıktı → TSL 68.600 $'a yükseldi.
- Bitcoin 68.500 $'a düştü → TSL tetiklendi, 68.600 $'dan sattın.

**Avantajı:** Kâr elde ederken esneklik sağlar. Kâr otomatik korunur ama "yukarı gidebilir" kapısı açık kalır.

---

### Risk/Ödül Oranı (R/R — Risk/Reward Ratio)

"Bu işlemde ne kadar riske atıyorum, ne kadar kazanmayı bekliyorum?"

**Formül:** `R/R = Hedef Kâr / Riske Atılan Miktar`

**Örnek:** SL 500 $, TP 1.500 $ → R/R = 3.0 (3'e 1 oran)

**Neden önemli?** R/R = 3.0 ise, 3 işlemden sadece 1'ini kazansan bile para kaybetmezsin. Düşük R/R ile yüksek win rate'e ihtiyaç duyarsın; yüksek R/R ile daha az kazanma oranıyla bile kârlı olabilirsin.

Sistemde `min_rr: 1.5` gibi değerler "bu oranın altında işlem açma" kuralını ifade eder.

---

### Win Rate (Kazanma Oranı)

100 işlemden kaç tanesini kârlı kapattığın.

**Yanıltıcı olabilir:** %70 win rate'e sahip ama R/R = 0.3 olan bir sistem, her kazandığında 30 $ kazanır, her kaybettiğinde 100 $ kaybeder. Net sonuç: zarar.

Sistemdeki analizde görülen **%35 win rate** ile **R/R = 4.2** kombinasyonu bu nedenle kârlı olabilir: az kazanıyorsun ama kazandığında çok kazanıyorsun.

---

## Dördüncü Bölüm: Teknik Analiz Araçları

*Fiyat hareketlerindeki örüntüleri matematiksel olarak tespit etmeye çalışan hesaplamalar.*

---

### Moving Average (MA) — Hareketli Ortalama

Son N mumun kapanış fiyatlarının ortalaması.

**Neden kullanılır?** Anlık fiyat "gürültülüdür" — her saniye yukarı aşağı oynar. Hareketli ortalama bu gürültüyü düzleştirir ve genel yönü gösterir.

**MA(5):** Son 5 mumun ortalaması → kısa vadeli yön.
**MA(20):** Son 20 mumun ortalaması → orta vadeli yön.

**Altın Kesişim (Sinyal):**
- Kısa MA, uzun MA'yı yukarı keser → alım sinyali (piyasa yukarı dönüyor)
- Kısa MA, uzun MA'yı aşağı keser → satım sinyali (piyasa aşağı dönüyor)

---

### RSI (Relative Strength Index) — Göreceli Güç Endeksi

0-100 arasında bir değer. Fiyatın "aşırı alınmış" veya "aşırı satılmış" olup olmadığını ölçer.

**RSI > 70:** Aşırı alınmış (overbought) — çok hızlı yükseltilmiş, düzeltme gelebilir.
**RSI < 30:** Aşırı satılmış (oversold) — çok hızlı düşürülmüş, toparlanma gelebilir.

**Neden kullanılır?** Trendin "yorgunluğunu" ölçer. Fiyat yükselse de RSI düşüyorsa trend zayıflıyor olabilir.

Sistemde `rsi_ob: 60.0` gibi değerler "RSI bu değeri geçerse aşırı alınmış say" eşiğidir. Piyasaya göre ayarlanır.

---

### MACD (Moving Average Convergence Divergence)

İki farklı hareketli ortalama arasındaki farkı izler ve trendin ivmesini ölçer.

**Üç bileşen:**
- **MACD Çizgisi:** Hızlı MA - Yavaş MA
- **Sinyal Çizgisi:** MACD'nin kendi ortalaması
- **Histogram:** İkisi arasındaki fark (momentum görselleştirmesi)

**Sinyal:**
- MACD, sinyal çizgisini yukarı keser → alım
- MACD, sinyal çizgisini aşağı keser → satım

---

### Bollinger Bands (BB) — Bollinger Bantları

Fiyatın etrafına çizilen "zarf" bantları. Orta bant hareketli ortalama, üst ve alt bantlar ise standart sapma ile belirlenir.

**Nasıl yorumlanır?**
- Fiyat alt banda yaklaşırsa → ucuz bölge, alım düşünülebilir
- Fiyat üst banda yaklaşırsa → pahalı bölge, satış düşünülebilir
- Bantlar daralırsa → volatilite düşük, büyük hareket yaklaşıyor olabilir
- Bantlar genişlerse → piyasa hareketli

**Neden kullanılır?** Fiyatın "normal aralığı" dışına çıkıp çıkmadığını gösterir.

---

### ATR (Average True Range) — Ortalama Gerçek Aralık

Fiyatın ortalama olarak ne kadar oynadığını ölçer.

**Neden önemli?** "Bu piyasa ne kadar volatil?" sorusunu sayısal olarak yanıtlar.

**Sistemdeki kullanımı:**
- `sl_atr_multiplier: 1.5` → SL mesafesi = ATR × 1.5
- Eğer bitcoin ortalama 500 $ oyuyorsa SL 750 $ uzağa konulur
- Eğer bitcoin ortalama 200 $ oyuyorsa SL 300 $ uzağa konulur

**Neden sabit % yerine ATR?** Piyasa bazen sakin, bazen çok hareketlidir. Sabit %0.5 SL sakin dönemde çok geniş, hareketli dönemde çok dardır. ATR piyasanın nabzına göre ayarlanır.

---

### ADX (Average Directional Index) — Ortalama Yön Endeksi

Trendin ne kadar güçlü olduğunu ölçer. Yönü değil, gücü!

**ADX < 20:** Trend zayıf, piyasa yatay hareket ediyor.
**ADX 20-40:** Orta güçlü trend.
**ADX > 40:** Çok güçlü trend.

**Sistemdeki kullanımı:** `adx_trend_threshold: 25.0` → ADX 25'i geçmişse güçlü trend var, karşı yönde işlem açma.

---

### Supertrend

ATR tabanlı dinamik destek/direnç çizgisi. Fiyat bu çizginin üstündeyse yükseliş trendi, altındaysa düşüş trendi.

---

## Beşinci Bölüm: Trend Analizi

### HTF (Higher Time Frame) — Üst Zaman Dilimi

"Büyük resim" trendi. 1 dakikalık grafik bakarken 1 saatlik veya 4 saatlik grafiğe bakmak gibi.

**Neden önemli?** 1 dakikalık grafik yukarı işaret etse de 4 saatlik grafik güçlü bir düşüş trendindeyse, kısa vadeli yükseliş yanıltıcı olabilir.

**Sistemdeki HTF Bullish/Bearish:**
- **HTF Bullish:** Büyük resim yükseliş trendi → SHORT (düşüşe bahis) açmak riskli
- **HTF Bearish:** Büyük resim düşüş trendi → LONG (yükselişe bahis) açmak riskli

`short_htf_block: true` → HTF Bullish iken SHORT açılmaz. Bu, sistemin 06 Nisan'daki hatalarından çıkardığı derstir: BTC yükseliş trendindeyken SHORT açılmış ve kaybedilmiştir.

---

### Trend Bias (Trend Yönelimi)

Kısa ve uzun SMA karşılaştırmasıyla belirlenen genel yön:
- **Bullish (Yükseliş):** Kısa SMA > Uzun SMA
- **Bearish (Düşüş):** Kısa SMA < Uzun SMA
- **Neutral (Nötr):** Fark çok küçük, net yön yok

---

## Altıncı Bölüm: Sinyal Üretimi ve Filtreleme

### Sinyal (Signal)

Sistemin "şimdi ne yapmalıyım?" kararı:
- **BUY (Al):** LONG pozisyon aç
- **SELL (Sat):** SHORT pozisyon aç (veya varsa LONG'u kapat)
- **HOLD (Bekle):** Hiçbir şey yapma

**Neden çoğunlukla HOLD?** Piyasada her an işlem yapmak karlı değildir. İyi sistemler zamanın büyük çoğunluğunu bekleyerek geçirir, sadece iyi fırsatlarda işlem açar.

---

### Filtre (Filter)

Sinyali "geçerli mi?" diye sorgulayan kontroller. Sinyal oluşsa bile filtreler onu engelleyebilir.

**Neden filtre?** Her sinyal iyi değildir. Filtreler kalitesiz sinyalleri eleyerek yanlış işlemleri azaltır.

**Sistemdeki filtreler:**
- **Trend filtresi:** Trende karşı sinyal engelle
- **HTF filtresi:** Büyük resim trende karşı sinyal engelle
- **Volatilite filtresi:** Piyasa çok sakin veya çok hareketliyse işlem açma
- **R/R filtresi:** Risk/Ödül oranı düşükse işlem açma
- **Hacim filtresi:** Hacim yetersizse işlem açma

---

### Blok (Block)

Bir sinyalin filtre tarafından engellendiği durum. Log'larda şöyle görünür:

```
HTF Bullish — SELL engellendi
```

Bu, "sistem SHORT sinyali üretti ama HTF yükseliş trendinde olduğu için açılmadı" demektir. İyi bir şey — hatalı işlem önlendi.

---

### Sinyal Güç Skoru (Composite Score)

Birden fazla strateji aynı anda çalışır ve her birinin ne kadar emin olduğu hesaplanır. Tüm sonuçlar birleştirilerek bir "güven skoru" oluşur.

`short_min_composite_score: 0.40` → Skor 0.40'ın altındaysa SHORT açma. Yani sistemin %40 güvenden az olduğu durumlarda SHORT açılmaz.

---

## Yedinci Bölüm: Pozisyon Yönetimi

### Breakeven (Başabaş) — Sıfır Zarar Noktası

Pozisyon kâra geçince SL'yi giriş fiyatına çekme stratejisi.

**Ne sağlar?** En kötü ihtimalle işlem sıfır kâr/zarar ile kapanır. Kayıp riski ortadan kalkar.

**Sistemdeki `breakeven_at_rr`:** R/R bu değere ulaşınca breakeven tetikle.

---

### Partial TP (Kısmi Kâr Al)

TP noktasına ulaşınca pozisyonun tamamını değil, bir kısmını kapat.

**Örnek:** TP'de %50 kapat, kalanını TSL ile yönet.

**Ne sağlar?** Hem kâr realize edilir hem de trend devam ederse daha fazla kazanma şansı korunur.

---

### Cooldown (Soğuma Süresi)

SL tetiklendikten sonra aynı sembole belirli süre yeni pozisyon açılmaması.

**Neden?** SL tetiklendikten sonra piyasa genellikle devam eder. Hemen tekrar girersen aynı hatayı tekrarlarsın. Cooldown "dur, bekle, piyasayı tekrar değerlendir" der.

Sistemde `sl_cooldown_secs: 600` → SL'den sonra 10 dakika aynı sembole işlem yok.

---

### Flip Cooldown

Yön değiştirme (LONG'dan SHORT'a veya tam tersi) sonrası kısa bekleme süresi.

**Neden?** Piyasa bir yönden diğerine aniden dönerse sistem peş peşe ters işlem açabilir. Flip cooldown bu "titreşimi" engeller.

---

## Sekizinci Bölüm: Otomatik Sistemin Yapısı

### Paper Trading (Kağıt Üzerinde Ticaret)

Gerçek para kullanmadan simülasyon. Sistem sanki gerçek işlem yapıyor gibi davranır ama para el değiştirmez.

**Neden kullanılır?** Stratejiyi gerçek parayı riske atmadan test etmek için. Sistem varsayılan olarak paper modda çalışır (`BINANCE_PAPER_MODE=true`).

---

### Live Trading

Gerçek parayla gerçek işlem. `BINANCE_PAPER_MODE=false` ile aktif edilir.

---

### Circuit Breaker (Devre Kesici)

API hatası veya sistem arızası durumunda işlemleri durduran güvenlik mekanizması.

**Nasıl çalışır?** Üst üste birkaç hata gelirse "devre açılır" ve yeni emir gönderilmez. Sorun çözülünce otomatik "kapanır".

**Neden var?** API hatası sırasında "pozisyon açık mı kapalı mı?" belli değildir. Bu belirsizlikte yeni emir göndermek durumu kötüleştirebilir.

---

### Drawdown (Çekilme)

Zirve değerden ne kadar uzaklaşıldığı. Hesap 10.000 $'a çıktıktan sonra 8.500 $'a düştüyse drawdown %15'tir.

**Max Drawdown Limiti:** Sistem belirli bir drawdown'a ulaşınca işlemleri durdurur. Çünkü o noktada "bir şeyler yanlış gidiyor, duraksama zamanı" demektir.

---

### Backtest (Geçmişi Test Etme)

Stratejiyi geçmiş fiyat verisine uygulayarak "geçmişte çalışsaydı ne olurdu?" sorusunu yanıtlamak.

**Sınırı:** Geçmişte iyi çalışmak geleceği garanti etmez. Ama en azından "tamamen saçma değil" olduğunu gösterir.

---

### Hyperopt (Hiperparametre Optimizasyonu)

"Hangi MA periyotları, hangi RSI eşikleri geçmişte en iyi sonucu verdi?" sorusunu cevaplayarak en iyi parametre kombinasyonunu bulmak.

Sistemde `best_strategy` alanı, hyperopt'un bulduğu en iyi stratejiyi gösterir.

---

### Evrimsel AI (Genetik Algoritma)

Biyolojik evrimi taklit ederek strateji parametrelerini optimize etme. "En iyi stratejiler hayatta kalır, kötüler elenir" prensibi.

**Nasıl çalışır?**
1. 50 farklı parametre seti ("popülasyon") oluştur
2. Her birini backtest et
3. En iyi performansı gösterenleri "çiftleştir" (karıştır)
4. Yeni nesil oluştur
5. Tekrarla

Sistem bu süreci otomatik yürütür. `evolve_every_n_cycles: 50` → Her 50 saatlik döngüde bir evrim çalışır.

---

### ML (Machine Learning — Makine Öğrenmesi)

Sinyal üretiminde yapay zeka katkısı. Geçmiş fiyat verilerinden örüntüler öğrenerek yön tahmini yapar.

Sistemdeki ML modeli iki parçadan oluşur:
- **LR (Linear Regression):** 19 göstergeyi ağırlıklı olarak birleştirir
- **GBT (Gradient Boosted Trees):** Daha karmaşık ilişkileri öğrenir

**Confidence (Güven):** 0.0-1.0 arası değer. 0.45 = %45 güven. Düşükse sinyal zayıf sayılır.

---

## Dokuzuncu Bölüm: Risk Yönetimi

### Neden Risk Yönetimi Her Şeyden Önemlidir?

Ticaretin matematiksel gerçeği şudur: **Hiçbir strateji her zaman kazanmaz.** Amaç, kaybedildiğinde az kaybetmek, kazanıldığında çok kazanmaktır.

Bir hesabı kaybetmenin tek yolu şudur: Büyük kayıpların küçük kazanları yemesine izin vermek. Risk yönetimi bunu engeller.

---

### Kelly Kriteri

"Her işlemde sermayenin ne kadarını riske at?" sorusuna matematiksel yanıt.

**Basit formül:** `f = (p × b - q) / b`
- `p`: Kazanma olasılığı
- `q`: Kaybetme olasılığı (1-p)
- `b`: Kazanç/kayıp oranı

Sistem Kelly Kriteri'ni pozisyon boyutlandırmak için kullanır ama tam Kelly yerine "yarım Kelly" (daha güvenli) tercih edilir.

---

### Komisyon (Commission)

Borsanın her işlem için aldığı ücret.

**Binance Futures:** İşlem başına yaklaşık %0.04 (giriş + çıkış = %0.08)

**Neden önemli?** Küçük görünür ama 163 işlemde birikerek kârlılığı ciddi etkiler. Sistemin komisyonu hesaba katmaması büyük bir hata olurdu. `commission_pct: 0.0004`

---

## Onuncu Bölüm: Sistemin Kendi Kendine Öğrenmesi

### Adaptive Parameters (Uyarlamalı Parametreler)

`config/adaptive_params.json` dosyasında saklanan ve sistem tarafından otomatik güncellenen değerler.

**Neden gerekli?** Piyasa değişir. Nisan ayında işe yarayan strateji Temmuz'da işe yaramayabilir. Sistem performansını izleyerek kendini uyarlar.

**Otomatik ayarlama kuralları:**
- Kazanma oranı çok düşerse → daha sıkı filtreler uygular (daha az ama kaliteli işlem)
- Kazanma oranı çok yüksekse → filtreleri gevşetir (daha fazla fırsat yakala)
- R/R düşerse → TP mesafesini artırır (daha fazla kâr hedefle)
- Ardışık SHORT kaybı artarsa → SHORT'ları durdurur

---

## Son Söz: Kavramların Nedensellik Zinciri

Tüm bu kavramlar aslında tek bir soruya yanıt arar:

**"Şu anda almalı mıyım, satmalı mıyım, yoksa beklemeli miyim?"**

Bu soruya yanıt vermek için:
1. Piyasanın genel yönünü anlıyoruz (HTF, trend bias)
2. Kısa vadeli momentumu ölçüyoruz (RSI, MACD, MA crossover)
3. Giriş noktasının kalitesini değerlendiriyoruz (R/R, volatilite, hacim)
4. İşlem açarsak ne kadar riske atabileceğimizi hesaplıyoruz (Kelly, marjin, kaldıraç)
5. İşlem açınca çıkış noktaları belirliyoruz (SL, TP, TSL)
6. Sonuçlardan öğrenerek kendimizi geliştiriyoruz (adaptive params, backtest, ML)

Her terim bu zincirin bir halkasıdır. Hiçbiri tek başına anlam ifade etmez; hepsi birlikte bir karar sistemi oluşturur.

---

*Belge sonu — memos_trading sistemi için hazırlanmıştır.*
*Son güncelleme: Nisan 2026*
