// src/robot/engines/master/positions.rs — Pozisyon açılışı (scalp-swing + paper) + blok kapısı.
// Faz 2 modülerleştirme: exit/kapanış yolu positions_close.rs'e ayrıldı (davranış birebir).
use super::*;

impl Engine {

    /// ScalpSwing A2: alt-kanal fırsat avcısı. cfg.scalp_enabled/swing_enabled
    /// gate'ler ile ScalpEngine/SwingEngine fırsat üretir, en yüksek skoru
    /// SlotGuard'dan geçirir ve uygunsa open_paper_position'a Some(TradeType)
    /// yolu ile dispatch eder. `true` döndüğünde caller turun bu sembolde
    /// klasik Strategy yolunu pas geçer (çakışma yok). `false` döndüğünde
    /// klasik akış aynen devam.
    pub(crate) async fn try_open_scalp_swing(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        candles: &[Candle],
        regime: crate::evolution::MarketRegime,
        regime_directional: bool,
    ) -> bool {
        use crate::robot::scalp_swing::{
            ScalpEngine, SwingEngine, SlotGuard, OpenSlot, ScalpSwingConfig,
        };

        // 1) cfg + mevcut açık pozisyonların kanal-bazlı slot'larını topla.
        //    Hem cfg hem slots tek kısa mutex skopunda alınır.
        let (cfg, existing_slots): (ScalpSwingConfig, Vec<OpenSlot>) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return false };
            let cfg = match st.brain.scalp_swing_config.read().ok() {
                Some(c) => c.clone(),
                None => return false,
            };
            if !cfg.scalp_enabled && !cfg.swing_enabled { return false; }
            let slots = st.finance.live_positions.read().ok().map(|m| {
                m.values().filter_map(|p| {
                    p.kind.map(|kind| OpenSlot {
                        symbol: p.symbol.clone(),
                        trade_type: kind,
                        is_long: p.is_long,
                    })
                }).collect::<Vec<_>>()
            }).unwrap_or_default();
            (cfg, slots)
        };

        // A6: Rejim-bazlı auto-gate. ScalpSwing'i otonomca rejim sinyaliyle
        // modüle eder — operatör flag'iyle değil:
        //   HighVolatility       → her iki kanal skip (kaos savunması)
        //   Ranging/LowVolatility → sadece Scalp uygun (kısa-vade fırsat)
        //   StrongUp/StrongDown   → sadece Swing uygun (trend takip)
        //   Weak*/Unknown         → her ikisi de aday (genel)
        // cfg.scalp_enabled/swing_enabled'in üstüne biner — disabled kanal
        // hiçbir koşulda açılmaz; enabled kanal yalnız uygun rejimde aday olur.
        // Rejim artık çağırandan (process_symbol_cycle) geliyor: HTF-tercihli, cache'li
        // RegimeContext (Adım 1) — base-1m classify_regime yerine. Geniş TF rejim →
        // scalp/swing dengesi otonom: HTF trend → swing, ranging → scalp.
        use crate::evolution::MarketRegime;
        if matches!(regime, MarketRegime::HighVolatility) {
            return false; // savunma
        }
        let (scalp_ok, swing_ok) = match regime {
            MarketRegime::Ranging | MarketRegime::LowVolatility => (true,  false),
            MarketRegime::StrongUptrend | MarketRegime::StrongDowntrend => (false, true),
            _ => (true, true),
        };

        let scalp_opp = if cfg.scalp_enabled && scalp_ok {
            ScalpEngine::evaluate(candles, cfg.scalp_min_score)
        } else { None };
        let swing_opp = if cfg.swing_enabled && swing_ok {
            SwingEngine::evaluate(candles, cfg.swing_min_adx, cfg.swing_min_score)
        } else { None };

        // 3) En yüksek skoru seç (Scalp ve Swing eşit ise Scalp önce —
        //    kısa-vade fırsat sermayeyi daha az bağlar).
        let opp = match (scalp_opp, swing_opp) {
            (Some(a), Some(b)) => if a.score >= b.score { a } else { b },
            (Some(a), None)    => a,
            (None,    Some(b)) => b,
            (None,    None)    => return false,
        };

        // 🧭 Rejim-yön kapısı (opt-in): ScalpSwing yönü de rejimle hizalı olmalı —
        // regular-strateji yoluyla AYNI kural (regime_confirms_direction, tek-kaynak).
        // Aksi halde StrongUptrend'de SwingEngine short açabiliyordu (ters-trend). Canlı
        // paper doğrulamasında bu boşluk yakalandı. default false → davranış değişmez.
        if regime_directional
            && !crate::robot::logic::market_regime::regime_confirms_direction(regime, opp.is_long)
        {
            if log_throttle_should_emit(symbol, "scalp_regime_dir_block", 60) {
                push_state_log(state, format!(
                    "🧭 ScalpSwing {} {} ⇒ REDDEDİLDİ (rejim-yön teyidi yok, rejim={})",
                    opp.trade_type.label(),
                    if opp.is_long { "BUY" } else { "SELL" },
                    regime.as_str(),
                ));
            }
            return false;
        }

        // 4) SlotGuard: kanal-bazlı kapasite + hedge engeli.
        let (ok, reason) = SlotGuard::can_open(
            &existing_slots, symbol, opp.trade_type, opp.is_long,
            cfg.max_scalp_per_symbol, cfg.max_swing_per_symbol,
        );
        if !ok {
            log::debug!(
                "ScalpSwing {} {} reddedildi: {}",
                opp.trade_type.label(), symbol, reason,
            );
            return false;
        }

        // 5) Açılış — kind=Some(TradeType) yolu. strategy_name etiketinde
        //    kanal kısaltması var ki UI/log paneli ayırt edebilsin.
        let signal = if opp.is_long {
            crate::core::types::Signal::Buy
        } else {
            crate::core::types::Signal::Sell
        };
        let strategy_name = format!(
            "{}_{}", opp.trade_type.label(),
            if opp.is_long { "BUY" } else { "SELL" },
        );
        // Bu "deneme" logu sembol başına throttle'lanır: fırsat her cycle (500ms)
        // tespit edilip açılış re-entry cooldown / risk ile bloklanınca olay günlüğünü
        // taşırıyordu (84/100 satır). Gerçek açılış zaten open_paper_position'da
        // "🚀 [PAPER-...] açıldı" ile onaylanıyor → bu yalnız bağlam (score/reason).
        if log_throttle_should_emit(symbol, "scalp_open_attempt", 60) {
            push_state_log(state, format!(
                "⚡ ScalpSwing {} açılış: {} score={:.2} | {}",
                opp.trade_type.label(), symbol, opp.score, opp.reason,
            ));
        }
        Self::open_paper_position(
            state, symbol, &signal, candles, &strategy_name, Some(opp.trade_type), None,
        ).await; // xs_sizing=None → ScalpSwing Kelly+resolve_leverage (mevcut davranış)
        true
    }

    /// 🧬 FAZ F3: OTONOM POZİSYON AÇILIŞ MOTORU (Paper + Live dispatcher)
    /// Kelly oranı, brain.ml_confidence ve loss_streak ile dinamik tahsisat yapar.
    /// Live executor bağlıysa ve dry-run değilse: gerçek market order gönderir.
    /// `kind` = ScalpSwing kanal mührü (Some(Scalp/Swing) → try_open_scalp_swing
    /// yolundan, None → klasik strategy yolu). PositionModel.kind alanına
    /// yazılır; close_paper_position kapanışta ScalpSwingStats güncelliyor.
    pub(crate) async fn open_paper_position(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        signal: &Signal,
        candles: &[Candle],
        strategy_name: &str,
        kind: Option<crate::robot::scalp_swing::TradeType>,
        // KESİTSEL override (adanmış mod): Some → eşit-ağırlık alloc (equity·frac, Kelly bypass) + SABİT
        // kaldıraç (resolve_leverage bypass). Market-nötr kitabın 1/k dengesi + risk kontrolü. None →
        // mevcut Kelly sizing + resolve_leverage (sıfır regresyon; XS-dışı tüm çağıranlar None).
        xs_sizing: Option<book_core::BookSizing>,
    ) {
        use crate::robot::risk::kelly::KellyCriterion;
        let last_candle = match candles.last() { Some(c) => c, None => return };

        // blocked_symbols savunması: config'te listelenen semboller için yeni pozisyon
        // açılışı kısa-devre reddedilir. Mevcut açık pozisyonlar bu engelden etkilenmez
        // (execute_trade_cycle yetim pozisyon kuralı onları yönetmeye devam eder).
        if Self::is_symbol_blocked(state, symbol) {
            push_state_log(state, format!("🚫 {} açılış reddedildi: blocked_symbols listesinde", symbol));
            return;
        }

        // ⏳ Re-entry cooldown: son kapanıştan REENTRY_COOLDOWN_SECS geçmediyse yeni
        // açılış engellenir — churn/flip-flop (aç→kapa→hemen aç) koruması. 0 → kapalı.
        // Her iki açılış yolu (scalp_swing + strateji) buradan geçtiği için tek-nokta.
        // KESİTSEL MUAF: XS kitabının churn kontrolü no-trade band + bar-başına kadans kapısıdır
        // (process_xs_book). Per-sembol cooldown XS'e uygulanınca flip'in açma yarısını (close→open
        // aynı cycle) bloklar ve bar-kadansı kapısı yüzünden bir sonraki bara kadar açtırmaz → bacak
        // bar boyu yanlış flat kalır. XS zaten Kelly+resolve_leverage'ı da bypass ediyor. [[project_xs_momentum]]
        let cooldown_block = state.lock().ok().and_then(|st| {
            if xs_sizing.is_some() { return None; }
            let cd = st.tuning.reentry_cooldown_secs;
            if cd == 0 { return None; }
            st.finance.last_close_at.read().ok()
                .and_then(|m| m.get(symbol).copied())
                .map(|t| t.elapsed().as_secs())
                .filter(|elapsed| *elapsed < cd)
                .map(|elapsed| (elapsed, cd))
        });
        if let Some((elapsed, cd)) = cooldown_block {
            if log_throttle_should_emit(symbol, "reentry_cooldown", 30) {
                push_state_log(state, format!(
                    "⏳ {} açılış atlandı: re-entry cooldown ({}/{}sn)", symbol, elapsed, cd,
                ));
            }
            return;
        }

        let is_long = matches!(signal, Signal::Buy);

        // 🛡️ Spot = long-only: spot piyasada short (Sell) mekanik olarak imkânsız
        // (borrow yok) → canlıda emir reddedilir, paper'da gerçeği yanlış modeller.
        // Tek choke-point (scalp+strateji) → yanlış config'e (TRADE_MARKET=spot iken
        // REGIME_DIRECTIONAL Sell üretmesi) karşı da korur. Futures/coinm'de serbest.
        // Market sınıflandırması Market::from_label tek-kaynağından (string serpme yok).
        if !is_long {
            let block = state.lock().ok().map(|st| (
                !crate::core::types::Market::from_label(&st.config.market).allows_short(),
                st.tuning.risk_block_log_cooldown_secs,
            ));
            if let Some((true, cooldown)) = block {
                if log_throttle_should_emit(symbol, "spot_short_block", cooldown) {
                    push_state_log(state, format!(
                        "🚫 {} SHORT açılış atlandı: spot piyasa long-only (borrow yok)", symbol,
                    ));
                }
                return;
            }
        }

        // 🔒 Sembol-tekliği + eş-zamanlı tavan — TEK lock skopu (atomik), yan-etki ÖNCESİ.
        // (1) AlreadyOpen: aynı sembolde zaten açık pozisyon varsa açma. live_positions
        //     sembol-anahtarlı (HashMap<String,_>) → ikinci insert eskisini SESSİZCE ezerdi
        //     (orphan pozisyon + PnL/closed_trades muhasebe kaybı). Gerçek bug deseni: Regular
        //     strateji pozisyonu açıkken scalp/swing aynı sembolü açıyordu — SlotGuard kind=None'ı
        //     saymadığı için engellemiyordu (AAVE: SUPERTREND→SCP, BCH: SUPERTREND→SCP→SWG).
        //     Guard choke-point'te → her iki açılış yolunu da kapatır. Kapanış sembolü
        //     live_positions'tan sildiği için meşru "kapat→yeniden aç" akışı bozulmaz.
        // (2) CapFull: eş-zamanlı açık pozisyon tavanı (max_concurrent_longs/shorts) — uçuş
        //     rezervasyonu. execute_trade_cycle sembolleri PARALEL açtığından yalnız
        //     live_positions saymak race'li; state-lock altında [açık(yön)+uçuştaki] sayılır,
        //     dolu → kısa-devre, değilse rezervasyon +1 ve RAII guard çıkışta -1. cap=0 →
        //     sınırsız. Log cooldown'ı RISK_BLOCK kardeşiyle aynı knob (risk_block_log_cooldown_secs).
        enum OpenGate {
            Ready(Option<OpenSlotGuard>),
            AlreadyOpen(u64),
            CapFull(u32, u32, u64),
        }
        let gate = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let log_cd = st.tuning.risk_block_log_cooldown_secs;
            let already_open = st.finance.live_positions.read()
                .map(|m| m.contains_key(symbol)).unwrap_or(false);
            if already_open {
                OpenGate::AlreadyOpen(log_cd)
            } else {
                let cap = if is_long { st.tuning.max_concurrent_longs } else { st.tuning.max_concurrent_shorts };
                if cap == 0 {
                    OpenGate::Ready(None)
                } else {
                    let counter = if is_long { &st.finance.pending_open_long } else { &st.finance.pending_open_short };
                    let open_dir = st.finance.live_positions.read()
                        .map(|m| m.values().filter(|p| p.is_long == is_long).count() as u32)
                        .unwrap_or(0);
                    let pending = counter.load(std::sync::atomic::Ordering::Relaxed);
                    if concurrency_cap_reached(open_dir, pending, cap) {
                        OpenGate::CapFull(open_dir + pending, cap, log_cd)
                    } else {
                        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        OpenGate::Ready(Some(OpenSlotGuard(std::sync::Arc::clone(counter))))
                    }
                }
            }
        };
        let _open_slot = match gate {
            OpenGate::Ready(slot) => slot,
            OpenGate::AlreadyOpen(log_cd) => {
                if log_throttle_should_emit(symbol, "symbol_already_open", log_cd) {
                    push_state_log(state, format!(
                        "↩️ {} açılış atlandı: sembolde zaten açık pozisyon var (sembol başına tek)",
                        symbol,
                    ));
                }
                return;
            }
            OpenGate::CapFull(total, cap, log_cd) => {
                if log_throttle_should_emit(symbol, "concurrency_cap", log_cd) {
                    push_state_log(state, format!(
                        "🔢 {} {} açılış atlandı: eş-zamanlı tavan dolu ({}/{})",
                        symbol, if is_long { "LONG" } else { "SHORT" }, total, cap,
                    ));
                }
                return;
            }
        };

        // Entry fiyatı: önce st.fleet.live_price (price_poll 5sn REST snapshot),
        // yoksa candles.last().close (DB son mum). Sadece candle close kullanılınca
        // DB 15dk eski olduğunda gerçek market ile uçurum açıldı → pozisyon eski
        // candle entry'siyle açılıyor, mark-to-market hemen TP'ye çarpıyor (entry
        // ile gerçek fiyat arasındaki fark TP eşiğini aştığı için) → phantom kazanç
        // döngüsü: aç @ stale → kapat @ TP → tekrar aç → tekrar TP. Equity sahte şişer.
        let candle_close = last_candle.close;
        // Entry fiyatı + price-sanity eşikleri tek lock skopunda okunur (eşikler
        // RuntimeTuning'den → per-open getenv yok).
        let (entry, max_dev_pct, candle_freshness_secs) = {
            match state.lock() {
                Ok(st) => {
                    let e = st.fleet.live_price.read().ok()
                        .and_then(|m| m.get(symbol).copied())
                        .filter(|&v| v > 0.0)
                        .unwrap_or(candle_close);
                    (e, st.tuning.max_entry_price_dev_pct, st.tuning.candle_freshness_secs)
                }
                Err(_) => {
                    let d = RuntimeTuning::default();
                    (candle_close, d.max_entry_price_dev_pct, d.candle_freshness_secs)
                }
            }
        };
        // 🛡️ PRICE SANITY GUARD: live_price ile DB son mum kapanışı arasındaki
        // sapma eşiği aşarsa pozisyon açma. BTCUSDC örneği (24 saatlik canlı):
        // live_price stale 87840.60 ↔ fresh 74749.18 arası salınıyordu →
        // OPEN @ stale, CLOSE @ fresh → tek trade'de sahte +$58 PnL.
        // Threshold default %5; env `MAX_ENTRY_PRICE_DEVIATION_PCT` ile ayarlanır,
        // 0 verilirse guard kapanır.
        //
        // Önemli: candle'ın kendisi stale ise (DB günlerce eski) candle.close
        // referans olamaz → guard pas geçer, live_price tek doğru kaynak.
        // Eşik (candle_freshness_secs, max_dev_pct) RuntimeTuning'den geldi.
        let candle_fresh = candle_is_fresh_within(&last_candle.timestamp, candle_freshness_secs);
        if candle_fresh && price_deviation_exceeds(entry, candle_close, max_dev_pct) {
            let dev_pct = price_deviation_pct(entry, candle_close);
            if let Ok(mut st) = state.lock() {
                st.push_log(format!(
                    "🚫 {} açılış reddedildi: entry ${:.4} ↔ candle ${:.4} sapması %{:.2} > %{:.2} (stale price)",
                    symbol, entry, candle_close, dev_pct, max_dev_pct,
                ));
                if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                    use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity};
                    pipe.push_anomaly(
                        AnomalySeverity::Warning,
                        AnomalyKind::Custom,
                        format!(
                            "price-sanity: {} entry={:.4} candle={:.4} dev=%{:.2}",
                            symbol, entry, candle_close, dev_pct,
                        ),
                    );
                }
            }
            return;
        }
        let atr = Self::calc_atr(candles, 14);
        let regime = Self::classify_regime(candles);
        let pos_id = crate::core::types::PositionId::new();
        let pos_id_str = pos_id.to_string();

        // Tüm sync hesap + state okumaları tek mutex skopunda — guard async sınırını geçemez.
        struct OpenPlan {
            new_pos: PositionModel,
            alloc_capital: f64,
            /// Kademeli giriş tam hedef sermayesi (Some → açılış 1. kademe; ek kademeler bundan boyutlanır).
            graded_target: Option<f64>,
            qty_val: f64,
            kelly_fraction: f64,
            risk_appetite: f64,
            ml_conf: f64,
            tp_pct: f64,
            sl_pct: f64,
            strategy_name: String,
            live_executor: Option<Arc<crate::robot::engines::binance_executor::BinanceFuturesExecutor>>,
            live_dry_run: bool,
            live_max_notional: f64,
            atr_mult: f64,
            use_limit_entry: bool,
            limit_entry_timeout_ms: u64,
            limit_entry_max_attempts: u32,
            limit_entry_max_spread_bps: f64,
            limit_entry_fallback_market: bool,
        }
        let plan: Option<OpenPlan> = {
            let mut st = state.lock().unwrap();
            st.fleet.phase = "Executing".into();

            let risk_appetite = st.finance.calculate_risk_appetite();
            let ml_conf = st.brain.ml_confidence;
            // Kelly pencereleri env-ayarlı (RuntimeTuning); closure'lardan önce lokal'e al.
            let kelly_loss_window = st.tuning.kelly_loss_streak_window;
            let kelly_stats_window = st.tuning.kelly_stats_window;
            let loss_streak = st.finance.live_closed_trades.read()
                .map(|tr| tr.iter().rev().take(kelly_loss_window).filter(|t| t.pnl < 0.0).count())
                .unwrap_or(0);
            let (wins, losses, sum_win, sum_loss) = st.finance.live_closed_trades.read().map(|tr| {
                let mut w = 0u32; let mut l = 0u32; let mut sw = 0.0f64; let mut sl = 0.0f64;
                for t in tr.iter().rev().take(kelly_stats_window) {
                    if t.pnl > 0.0 { w += 1; sw += t.pnl; }
                    else if t.pnl < 0.0 { l += 1; sl += -t.pnl; }
                }
                (w, l, sw, sl)
            }).unwrap_or((0, 0, 0.0, 0.0));
            let total = (wins + losses) as f64;
            let win_prob = if total > 0.0 { wins as f64 / total } else { 0.5 };
            let avg_win = if wins > 0 { sum_win / wins as f64 } else { 1.0 };
            let avg_loss = if losses > 0 { sum_loss / losses as f64 } else { 1.0 };
            let kelly = KellyCriterion::calculate(win_prob, avg_win, avg_loss);

            let base_alloc = st.finance.equity * st.tuning.base_alloc_fraction * risk_appetite;
            // EŞİT-AĞIRLIK (kesitsel mod): xs_sizing=Some → her bacak equity·alloc_frac (Kelly atlanır,
            // market-nötr kitabın 1/k dengesi korunur). None → Kelly dinamik ölçek (mevcut davranış).
            let full_alloc = match xs_sizing {
                Some(s) => (st.finance.equity * s.alloc_frac).max(0.0),
                None => kelly.calculate_dynamic_scale(base_alloc, loss_streak, ml_conf)
                    .max(base_alloc * st.tuning.alloc_floor_fraction),
            };
            // KADEMELİ GİRİŞ (XS hariç): açılış yalnız İLK kademe (full·weight[0]); kalan kademeler
            // try_add_graded_tranche ile rejime-göre (pyramiding/averaging) eklenir. graded_target = tam
            // hedef sermaye (ek kademe boyutu bundan türetilir). xs_sizing=Some (kesitsel) → uygulanmaz.
            let graded = st.brain.parameters.read().ok().map(|p| p.graded_entry.clone());
            let graded_target: Option<f64> = match (&xs_sizing, &graded) {
                (None, Some(g)) if g.enabled && g.tranche_count() >= 2 => Some(full_alloc),
                _ => None,
            };
            let alloc_capital = match (graded_target, &graded) {
                (Some(target), Some(g)) => (target * g.weight_at(0)).max(0.0),
                _ => full_alloc,
            };
            let qty_val = (alloc_capital / entry).max(0.0);
            if qty_val <= 0.0 { return; }

            // Faz 2 c4: TP/SL artık rejim-bazlı override'a açık. Store'da o rejim için
            // patch varsa onun trade_risk'i, yoksa base trade_risk kullanılır.
            // HyperOpt rejim-aware tuning yaptıkça (Faz 3'te) patch'leri besleyecek.
            let (tp_pct, sl_pct) = st.brain.parameters.read()
                .map(|p| {
                    let tr = p.trade_risk_for(regime.as_str());
                    (tr.take_profit_pct, tr.stop_loss_pct)
                })
                .unwrap_or((st.tuning.fallback_tp_pct, st.tuning.fallback_sl_pct));
            let sl_pct = sl_pct.max(0.1);
            // LET_WINNERS_RUN: sabit TP'yi çok uzağa it (≥%50) → kâr çıkışını ATR
            // trailing yönetir. Geçerli pozitif bir TP kalır (live koruma emri bozulmaz)
            // ama normal hareketlerde tetiklenmez. Backtest: HTF'de net pozitif (opt-in).
            let tp_pct = if st.tuning.let_winners_run {
                tp_pct.max(50.0)
            } else {
                tp_pct.max(0.1)
            };
            let (stop_loss, take_profit) = if is_long {
                (entry * (1.0 - sl_pct / 100.0), entry * (1.0 + tp_pct / 100.0))
            } else {
                (entry * (1.0 + sl_pct / 100.0), entry * (1.0 - tp_pct / 100.0))
            };
            // ATR-trail mult: sembol×interval noise floor + strateji niyetine bağlı target_pct.
            // Aynı resolve zinciri check_exit_conditions ile birebir → open/exit tutarlı.
            let default_mult = st.brain.best_params.get("pos_atr_trail_mult").copied().unwrap_or(2.0);
            let interval_for_resolve = st.config.interval.clone();
            // Rejim-farkında: per-rejim trail A/B hedefi target_trail_pct_resolved'a girer.
            let atr_mult = st.brain.parameters.read().ok()
                .map(|p| p.resolve_atr_mult_for_regime(
                    symbol, &interval_for_resolve, strategy_name, default_mult, Some(regime.as_str())))
                .unwrap_or(default_mult);
            let trailing_stop = if is_long { entry - atr * atr_mult }
                                else       { entry + atr * atr_mult };
            // 🧊 KESİTSEL STOPSUZ: XS pozisyonları per-bacak stop KULLANMAZ — risk rank-rebalance +
            // rejim-gate + portföy devre kesici ile yönetilir (tek-bacak stop market-nötr dengeyi
            // bozar). Bu döngü XS'i zaten enforce etmiyordu; seviyeleri 0'la → TUI/bot/snapshot "—"
            // gösterir (yanıltıcı ÖLÜ rakam olmaz). XS çıkışı (StrategySignal) live fiyat kullanır,
            // stop_loss'a bağlı değil → sıfırlama güvenli. [[project_xs_momentum]]
            let (stop_loss, take_profit, trailing_stop) = if xs_sizing.is_some() {
                (0.0, 0.0, 0.0)
            } else {
                (stop_loss, take_profit, trailing_stop)
            };
            // Otonom leverage: ParameterStore.resolve_leverage rejim/conf/win_rate/noise
            // ağırlıklı bir değer döndürür. LEVERAGE_ENABLED=false (default) ise 1.0
            // → spot davranış. Stats yoksa noise faktörü None ile devre dışı.
            let noise_floor_opt = st.brain.parameters.read().ok().and_then(|p| {
                p.symbol_stats.get(&(symbol.to_string(), interval_for_resolve.clone()))
                    .map(|s| s.noise_floor_pct)
            });
            // KESİTSEL: sabit kaldıraç (rejim-değişken resolve_leverage bypass; marjinal nötr edge'de
            // mütevazı L — anlamlılık L-invariant). None → otonom resolve_leverage (mevcut davranış).
            let leverage_resolved = match xs_sizing {
                Some(s) => s.leverage.max(1.0),
                None => st.brain.parameters.read().ok()
                    .map(|p| p.resolve_leverage(regime.as_str(), ml_conf, win_prob, noise_floor_opt))
                    .unwrap_or(1.0),
            };
            // strategy_name caller'dan geliyor — process_symbol_cycle StrategySelector ile
            // rejime göre seçti (SUPERTREND / BB / MA_CROSSOVER vb.). trade_type bunu mühürler;
            // check_exit_conditions açılışla aynı target_pct'i okuyabilsin diye.
            let new_pos = PositionModel {
                pos_id: pos_id_str.clone(),
                symbol: symbol.to_string(),
                entry_price: entry, current_price: entry,
                qty: qty_val, leverage: leverage_resolved,
                market: st.config.market.clone(),
                // Pozisyonun GERÇEK trade TF'i: mumlar per-symbol interval_c'den geldi
                // (loop_core symbol_interval'i okur; seed Fix A 1d set edebilir). config.interval
                // (global) DEĞİL — candle serisinin kendi interval'ı tek-doğru kaynak. Boşsa
                // (eski/defaultlı candle) global'e düş.
                interval: {
                    let civ = last_candle.interval.clone();
                    if civ.is_empty() { st.config.interval.clone() } else { civ }
                },
                // trade_type artık stratejik etiket (önceki "LONG"/"SHORT" zaten
                // is_long ile aynı bilgiyi tekrar ediyordu); UI "Strateji" sütununda
                // hangi karar mekanizmasının açtığını göstersin.
                trade_type: strategy_name.to_string(),
                is_long,
                opened_at: chrono::Utc::now().to_rfc3339(),
                stop_loss, take_profit, trailing_stop,
                max_favorable_price: entry,
                breakeven_activated: false,
                // A1+A2: ScalpSwing kanal mührü. None → Regular (klasik strateji
                // yolu), Some(Scalp/Swing) → try_open_scalp_swing dispatch'i.
                // close_paper_position kapanışta bu alanı okuyup
                // ScalpSwingStatsTable'a kanal-bazlı kayıt geçiyor.
                kind,
                entry_commission: 0.0, // açılışta gerçek komisyon hesaplanınca mühürlenir (aşağıda)
            };
            Some(OpenPlan {
                new_pos, alloc_capital, graded_target, qty_val,
                kelly_fraction: kelly.kelly_fraction, risk_appetite, ml_conf,
                tp_pct, sl_pct, strategy_name: strategy_name.to_string(),
                live_executor: st.live_executor.clone(),
                live_dry_run: st.live_dry_run,
                live_max_notional: st.live_max_notional_usd,
                atr_mult,
                use_limit_entry: st.tuning.use_limit_entry,
                limit_entry_timeout_ms: st.tuning.limit_entry_timeout_ms,
                limit_entry_max_attempts: st.tuning.limit_entry_max_attempts,
                limit_entry_max_spread_bps: st.tuning.limit_entry_max_spread_bps,
                limit_entry_fallback_market: st.tuning.limit_entry_fallback_market,
            })
        }; // st burada drop
        let plan = match plan { Some(p) => p, None => return };

        // 💱 LIVE Mode dispatcher (3 koşullu onay zinciri):
        let live_executor = plan.live_executor.clone();
        let live_dry_run = plan.live_dry_run;
        let live_max_notional = plan.live_max_notional;
        let alloc_capital = plan.alloc_capital;
        let qty_val = plan.qty_val;
        let new_pos = plan.new_pos.clone();
        let kelly_fraction = plan.kelly_fraction;
        let risk_appetite = plan.risk_appetite;
        let ml_conf = plan.ml_conf;
        let tp_pct = plan.tp_pct;
        let sl_pct = plan.sl_pct;
        let strategy_name = plan.strategy_name.clone();
        let atr_mult = plan.atr_mult;
        let use_limit_entry = plan.use_limit_entry;
        let limit_entry_timeout_ms = plan.limit_entry_timeout_ms;
        let limit_entry_max_attempts = plan.limit_entry_max_attempts;
        let limit_entry_max_spread_bps = plan.limit_entry_max_spread_bps;
        let limit_entry_fallback_market = plan.limit_entry_fallback_market;

        let mut live_order_id: Option<String> = None;
        // Giriş gerçekten maker (POST_ONLY) dolumuyla mı gerçekleşti? Komisyon
        // muhasebesi (maker vs taker oranı) ve log etiketi buna göre seçilir.
        let mut used_maker = false;
        // Filtre sonrası qty ve SL/TP fiyatları burada güncellenir; local pozisyon
        // (new_pos) borsaya gönderilen değerle birebir eşleşsin diye mutable.
        let mut qty_val = qty_val;
        let mut new_pos = new_pos;
        if let Some(executor) = live_executor.as_ref() {
            let side = if is_long { "BUY" } else { "SELL" };
            if alloc_capital > live_max_notional {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🛑 LIVE veto: notional ${:.2} > tavan ${:.2} ({} {} iptal edildi)",
                        alloc_capital, live_max_notional, symbol, side,
                    ));
                }
                return;
            }

            // 🧮 ExchangeInfo filtre kontrolü (LOT_SIZE / MIN_NOTIONAL / PRICE_FILTER).
            // qty stepSize'a aşağı yuvarlanır, qty*price minNotional altındaysa emir
            // gönderilmeden veto edilir → Binance -1013 reddini önler.
            match executor.apply_filters(symbol, qty_val, entry).await {
                Ok(rounded) => {
                    if (rounded - qty_val).abs() > f64::EPSILON {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "🧮 [LIVE-FILTER] {} qty {:.8} → {:.8} (stepSize'a yuvarlandı)",
                                symbol, qty_val, rounded,
                            ));
                        }
                        qty_val = rounded;
                        new_pos.qty = rounded;
                    }
                    if let Ok(map) = executor.filters.read() {
                        if let Some(f) = map.get(symbol) {
                            let sl_r = f.round_price(new_pos.stop_loss);
                            let tp_r = f.round_price(new_pos.take_profit);
                            new_pos.stop_loss = sl_r;
                            new_pos.take_profit = tp_r;
                            new_pos.trailing_stop = f.round_price(new_pos.trailing_stop);
                        }
                    }
                }
                Err(reason) => {
                    if let Ok(mut st2) = state.lock() {
                        st2.push_log(format!(
                            "🛑 [LIVE-FILTER-VETO] {} {} reddedildi: {}",
                            symbol, side, reason,
                        ));
                        st2.guardian.repair_log.push_back(format!(
                            "[{}] LOT_SIZE/MIN_NOTIONAL veto: {} {} ({})",
                            chrono::Local::now().format("%H:%M:%S"), symbol, side, reason,
                        ));
                        while st2.guardian.repair_log.len() > 100 { st2.guardian.repair_log.pop_front(); }
                    }
                    return;
                }
            }

            if live_dry_run {
                if let Ok(mut st2) = state.lock() {
                    let mode = if use_limit_entry { "LIMIT-MAKER" } else { "MARKET" };
                    st2.push_log(format!(
                        "🟡 [LIVE-DRY-RUN] {} {} {:.4} @ {:.2} (${:.2}) [{}] → emir gönderilmedi",
                        symbol, side, qty_val, entry, alloc_capital, mode,
                    ));
                }
            } else {
                // ── Giriş emri: maker (opt-in, POST_ONLY) → taker MARKET dispatch ──
                // use_limit_entry ise önce best_bid/ask'e katılan maker denenir; N
                // deneme dolmazsa limit_entry_fallback_market'a göre taker'a düşülür
                // ya da trade atlanır. Kapalıysa eski davranış (doğrudan MARKET).
                // Dönüş: dolmuş emir Value'su (orderId + maker yolunda avgPrice).
                let entry_resp: std::result::Result<serde_json::Value, ()> = if use_limit_entry {
                    match executor.place_smart_limit_entry(
                        symbol, side, qty_val,
                        limit_entry_timeout_ms, limit_entry_max_attempts, limit_entry_max_spread_bps,
                    ).await {
                        Ok(r) => { used_maker = true; Ok(r) }
                        Err(e) => {
                            if limit_entry_fallback_market {
                                if let Ok(mut st2) = state.lock() {
                                    st2.push_log(format!(
                                        "↩️ [LIVE-MAKER→MARKET] {} {} maker dolmadı ({:?}) → taker fallback",
                                        symbol, side, e,
                                    ));
                                }
                                executor.place_market_order(symbol, side, qty_val).await.map_err(|me| {
                                    if let Ok(mut st2) = state.lock() {
                                        st2.push_log(format!(
                                            "❌ [LIVE] {} {} fallback market hatası: {:?} — pozisyon kaydedilmedi",
                                            symbol, side, me,
                                        ));
                                    }
                                })
                            } else {
                                if let Ok(mut st2) = state.lock() {
                                    st2.push_log(format!(
                                        "⏭️ [LIVE-MAKER] {} {} maker dolmadı ({:?}) → trade atlandı (fallback kapalı)",
                                        symbol, side, e,
                                    ));
                                }
                                Err(())
                            }
                        }
                    }
                } else {
                    executor.place_market_order(symbol, side, qty_val).await.map_err(|e| {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "❌ [LIVE] {} {} emir hatası: {:?} — pozisyon kaydedilmedi",
                                symbol, side, e,
                            ));
                        }
                    })
                };

                // Giriş emri başarısız (ve fallback yok / fallback de patladı) → paper'ı da çalıştırma.
                let resp = match entry_resp { Ok(r) => r, Err(()) => return };
                live_order_id = resp.get("orderId").map(|v| v.to_string());

                // 🧮 Maker dolum reconciliation: gerçek fill fiyatı entry'den saparsa
                // pozisyonu, SL/TP'yi ve trailing'i fill fiyatından yeniden hesapla
                // (PnL ve borsa koruma emirleri gerçeğe otursun). Market yolunda futures
                // yanıtı avgPrice taşımayabilir → entry referansı korunur (eski davranış).
                if used_maker {
                    let fill_price = resp.get("avgPrice")
                        .and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok())
                        .filter(|&p| p > 0.0)
                        .unwrap_or(entry);
                    if (fill_price - entry).abs() > f64::EPSILON {
                        new_pos.entry_price = fill_price;
                        new_pos.current_price = fill_price;
                        new_pos.max_favorable_price = fill_price;
                        new_pos.stop_loss = if is_long { fill_price * (1.0 - sl_pct / 100.0) }
                                            else        { fill_price * (1.0 + sl_pct / 100.0) };
                        new_pos.take_profit = if is_long { fill_price * (1.0 + tp_pct / 100.0) }
                                              else        { fill_price * (1.0 - tp_pct / 100.0) };
                        new_pos.trailing_stop = if is_long { fill_price - atr * atr_mult }
                                                else        { fill_price + atr * atr_mult };
                        if let Ok(map) = executor.filters.read() {
                            if let Some(f) = map.get(symbol) {
                                new_pos.stop_loss = f.round_price(new_pos.stop_loss);
                                new_pos.take_profit = f.round_price(new_pos.take_profit);
                                new_pos.trailing_stop = f.round_price(new_pos.trailing_stop);
                            }
                        }
                    }
                }

                let live_label = if used_maker { "LIVE-MAKER" } else { "LIVE" };
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "💱 [{}] {} {} {:.4} @ {:.2} (${:.2}) ✓ order={}",
                        live_label, symbol, side, qty_val, new_pos.entry_price, alloc_capital,
                        live_order_id.as_deref().unwrap_or("?"),
                    ));
                }

                // 🛡️ Borsa-tarafı koruma: SL ve TP emirlerini hemen yerleştir.
                // Bot ölse / network kopsa bile pozisyon korumalı kalır.
                let pos_sl = new_pos.stop_loss;
                let pos_tp = new_pos.take_profit;
                // 🧊 STOPSUZ POZİSYON (XS: SL=TP=0) → borsa koruma emri ATLA. Riski kitap-düzeyinde
                // (rank-rebalance + devre kesici + kitap take-profit) yönetilir. stopPrice=0 emri borsa
                // tarafından reddedilir → sl_res=Err → emergency-close zinciri tetiklenir → her XS açılışı
                // anında kapanır + Critical alarm spam'i + komisyon churn'ü. Stopsuzsa koruma yolu komple
                // baypas edilir; live_orders'a yalnız entry id mühürlenir (kapanışta cancel_all fallback).
                // [[project_xs_momentum]]
                if pos_sl <= 0.0 && pos_tp <= 0.0 {
                    if let Ok(mut st2) = state.lock() {
                        if let Ok(mut map) = st2.finance.live_orders.write() {
                            map.insert(symbol.to_string(), crate::core::model::LiveOrderRefs {
                                entry_order_id: live_order_id.clone(),
                                sl_order_id: None,
                                tp_order_id: None,
                                placed_at: chrono::Utc::now().to_rfc3339(),
                            });
                        }
                        st2.push_log(format!(
                            "🛡️ [LIVE-PROTECT] {} stopsuz pozisyon → borsa SL/TP atlandı (risk kitap-düzeyinde)",
                            symbol,
                        ));
                    }
                } else {
                    let (sl_res, tp_res) = executor.place_protection_orders(
                        symbol, is_long, qty_val, pos_sl, pos_tp,
                    ).await;
                    let sl_id = sl_res.as_ref().ok()
                        .and_then(|r| r.get("orderId").map(|v| v.to_string()));
                    let tp_id = tp_res.as_ref().ok()
                        .and_then(|r| r.get("orderId").map(|v| v.to_string()));
                    let sl_status = match &sl_res {
                        Ok(_)  => format!("SL ✓ ({})", sl_id.as_deref().unwrap_or("?")),
                        Err(e) => format!("SL ❌ {:?}", e),
                    };
                    let tp_status = match &tp_res {
                        Ok(_)  => format!("TP ✓ ({})", tp_id.as_deref().unwrap_or("?")),
                        Err(e) => format!("TP ❌ {:?}", e),
                    };
                    // Order ID eşlemesini state'e mühürle (cancel için audit trail).
                    if let Ok(mut st2) = state.lock() {
                        if let Ok(mut map) = st2.finance.live_orders.write() {
                            map.insert(symbol.to_string(), crate::core::model::LiveOrderRefs {
                                entry_order_id: live_order_id.clone(),
                                sl_order_id: sl_id.clone(),
                                tp_order_id: tp_id.clone(),
                                placed_at: chrono::Utc::now().to_rfc3339(),
                            });
                        }
                        st2.push_log(format!(
                            "🛡️ [LIVE-PROTECT] {} @ SL={:.4} TP={:.4} · {} · {}",
                            symbol, pos_sl, pos_tp, sl_status, tp_status,
                        ));
                    }

                    // Kritik uyarı: SL emri BEKLENİYORDU (pos_sl>0) ama başarısız → pozisyon korumasız → emergency.
                    if pos_sl > 0.0 && sl_res.is_err() {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_alert(
                                "LIVE-EMERGENCY",
                                crate::robot::infra::telegram_notifier::Severity::Critical,
                                format!(
                                    "[LIVE-EMERGENCY] {} SL emri verilemedi → pozisyon acil kapatılıyor",
                                    symbol,
                                ),
                            );
                            st2.guardian.repair_log.push_back(format!(
                                "[{}] live SL hatası: {} emergency close",
                                chrono::Local::now().format("%H:%M:%S"), symbol,
                            ));
                            while st2.guardian.repair_log.len() > 100 { st2.guardian.repair_log.pop_front(); }
                        }
                        // Hemen pozisyonu kapat — koruma sağlanamadı.
                        let _ = executor.close_position(symbol).await;
                        return;
                    }
                }
            }
        }

        // Mutex'i geri al
        let mut st = state.lock().unwrap();

        let new_pos_for_log = new_pos.clone();
        {
            if let Ok(mut positions) = st.finance.live_positions.write() {
                positions.insert(symbol.to_string(), new_pos);
            }
        }
        // 🪜 Kademeli giriş durumu: açılış = 1. kademe (sayaç=1); tam hedef sermaye saklanır → ek
        // kademeler try_add_graded_tranche'de target·weight[k] ile boyutlanır. None → kademeli kapalı.
        if let Some(target) = plan.graded_target {
            if let Ok(mut m) = st.finance.graded_tranches.write() {
                m.insert(symbol.to_string(), crate::robot::robotic_loop::GradedPosState {
                    tranches_filled: 1, target_capital: target,
                });
            }
        }

        // Açılış komisyonu (0.1%): hem cost tracker'a yazılır HEM equity'den düşülür.
        // Önceki davranış sadece tracker'a yazıyordu → equity her trade'de açılış
        // komisyonu kadar abartılıyordu (kapanışta exit_commission düşülüyor ama
        // entry hiç düşmüyordu = asimetri). Bu, paper performansını gerçeğinden
        // yüksek gösteriyordu ve "fee sonrası gerçekten karlı mı?" sorusunu maskeliyordu.
        // Artık entry/exit komisyon muhasebesi simetrik.
        // Maker dolumda (POST_ONLY giriş) taker'dan düşük maker_commission_rate uygulanır;
        // taker/market girişte normal commission_rate. used_maker yalnız gerçek maker
        // dolumunda true (fallback-market → false).
        // KESİTSEL maker icra: XS pozisyonları (strategy_name=XS tag) maker için TASARLANDI
        // (net edge maker 2bps'te doğrulandı). Operatör USE_LIMIT_ENTRY ile maker'a opt-in ettiyse
        // paper komisyon muhasebesi de maker oranını yansıtsın → P&L doğrulanan senaryoya sadık.
        let xs_maker = (strategy_name == crate::robot::engines::master::xs_live::XS_STRATEGY_TAG
            || strategy_name == crate::robot::engines::master::carry_live::CARRY_STRATEGY_TAG)
            && st.tuning.use_limit_entry;
        let entry_commission_rate = if used_maker || xs_maker { st.tuning.maker_commission_rate }
                                     else                     { st.tuning.commission_rate };
        let commission = alloc_capital * entry_commission_rate;
        if let Ok(mut costs) = st.finance.live_execution_costs.write() {
            costs.commission_usd += commission;
            costs.total_cost_usd += commission;
            costs.trade_count    += 1;
        }
        st.finance.equity -= commission;
        // Per-trade NET P&L için giriş komisyonunu pozisyona mühürle (kapanışta
        // net_pnl = gross − entry_commission − exit_commission). Pozisyon 760'ta insert edildi.
        if let Ok(mut positions) = st.finance.live_positions.write() {
            if let Some(p) = positions.get_mut(symbol) { p.entry_commission = commission; }
        }

        // IntelligenceHub.track_trade — kapanışta learn_from_exit ile eşleşecek.
        if let Ok(mut hub) = st.brain.intelligence_hub.write() {
            hub.track_trade(pos_id, regime, strategy_name.clone());
        }

        let mode_tag = if live_order_id.is_some() { "LIVE" }
                       else if live_executor.is_some() && live_dry_run { "DRY-RUN" }
                       else { "PAPER" };
        let pos_for_log = new_pos_for_log;
        st.push_log(format!(
            "🚀 [{}-{}] {} açıldı @ {:.2} | Qty={:.4} ${:.2} | SL={:.2} TP={:.2} Trail={:.2} (ATR={:.4} ×{:.1})",
            mode_tag,
            if is_long { "BUY" } else { "SELL" },
            symbol, entry, qty_val, alloc_capital,
            pos_for_log.stop_loss, pos_for_log.take_profit, pos_for_log.trailing_stop, atr, atr_mult,
        ));
        st.push_log(format!(
            "    └─ Kelly f*={:.3} · risk_iştah={:.2} · ML={:.2} · TP%={:.2} SL%={:.2} · Lev={:.1}x · Rejim={} · Strat={}",
            kelly_fraction, risk_appetite, ml_conf, tp_pct, sl_pct,
            pos_for_log.leverage, regime.as_str(), strategy_name,
        ));
        // 💬 Telegram: açılış özeti. Telegram-only — UI log'u yukarıda zaten var. Per-sembol key.
        if let Some(n) = st.notifier.as_ref() {
            n.notify(
                &format!("open-{symbol}"),
                crate::robot::infra::telegram_notifier::Severity::Info,
                &format!("📈 {symbol} {} açıldı @ {:.4} · {} · ×{:.1}",
                    if is_long { "LONG" } else { "SHORT" }, entry, strategy_name, pos_for_log.leverage),
            );
        }
        // 📝 Periyodik dosya logu: TRADE_OPEN. Logger Arc'ını lock altında clone'la,
        // unlock sonrası IO yap.
        let logger_for_event = st.trading_logger.clone();
        let equity_now = st.finance.equity;
        drop(st);
        if let Some(logger) = logger_for_event {
            let ev = crate::robot::infra::logger::TradeEvent::trade_open(
                symbol, &strategy_name, is_long, entry, qty_val, equity_now,
                pos_for_log.leverage,
            );
            let _ = logger.log_event(&ev);
        }
        let _ = live_order_id; // ileride pos_id ↔ order_id eşlemesi için saklanabilir

        // ─── Faz 5 (Execute): pozisyon başarıyla mühürlendi ───────────────
        Self::mark_pipeline_stage(
            state,
            crate::robot::data_pipeline::canon::PipelineStage::Execute,
            crate::robot::data_pipeline::StepStatus::Done,
        );

        // 💾 RECOVERY: pozisyon haritası değişti → DB snapshot'ı güncelle.
        // Crash + restart sonrası hydrate_open_positions_from_db bu durumu okur.
        Self::persist_open_positions_snapshot(state);
        // Equity entry commission ile düştü → account_state'i de mühürle.
        Self::persist_account_state(state);
        // Phase Executing göstergesi için: en son trade epoch'unu kaydet
        // (heartbeat sticky phase okuması bu değere bakar).
        Self::mark_execution_epoch(state);
    }

    /// Sembolün config.blocked_symbols listesinde olup olmadığını döner.
    /// Case-insensitive karşılaştırma; lock alınamazsa false (savunmacı varsayılan
    /// — sistemi blocked-shaped failure'a sokmamak için kapı açık bırakılır).
    pub fn is_symbol_blocked(state: &Arc<Mutex<AppState>>, symbol: &str) -> bool {
        state.lock().ok().map(|st| {
            st.config.blocked_symbols.iter().any(|b| b.eq_ignore_ascii_case(symbol))
        }).unwrap_or(false)
    }
}

