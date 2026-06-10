# Funding-carry canlı entegrasyon — tasarım planı (onay bekliyor)

> Durum: TASLAK / onay öncesi. Kod YAZILMADI. Ara sonrası bu doküman üzerinden onaya bakılacak.
> İlgili hafıza: `project_funding_carry`, `project_xs_momentum`, `project_maker_limit_entry`.

## 1. Bağlam — ne kanıtlandı

İki **dik** (ρ=0.11), doğrulanmış pooled edge var:
- **Kesitsel momentum** — canlıda zaten var (`xs_live.rs`, XS_LIVE_ENABLED, Faz 2 tamam).
- **Funding-carry** — yeni. WF-OOS p=0.044 @10bps (iki-haftalık rebalance), backtest+harness tamam.

Gerçekçi maliyet (10bps + carry iki-haftalık) birleşik iki-faktör kitap: **Sharpe 0.91, NW-t 2.49, +%27**.

**Carry'nin canlı için iki demir kuralı (backtest'ten):**
1. **Düşük turnover ŞART** — günlük rebalance fee'ye yenik (p 0.056→0.118); **≥iki-haftalık** rebalance gerekir.
2. **Maker icra tercih** — `USE_LIMIT_ENTRY` yolu (positions.rs:819 `xs_maker`) carry tag'ine de uygulanmalı.

## 2. MERKEZİ MİMARİ KARAR — iki ayrı kitap mı, birleşik kitap mı?

Sorun: carry sepeti ile momentum sepeti AYNI majörler → **sembol başına tek-pozisyon invariantı** (`project_one_position_per_symbol`) iki ayrı kitabın aynı sembolü eşzamanlı yönetmesine izin vermez. Ölçtüğümüz "+%27" **iki ayrı getiri serisinin toplamıydı**; canlıda bunu tek-pozisyon altında ifade etmenin üç yolu:

| Seçenek | Ne | Artı / Eksi |
|---|---|---|
| **A. Ayrık sepet** | Bazı majörler momentum'a, bazıları carry'ye | Basit, mevcut iskelet birebir; AMA ölçtüğümüz portföy DEĞİL, güç düşer |
| **B. Harmanlanmış sinyal** | Her sembolü `z(momentum)·wm + z(carry)·wc` ile sırala → TEK kitap | Tek-pozisyon temiz; momentum+carry tek sıralamada; normalize (cross-sectional z-score) gerekir |
| **C. Net ağırlık** | Her sembolün momentum-hedef-ağırlığı + carry-hedef-ağırlığı netlenir → tek değişken-ağırlıklı pozisyon | Ölçülen portföye EN sadık; AMA icra "eşit-ağırlık 1/k" yerine **değişken-ağırlık** ister (open_paper_position değişikliği) |

