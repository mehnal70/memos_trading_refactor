# Memos RTC — İşletme Kılavuzu

> Tek komutla yönetilen otonom trading motoru. Bu kılavuz: **açma → kontrol → uzaktan izleme → kapatma** akışını adım adım verir.
> Tüm komutlar repo kökünde çalışır: `cd ~/PyCharmMiscProject/memos_trading_refactor`

---

## 0. Hızlı Başvuru

| İhtiyaç | Komut |
|---|---|
| Sistemi başlat (motor + panel) | `./memos up` |
| Durum kontrolü | `./memos status` |
| Canlı panelleri izle (terminal) | `./memos attach` → bırak: **Ctrl-b** sonra **d** |
| Telefon paneli (her yerden) | `http://100.103.57.61:8090` |
| Ayarları düzenle | `./memos config` |
| Durumu sıfırla (temiz baz) | `./memos down` → `./memos reset --state` → `./memos up` |
| Sistemi durdur | `./memos down` |
| Canlı log akışı | `./memos logs` |

**İki izleme kanalı (birbirini tamamlar):**
- 📲 **Panel** (PULL): bakmak istediğinde tüm canlı tablo — `http://100.103.57.61:8090`
- 💬 **Telegram** (PUSH): bir şey olunca seni bulur — bot **@Memos970Bot**

---

## 1. Sistem Mimarisi (kısa)

```
systemd (boot'ta) ──► watchdog ──► motor (rtc_tui, tmux 'memos' oturumu)
                                      │  her 1s → snapshot.json
                   └─► dashboard ◄────┘  (telefon panelinin okuduğu)
motor ──► Telegram (@Memos970Bot)  kritik olay + trade + özet push'u
```

- **Motor**: `rtc_tui`, `tmux` oturumunda (adı `memos`) 7/24 koşar.
- **Watchdog**: motor ölür/donarsa otomatik yeniden başlatır.
- **Panel**: `snapshot.json`'ı sunar; telefon tarayıcısı bunu okur.
- **systemd**: boot'ta + logout'ta hepsini ayakta tutar (`boot-install` ile kurulu).

---

## 2. Bir Kerelik Hazırlık (✅ zaten yapıldı — referans / yeni makine için)

Bu üçü kuruluysa atla. Yeni makinede ya da sıfırdan kurulum gerekirse:

**a) Boot'ta otomatik (systemd):**
```
./memos boot-install
```
→ motor + panel boot'ta + logout'ta otomatik kalkar (kök gerektirmez, `--user` servis + linger).

**b) Tailscale (her yerden erişim tüneli):** — bu makinede kurulu, IP `100.103.57.61`
```
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up        # çıkan linki tarayıcıda onayla
tailscale ip -4          # bu makinenin Tailscale IP'sini gösterir
```
Telefona **Tailscale** uygulamasını kur, **aynı hesapla** giriş yap.

**c) Telegram (push bildirimleri):** — kurulu, bot `@Memos970Bot`
- **@BotFather** → `/newbot` → token al
- **@userinfobot** → chat id al
- `./memos config` → **Telegram** grubu → `TELEGRAM_BOT_TOKEN` + `TELEGRAM_CHAT_ID` doldur → kaydet

---

## 3. Sistemi Açma

> Not: `boot-install` yapıldıysa makine açıldığında sistem **zaten çalışıyordur**. `./memos status` ile teyit et; çalışıyorsa tekrar `up` demene gerek yok.

**Adım adım:**

1. **(Opsiyonel) Ayarları gözden geçir:**
   ```
   ./memos config
   ```
   Menüde numarayla düzenle → `s` ile başlat ya da `k` ile sadece kaydet+çık.

2. **(Opsiyonel) Temiz baz isteniyorsa** — equity'i 10.000'e döndür, açık pozisyonları temizle:
   ```
   ./memos down
   ./memos reset --state      # öğrenilmiş paramlar + mumlar KORUNUR
   ```

