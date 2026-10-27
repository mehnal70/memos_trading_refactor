// src/robot/engines/master/positions.rs — Pozisyon açılışı (scalp-swing + paper) + blok kapısı.
// Faz 2 modülerleştirme: exit/kapanış yolu positions_close.rs'e ayrıldı (davranış birebir).
use super::*;
use crate::core::model::TradingMode;
use crate::core::types::Exchange;

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
            OpenSlot, ScalpEngine, ScalpSwingConfig, SlotGuard, SwingEngine,
        };

        // 1) cfg + mevcut açık pozisyonların kanal-bazlı slot'larını topla.
        //    Hem cfg hem slots tek kısa mutex skopunda alınır.
        let (cfg, existing_slots): (ScalpSwingConfig, Vec<OpenSlot>) = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return false,
            };
            let cfg = match st.brain.scalp_swing_config.read().ok() {
                Some(c) => c.clone(),
                None => return false,
            };
            if !cfg.scalp_enabled && !cfg.swing_enabled {
                return false;
            }
            let slots = st
                .finance
                .live_positions
                .read()
                .ok()
                .map(|m| {
                    m.values()
                        .filter_map(|p| {
                            p.kind.map(|kind| OpenSlot {
                                symbol: p.symbol.clone(),
                                trade_type: kind,
                                is_long: p.is_long,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
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
            MarketRegime::Ranging | MarketRegime::LowVolatility => (true, false),
            MarketRegime::StrongUptrend | MarketRegime::StrongDowntrend => (false, true),
            _ => (true, true),
        };

        let scalp_interval = cfg.scalp_interval.clone();
        let swing_interval = cfg.swing_interval.clone();
        let load_interval_candles = |interval: &str, min_needed: usize| -> Vec<Candle> {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
            let mut bars =
                st.fleet
                    .opportunity_cache
                    .get_latest(symbol, interval, min_needed.max(120));
            if bars.len() < min_needed {
                // Kilit zaten tutuluyor → read_candles_routed (state.lock) DEADLOCK olur;
                // route guard üzerinden yapılır. Global config.market yerine venue namespace.
                let db_bars = match st.venue_registry.route(symbol) {
                    Some((v, bare)) => {
                        let (ex, mk) = v.storage_namespace();
                        crate::persistence::reader::read_candles_ns(
                            &st.config.db_path,
                            bare,
                            interval,
                            ex,
                            mk,
                            min_needed.max(120),
                        )
                        .unwrap_or_default()
                    }
                    None => crate::persistence::reader::read_candles_market(
                        &st.config.db_path,
                        symbol,
                        interval,
                        &st.config.market,
                        min_needed.max(120),
                    )
                    .unwrap_or_default(),
                };
                if db_bars.len() > bars.len() {
                    bars = db_bars;
                }
            }
            bars
        };
        let scalp_candles = load_interval_candles(&scalp_interval, 30);
        let swing_candles = load_interval_candles(&swing_interval, 60);

        let scalp_opp = if cfg.scalp_enabled && scalp_ok {
            ScalpEngine::evaluate(&scalp_candles, cfg.scalp_min_score)
        } else {
            None
        };
        let swing_opp = if cfg.swing_enabled && swing_ok {
            SwingEngine::evaluate(&swing_candles, cfg.swing_min_adx, cfg.swing_min_score)
        } else {
            None
        };

        // 🧠 5m ve 15m fırsat-hafızaları: scalp ve swing için ayrı rolling snapshot bias'ı.
        let (mem5m_snapshot, mem5m_weight, mem15m_snapshot, mem15m_weight) = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return false,
            };

            let mem5m_snapshot = if st.tuning.scalp_swing_mem5m_enable {
                crate::robot::data_pipeline::MemoryOpportunitySnapshot::from_candles(
                    symbol,
                    &scalp_interval,
                    &scalp_candles,
                )
                .filter(|s| s.bars >= st.tuning.scalp_swing_mem5m_min_bars)
            } else {
                None
            };
            let mem15m_snapshot = if st.tuning.scalp_swing_mem15m_enable {
                crate::robot::data_pipeline::MemoryOpportunitySnapshot::from_candles(
                    symbol,
                    &swing_interval,
                    &swing_candles,
                )
                .filter(|s| s.bars >= st.tuning.scalp_swing_mem15m_min_bars)
            } else {
                None
            };

            (
                mem5m_snapshot,
                st.tuning.scalp_swing_mem5m_bias_weight,
                mem15m_snapshot,
                st.tuning.scalp_swing_mem15m_bias_weight,
            )
        };
        let adjusted_score =
            |opp: &crate::robot::scalp_swing::TradeOpportunity,
             snapshot: Option<&crate::robot::data_pipeline::MemoryOpportunitySnapshot>,
             weight: f64|
             -> f64 {
                let bias = snapshot
                    .map(|snap| {
                        let b = match opp.trade_type {
                            crate::robot::scalp_swing::TradeType::Scalp => snap.scalp_bias,
                            crate::robot::scalp_swing::TradeType::Swing => snap.swing_bias,
                            crate::robot::scalp_swing::TradeType::Regular => 0.0,
                        };
                        if opp.is_long {
                            b
                        } else {
                            -b
                        }
                    })
                    .unwrap_or(0.0);
                opp.score + (weight * bias.clamp(-1.0, 1.0))
            };

        // 3) En yüksek skoru seç (Scalp ve Swing eşit ise Scalp önce —
        //    kısa-vade fırsat sermayeyi daha az bağlar).
        let opp = match (scalp_opp, swing_opp) {
            (Some(a), Some(b)) => {
                let a_score = adjusted_score(&a, mem5m_snapshot.as_ref(), mem5m_weight);
                let b_score = adjusted_score(&b, mem15m_snapshot.as_ref(), mem15m_weight);
                if a_score >= b_score {
                    a
                } else {
                    b
                }
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => return false,
        };

        // 🧭 Rejim-yön kapısı (opt-in): ScalpSwing yönü de rejimle hizalı olmalı —
        // regular-strateji yoluyla AYNI kural (regime_confirms_direction, tek-kaynak).
        // Aksi halde StrongUptrend'de SwingEngine short açabiliyordu (ters-trend). Canlı
        // paper doğrulamasında bu boşluk yakalandı. default false → davranış değişmez.
        if regime_directional
            && !crate::robot::logic::market_regime::regime_confirms_direction(regime, opp.is_long)
        {
            if log_throttle_should_emit(symbol, "scalp_regime_dir_block", 60) {
                push_state_log(
                    state,
                    format!(
                        "🧭 ScalpSwing {} {} ⇒ REDDEDİLDİ (rejim-yön teyidi yok, rejim={})",
                        opp.trade_type.label(),
                        if opp.is_long { "BUY" } else { "SELL" },
                        regime.as_str(),
                    ),
                );
            }
            return false;
        }

        // ⚖️ Blend çekirdeği (BLEND_FACTOR) aktifken ScalpSwing yalnız kontrollü overlay olarak
        // açılır: net-delta ve overlay-slot limitleri aşılırsa fırsat reddedilir.
        let blend_overlay_block_reason = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return false,
            };
            let top_k = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| p.blend_live.top_k)
                .unwrap_or(0);
            let (blend_long, blend_short, overlay_long, overlay_short, overlay_open) = st
                .finance
                .live_positions
                .read()
                .ok()
                .map(|m| {
                    let mut bl = 0usize;
                    let mut bs = 0usize;
                    let mut ol = 0usize;
                    let mut os = 0usize;
                    let mut oo = 0usize;
                    for p in m.values() {
                        if p.trade_type == super::blend_live::BLEND_STRATEGY_TAG {
                            if p.is_long {
                                bl += 1;
                            } else {
                                bs += 1;
                            }
                        }
                        if p.kind.is_some() {
                            oo += 1;
                            if p.is_long {
                                ol += 1;
                            } else {
                                os += 1;
                            }
                        }
                    }
                    (bl, bs, ol, os, oo)
                })
                .unwrap_or((0, 0, 0, 0, 0));

            let core_active = top_k > 0 && blend_long >= top_k && blend_short >= top_k;
            if !core_active {
                None
            } else if !st.tuning.blend_overlay_enable {
                Some("blend çekirdeği aktifken overlay kapalı (BLEND_OVERLAY_ENABLE=0)".to_string())
            } else if overlay_open as u32 >= st.tuning.blend_overlay_max_open {
                Some(format!(
                    "blend overlay slot dolu ({}/{})",
                    overlay_open, st.tuning.blend_overlay_max_open
                ))
            } else {
                let signed = |is_long: bool| if is_long { 1 } else { -1 };
                let projected_net_delta = (blend_long as i32 - blend_short as i32)
                    + (overlay_long as i32 - overlay_short as i32)
                    + signed(opp.is_long);
                let max_net = st.tuning.blend_overlay_max_net_delta.max(0);
                if projected_net_delta.abs() > max_net {
                    Some(format!(
                        "blend net-delta limiti aşılıyor (proj={} > lim={})",
                        projected_net_delta.abs(),
                        max_net
                    ))
                } else {
                    None
                }
            }
        };
        if let Some(reason) = blend_overlay_block_reason {
            if log_throttle_should_emit(symbol, "blend_overlay_block", 60) {
                push_state_log(
                    state,
                    format!(
                        "⚖️ ScalpSwing {} {} ⇒ REDDEDİLDİ ({})",
                        opp.trade_type.label(),
                        if opp.is_long { "BUY" } else { "SELL" },
                        reason,
                    ),
                );
            }
            return false;
        }

        // 4) SlotGuard: kanal-bazlı kapasite + hedge engeli.
        let (ok, reason) = SlotGuard::can_open(
            &existing_slots,
            symbol,
            opp.trade_type,
            opp.is_long,
            cfg.max_scalp_per_symbol,
            cfg.max_swing_per_symbol,
        );
        if !ok {
            log::debug!(
                "ScalpSwing {} {} reddedildi: {}",
                opp.trade_type.label(),
                symbol,
                reason,
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
            "{}_{}",
            opp.trade_type.label(),
            if opp.is_long { "BUY" } else { "SELL" },
        );
        // Bu "deneme" logu sembol başına throttle'lanır: fırsat her cycle (500ms)
        // tespit edilip açılış re-entry cooldown / risk ile bloklanınca olay günlüğünü
        // taşırıyordu (84/100 satır). Gerçek açılış zaten open_paper_position'da
        // "🚀 [PAPER-...] açıldı" ile onaylanıyor → bu yalnız bağlam (score/reason).
        if log_throttle_should_emit(symbol, "scalp_open_attempt", 60) {
            push_state_log(
                state,
                format!(
                    "⚡ ScalpSwing {} açılış: {} score={:.2} | {}",
                    opp.trade_type.label(),
                    symbol,
                    opp.score,
                    opp.reason,
                ),
            );
        }
        Self::open_paper_position(
            state,
            symbol,
            &signal,
            candles,
            &strategy_name,
            Some(opp.trade_type),
            None,
        )
        .await; // xs_sizing=None → ScalpSwing Kelly+resolve_leverage (mevcut davranış)
        true
    }

    /// ⚖️ Blend çekirdeği aktifken açık overlay (Scalp/Swing) bacağını gerekirse otomatik kapatır:
    /// - max holding süresi aşıldıysa,
    /// - net-delta limiti ihlalinde dengesiz yöndeki overlay bacağıysa.
    pub(crate) async fn enforce_blend_overlay_rebalance_for_open_symbol(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        candles: &[Candle],
    ) -> bool {
        let decision: Option<String> = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return false,
            };
            let top_k = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| p.blend_live.top_k)
                .unwrap_or(0);
            let p = st
                .finance
                .live_positions
                .read()
                .ok()
                .and_then(|m| m.get(symbol).cloned());
            let p = match p {
                Some(p) => p,
                None => return false,
            };
            if p.kind.is_none() {
                None
            } else {
                let (blend_long, blend_short, overlay_long, overlay_short) = st
                    .finance
                    .live_positions
                    .read()
                    .ok()
                    .map(|m| {
                        let mut bl = 0i32;
                        let mut bs = 0i32;
                        let mut ol = 0i32;
                        let mut os = 0i32;
                        for x in m.values() {
                            if x.trade_type == super::blend_live::BLEND_STRATEGY_TAG {
                                if x.is_long {
                                    bl += 1;
                                } else {
                                    bs += 1;
                                }
                            }
                            if x.kind.is_some() {
                                if x.is_long {
                                    ol += 1;
                                } else {
                                    os += 1;
                                }
                            }
                        }
                        (bl, bs, ol, os)
                    })
                    .unwrap_or((0, 0, 0, 0));
                let core_active =
                    top_k > 0 && blend_long >= top_k as i32 && blend_short >= top_k as i32;
                if !core_active {
                    None
                } else {
                    let now = chrono::Utc::now();
                    let max_hold = st.tuning.blend_overlay_max_holding_secs;
                    if max_hold > 0 {
                        let opened_at = chrono::DateTime::parse_from_rfc3339(&p.opened_at)
                            .ok()
                            .map(|dt| dt.with_timezone(&chrono::Utc));
                        if let Some(ts) = opened_at {
                            let age = (now - ts).num_seconds().max(0) as u64;
                            if age >= max_hold {
                                Some(format!(
                                    "overlay süre limiti doldu ({}sn >= {}sn)",
                                    age, max_hold
                                ))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                    .or_else(|| {
                        let net = (blend_long - blend_short) + (overlay_long - overlay_short);
                        let max_net = st.tuning.blend_overlay_max_net_delta.max(0);
                        if net.abs() > max_net {
                            let reducing = (net > 0 && p.is_long) || (net < 0 && !p.is_long);
                            if reducing {
                                Some(format!(
                                    "overlay net-delta rebalance (|{}| > {})",
                                    net, max_net
                                ))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                }
            }
        };

        if let Some(reason) = decision {
            if log_throttle_should_emit(symbol, "blend_overlay_auto_close", 30) {
                push_state_log(
                    state,
                    format!("⚖️ {} overlay kapatılıyor: {}", symbol, reason,),
                );
            }
            Self::close_paper_position(state, symbol, candles, ExitReason::StrategySignal).await;
            return true;
        }
        false
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
        let last_candle = match candles.last() {
            Some(c) => c,
            None => return,
        };

        // blocked_symbols savunması: config'te listelenen semboller için yeni pozisyon
        // açılışı kısa-devre reddedilir. Mevcut açık pozisyonlar bu engelden etkilenmez
        // (execute_trade_cycle yetim pozisyon kuralı onları yönetmeye devam eder).
        // RİSK HALT kapısı (Adım 4a): risk_gate kritik seviye mühürlediyse yeni pozisyon
        // AÇILMAZ (paper+canlı). Mevcut pozisyonlar execute_trade_cycle ile normal yönetilir.
        // Tek-nokta: her iki açılış yolu (scalp_swing + strateji) buradan geçer.
        if Self::risk_halt_active(state) {
            if log_throttle_should_emit(symbol, "risk_halt_block", 60) {
                push_state_log(
                    state,
                    format!("🛑 {} açılış reddedildi: RISK-HALT aktif", symbol),
                );
            }
            return;
        }
        if let Some(reason) = Self::live_entry_gate(state, symbol) {
            if log_throttle_should_emit(symbol, "live_entry_gate", 300) {
                push_state_log(
                    state,
                    format!("🛑 {} canlı açılış reddedildi: {}", symbol, reason),
                );
            }
            return;
        }
        if Self::is_symbol_blocked(state, symbol) {
            push_state_log(
                state,
                format!(
                    "🚫 {} açılış reddedildi: blocked_symbols listesinde",
                    symbol
                ),
            );
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
            if xs_sizing.is_some() {
                return None;
            }
            let cd = st.tuning.reentry_cooldown_secs;
            if cd == 0 {
                return None;
            }
            st.finance
                .last_close_at
                .read()
                .ok()
                .and_then(|m| m.get(symbol).copied())
                .map(|t| t.elapsed().as_secs())
                .filter(|elapsed| *elapsed < cd)
                .map(|elapsed| (elapsed, cd))
        });
        if let Some((elapsed, cd)) = cooldown_block {
            if log_throttle_should_emit(symbol, "reentry_cooldown", 30) {
                push_state_log(
                    state,
                    format!(
                        "⏳ {} açılış atlandı: re-entry cooldown ({}/{}sn)",
                        symbol, elapsed, cd,
                    ),
                );
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
            let block = state.lock().ok().map(|st| {
                (
                    !crate::core::types::Market::from_label(&st.config.market).allows_short(),
                    st.tuning.risk_block_log_cooldown_secs,
                )
            });
            if let Some((true, cooldown)) = block {
                if log_throttle_should_emit(symbol, "spot_short_block", cooldown) {
                    push_state_log(
                        state,
                        format!(
                            "🚫 {} SHORT açılış atlandı: spot piyasa long-only (borrow yok)",
                            symbol,
                        ),
                    );
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
            VenueCapFull(crate::core::types::Exchange, u32, u32, u64),
            VenueDailyLoss(crate::core::types::Exchange, f64, f64, u64),
        }
        let gate = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            let log_cd = st.tuning.risk_block_log_cooldown_secs;
            let (bare_symbol, tagged_ex) = crate::robot::venue::registry::split_venue_tag(symbol);
            let venue_ex =
                tagged_ex.unwrap_or_else(|| crate::core::types::Exchange::classify(bare_symbol));
            let directional_gate = |st: &AppState| {
                let cap = if is_long {
                    st.tuning.max_concurrent_longs
                } else {
                    st.tuning.max_concurrent_shorts
                };
                if cap == 0 {
                    OpenGate::Ready(None)
                } else {
                    let counter = if is_long {
                        &st.finance.pending_open_long
                    } else {
                        &st.finance.pending_open_short
                    };
                    let open_dir = st
                        .finance
                        .live_positions
                        .read()
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
            };
            let already_open = st
                .finance
                .live_positions
                .read()
                .map(|m| m.contains_key(symbol))
                .unwrap_or(false);
            if already_open {
                OpenGate::AlreadyOpen(log_cd)
            } else if xs_sizing.is_some() {
                // 🔒 KESİTSEL MUAF: market-nötr kitap (XS/carry/blend) k-long+k-short dengesini KENDİ
                // rank-rebalance'ıyla yönetir → genel YÖNLÜ cap (max_concurrent_longs/shorts, genel
                // motorun risk limiti) kitaba uygulanırsa dengesiz bırakır: örn. .env MAX_CONCURRENT_LONGS=1
                // → 1 long + k short = NET-SHORT, market-nötrlük bozulur (2026-06-14 paper bulgusu).
                // reentry cooldown muafiyetiyle (yukarı, satır ~177) simetrik. AlreadyOpen yine geçerli
                // (sembol başına tek-pozisyon korunur). [[project_blend_live_failure]] [[project_xs_momentum]]
                OpenGate::Ready(None)
            } else {
                let venue_cap = st
                    .tuning
                    .venue_max_concurrent_open
                    .get(&venue_ex)
                    .copied()
                    .unwrap_or(0);
                if venue_cap > 0 {
                    let venue_open = st
                        .finance
                        .live_positions
                        .read()
                        .map(|m| {
                            m.values()
                                .filter(|p| {
                                    let (pb, ptag) =
                                        crate::robot::venue::registry::split_venue_tag(&p.symbol);
                                    let px = ptag.unwrap_or_else(|| {
                                        crate::core::types::Exchange::classify(pb)
                                    });
                                    px == venue_ex
                                })
                                .count() as u32
                        })
                        .unwrap_or(0);
                    if venue_open >= venue_cap {
                        OpenGate::VenueCapFull(venue_ex, venue_open, venue_cap, log_cd)
                    } else {
                        let venue_loss_pct = st
                            .tuning
                            .venue_daily_loss_pct
                            .get(&venue_ex)
                            .copied()
                            .unwrap_or(0.0);
                        if venue_loss_pct > 0.0 {
                            let day_prefix = chrono::Utc::now().format("%Y-%m-%d").to_string();
                            let venue_daily_loss = st
                                .finance
                                .live_closed_trades
                                .read()
                                .map(|tr| {
                                    tr.iter()
                                        .filter(|t| {
                                            let (tb, ttag) =
                                                crate::robot::venue::registry::split_venue_tag(
                                                    &t.symbol,
                                                );
                                            let tx = ttag.unwrap_or_else(|| {
                                                crate::core::types::Exchange::classify(tb)
                                            });
                                            tx == venue_ex && t.closed_at.starts_with(&day_prefix)
                                        })
                                        .map(|t| {
                                            let net = if t.net_pnl != 0.0 {
                                                t.net_pnl
                                            } else {
                                                t.pnl - t.commission
                                            };
                                            if net < 0.0 {
                                                -net
                                            } else {
                                                0.0
                                            }
                                        })
                                        .sum::<f64>()
                                })
                                .unwrap_or(0.0);
                            let venue_daily_limit =
                                (st.finance.starting_capital * (venue_loss_pct / 100.0)).max(0.0);
                            if venue_daily_limit > 0.0 && venue_daily_loss >= venue_daily_limit {
                                OpenGate::VenueDailyLoss(
                                    venue_ex,
                                    venue_daily_loss,
                                    venue_daily_limit,
                                    log_cd,
                                )
                            } else {
                                directional_gate(&st)
                            }
                        } else {
                            directional_gate(&st)
                        }
                    }
                } else {
                    directional_gate(&st)
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
                    push_state_log(
                        state,
                        format!(
                            "🔢 {} {} açılış atlandı: eş-zamanlı tavan dolu ({}/{})",
                            symbol,
                            if is_long { "LONG" } else { "SHORT" },
                            total,
                            cap,
                        ),
                    );
                }
                return;
            }
            OpenGate::VenueCapFull(ex, total, cap, log_cd) => {
                if log_throttle_should_emit(symbol, "venue_concurrency_cap", log_cd) {
                    push_state_log(
                        state,
                        format!(
                            "🏛️ {} açılış atlandı: {} venue tavanı dolu ({}/{})",
                            symbol,
                            ex.as_str(),
                            total,
                            cap,
                        ),
                    );
                }
                return;
            }
            OpenGate::VenueDailyLoss(ex, loss, limit, log_cd) => {
                if log_throttle_should_emit(symbol, "venue_daily_loss_cap", log_cd) {
                    push_state_log(
                        state,
                        format!(
                            "📉 {} açılış atlandı: {} günlük zarar limiti aşıldı (${:.2}/${:.2})",
                            symbol,
                            ex.as_str(),
                            loss,
                            limit,
                        ),
                    );
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
        let (entry, max_dev_pct, candle_freshness_secs, live_mode, live_price_age_secs) = {
            match state.lock() {
                Ok(st) => {
                    let e = st
                        .fleet
                        .live_price
                        .read()
                        .ok()
                        .and_then(|m| m.get(symbol).copied())
                        .filter(|&v| v > 0.0)
                        .unwrap_or(candle_close);
                    let price_age = st
                        .fleet
                        .live_price_updated_at
                        .read()
                        .ok()
                        .and_then(|m| m.get(symbol).copied())
                        .map(|timestamp| (chrono::Utc::now() - timestamp).num_seconds());
                    (
                        e,
                        st.tuning.max_entry_price_dev_pct,
                        st.tuning.candle_freshness_secs,
                        matches!(st.config.trading_mode, TradingMode::Live) && !st.live_dry_run,
                        price_age,
                    )
                }
                Err(_) => {
                    let d = RuntimeTuning::default();
                    (
                        candle_close,
                        d.max_entry_price_dev_pct,
                        d.candle_freshness_secs,
                        false,
                        None,
                    )
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
        if live_mode && !candle_fresh {
            push_state_log(
                state,
                format!(
                    "🚫 {} canlı açılış reddedildi: candle verisi stale (izin verilen yaş: {} sn)",
                    symbol, candle_freshness_secs
                ),
            );
            return;
        }
        let live_price_max_age_secs = std::env::var("LIVE_PRICE_MAX_AGE_SECS")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(30);
        if live_mode
            && !matches!(live_price_age_secs, Some(age) if age >= 0 && age <= live_price_max_age_secs)
        {
            push_state_log(
                state,
                format!(
                    "🚫 {} canlı açılış reddedildi: canlı fiyat yok veya stale (yaş={:?} sn, sınır={} sn)",
                    symbol, live_price_age_secs, live_price_max_age_secs
                ),
            );
            return;
        }
        if candle_fresh && price_deviation_exceeds(entry, candle_close, max_dev_pct) {
            let dev_pct = price_deviation_pct(entry, candle_close);
            let anomaly_msg = format!(
                "price-sanity: {} entry={:.4} candle={:.4} dev=%{:.2}",
                symbol, entry, candle_close, dev_pct,
            );
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
                        anomaly_msg.clone(),
                    );
                }
            }
            emit_pipeline_anomaly_event(
                state,
                crate::robot::data_pipeline::AnomalySeverity::Warning,
                crate::robot::data_pipeline::AnomalyKind::Custom,
                &anomaly_msg,
            );
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
            trading_mode: TradingMode,
            real_execution_allowed: bool,
            live_executor: Option<Arc<dyn crate::robot::venue::adapter::VenueAdapter>>,
            live_venue: Option<Arc<dyn crate::robot::venue::adapter::VenueAdapter>>,
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
            let mut st = lock_state_recover(state);
            st.fleet.phase = "Executing".into();

            let risk_appetite = st.finance.calculate_risk_appetite();
            let ml_conf = st.brain.ml_confidence;
            // Kelly pencereleri env-ayarlı (RuntimeTuning); closure'lardan önce lokal'e al.
            let kelly_loss_window = st.tuning.kelly_loss_streak_window;
            let kelly_stats_window = st.tuning.kelly_stats_window;
            let loss_streak = st
                .finance
                .live_closed_trades
                .read()
                .map(|tr| {
                    tr.iter()
                        .rev()
                        .take(kelly_loss_window)
                        .filter(|t| t.pnl < 0.0)
                        .count()
                })
                .unwrap_or(0);
            let (wins, losses, sum_win, sum_loss) = st
                .finance
                .live_closed_trades
                .read()
                .map(|tr| {
                    let mut w = 0u32;
                    let mut l = 0u32;
                    let mut sw = 0.0f64;
                    let mut sl = 0.0f64;
                    for t in tr.iter().rev().take(kelly_stats_window) {
                        if t.pnl > 0.0 {
                            w += 1;
                            sw += t.pnl;
                        } else if t.pnl < 0.0 {
                            l += 1;
                            sl += -t.pnl;
                        }
                    }
                    (w, l, sw, sl)
                })
                .unwrap_or((0, 0, 0.0, 0.0));
            let total = (wins + losses) as f64;
            let win_prob = if total > 0.0 {
                wins as f64 / total
            } else {
                0.5
            };
            let avg_win = if wins > 0 { sum_win / wins as f64 } else { 1.0 };
            let avg_loss = if losses > 0 {
                sum_loss / losses as f64
            } else {
                1.0
            };
            let kelly = KellyCriterion::calculate(win_prob, avg_win, avg_loss);

            // SAFE-MODE çarpanı (Adım 4a): mühür aktifse tahsisat kısılır. Bayrak DOĞRUDAN
            // guard'dan okunur — safe_mode_active() kilit alacağı için burada deadlock olurdu.
            let safe_mult = if st
                .guardian
                .risk_safe_mode_until
                .map(|u| chrono::Utc::now() < u)
                .unwrap_or(false)
            {
                st.tuning.safe_mode_alloc_mult
            } else {
                1.0
            };
            let base_alloc =
                st.finance.equity * st.tuning.base_alloc_fraction * risk_appetite * safe_mult;
            // EŞİT-AĞIRLIK (kesitsel mod): xs_sizing=Some → her bacak equity·alloc_frac (Kelly atlanır,
            // market-nötr kitabın 1/k dengesi korunur). None → Kelly dinamik ölçek (mevcut davranış).
            let full_alloc = match xs_sizing {
                Some(s) => (st.finance.equity * s.alloc_frac).max(0.0),
                None => kelly
                    .calculate_dynamic_scale(base_alloc, loss_streak, ml_conf)
                    .max(base_alloc * st.tuning.alloc_floor_fraction),
            };
            // KADEMELİ GİRİŞ (XS hariç): açılış yalnız İLK kademe (full·weight[0]); kalan kademeler
            // try_add_graded_tranche ile rejime-göre (pyramiding/averaging) eklenir. graded_target = tam
            // hedef sermaye (ek kademe boyutu bundan türetilir). xs_sizing=Some (kesitsel) → uygulanmaz.
            let graded = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| p.graded_entry.clone());
            let graded_target: Option<f64> = match (&xs_sizing, &graded) {
                (None, Some(g)) if g.enabled && g.tranche_count() >= 2 => Some(full_alloc),
                _ => None,
            };
            let alloc_capital = match (graded_target, &graded) {
                (Some(target), Some(g)) => (target * g.weight_at(0)).max(0.0),
                _ => full_alloc,
            };
            let qty_val = (alloc_capital / entry).max(0.0);
            if qty_val <= 0.0 {
                return;
            }

            // Faz 2 c4: TP/SL artık rejim-bazlı override'a açık. Store'da o rejim için
            // patch varsa onun trade_risk'i, yoksa base trade_risk kullanılır.
            // HyperOpt rejim-aware tuning yaptıkça (Faz 3'te) patch'leri besleyecek.
            let (tp_pct, sl_pct) = st
                .brain
                .parameters
                .read()
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
                (
                    entry * (1.0 - sl_pct / 100.0),
                    entry * (1.0 + tp_pct / 100.0),
                )
            } else {
                (
                    entry * (1.0 + sl_pct / 100.0),
                    entry * (1.0 - tp_pct / 100.0),
                )
            };
            // ATR-trail mult: sembol×interval noise floor + strateji niyetine bağlı target_pct.
            // Aynı resolve zinciri check_exit_conditions ile birebir → open/exit tutarlı.
            let default_mult = st
                .brain
                .best_params
                .get("pos_atr_trail_mult")
                .copied()
                .unwrap_or(2.0);
            let interval_for_resolve = st.config.interval.clone();
            // Rejim-farkında: per-rejim trail A/B hedefi target_trail_pct_resolved'a girer.
            let atr_mult = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| {
                    p.resolve_atr_mult_for_regime(
                        symbol,
                        &interval_for_resolve,
                        strategy_name,
                        default_mult,
                        Some(regime.as_str()),
                    )
                })
                .unwrap_or(default_mult);
            let trailing_stop = if is_long {
                entry - atr * atr_mult
            } else {
                entry + atr * atr_mult
            };
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
            if xs_sizing.is_none() {
                if let Err(err) = crate::robot::order_management::OrderValidator::with_defaults()
                    .validate_risk_reward(entry, Some(stop_loss), Some(take_profit))
                {
                    st.push_log(format!("⛔ {} R/R doğrulamasını geçemedi: {}", symbol, err));
                    return;
                }
            }
            // Otonom leverage: ParameterStore.resolve_leverage rejim/conf/win_rate/noise
            // ağırlıklı bir değer döndürür. LEVERAGE_ENABLED=false (default) ise 1.0
            // → spot davranış. Stats yoksa noise faktörü None ile devre dışı.
            let noise_floor_opt = st.brain.parameters.read().ok().and_then(|p| {
                p.symbol_stats
                    .get(&(symbol.to_string(), interval_for_resolve.clone()))
                    .map(|s| s.noise_floor_pct)
            });
            // KESİTSEL: sabit kaldıraç (rejim-değişken resolve_leverage bypass; marjinal nötr edge'de
            // mütevazı L — anlamlılık L-invariant). None → otonom resolve_leverage (mevcut davranış).
            let leverage_resolved = match xs_sizing {
                Some(s) => s.leverage.max(1.0),
                None => st
                    .brain
                    .parameters
                    .read()
                    .ok()
                    .map(|p| {
                        p.resolve_leverage(regime.as_str(), ml_conf, win_prob, noise_floor_opt)
                    })
                    .unwrap_or(1.0),
            };
            // strategy_name caller'dan geliyor — process_symbol_cycle StrategySelector ile
            // rejime göre seçti (SUPERTREND / BB / MA_CROSSOVER vb.). trade_type bunu mühürler;
            // check_exit_conditions açılışla aynı target_pct'i okuyabilsin diye.
            let new_pos = PositionModel {
                pos_id: pos_id_str.clone(),
                symbol: symbol.to_string(),
                venue: st
                    .venue_registry
                    .for_symbol(symbol)
                    .map(|venue| venue.venue_key()),
                entry_price: entry,
                current_price: entry,
                qty: qty_val,
                leverage: leverage_resolved,
                market: st.config.market.clone(),
                // Pozisyonun GERÇEK trade TF'i: mumlar per-symbol interval_c'den geldi
                // (loop_core symbol_interval'i okur; seed Fix A 1d set edebilir). config.interval
                // (global) DEĞİL — candle serisinin kendi interval'ı tek-doğru kaynak. Boşsa
                // (eski/defaultlı candle) global'e düş.
                interval: {
                    let civ = last_candle.interval.clone();
                    if civ.is_empty() {
                        st.config.interval.clone()
                    } else {
                        civ
                    }
                },
                // trade_type artık stratejik etiket (önceki "LONG"/"SHORT" zaten
                // is_long ile aynı bilgiyi tekrar ediyordu); UI "Strateji" sütununda
                // hangi karar mekanizmasının açtığını göstersin.
                trade_type: strategy_name.to_string(),
                is_long,
                opened_at: chrono::Utc::now().to_rfc3339(),
                stop_loss,
                take_profit,
                trailing_stop,
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
                new_pos,
                alloc_capital,
                graded_target,
                qty_val,
                kelly_fraction: kelly.kelly_fraction,
                risk_appetite,
                ml_conf,
                tp_pct,
                sl_pct,
                strategy_name: strategy_name.to_string(),
                trading_mode: st.config.trading_mode.clone(),
                real_execution_allowed: crate::robot::robotic_loop::live_execution_is_armed()
                    && !matches!(
                        st.config.venue_profile,
                        crate::core::types::VenueProfile::Paper
                            | crate::core::types::VenueProfile::Demo
                    ),
                // Executor venue-farkında seçilir: sembolün venue'sü Binance ise canlı executor,
                // değilse (data-only/paper/@bybit) None → paper yolu. Binance sembolde
                // st.live_executor ile birebir aynı (registry onunla kuruldu). [[venue]]
                live_executor: st
                    .venue_registry
                    .route(symbol)
                    .filter(|(v, _)| v.execution_engine().is_some())
                    .map(|(v, _)| Arc::clone(v)),
                live_venue: st.venue_registry.route(symbol).map(|(v, _)| Arc::clone(v)),
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
        let plan = match plan {
            Some(p) => p,
            None => return,
        };

        // 💱 LIVE Mode dispatcher (3 koşullu onay zinciri):
        let trading_mode = plan.trading_mode.clone();
        let live_executor =
            if plan.real_execution_allowed && matches!(trading_mode, TradingMode::Live) {
                plan.live_executor.clone()
            } else {
                if matches!(trading_mode, TradingMode::Live) {
                    if let Ok(mut st2) = state.lock() {
                        let profile = st2.config.venue_profile.as_str().to_string();
                        st2.push_log(format!(
                        "🟡 [PROFILE-GATE] {} gerçek emir engellendi (profile={}) → paper akışı",
                        symbol, profile,
                    ));
                    }
                }
                None
            };
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
        let live_venue = plan.live_venue.clone();
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
        let protection_check = match state.lock() {
            Ok(st) => match st.finance.live_positions.read() {
                Ok(positions) => Ok(positions
                    .values()
                    .any(|position| position.stop_loss <= 0.0 && position.take_profit <= 0.0)),
                Err(_) => Err("aktif pozisyon koruma kilidi okunamadı"),
            },
            Err(_) => Err("AppState kilidi alınamadı"),
        };
        let protection_gap = match protection_check {
            Ok(gap) => gap,
            Err(reason) if live_executor.is_some() && !live_dry_run => {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🛡️ [PROTECTION-GATE] {} canlı açılışı engellendi: {}",
                        symbol, reason
                    ));
                }
                return;
            }
            Err(_) => false,
        };
        if live_executor.is_some() && !live_dry_run && protection_gap {
            if let Ok(mut st2) = state.lock() {
                st2.push_log(format!(
                    "🛡️ [PROTECTION-GATE] {} canlı açılışı engellendi: mevcut pozisyonda broker-side SL/TP açığı var",
                    symbol,
                ));
            }
            return;
        }
        if let Some(executor) = live_executor.as_ref() {
            let side = if is_long { "BUY" } else { "SELL" };
            // 🌍 Bölge/IP bloğu devre-kesici: borsa SİSTEMİK reddediyorsa (HTTP 451/403/-2015/
            // -1003) cooldown boyunca canlı emir DENEME — API'yi dövmeyi kes (IP-ban tırmanışını
            // önle). dry_run değil gerçek-canlıda anlamlı (LIVE_DRY_RUN'da emir gitmez zaten,
            // gate'i atla → paper akışı tam koşsun). Süre dolunca buradan geçen ilk emir
            // auto-probe olur. [[project_one_position_per_symbol]]
            if !live_dry_run && Self::live_block_active(state) {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🌍 [LIVE-BLOCK] {} {} → borsa bloğu aktif, canlı emir atlandı (cooldown)",
                        symbol, side,
                    ));
                }
                return;
            }
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
                    for price in [
                        &mut new_pos.stop_loss,
                        &mut new_pos.take_profit,
                        &mut new_pos.trailing_stop,
                    ] {
                        if *price > 0.0 {
                            if let Ok(rounded) = executor.round_price(symbol, *price).await {
                                *price = rounded;
                            }
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
                            chrono::Local::now().format("%H:%M:%S"),
                            symbol,
                            side,
                            reason,
                        ));
                        while st2.guardian.repair_log.len() > 100 {
                            st2.guardian.repair_log.pop_front();
                        }
                    }
                    return;
                }
            }

            if live_dry_run {
                if let Ok(mut st2) = state.lock() {
                    let mode = if use_limit_entry {
                        "LIMIT-MAKER"
                    } else {
                        "MARKET"
                    };
                    st2.push_log(format!(
                        "🟡 [LIVE-DRY-RUN] {} {} {:.4} @ {:.2} (${:.2}) [{}] → emir gönderilmedi",
                        symbol, side, qty_val, entry, alloc_capital, mode,
                    ));
                }
            } else {
                // 🎚️ Futures kaldıracını borsada set et (sizing varsayımıyla tutarlı, B#2).
                // notional = teminat × kaldıraç → borsa-default kaldıraç farklıysa pozisyon
                // boyutu varsayımı bozulur. Spot'ta no-op; set edilemezse açmadan VETO.
                if executor.market() != memos_venue_api::Market::Spot {
                    let lev = new_pos.leverage.round().max(1.0) as u32;
                    if let Err(e) = executor.set_leverage(symbol, lev).await {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "🛑 [LIVE-LEVERAGE-VETO] {} {} kaldıraç {}x set edilemedi ({:?}) — pozisyon açılmadı",
                                symbol, side, lev, e,
                            ));
                        }
                        // 🌍 Sistemik blok (bölge/IP/izin) ilk bu çağrıda görünebilir → devre-kesici.
                        if e.is_exchange_block() {
                            Self::trip_live_block(state, symbol, "set_leverage");
                        }
                        return;
                    }
                }
                // ── Giriş emri: maker (opt-in, POST_ONLY) → taker MARKET dispatch ──
                // use_limit_entry ise önce best_bid/ask'e katılan maker denenir; N
                // deneme dolmazsa limit_entry_fallback_market'a göre taker'a düşülür
                // ya da trade atlanır. Kapalıysa eski davranış (doğrudan MARKET).
                // Dönüş: dolmuş emir Value'su (orderId + maker yolunda avgPrice).
                let entry_resp: std::result::Result<memos_venue_api::OrderReceipt, ()> =
                    if use_limit_entry {
                        executor
                            .smart_limit_entry(
                                symbol,
                                memos_venue_api::OrderSide::from_long(is_long),
                                qty_val,
                                limit_entry_timeout_ms,
                                limit_entry_max_attempts,
                                limit_entry_max_spread_bps,
                                limit_entry_fallback_market,
                            )
                            .await
                            .map(|(receipt, maker)| {
                                used_maker = maker;
                                receipt
                            })
                            .map_err(|e| {
                                if e.is_exchange_block() {
                                    Self::trip_live_block(state, symbol, "entry");
                                }
                                log::warn!("LIVE {} {} giriş hatası: {:?}", symbol, side, e);
                            })
                    } else {
                        executor
                            .submit_order(&memos_venue_api::OrderRequest::market(
                                symbol,
                                memos_venue_api::OrderSide::from_long(is_long),
                                qty_val,
                            ))
                            .await
                            .map_err(|e| {
                                if e.is_exchange_block() {
                                    Self::trip_live_block(state, symbol, "market_order");
                                }
                                log::warn!("LIVE {} {} emir hatası: {:?}", symbol, side, e);
                            })
                    };

                // Giriş emri başarısız (ve fallback yok / fallback de patladı) → paper'ı da çalıştırma.
                let resp = match entry_resp {
                    Ok(r) => r,
                    Err(()) => return,
                };
                if !matches!(
                    resp.status,
                    memos_venue_api::OrderStatus::Filled
                        | memos_venue_api::OrderStatus::PartiallyFilled
                ) {
                    Self::persist_uncertain_entry(
                        state,
                        symbol,
                        side,
                        qty_val,
                        resp.filled_qty,
                        resp.venue_order_id.clone(),
                        format!("{:?}", resp.status),
                        "entry sonucu kesin değil",
                    );
                    if let Ok(mut st2) = state.lock() {
                        st2.push_alert(
                            "LIVE-ENTRY-UNCERTAIN",
                            crate::robot::infra::telegram_notifier::Severity::Critical,
                            format!(
                                "{} entry sonucu kesin değil veya kabul edilmedi: {:?}; retry yapılmayacak",
                                symbol, resp.status
                            ),
                        );
                    }
                    Self::trip_live_block(state, symbol, "entry_uncertain");
                    return;
                }
                live_order_id = resp.venue_order_id.clone();
                // 🌍 Auto-probe başarısı: blok cooldown'u sonrası buradan geçen ilk emir GEÇTİ →
                // devre-kesiciyi kapat (bayrak set değilse no-op).
                Self::clear_live_block(state, symbol);

                // 🧮 Maker dolum reconciliation: gerçek fill fiyatı entry'den saparsa
                // pozisyonu, SL/TP'yi ve trailing'i fill fiyatından yeniden hesapla
                // (PnL ve borsa koruma emirleri gerçeğe otursun). Market yolunda futures
                // yanıtı avgPrice taşımayabilir → entry referansı korunur (eski davranış).
                if used_maker {
                    let fill_price = (resp.avg_price > 0.0)
                        .then_some(resp.avg_price)
                        .unwrap_or(entry);
                    if (fill_price - entry).abs() > f64::EPSILON {
                        new_pos.entry_price = fill_price;
                        new_pos.current_price = fill_price;
                        new_pos.max_favorable_price = fill_price;
                        new_pos.stop_loss = if is_long {
                            fill_price * (1.0 - sl_pct / 100.0)
                        } else {
                            fill_price * (1.0 + sl_pct / 100.0)
                        };
                        new_pos.take_profit = if is_long {
                            fill_price * (1.0 + tp_pct / 100.0)
                        } else {
                            fill_price * (1.0 - tp_pct / 100.0)
                        };
                        new_pos.trailing_stop = if is_long {
                            fill_price - atr * atr_mult
                        } else {
                            fill_price + atr * atr_mult
                        };
                        for price in [
                            &mut new_pos.stop_loss,
                            &mut new_pos.take_profit,
                            &mut new_pos.trailing_stop,
                        ] {
                            if let Ok(rounded) = executor.round_price(symbol, *price).await {
                                *price = rounded;
                            }
                        }
                    }
                }

                let live_label = if used_maker { "LIVE-MAKER" } else { "LIVE" };
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "💱 [{}] {} {} {:.4} @ {:.2} (${:.2}) ✓ order={}",
                        live_label,
                        symbol,
                        side,
                        qty_val,
                        new_pos.entry_price,
                        alloc_capital,
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
                            map.insert(
                                symbol.to_string(),
                                crate::core::model::LiveOrderRefs {
                                    entry_order_id: live_order_id.clone(),
                                    sl_order_id: None,
                                    tp_order_id: None,
                                    placed_at: chrono::Utc::now().to_rfc3339(),
                                    ..Default::default()
                                },
                            );
                        }
                        st2.push_log(format!(
                            "🛡️ [LIVE-PROTECT] {} stopsuz pozisyon → borsa SL/TP atlandı (risk kitap-düzeyinde)",
                            symbol,
                        ));
                    }
                } else {
                    let (sl_res, tp_res) = executor
                        .place_protection_orders(symbol, is_long, qty_val, pos_sl, pos_tp)
                        .await;
                    let sl_id = sl_res.as_ref().ok().and_then(|r| r.venue_order_id.clone());
                    let tp_id = tp_res.as_ref().ok().and_then(|r| r.venue_order_id.clone());
                    let sl_status = match &sl_res {
                        Ok(_) => format!("SL ✓ ({})", sl_id.as_deref().unwrap_or("?")),
                        Err(e) => format!("SL ❌ {:?}", e),
                    };
                    let tp_status = match &tp_res {
                        Ok(_) => format!("TP ✓ ({})", tp_id.as_deref().unwrap_or("?")),
                        Err(e) => format!("TP ❌ {:?}", e),
                    };
                    let protection_failed =
                        (pos_sl > 0.0 && sl_res.is_err()) || (pos_tp > 0.0 && tp_res.is_err());
                    if protection_failed {
                        let _ = executor.cancel_all(symbol).await;
                        let close_result = executor.close_position(symbol).await;
                        if let Ok(mut st2) = state.lock() {
                            st2.push_alert(
                                "LIVE-PROTECT-HARD-GATE",
                                crate::robot::infra::telegram_notifier::Severity::Critical,
                                format!(
                                    "{} girişinden sonra broker-side protection doğrulanamadı ({}; {}) — pozisyon kapatma sonucu: {:?}",
                                    symbol, sl_status, tp_status, close_result
                                ),
                            );
                        }
                        Self::trip_live_block(state, symbol, "protection_hard_gate");
                        return;
                    }
                    // Order ID eşlemesini state'e mühürle (cancel için audit trail).
                    if let Ok(mut st2) = state.lock() {
                        if let Ok(mut map) = st2.finance.live_orders.write() {
                            map.insert(
                                symbol.to_string(),
                                crate::core::model::LiveOrderRefs {
                                    entry_order_id: live_order_id.clone(),
                                    sl_order_id: sl_id.clone(),
                                    tp_order_id: tp_id.clone(),
                                    placed_at: chrono::Utc::now().to_rfc3339(),
                                    ..Default::default()
                                },
                            );
                        }
                        st2.push_log(format!(
                            "🛡️ [LIVE-PROTECT] {} @ SL={:.4} TP={:.4} · {} · {}",
                            symbol, pos_sl, pos_tp, sl_status, tp_status,
                        ));
                    }

                    // Borsa-tarafı SL emri BEKLENİYORDU (pos_sl>0) ama başarısız.
                    // Tipik kök: Binance -4120 ("Order type not supported for this endpoint")
                    // — bazı hesaplar/bölgeler STOP_MARKET/TAKE_PROFIT_MARKET'i /fapi/v1/order'dan
                    // kabul etmez. ESKİ davranış: emergency-close + return → pozisyon İÇSEL KAYDEDİLMEDEN
                    // (804+ atlanırdı) kapanıyordu = her girişte aç→kapa CHURN (komisyon sızıntısı) +
                    // muhasebe kopukluğu (equity/closed_count/TRADE log güncellenmiyordu, TUI gerçeği
                    // göstermiyordu). YENİ: pozisyonu KORU, çıkışı bot-tarafı yönet — check_exit_conditions
                    // her cycle SL/TP/trailing'i fiyattan tetikler → close_paper_position gerçek MARKET
                    // ile kapatır (positions_close.rs). Tradeoff: borsa-tarafı koruma yoksa bot kesintisinde
                    // pozisyon korumasız kalır; sürekli watchdog + bot-tarafı izleme ile kabul edilebilir
                    // ve her durumda churn'den iyidir. [[project_one_position_per_symbol]]
                    if pos_sl > 0.0 && sl_res.is_err() {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_alert(
                                "LIVE-PROTECT-FALLBACK",
                                crate::robot::infra::telegram_notifier::Severity::Warning,
                                format!(
                                    "[LIVE-PROTECT-FALLBACK] {} borsa SL emri verilemedi ({:?}) → SL/TP bot-tarafı yönetilecek (pozisyon korunuyor)",
                                    symbol, sl_res.as_ref().err(),
                                ),
                            );
                            st2.guardian.repair_log.push_back(format!(
                                "[{}] borsa SL hatası: {} → bot-tarafı yönetim (churn fix)",
                                chrono::Local::now().format("%H:%M:%S"),
                                symbol,
                            ));
                            while st2.guardian.repair_log.len() > 100 {
                                st2.guardian.repair_log.pop_front();
                            }
                        }
                        // NOT: emergency-close + return KALDIRILDI → pozisyon normal kaydedilir (804+)
                        // ve bot-tarafı exit devreye girer (churn + izleme kopukluğu çözümü).
                        // 🌍 SL reddi sistemik blok (-2015/451) ise (yalnız -4120 endpoint değil) →
                        // devre-kesici: pozisyon korunur ama YENİ canlı girişler cooldown'da durur.
                        if sl_res
                            .as_ref()
                            .err()
                            .map(|e| e.is_exchange_block())
                            .unwrap_or(false)
                        {
                            Self::trip_live_block(state, symbol, "place_protection_orders");
                        }
                    }
                }
            }
        } else if matches!(trading_mode, TradingMode::Live)
            && live_venue
                .as_ref()
                .map(|v| v.exchange() != Exchange::Binance)
                .unwrap_or(false)
        {
            let venue = live_venue.as_ref().expect("live venue present");
            let side = if is_long {
                crate::robot::venue::types::OrderSide::Buy
            } else {
                crate::robot::venue::types::OrderSide::Sell
            };
            if alloc_capital > live_max_notional {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🛑 LIVE veto: notional ${:.2} > tavan ${:.2} ({} {:?} iptal edildi)",
                        alloc_capital, live_max_notional, symbol, side,
                    ));
                }
                return;
            }
            if live_dry_run {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🟡 [LIVE-DRY-RUN] {} {:?} {:.4} @ {:.2} (${:.2}) [{}:{}] → emir gönderilmedi",
                        symbol,
                        side,
                        qty_val,
                        entry,
                        alloc_capital,
                        venue.exchange().as_str(),
                        venue.market().as_str(),
                    ));
                }
            } else {
                let lev = new_pos.leverage.round().max(1.0) as u32;
                if let Err(e) = venue.set_leverage(symbol, lev).await {
                    if let Ok(mut st2) = state.lock() {
                        st2.push_log(format!(
                            "⚠️ [LIVE->PAPER] {} leverage {}x set edilemedi ({:?}) — paper izlemeye düşüldü",
                            symbol, lev, e,
                        ));
                    }
                }

                let req = crate::robot::venue::types::OrderRequest::market(symbol, side, qty_val);
                match venue.submit_order(&req).await {
                    Ok(receipt) => {
                        if !matches!(
                            receipt.status,
                            crate::robot::venue::types::OrderStatus::Filled
                                | crate::robot::venue::types::OrderStatus::PartiallyFilled
                        ) {
                            Self::persist_uncertain_entry(
                                state,
                                symbol,
                                &format!("{:?}", side),
                                qty_val,
                                receipt.filled_qty,
                                receipt.venue_order_id.clone(),
                                format!("{:?}", receipt.status),
                                "generic venue entry sonucu kesin değil",
                            );
                            if let Ok(mut st2) = state.lock() {
                                st2.push_alert(
                                    "LIVE-ENTRY-UNCERTAIN",
                                    crate::robot::infra::telegram_notifier::Severity::Critical,
                                    format!(
                                        "{} generic venue entry sonucu kesin değil veya kabul edilmedi: {:?}; retry yapılmayacak",
                                        symbol, receipt.status
                                    ),
                                );
                            }
                            Self::trip_live_block(state, symbol, "entry_uncertain");
                            return;
                        }
                        live_order_id = receipt.venue_order_id.clone();
                        let event_symbol = if symbol.contains('@') {
                            symbol.to_string()
                        } else {
                            format!("{}@{}", symbol, venue.exchange().as_str())
                        };
                        let side_str = match side {
                            crate::robot::venue::types::OrderSide::Buy => "BUY",
                            crate::robot::venue::types::OrderSide::Sell => "SELL",
                        };
                        let status_str = match receipt.status {
                            crate::robot::venue::types::OrderStatus::New => "NEW",
                            crate::robot::venue::types::OrderStatus::PartiallyFilled => {
                                "PARTIALLY_FILLED"
                            }
                            crate::robot::venue::types::OrderStatus::Filled => "FILLED",
                            crate::robot::venue::types::OrderStatus::Canceled => "CANCELED",
                            crate::robot::venue::types::OrderStatus::Rejected => "REJECTED",
                            crate::robot::venue::types::OrderStatus::Unknown => "UNKNOWN",
                        };
                        let receipt_reason = receipt
                            .raw
                            .as_ref()
                            .and_then(|raw| {
                                raw.get("error")
                                    .and_then(|v| v.as_str())
                                    .or_else(|| raw.get("msg").and_then(|v| v.as_str()))
                            })
                            .unwrap_or("");
                        emit_trade_event(state, || {
                            crate::robot::infra::logger::TradeEvent::order_status(
                                &event_symbol,
                                status_str,
                                side_str,
                                live_order_id.as_deref().unwrap_or(""),
                                if receipt.filled_qty > 0.0 {
                                    receipt.filled_qty
                                } else {
                                    qty_val
                                },
                                receipt_reason,
                            )
                        });
                        match receipt.status {
                            crate::robot::venue::types::OrderStatus::Filled => {
                                emit_trade_event(state, || {
                                    crate::robot::infra::logger::TradeEvent::order_filled(
                                        &event_symbol,
                                        live_order_id.as_deref().unwrap_or(""),
                                        false,
                                    )
                                });
                            }
                            crate::robot::venue::types::OrderStatus::PartiallyFilled => {
                                let equity_snapshot =
                                    state.lock().ok().map(|s| s.finance.equity).unwrap_or(0.0);
                                let fill_pct = if qty_val > 0.0 {
                                    (receipt.filled_qty / qty_val * 100.0).clamp(0.0, 100.0)
                                } else {
                                    0.0
                                };
                                let partial_price = if receipt.avg_price > 0.0 {
                                    receipt.avg_price
                                } else {
                                    entry
                                };
                                emit_trade_event(state, || {
                                    crate::robot::infra::logger::TradeEvent::order_partial(
                                        &event_symbol,
                                        side_str,
                                        partial_price,
                                        receipt.filled_qty,
                                        receipt.filled_qty,
                                        qty_val,
                                        fill_pct,
                                        equity_snapshot,
                                    )
                                });
                            }
                            _ => {}
                        }
                        if receipt.avg_price > 0.0 {
                            new_pos.entry_price = receipt.avg_price;
                            new_pos.current_price = receipt.avg_price;
                            new_pos.max_favorable_price = receipt.avg_price;
                        }
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "💱 [LIVE:{}] {} {:?} {:.4} @ {:.2} (${:.2}) ✓ order={}",
                                venue.exchange().as_str(),
                                symbol,
                                side,
                                qty_val,
                                new_pos.entry_price,
                                alloc_capital,
                                live_order_id.as_deref().unwrap_or("?"),
                            ));
                        }
                    }
                    Err(e) => {
                        let event_symbol = if symbol.contains('@') {
                            symbol.to_string()
                        } else {
                            format!("{}@{}", symbol, venue.exchange().as_str())
                        };
                        let side_str = match side {
                            crate::robot::venue::types::OrderSide::Buy => "BUY",
                            crate::robot::venue::types::OrderSide::Sell => "SELL",
                        };
                        emit_trade_event(state, || {
                            crate::robot::infra::logger::TradeEvent::order_status(
                                &event_symbol,
                                "REJECTED",
                                side_str,
                                "",
                                qty_val,
                                &format!("{}", e),
                            )
                        });
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "⚠️ [LIVE->PAPER] {} {:?} emir hatası: {:?} — paper izlemeye düşüldü",
                                symbol, side, e,
                            ));
                        }
                    }
                }
            }
        }

        if live_executor.is_none()
            && live_order_id.is_some()
            && !live_dry_run
            && (new_pos.stop_loss > 0.0 || new_pos.take_profit > 0.0)
        {
            if let Some(venue) = live_venue.as_ref() {
                let (sl_res, tp_res) = venue
                    .place_protection_orders(
                        symbol,
                        is_long,
                        qty_val,
                        new_pos.stop_loss,
                        new_pos.take_profit,
                    )
                    .await;
                let sl_id = sl_res.as_ref().ok().and_then(|r| r.venue_order_id.clone());
                let tp_id = tp_res.as_ref().ok().and_then(|r| r.venue_order_id.clone());
                let protection_failed = (new_pos.stop_loss > 0.0 && sl_res.is_err())
                    || (new_pos.take_profit > 0.0 && tp_res.is_err());
                if let Ok(st2) = state.lock() {
                    if let Ok(mut map) = st2.finance.live_orders.write() {
                        map.insert(
                            symbol.to_string(),
                            crate::core::model::LiveOrderRefs {
                                entry_order_id: live_order_id.clone(),
                                sl_order_id: sl_id,
                                tp_order_id: tp_id,
                                placed_at: chrono::Utc::now().to_rfc3339(),
                                ..Default::default()
                            },
                        );
                    }
                }
                if protection_failed {
                    let _ = venue.cancel_all(symbol).await;
                    let close_result = venue.close_position(symbol).await;
                    if let Ok(mut st2) = state.lock() {
                        st2.push_alert(
                            "LIVE-PROTECT-HARD-GATE",
                            crate::robot::infra::telegram_notifier::Severity::Critical,
                            format!(
                                "{} girişinden sonra generic venue protection doğrulanamadı (SL={:?}, TP={:?}) — pozisyon kapatma sonucu: {:?}",
                                symbol, sl_res, tp_res, close_result
                            ),
                        );
                    }
                    Self::trip_live_block(state, symbol, "protection_hard_gate");
                    return;
                }
            }
        }

        // Mutex'i geri al
        let mut st = lock_state_recover(state);

        let new_pos_for_log = new_pos.clone();
        st.insert_live_position(symbol.to_string(), new_pos);
        // 🪜 Kademeli giriş durumu: açılış = 1. kademe (sayaç=1); tam hedef sermaye saklanır → ek
        // kademeler try_add_graded_tranche'de target·weight[k] ile boyutlanır. None → kademeli kapalı.
        if let Some(target) = plan.graded_target {
            if let Ok(mut m) = st.finance.graded_tranches.write() {
                m.insert(
                    symbol.to_string(),
                    crate::robot::robotic_loop::GradedPosState {
                        tranches_filled: 1,
                        target_capital: target,
                    },
                );
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
            || strategy_name == crate::robot::engines::master::carry_live::CARRY_STRATEGY_TAG
            || strategy_name == crate::robot::engines::master::blend_live::BLEND_STRATEGY_TAG)
            && st.tuning.use_limit_entry;
        let entry_commission_rate = if used_maker || xs_maker {
            st.tuning.maker_commission_rate
        } else {
            st.tuning.commission_rate
        };
        let commission = alloc_capital * entry_commission_rate;
        if let Ok(mut costs) = st.finance.live_execution_costs.write() {
            costs.commission_usd += commission;
            costs.total_cost_usd += commission;
            costs.trade_count += 1;
        }
        st.finance.equity -= commission;
        // Per-trade NET P&L için giriş komisyonunu pozisyona mühürle (kapanışta
        // net_pnl = gross − entry_commission − exit_commission). Pozisyon 760'ta insert edildi.
        if let Ok(mut positions) = st.finance.live_positions.write() {
            if let Some(p) = positions.get_mut(symbol) {
                p.entry_commission = commission;
            }
        }

        // IntelligenceHub.track_trade — kapanışta learn_from_exit ile eşleşecek.
        if let Ok(mut hub) = st.brain.intelligence_hub.write() {
            hub.track_trade(pos_id, regime, strategy_name.clone());
        }

        let mode_tag = if live_order_id.is_some() {
            "LIVE"
        } else if (live_executor.is_some() || live_venue.is_some()) && live_dry_run {
            "DRY-RUN"
        } else {
            "PAPER"
        };
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
                &format!(
                    "📈 {symbol} {} açıldı @ {:.4} · {} · ×{:.1}",
                    if is_long { "LONG" } else { "SHORT" },
                    entry,
                    strategy_name,
                    pos_for_log.leverage
                ),
            );
        }
        // 📝 Periyodik dosya logu: TRADE_OPEN. Logger Arc'ını lock altında clone'la,
        // unlock sonrası IO yap.
        let logger_for_event = st.trading_logger.clone();
        let equity_now = st.finance.equity;
        drop(st);
        if let Some(logger) = logger_for_event {
            let ev = crate::robot::infra::logger::TradeEvent::trade_open(
                symbol,
                &strategy_name,
                is_long,
                entry,
                qty_val,
                equity_now,
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
        state
            .lock()
            .ok()
            .map(|st| {
                st.config
                    .blocked_symbols
                    .iter()
                    .any(|b| b.eq_ignore_ascii_case(symbol))
            })
            .unwrap_or(false)
    }

    fn operator_live_trading_paused() -> Result<bool, String> {
        let path = std::env::var("OPERATOR_CONTROLS_PATH")
            .unwrap_or_else(|_| "data/operator_controls.json".into());
        let path = std::path::Path::new(&path);
        if !path.exists() {
            return Ok(false);
        }
        let body = std::fs::read(path)
            .map_err(|error| format!("operatör kontrol dosyası okunamadı: {error}"))?;
        let controls: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|error| format!("operatör kontrol dosyası geçersiz JSON: {error}"))?;
        Ok(controls
            .get("live_trading_paused")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false))
    }

    /// Live giriş için account ve broker-side protection ön koşullarını tek noktada
    /// değerlendirir. Paper ve live-dry-run akışları bu kapıdan etkilenmez.
    fn live_entry_gate(state: &Arc<Mutex<AppState>>, symbol: &str) -> Option<String> {
        let st = match state.lock() {
            Ok(st) => st,
            Err(_) => return Some("AppState kilidi alınamadı".to_string()),
        };
        if !matches!(st.config.trading_mode, TradingMode::Live) || st.live_dry_run {
            return None;
        }
        if let Ok(uncertain) = st.finance.uncertain_orders.read() {
            if let Some(order) = uncertain.get(symbol) {
                return Some(format!(
                    "çözülmemiş belirsiz emir mevcut (status={}, order_id={:?}); reconciliation/operator çözümü bekleniyor",
                    order.status, order.broker_order_id
                ));
            }
        } else {
            return Some("belirsiz emir state kilidi okunamadı".into());
        }
        match Self::operator_live_trading_paused() {
            Ok(true) => return Some("operatör canlı işlem pause kontrolü aktif".into()),
            Ok(false) => {}
            Err(error) => return Some(format!("operatör kontrolü doğrulanamadı: {error}")),
        }
        let (venue, _) = st.venue_registry.route(symbol)?;
        if venue.execution_engine().is_none() {
            return None;
        }

        let venue_key = venue.name();
        let account = match st.venue_runtime.read() {
            Ok(runtimes) => runtimes
                .get(&venue_key)
                .and_then(|runtime| runtime.account_snapshot.clone()),
            Err(_) => {
                return Some(format!("{} account snapshot kilidi okunamadı", venue_key));
            }
        };
        let account_ready = account
            .as_ref()
            .map(|snapshot| {
                snapshot.status == memos_venue_api::AccountSnapshotStatus::Fresh
                    && snapshot.timestamp >= chrono::Utc::now() - chrono::Duration::seconds(600)
                    && snapshot
                        .equity
                        .map(|equity| equity.is_finite() && equity > 0.0)
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        if !account_ready {
            return Some(format!(
                "{} account snapshot fresh/positive equity olarak doğrulanamadı",
                venue_key
            ));
        }

        let exchange_key = venue.name();
        let reconciliation = match st.venue_runtime.read() {
            Ok(runtimes) => runtimes
                .get(&exchange_key)
                .map(|runtime| runtime.reconciliation_status.clone()),
            Err(_) => {
                return Some(format!(
                    "{} pozisyon mutabakatı durumu okunamadı",
                    venue_key
                ));
            }
        };
        if reconciliation.as_deref() != Some("MATCHED") {
            return Some(format!(
                "{} pozisyon mutabakatı doğrulanmadı ({})",
                venue_key,
                reconciliation.as_deref().unwrap_or("UNKNOWN")
            ));
        }

        let suffix = format!("@{}", venue.exchange().as_str());
        let unprotected = match st.finance.live_positions.read() {
            Ok(positions) => positions.values().any(|position| {
                position.symbol.ends_with(&suffix)
                    && position.stop_loss <= 0.0
                    && position.take_profit <= 0.0
            }),
            Err(_) => {
                return Some(format!(
                    "{} aktif pozisyon koruma kilidi okunamadı",
                    venue_key
                ));
            }
        };
        if unprotected {
            return Some(format!(
                "{} venue'unda broker-side SL/TP olmayan aktif pozisyon var",
                venue_key
            ));
        }
        None
    }

    /// 🌍 Bölge/IP bloğu devre-kesici AKTİF mi? `Some(until)` ve `now < until` → canlı emir
    /// denemesi atlanmalı. Süre dolduysa (`now >= until`) → `false` döner = auto-probe penceresi
    /// (tek deneme yapılır; başarı `clear_live_block`, blok `trip_live_block` ile yenilenir).
    /// Lock alınamazsa false (savunmacı: kapı açık).
    fn live_block_active(state: &Arc<Mutex<AppState>>) -> bool {
        state
            .lock()
            .ok()
            .and_then(|st| st.guardian.live_block_until)
            .map(|until| chrono::Utc::now() < until)
            .unwrap_or(false)
    }

    // ─── RİSK KAPISI YAPTIRIMI (Adım 4a) ───────────────────────────────────────
    // ÖNCEDEN: risk_gate `halt`/`enter_safe_mode` üretiyordu ama loop_core bunları
    // yalnız LOG ETİKETİ olarak kullanıp `return` ediyordu → kill-switch fiilen yoktu;
    // sonraki cycle aynı sembolde yeniden deniyordu. Aşağıdaki dört fonksiyon o boşluğu
    // kapatır. `live_block_*`'tan AYRI: o borsa reddi içindir ve başarılı emirde temizlenir.

    /// Risk HALT aktif mi → yeni pozisyon açılmamalı. Lock alınamazsa false (savunmacı).
    pub(crate) fn risk_halt_active(state: &Arc<Mutex<AppState>>) -> bool {
        state
            .lock()
            .ok()
            .and_then(|st| st.guardian.risk_halt_until)
            .map(|until| chrono::Utc::now() < until)
            .unwrap_or(false)
    }

    /// SAFE-MODE aktif mi → tahsisat kısılmalı.
    pub(crate) fn safe_mode_active(state: &Arc<Mutex<AppState>>) -> bool {
        state
            .lock()
            .ok()
            .and_then(|st| st.guardian.risk_safe_mode_until)
            .map(|until| chrono::Utc::now() < until)
            .unwrap_or(false)
    }

    /// Risk HALT'ı mühürle. İlk triplemede (veya süresi dolmuş mühür yenilenirken) TEK
    /// Critical alarm + repair_log; zaten aktifse sessiz (cycle spam yok).
    /// `cooldown == 0` → yaptırım kapalı (no-op).
    pub(crate) fn trip_risk_halt(state: &Arc<Mutex<AppState>>, reason: &str) {
        if let Ok(mut st) = state.lock() {
            let cooldown = st.tuning.risk_halt_cooldown_secs;
            if cooldown == 0 {
                return;
            }
            let was_active = st
                .guardian
                .risk_halt_until
                .map(|u| chrono::Utc::now() < u)
                .unwrap_or(false);
            st.guardian.risk_halt_until =
                Some(chrono::Utc::now() + chrono::Duration::seconds(cooldown as i64));
            if !was_active {
                st.push_alert(
                    "RISK-HALT",
                    crate::robot::infra::telegram_notifier::Severity::Critical,
                    format!(
                        "[RISK-HALT] Risk kapısı kritik seviye ({}) → YENİ pozisyon açılışı {}sn DURDURULDU. Mevcut pozisyonlar normal yönetiliyor (SL/TP/trailing).",
                        reason, cooldown,
                    ),
                );
                st.guardian.repair_log.push_back(format!(
                    "[{}] RISK-HALT: {} → yeni giriş {}sn durdu",
                    chrono::Utc::now().format("%H:%M:%S"),
                    reason,
                    cooldown,
                ));
            }
        }
    }

    /// SAFE-MODE'u mühürle: tahsisat `safe_mode_alloc_mult` ile kısılır. İşlem DURMAZ.
    pub(crate) fn trip_safe_mode(state: &Arc<Mutex<AppState>>, reason: &str) {
        if let Ok(mut st) = state.lock() {
            let cooldown = st.tuning.safe_mode_cooldown_secs;
            if cooldown == 0 {
                return;
            }
            let was_active = st
                .guardian
                .risk_safe_mode_until
                .map(|u| chrono::Utc::now() < u)
                .unwrap_or(false);
            st.guardian.risk_safe_mode_until =
                Some(chrono::Utc::now() + chrono::Duration::seconds(cooldown as i64));
            if !was_active {
                let mult = st.tuning.safe_mode_alloc_mult;
                st.push_alert(
                    "SAFE-MODE",
                    crate::robot::infra::telegram_notifier::Severity::Warning,
                    format!(
                        "[SAFE-MODE] Risk kapısı uyarı seviyesi ({}) → pozisyon boyutu {}sn boyunca {:.2}× kısıldı.",
                        reason, cooldown, mult,
                    ),
                );
                st.guardian.repair_log.push_back(format!(
                    "[{}] SAFE-MODE: {} → tahsisat {:.2}× ({}sn)",
                    chrono::Utc::now().format("%H:%M:%S"),
                    reason,
                    mult,
                    cooldown,
                ));
            }
        }
    }

    /// 🌍 Devre-kesiciyi tetikle: `live_block_until`'i cooldown kadar ileri mühürle. İlk
    /// triplemede (ya da süresi dolmuş blok yenilenirken) TEK Critical alarm + repair_log —
    /// zaten aktif bloktaysa sessiz (per-symbol/cycle spam yok). `cooldown==0` → devre-kesici
    /// kapalı (no-op). Paper akışı etkilenmez; yalnız canlı emir denemesi durur.
    fn trip_live_block(state: &Arc<Mutex<AppState>>, symbol: &str, reason: &str) {
        if let Ok(mut st) = state.lock() {
            let cooldown = st.tuning.live_block_cooldown_secs;
            if cooldown == 0 {
                return;
            }
            let was_active = st
                .guardian
                .live_block_until
                .map(|u| chrono::Utc::now() < u)
                .unwrap_or(false);
            st.guardian.live_block_until =
                Some(chrono::Utc::now() + chrono::Duration::seconds(cooldown as i64));
            if !was_active {
                st.push_alert(
                    "LIVE-BLOCK",
                    crate::robot::infra::telegram_notifier::Severity::Critical,
                    format!(
                        "[LIVE-BLOCK] Borsa sistemik reddetti ({} · {}) → canlı emir {}sn DURDURULDU (paper akışı sürüyor). Süre sonunda auto-probe.",
                        symbol, reason, cooldown,
                    ),
                );
                st.guardian.repair_log.push_back(format!(
                    "[{}] LIVE-BLOCK: {} ({}) → canlı emir {}sn durdu (bölge/IP/izin)",
                    chrono::Local::now().format("%H:%M:%S"),
                    symbol,
                    reason,
                    cooldown,
                ));
                while st.guardian.repair_log.len() > 100 {
                    st.guardian.repair_log.pop_front();
                }
            }
        }
    }

    /// 🌍 Auto-probe başarısı: blok süresi dolmuşken bir emir GEÇERSE devre-kesiciyi kapat
    /// (`live_block_until=None`) + recovery log. Yalnız bayrak set ise iş yapar (no-op aksi halde).
    fn clear_live_block(state: &Arc<Mutex<AppState>>, symbol: &str) {
        if let Ok(mut st) = state.lock() {
            if st.guardian.live_block_until.is_some() {
                st.guardian.live_block_until = None;
                st.guardian.repair_log.push_back(format!(
                    "[{}] LIVE-BLOCK temizlendi: {} auto-probe geçti → canlı emir yeniden açık",
                    chrono::Local::now().format("%H:%M:%S"),
                    symbol,
                ));
                while st.guardian.repair_log.len() > 100 {
                    st.guardian.repair_log.pop_front();
                }
                st.push_log(format!(
                    "✅ [LIVE-BLOCK→CLEAR] {} borsa yeniden kabul ediyor",
                    symbol
                ));
            }
        }
    }

    fn persist_uncertain_entry(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        side: &str,
        requested_quantity: f64,
        filled_quantity: f64,
        broker_order_id: Option<String>,
        status: String,
        reason: &str,
    ) {
        let (db_path, profile) = match state.lock() {
            Ok(st) => (
                st.config.state_db_path.clone(),
                st.config.venue_profile.as_str().to_string(),
            ),
            Err(_) => return,
        };
        let now = chrono::Utc::now().to_rfc3339();
        let order = crate::core::model::UncertainOrderState {
            symbol: symbol.to_owned(),
            side: side.to_owned(),
            requested_quantity,
            filled_quantity,
            broker_order_id,
            status,
            reason: reason.to_owned(),
            created_at: now.clone(),
            last_checked_at: now,
            resolved_at: None,
            healthy_reconcile_count: 0,
        };
        if let Ok(st) = state.lock() {
            if let Ok(mut uncertain) = st.finance.uncertain_orders.write() {
                uncertain.insert(symbol.to_owned(), order.clone());
            }
        }
        let result = crate::persistence::open_db(&db_path).and_then(|conn| {
            crate::persistence::writer::save_uncertain_order_for_profile(&conn, &profile, &order)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
        });
        if let Err(error) = result {
            log::error!(
                "⚠️ [UNCERTAIN-ORDER] {} kalıcı state'e yazılamadı: {}",
                symbol,
                error
            );
        }
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
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn cap_reached_logic() {
        assert!(!concurrency_cap_reached(0, 0, 0), "cap=0 → sınırsız");
        assert!(!concurrency_cap_reached(9, 9, 0), "cap=0 → her zaman false");
        assert!(!concurrency_cap_reached(1, 0, 2), "1 açık < 2");
        assert!(concurrency_cap_reached(2, 0, 2), "2 açık = cap → dolu");
        assert!(
            concurrency_cap_reached(1, 1, 2),
            "1 açık + 1 uçuşta = cap → dolu"
        );
        assert!(concurrency_cap_reached(3, 0, 2), "tavan aşılmışsa dolu");
    }

    #[test]
    fn guard_releases_reservation_on_drop() {
        let counter = Arc::new(AtomicU32::new(0));
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        {
            let _g = OpenSlotGuard(Arc::clone(&counter));
            assert_eq!(
                counter.load(Ordering::Relaxed),
                1,
                "guard içindeyken rezervasyon durur"
            );
        }
        assert_eq!(
            counter.load(Ordering::Relaxed),
            0,
            "drop → rezervasyon serbest"
        );
    }
}

#[cfg(test)]
mod risk_enforcement_tests {
    use super::*;
    use crate::core::model::RoboticLoopConfig;

    /// `AppState::new` MT5 listede olduğunda TCP portunu erken bağlar (registry.rs) →
    /// senkron/tek-thread testte panikler. `VENUES=binance` MT5 kaydını devre dışı bırakır.
    fn state() -> Arc<Mutex<AppState>> {
        std::env::set_var("VENUES", "binance");
        Arc::new(Mutex::new(AppState::new(RoboticLoopConfig::default())))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn risk_halt_starts_inactive_and_seals_on_trip() {
        let st = state();
        assert!(!Engine::risk_halt_active(&st), "başlangıçta halt olmamalı");
        Engine::trip_risk_halt(&st, "Kritik Max DD aşıldı: 12.0%");
        assert!(
            Engine::risk_halt_active(&st),
            "trip sonrası halt aktif olmalı"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn safe_mode_seals_independently_of_halt() {
        let st = state();
        Engine::trip_safe_mode(&st, "Günlük kayıp 2.4%");
        assert!(Engine::safe_mode_active(&st), "safe-mode aktif olmalı");
        assert!(
            !Engine::risk_halt_active(&st),
            "safe-mode HALT'ı tetiklememeli"
        );
    }

    /// cooldown=0 → yaptırım kapalı (operatör kaçış kapısı). Sessizce no-op olmalı.
    #[tokio::test(flavor = "multi_thread")]
    async fn zero_cooldown_disables_enforcement() {
        let st = state();
        if let Ok(mut g) = st.lock() {
            // tuning bir Arc — mutasyon yok; klonlayıp yeniden mühürlüyoruz.
            let mut t = (*g.tuning).clone();
            t.risk_halt_cooldown_secs = 0;
            t.safe_mode_cooldown_secs = 0;
            g.tuning = std::sync::Arc::new(t);
        }
        Engine::trip_risk_halt(&st, "x");
        Engine::trip_safe_mode(&st, "y");
        assert!(!Engine::risk_halt_active(&st), "cooldown=0 → halt kapalı");
        assert!(
            !Engine::safe_mode_active(&st),
            "cooldown=0 → safe-mode kapalı"
        );
    }

    /// HALT mührü `live_block_until`'den AYRI olmalı: borsa-reddi devre-kesicisi
    /// başarılı emirde temizlenir, drawdown halt'ı öyle temizlenmemeli.
    #[tokio::test(flavor = "multi_thread")]
    async fn risk_halt_is_not_cleared_by_live_block_recovery() {
        let st = state();
        Engine::trip_risk_halt(&st, "DD");
        Engine::clear_live_block(&st, "BTCUSDT");
        assert!(
            Engine::risk_halt_active(&st),
            "clear_live_block RISK-HALT'ı kaldırmamalı"
        );
    }

    /// SAFE-MODE çarpanı tahsisatı gerçekten kısmalı (positions.rs base_alloc aritmetiği).
    #[tokio::test(flavor = "multi_thread")]
    async fn safe_mode_multiplier_reduces_allocation() {
        let st = state();
        let (equity, frac, mult) = {
            let g = st.lock().unwrap();
            (
                g.finance.equity,
                g.tuning.base_alloc_fraction,
                g.tuning.safe_mode_alloc_mult,
            )
        };
        let appetite = st.lock().unwrap().finance.calculate_risk_appetite();
        let normal = equity * frac * appetite * 1.0;
        Engine::trip_safe_mode(&st, "DD");
        let active = st
            .lock()
            .ok()
            .and_then(|g| g.guardian.risk_safe_mode_until)
            .map(|u| chrono::Utc::now() < u)
            .unwrap_or(false);
        assert!(active);
        let reduced = equity * frac * appetite * mult;
        assert!(
            reduced < normal,
            "safe-mode tahsisatı kısmalı: {reduced} < {normal}"
        );
        assert!(
            (reduced - normal * mult).abs() < 1e-9,
            "çarpan birebir uygulanmalı"
        );
    }
}
