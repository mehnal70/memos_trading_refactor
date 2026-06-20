# MT5 köprüsü (MetaTrader 5 venue adaptörü — Faz 1: veri, Faz 2: yürütme)

Memos trading-core ile yerel MetaTrader 5 terminali arasında **satır-sınırlı JSON (NDJSON)**
köprü. Forex/emtia/endeks CFD verisini motora `VenueAdapter` soyutlaması üzerinden taşır.

## Mimari (neden Rust server, MQL5 client?)

MQL5'in native soket fonksiyonları yalnız **client**'tır — EA bir TCP server açamaz (listen yok).
Bu yüzden:

```
Rust (tokio TCP listener, SERVER)  <---->  MT5 EA (SocketConnect, CLIENT)
        Mt5Bridge                            MemosBridge.mq5
```

- **Rust** bir portu dinler (`MT5_BRIDGE_ADDR`, varsayılan `127.0.0.1:9001`).
- **EA** dışarı bağlanır, döngüde: istek satırı OKU → işle (`CopyRates`/`SymbolInfoTick`/...) →
  yanıt satırı YAZ.
- Tek persistan bağlantıda istek↔yanıt **seri** (Rust tarafında muteks). DLL **gerekmez**
  (saf MQL5 + saf tokio).

## Protokol

İstek (Rust → EA), tek satır JSON:

| cmd            | alanlar                                  | yanıt (ok:true)                                   |
|----------------|------------------------------------------|---------------------------------------------------|
| `candles`      | `symbol`, `tf` (1m/1h/1d…), `limit`      | `candles`: `[[ts_ms,o,h,l,c,v], …]` (artan)       |
| `tick`         | `symbol`                                 | `bid`, `ask`                                      |
| `filters`      | `symbol`                                 | `lot_step`, `min_lot`, `tick_size`, `min_notional`|
| `balance`      | —                                        | `balance`                                         |
| `order`        | `symbol`,`side`,`qty`,`kind`,`price?`,`reduce_only` | `order_id`,`status`,`filled_qty`,`avg_price` |
| `cancel_all`   | `symbol`                                 | `canceled` (silinen bekleyen emir sayısı)         |
| `set_leverage` | `symbol`,`leverage`                      | `ok:true` (MT5'te kaldıraç hesap-seviyesi → no-op)|

Hata: `{"id":N,"ok":false,"error":"…"}`. Sahte başarı asla dönmez.

**Yürütme semantiği (Faz 2):**
- `order.kind="market"` → `CTrade.Buy/Sell` (anında dolum, `status:"filled"`, `filled_qty`/`avg_price` dolu).
- `order.kind="limit"` → `CTrade.BuyLimit/SellLimit` (bekleyen, `status:"placed"`, dolum 0). POST_ONLY ≈ piyasadan uzak bekleyen limit (doğal maker).
- `reduce_only` netting hesabında ters market emriyle pozisyonu netler; hedging hesapta ayrı pozisyon açar — netting hesap önerilir.
- `cancel_all` yalnız **bekleyen emirleri** siler (koruma/limit); açık pozisyonu kapatmaz (o, `reduce_only` market ile yapılır).

## Kurulum

1. **EA'yı derle:** `MemosBridge.mq5` → MT5 `MQL5/Experts/` klasörüne kopyala, MetaEditor'da derle.
2. **Soket izni:** MT5 → Tools › Options › Expert Advisors → soket adres listesine `127.0.0.1`
   ekle (build ≥ 1930 native soket izni). DLL/WebRequest gerekmez.
3. **Önce Rust köprüsünü ayağa kaldır** (motor `mt5:*` venue'lu profil ile başlatılınca köprü
   ilk istekte dinlemeye geçer), **sonra** EA'yı bir grafiğe ekle. EA bağlanıp istekleri yanıtlar.
4. EA input'ları: `InpHost`/`InpPort` (Rust köprüsüyle aynı), `InpPollMs` (varsayılan 50ms),
   `InpEnableExec` (Faz 2 emir yürütme kapısı, **varsayılan kapalı**), `InpMagic` (emir kimliği).

**Faz 2 (yürütme) için ek izin:** Gerçek emir göndermek için MT5'te **AutoTrading** açık olmalı
(araç çubuğundaki "Algo Trading" butonu) **ve** EA'da `InpEnableExec=true` verilmeli. İkisi de
kapalıyken `order`/`cancel_all`/`set_leverage` açık `ok:false` döner (sahte dolum yok). Canlı MT5
yönlendirmesi ayrıca edge doğrulamasına bağlıdır (aşağıdaki faz duvarı).

## Motora bağlama (Faz 1: izole edge ölçümü)

MT5 sembolleri (EURUSD/XAUUSD) BIST equity şekliyle çakıştığı için **oto-sınıflanmaz** —
**explicit routing** ile kullanılır:

- Aktif venue'lara MT5 ekle: `VENUES=binance:futures,mt5:spot`
- Köprü adresi (varsayılan dışıysa): `MT5_BRIDGE_ADDR=127.0.0.1:9001`
- Sembolü etiketle: `EURUSD@mt5`, `XAUUSD@mt5`

### Veriyi DB'ye alma (edge ölçümü yakıtı)

EA bağlıyken `download_mt5` örneği mumu kanonik `candles` şemasına yazar:

```
cargo run --release --example download_mt5 -- mt5 1h EURUSD,GBPUSD,XAUUSD 2000
```

İzolasyon: MT5 verisi `exchange="mt5"`, `market=<market_tag>` (örn. `mt5`) ile saklanır.
`read_candles_market` yalnız `market` ile filtrelediğinden, MT5'e **ayrı etiket** verilir
(kripto `spot` / Yahoo `forex` ile karışmasın). Env: `DB_PATH`, `MT5_BRIDGE_ADDR`.

> **Faz duvarı:** Yürütme kablolaması (Rust `Mt5Venue::submit_order/...` + EA `HandleOrder`/
> `HandleCancelAll`/`HandleSetLeverage`) Faz 2'de **uygulandı** ve `InpEnableExec` arkasında.
> Ama köprünün hazır olması canlıya geçme kararı DEĞİLDİR: motorun MT5'e gerçek emir
> yönlendirmesi hâlâ forex/emtia için izole edge ölçümünün doğrulanmasına bağlıdır.