3. **Başlat:**
   ```
   ./memos up
   ```
   Çıktıda LAN + Tailscale panel URL'leri görünür.

4. **Teyit et:**
   ```
   ./memos status
   ```
   Beklenen: Motor ✅ canlı, Watchdog çalışıyor, Panel çalışıyor, heartbeat yaşı küçük.

5. **(İlk kurulumda) Boot'a yaz** ki bir daha elle açmayasın:
   ```
   ./memos boot-install
   ```

> Telefonuna boot'ta bir **📊 Memos özet** mesajı düşerse motor + Telegram sağlıklı demektir.

---

## 4. Ara Kontroller (sağlık)

**Hızlı bakış:**
```
./memos status
```

| Bölüm | Sağlıklı | Sorunlu |
|---|---|---|
| Motor | `✅ TUI canlı`, heartbeat yaşı < ~70s | `❌ TUI yok` veya heartbeat çok eski |
| Watchdog | `active` / çalışıyor | kapalı veya `⏸️ DURAKLATILMIŞ` |
| Panel | çalışıyor (port 8090) | kapalı |

**Panelden (telefon ya da tarayıcı):** `http://100.103.57.61:8090`
- Üstteki nokta **yeşil** = canlı, **sarı** = snapshot bayat (motor durmuş olabilir).
- Bakılacaklar: equity & getiri%, açık pozisyonlar (XS kitabı + SL/TP), kapanan trade'lerde **net P&L**, **anomali sayısı**.

**Neyin normal olduğu:**
- Açık P&L'in artı/eksi dalgalanması normaldir (XS market-nötr kitap; bazı bacaklar su altında olabilir).
- **Anomali = 0** olmalı. Sıfırdan büyükse logları incele.
- Heartbeat sürekli tazeleniyor olmalı (panel sarıya dönmemeli).

**Log akışı:**
```
./memos logs          # canlı trade/olay akışı (Ctrl-C ile çık)
```

**Terminalde tam panelleri izlemek:**
```
./memos attach        # tmux'a bağlan; bırakmak için Ctrl-b sonra d (motor çalışmaya devam eder)
```

---

## 5. Tailscale ile Telefondan İzleme (her yerden)

1. Telefonda **Tailscale** açık ve **aynı hesapta** girişli olmalı (bir kez kur, hep açık kalsın).
2. Tarayıcıda: **`http://100.103.57.61:8090`**
3. Chrome menü → **"Ana ekrana ekle"** → uygulama gibi tam ekran (PWA).

**Aynı evde (wifi) alternatif:** `http://192.168.0.104:8090`

**Açılmıyorsa kontrol:**
- Telefonda Tailscale **bağlı mı** (uygulamada yeşil/Connected)?
- Bu makinede panel çalışıyor mu: `./memos status` → Panel bölümü.
- Tailscale ayakta mı: `tailscale status` (bu makinede).

---

## 6. Telegram Bildirimleri

**Bot:** @Memos970Bot — bağlantı **aktif**.