/// Eş-zamanlı pozisyon tavanı dolu mu? `cap == 0` → sınırsız (her zaman false).
/// `open_count` = o yönde halihazırda açık, `pending` = uçuştaki rezervasyon.
fn concurrency_cap_reached(open_count: u32, pending: u32, cap: u32) -> bool {
    cap > 0 && open_count + pending >= cap
}

/// Uçuş-rezervasyonu RAII guard'ı: `open_paper_position`'ın TÜM çıkış yollarında
/// (erken-return dahil) pending sayacı otomatik düşürür → eş-zamanlı tavan paralel
/// açılışta da tutar (rezervasyon insert'ten sonra fn dönene dek +1 kalır = konservatif).
struct OpenSlotGuard(std::sync::Arc<std::sync::atomic::AtomicU32>);
impl Drop for OpenSlotGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::{concurrency_cap_reached, OpenSlotGuard};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn cap_reached_logic() {
        assert!(!concurrency_cap_reached(0, 0, 0), "cap=0 → sınırsız");
        assert!(!concurrency_cap_reached(9, 9, 0), "cap=0 → her zaman false");
        assert!(!concurrency_cap_reached(1, 0, 2), "1 açık < 2");
        assert!(concurrency_cap_reached(2, 0, 2), "2 açık = cap → dolu");
        assert!(concurrency_cap_reached(1, 1, 2), "1 açık + 1 uçuşta = cap → dolu");
        assert!(concurrency_cap_reached(3, 0, 2), "tavan aşılmışsa dolu");
    }

    #[test]
    fn guard_releases_reservation_on_drop() {
        let counter = Arc::new(AtomicU32::new(0));
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        {
            let _g = OpenSlotGuard(Arc::clone(&counter));
            assert_eq!(counter.load(Ordering::Relaxed), 1, "guard içindeyken rezervasyon durur");
        }
        assert_eq!(counter.load(Ordering::Relaxed), 0, "drop → rezervasyon serbest");
    }
}