**Öneri: fazlı.** Faz 1 = **(A)-benzeri ama carry-tek** (momentum'a dokunmadan carry'yi adanmış kitap olarak kanıtla). Faz 2 = **(B) harmanlanmış sinyal** (tek kitap, değişken-ağırlık gerektirmez → C'den daha az invaziv, ölçülen edge'e A'dan yakın).

## 3. FAZ 1 — carry adanmış kitap (MVP, momentum'dan bağımsız)

`xs_live.rs`'in carry ikizi. **Amaç:** carry edge'inin canlı paper'da iki-haftalık+maker ile davranışını doğrulamak. Momentum kapalıyken veya ayrık sepette koşar (çakışma yok).

### Bileşenler ve temas noktaları
1. **Config** — `parameters/types.rs`: yeni `CarryLiveParams` (XsLiveParams ikizi) +
   - `rebalance_bars: usize` (default **14** — KADANS, XS'teki "bar-başına"nın yerine)
   - `lookback` = trailing funding penceresi (default 14), momentum bool YOK (yön sabit: yüksek-funding short)
   - ortak alanlar: enabled(false), symbols, interval("1d"), top_k(3), exit_buffer(1), position_pct, leverage, regime_gate, max_drawdown_pct, cb/tp cooldown.
   - `ParameterStore.carry_live` (store.rs:108 deseni) + env yükleme `CARRY_LIVE_*` (store.rs:408 deseni).
2. **Sinyal** — yeni saf fn `latest_carry_signal(funding_bars: &[f64], lookback) -> Option<f64>` = −(son `lookback` funding ortalaması). `latest_signal` (xs_live.rs:50) ikizi, testli.
3. **Kitap kurucu** — yeni `engines/master/carry_live.rs` · `process_carry_book(state)`:
   - sepet sembollerinin son mumları (fiyat/icra/tazelik) + funding'i (`read_funding_market`, reader.rs) yükle.
   - carry sinyali → **`xs_target_book` AYNEN** (sinyal-agnostik, DRY) → hedef long/short.
   - **KADANS KAPISI (kritik fark):** `process_xs_book`'taki "aynı-bar → skip" yerine "**son rebalance'tan beri < rebalance_bars bar geçti → skip**". Yeni state alanı `carry_last_rebalance_bar` (robotic_loop.rs:78 `xs_last_rebalance_bar` deseni).
   - devre-kesici / rejim-gate / cooldown / take-profit makinesi **birebir yeniden kullanılır** (xs_live.rs:178-251).
   - infaz: `open/close_paper_position` + yeni tag `CARRY_STRATEGY_TAG="FUNDING_CARRY"` + `XsSizing` (eşit-ağırlık).
4. **Cycle wiring** — `loop_core.rs:209` `process_xs_book` çağrısının yanına `process_carry_book`. `xs_basket` exclusion'a (loop_core.rs:168) carry sepetini de ekle (normal döngüden hariç).
5. **Maker icra** — positions.rs:819 `xs_maker` koşuluna `|| tag==CARRY_STRATEGY_TAG` ekle (carry da maker komisyon).
6. **Exit komisyon** — positions_close.rs:308 aynı şekilde carry tag'ini tanısın.

### Veri katmanı (canlı funding feed)
- Şu an `download_funding` ELLE araç. Canlı için: `jobs_download.rs` `run_download_job` içine carry sepeti sembolleri için **artımlı funding fetch** bloğu (gap-farkında, `last_funding_ts` + `fetch_funding_history` + `save_funding`). Funding 8 saatte bir → her download cycle'ında bir refresh fazlasıyla yeter.
- **Funding-tazelik kapısı:** son funding `>N saat` eskiyse o sembol kitaba girmesin (mum stale-feed kapısının funding ikizi; phantom carry önler).

## 4. FAZ 2 — birleşik iki-faktör kitap (seçenek B)

Faz 1 carry'yi kanıtladıktan sonra: momentum + carry'yi **tek kitapta** birleştir.
- Her sembol için cross-sectional **z-score**: `z_m = (mom_sig − μ_m)/σ_m`, `z_c = (carry_sig − μ_c)/σ_c`.
- Harman skoru: `wm·z_m + wc·z_c` (default wc≈0.6, ölçülen optimal). Bu skorla `xs_target_book` → tek market-nötr kitap.
- Tek kitap → tek-pozisyon invariantı temiz, değişken-ağırlık GEREKMEZ (eşit-ağırlık 1/k korunur).
- `XsLiveParams`/`CarryLiveParams` birleşir veya `BlendLiveParams` üst-katmanı (`signal_weights`, `factors: [Momentum, FundingCarry]`).
- Bu, `XsSignal` genelleştirmesinin (project_xs_factors) canlı karşılığı — kesitsel faktör harmanı.

## 5. DRY envanteri — yeniden kullanılacak (yazılmayacak)
`xs_target_book`/`select_books` (sıralama+band), `open/close_paper_position` (+XsSizing), devre-kesici/rejim-gate/cooldown/take-profit bloğu, `cycle_load_candles`, stale-feed + closed-bar kapıları, maker icra yolu, `read_funding_market`/`save_funding`/`fetch_funding_history` (yeni eklenenler). **Yeni kod yalnızca:** CarryLiveParams, latest_carry_signal, process_carry_book + kadans kapısı, funding canlı refresh + tazelik kapısı.

## 6. Riskler & açık sorular
- **Çakışma (en önemli):** Faz 1'de carry+momentum aynı sepette açık koşarsa tek-pozisyon ihlali. → Faz 1'de ya ayrık sepet ya momentum kapalı; tam birleşim Faz 2.
- **Funding feed gecikmesi/eksik:** delisted sembol funding'i durur (MKR örneği) → tazelik kapısı + eligibility ele.
- **Kadans fazı:** "14 bar geçti mi" sayımı hangi referansla? (son rebalance bar open-time; boot'ta hemen bir kez kur, sonra say.)
- **Paper→live:** carry market-nötr + maker → düşük-sermaye live'a momentum'la aynı kapılardan geçer.

## 7. Doğrulama planı (kod sonrası)
1. Paper smoke: boot'ta dengeli carry kitabı kurulur (yüksek-funding short / düşük-funding long), iki-haftalık kadansta rebalance loglanır, maker komisyon uygulanır.
2. Strateji-bazlı P&L attribution (`project_xs_momentum` 995d53f deseni) FUNDING_CARRY etiketiyle.
3. Uzun paper P&L → momentum ile korelasyon canlıda da ~düşük mü (offline ρ=0.11 doğrulaması).

## 8. Tahmini büyüklük
- Faz 1: ~orta (CarryLiveParams + carry_live.rs + funding refresh + wiring + testler). xs_live.rs çoğunlukla kopyala-uyarla.
- Faz 2: ~orta-büyük (z-score harman katmanı + config birleşimi).