**Telefonuna düşecekler:**
| İşaret | Olay |
|---|---|
| 📈 | Trade **açılış** (sembol/yön/fiyat/strateji) |
| 🟢 / 🔴 | Trade **kapanış** — net P&L + neden |
| 🔌 | XS **devre kesici** (felaket freni tetiği) |
| 📊 | **Periyodik özet** (boot'ta + günde bir) |
| 🚨 / ⚠️ | Kritik operasyonel (acil kapanış, regime, fleet) |

(Aynı olay 60s içinde tekrarlanırsa spam'lenmez.)

**Çalıştığını doğrula** (terminalden test mesajı):
```
. scripts/lib_launchconf.sh; load_launch_conf scripts/.launch.conf
curl -s --data-urlencode "chat_id=${TELEGRAM_CHAT_ID}" \
     --data-urlencode "text=test" \
     "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendMessage" >/dev/null && echo gönderildi
```

**Özet sıklığını değiştir:** `./memos config` → Telegram grubu, ya da `.launch.conf`'ta
`SCHEDULER_REPORT_EVERY_MINS` (dakika; default 1440=günlük, 0=kapalı). Değişiklik **restart** ister.

---

## 7. Sistemi Kapatma

**Geçici durdurma (tekrar `up` ile açarsın):**
```
./memos down
```
→ watchdog + motor + panel hepsi temiz durur.

**Bakım için duraklatma** (motoru kapatmadan watchdog'un restart'ını durdur):
```
touch logs/.watchdog.pause     # watchdog artık restart yapmaz
rm    logs/.watchdog.pause      # geri al
```

**Boot otomatiğini tamamen kaldır** (artık makine açılışında başlamasın):
```
./memos boot-uninstall
./memos down                    # çalışıyorsa durdur
```

> Önemli: `reset` çalıştırmadan önce motoru **mutlaka durdur** (`./memos down`). Çalışırken reset DB yazma yarışı yüzünden reddedilir.

---

## 8. Sorun Giderme

| Belirti | Çözüm |
|---|---|
| `status` → Motor ❌ veya heartbeat çok eski | `./memos restart` (down+up). systemd kuruluysa watchdog zaten denemiştir; loglara bak: `tail logs/watchdog.log` |
| Panel sarı nokta / "bayat" | Motor durmuş. `./memos status` → gerekirse `./memos up` |
| Panel telefonda açılmıyor | Tailscale bağlı mı? `./memos status` Panel çalışıyor mu? `tailscale status` |
| Telegram gelmiyor | Token/chat doğru mu (§6 test). Motor restart edildi mi? (token eklenince restart şart) |
| Watchdog `⏸️ DURAKLATILMIŞ` | Crash-loop koruması devrede. Sebebi düzelt → `rm logs/.watchdog.pause` |
| "reset reddedildi" | Motor çalışıyor → önce `./memos down` |
| Değişiklik/yeni ayar devreye girmiyor | Çoğu ayar **restart** ister: `./memos down && ./memos up` |

**Önemli loglar** (`logs/` altında):
- `heartbeat.jsonl` — motorun nabzı (her ~1 dk bir tick)
- `trades.jsonl` — açılan/kapanan işlemler
- `robotic_trading.log` — olay günlüğü
- `watchdog.log` — restart geçmişi
- `dashboard.log` — panel sunucusu

---

## 9. Komut Sözlüğü (tam)

```
./memos config            # başlatma parametrelerini düzenle (menü → .launch.conf)
./memos start             # ÖN PLANDA TUI (bu terminalde)
./memos up                # 7/24 ARKA PLAN: motor + watchdog + panel
./memos down              # hepsini durdur
./memos restart           # down + up
./memos status            # motor + watchdog + panel + heartbeat
./memos attach            # tmux panellerini izle (bırak: Ctrl-b d)
./memos logs              # canlı log akışı
./memos dashboard [stop]  # paneli ayrıca başlat/durdur (up zaten başlatır)
./memos reset [args]      # --state | --history | --all -y  (önce ./memos down)
./memos boot-install      # boot'ta otomatik (systemd: motor + panel + linger)
./memos boot-uninstall    # boot otomatiğini kaldır
./memos build             # sadece release binary derle
```

**Sık akışlar:**
- Günlük açılış: `./memos up` → `./memos status`
- Temiz yeniden başlangıç: `./memos down` → `./memos reset --state` → `./memos up`
- Ayar değişikliği: `./memos config` → `./memos down && ./memos up`
- Uzaktan izleme: telefonda `http://100.103.57.61:8090` + Telegram @Memos970Bot

---

*Bu kılavuz `docs/ISLETME_KILAVUZU.md` dosyasından üretildi. Komutlar repo kökünde çalıştırılır.*
