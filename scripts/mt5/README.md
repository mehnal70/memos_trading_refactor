# MT5 köprüsü (MetaTrader 5 venue adaptörü — Faz 1: veri)

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
| `order`        | `symbol`,`side`,`qty`,`kind`,`price?`    | **Faz 2** — şu an `ok:false`                      |
| `cancel_all`   | `symbol`                                 | **Faz 2** — şu an `ok:false`                      |
| `set_leverage` | `symbol`,`leverage`                      | **Faz 2** — şu an `ok:false`                      |

Hata: `{"id":N,"ok":false,"error":"…"}`. Sahte başarı asla dönmez.

## Kurulum

1. **EA'yı derle:** `MemosBridge.mq5` → MT5 `MQL5/Experts/` klasörüne kopyala, MetaEditor'da derle.
2. **Soket izni:** MT5 → Tools › Options › Expert Advisors → soket adres listesine `127.0.0.1`
   ekle (build ≥ 1930 native soket izni). DLL/WebRequest gerekmez.
3. **Önce Rust köprüsünü ayağa kaldır** (motor `mt5:*` venue'lu profil ile başlatılınca köprü
   ilk istekte dinlemeye geçer), **sonra** EA'yı bir grafiğe ekle. EA bağlanıp istekleri yanıtlar.
4. EA input'ları: `InpHost`/`InpPort` (Rust köprüsüyle aynı), `InpPollMs` (varsayılan 50ms).

## Motora bağlama (Faz 1: izole edge ölçümü)

MT5 sembolleri (EURUSD/XAUUSD) BIST equity şekliyle çakıştığı için **oto-sınıflanmaz** —
**explicit routing** ile kullanılır:

- Aktif venue'lara MT5 ekle: `VENUES=binance:futures,mt5:spot`
- Köprü adresi (varsayılan dışıysa): `MT5_BRIDGE_ADDR=127.0.0.1:9001`
- Sembolü etiketle: `EURUSD@mt5`, `XAUUSD@mt5`

> **Faz duvarı:** Yürütme (`order`) Faz 2'dir. Önce MT5 verisiyle izole edge ölçümü yapılır
> (forex/emtia), edge doğrulanırsa `OrderExecution` (Rust `Mt5Venue` + EA `HandleOrder`) açılır.
