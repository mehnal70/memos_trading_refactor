// src/robot/engines/master/positions.rs — Pozisyon aç/kapat + exit denetimi + scalp-swing
// Faz 1 modülerleştirme: master.rs'ten taşındı (davranış birebir korunur).
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
            state, symbol, &signal, candles, &strategy_name, Some(opp.trade_type),
        ).await;
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
        let cooldown_block = state.lock().ok().and_then(|st| {
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
            let alloc_capital = kelly.calculate_dynamic_scale(base_alloc, loss_streak, ml_conf)
                .max(base_alloc * st.tuning.alloc_floor_fraction);
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
            let atr_mult = st.brain.parameters.read().ok()
                .map(|p| p.resolve_atr_mult(symbol, &interval_for_resolve, strategy_name, default_mult))
                .unwrap_or(default_mult);
            let trailing_stop = if is_long { entry - atr * atr_mult }
                                else       { entry + atr * atr_mult };
            // Otonom leverage: ParameterStore.resolve_leverage rejim/conf/win_rate/noise
            // ağırlıklı bir değer döndürür. LEVERAGE_ENABLED=false (default) ise 1.0
            // → spot davranış. Stats yoksa noise faktörü None ile devre dışı.
            let noise_floor_opt = st.brain.parameters.read().ok().and_then(|p| {
                p.symbol_stats.get(&(symbol.to_string(), interval_for_resolve.clone()))
                    .map(|s| s.noise_floor_pct)
            });
            let leverage_resolved = st.brain.parameters.read().ok()
                .map(|p| p.resolve_leverage(regime.as_str(), ml_conf, win_prob, noise_floor_opt))
                .unwrap_or(1.0);
            // strategy_name caller'dan geliyor — process_symbol_cycle StrategySelector ile
            // rejime göre seçti (SUPERTREND / BB / MA_CROSSOVER vb.). trade_type bunu mühürler;
            // check_exit_conditions açılışla aynı target_pct'i okuyabilsin diye.
            let new_pos = PositionModel {
                pos_id: pos_id_str.clone(),
                symbol: symbol.to_string(),
                entry_price: entry, current_price: entry,
                qty: qty_val, leverage: leverage_resolved,
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
            };
            Some(OpenPlan {
                new_pos, alloc_capital, qty_val,
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

                // Kritik uyarı: SL emri başarısızsa pozisyon korumasız — emergency.
                if sl_res.is_err() {
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

        // Mutex'i geri al
        let mut st = state.lock().unwrap();

        let new_pos_for_log = new_pos.clone();
        {
            if let Ok(mut positions) = st.finance.live_positions.write() {
                positions.insert(symbol.to_string(), new_pos);
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
        let entry_commission_rate = if used_maker { st.tuning.maker_commission_rate }
                                     else          { st.tuning.commission_rate };
        let commission = alloc_capital * entry_commission_rate;
        if let Ok(mut costs) = st.finance.live_execution_costs.write() {
            costs.commission_usd += commission;
            costs.total_cost_usd += commission;
            costs.trade_count    += 1;
        }
        st.finance.equity -= commission;

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

    /// 🛡️ POZİSYON ÇIKIŞ KONTROLÜ: Açık her pozisyon için SL/TP/Trailing/Breakeven
    /// koşullarını sırasıyla denetler. Tetiklenmişse Some(ExitReason) döner ve
    /// pozisyonun max_favorable_price / breakeven_activated / trailing_stop alanlarını
    /// günceller.
    pub fn check_exit_conditions(
        position: &mut PositionModel,
        last_price: f64,
        atr: f64,
        atr_trail_mult: f64,
        breakeven_rr: f64,
    ) -> Option<ExitReason> {
        if last_price <= 0.0 { return None; }

        // 1) Favorable price güncellemesi (long en yüksek, short en düşük)
        if position.is_long {
            if last_price > position.max_favorable_price { position.max_favorable_price = last_price; }
        } else {
            if position.max_favorable_price == 0.0 || last_price < position.max_favorable_price {
                position.max_favorable_price = last_price;
            }
        }

        // 2) SL — statik (breakeven aktifse SL = entry'e taşınmış olur).
        if position.stop_loss > 0.0 {
            if position.is_long && last_price <= position.stop_loss {
                return Some(if position.breakeven_activated { ExitReason::Breakeven }
                            else { ExitReason::StopLoss });
            }
            if !position.is_long && last_price >= position.stop_loss {
                return Some(if position.breakeven_activated { ExitReason::Breakeven }
                            else { ExitReason::StopLoss });
            }
        }

        // 3) TP — statik.
        if position.take_profit > 0.0 {
            if position.is_long && last_price >= position.take_profit {
                return Some(ExitReason::TakeProfit);
            }
            if !position.is_long && last_price <= position.take_profit {
                return Some(ExitReason::TakeProfit);
            }
        }

        // 4) Breakeven aktivasyonu — TP'nin yarısına ulaştığında SL'i entry'e taşı.
        //    breakeven_rr: ROE eşiği (örn. 1.0 = RR 1:1, yani SL kadar kazanç).
        if !position.breakeven_activated && position.entry_price > 0.0 && position.stop_loss > 0.0 {
            let risk = (position.entry_price - position.stop_loss).abs();
            if risk > 0.0 {
                let gain = if position.is_long { last_price - position.entry_price }
                           else                 { position.entry_price - last_price };
                if gain >= risk * breakeven_rr {
                    position.breakeven_activated = true;
                    position.stop_loss = position.entry_price; // SL'i entry'e taşı
                }
            }
        }

        // 5) Trailing stop — ATR × mult uzaklıkta, sadece elverişli yönde kayar.
        if atr > 0.0 && atr_trail_mult > 0.0 {
            let delta = atr * atr_trail_mult;
            if position.is_long {
                let new_trail = position.max_favorable_price - delta;
                if new_trail > position.trailing_stop { position.trailing_stop = new_trail; }
                if position.trailing_stop > 0.0 && last_price <= position.trailing_stop {
                    return Some(ExitReason::TrailingStop);
                }
            } else {
                let new_trail = position.max_favorable_price + delta;
                if position.trailing_stop == 0.0 || new_trail < position.trailing_stop {
                    position.trailing_stop = new_trail;
                }
                if position.trailing_stop > 0.0 && last_price >= position.trailing_stop {
                    return Some(ExitReason::TrailingStop);
                }
            }
        }

        None
    }

    /// 🧬 FAZ F3: OTONOM POZİSYON KAPATMA MOTORU (Paper + Live dispatcher)
    pub(crate) async fn close_paper_position(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        candles: &[Candle],
        reason: ExitReason,
    ) {
        let last_candle = match candles.last() { Some(c) => c, None => return };

        // Min holding süresi koruması — sadece StrategySignal için.
        // ScalpEngine BUY açıyor → bir sonraki cycle'da klasik Strategy SELL
        // tetikleyince STRATEGY_SIGNAL ile kapatma döngüsü oluşuyordu (saniyede
        // 1 cycle × komisyon = $1800/gün live erozyonu). Min hold ile pozisyon
        // en az N sn yaşamalı (default 30sn; MIN_HOLDING_SECS_STRATEGY env).
        // SL/TP/Trailing etkilenmez — risk yönetimi anlık olmalı.
        if matches!(reason, ExitReason::StrategySignal) {
            // Eşik + opened_at tek lock skopunda (min_hold RuntimeTuning'den → getenv yok).
            let (min_hold_secs, opened_at_str) = state.lock().ok()
                .map(|st| (
                    st.tuning.min_holding_secs_strategy,
                    st.finance.live_positions.read().ok()
                        .and_then(|p| p.get(symbol).map(|pos| pos.opened_at.clone())),
                ))
                .unwrap_or((RuntimeTuning::default().min_holding_secs_strategy, None));
            if let Some(s) = opened_at_str {
                if let Ok(opened) = chrono::DateTime::parse_from_rfc3339(&s) {
                    let age_secs = (chrono::Utc::now() - opened.with_timezone(&chrono::Utc))
                        .num_seconds();
                    if age_secs < min_hold_secs {
                        // Throttle: cycle başına 1 ters sinyal → saniyede 1 reject log
                        // spam'ı oluyordu (TUI panel'ini doldurup operatöre engel olduğu
                        // gözlendi). Sembol başına 60sn cooldown — tek "hâlâ erken"
                        // bildirimi yeterli.
                        if log_throttle_should_emit(symbol, "strategy_min_hold", 60) {
                            push_state_log(state, format!(
                                "⏳ {} STRATEGY_SIGNAL erken kapanış reddedildi (age={}s < min={}s)",
                                symbol, age_secs, min_hold_secs,
                            ));
                        }
                        return;
                    }
                }
            }
        }

        // Mutex guard'ı async sınırını geçemez (MutexGuard !Send). Tüm sync iş bu skopta:
        let (target_pos, live_executor, live_dry_run, mode_tag) = {
            let mut st = state.lock().unwrap();
            st.fleet.phase = "Executing".into();
            let target_pos = if let Ok(mut positions) = st.finance.live_positions.write() {
                positions.remove(symbol)
            } else { None };
            let exec = st.live_executor.clone();
            let dry = st.live_dry_run;
            let tag = if exec.is_some() && !dry { "LIVE" }
                      else if exec.is_some() && dry { "DRY-RUN" }
                      else { "PAPER" };
            (target_pos, exec, dry, tag)
        }; // st burada otomatik drop olur

        if let Some(executor) = live_executor.as_ref() {
            if live_dry_run {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🟡 [LIVE-DRY-RUN] {} close ({:?}) → emir gönderilmedi", symbol, reason,
                    ));
                }
            } else {
                // 1. Bekleyen koruma emirlerini hedefli olarak iptal et.
                //    live_orders map'inden SL ve TP order_id'leri okunur; sadece bu emirler
                //    cancel edilir (paralel sembollerdeki orphan'lar etkilenmesin).
                //    Map'te kayıt yoksa fallback: cancel_all_orders (eski davranış).
                let refs = state.lock().ok()
                    .and_then(|s| s.finance.live_orders.read().ok()
                        .and_then(|m| m.get(symbol).cloned()));

                let cancel_result = if let Some(refs) = refs {
                    let mut summary: Vec<String> = Vec::new();
                    if let Some(sl_id_str) = refs.sl_order_id.as_deref() {
                        if let Ok(id) = sl_id_str.trim_matches('"').parse::<u64>() {
                            match executor.cancel_order(symbol, id).await {
                                Ok(_) => summary.push(format!("SL#{} ✓", id)),
                                Err(e) => summary.push(format!("SL#{} ❌ {:?}", id, e)),
                            }
                        }
                    }
                    if let Some(tp_id_str) = refs.tp_order_id.as_deref() {
                        if let Ok(id) = tp_id_str.trim_matches('"').parse::<u64>() {
                            match executor.cancel_order(symbol, id).await {
                                Ok(_) => summary.push(format!("TP#{} ✓", id)),
                                Err(e) => summary.push(format!("TP#{} ❌ {:?}", id, e)),
                            }
                        }
                    }
                    Some(summary)
                } else {
                    None
                };

                match cancel_result {
                    Some(summary) if !summary.is_empty() => {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "🧹 [LIVE] {} hedefli iptal: {}", symbol, summary.join(" · "),
                            ));
                        }
                    }
                    _ => {
                        // Fallback: order_id eşlemesi yoksa cancel_all (geriye uyum)
                        match executor.cancel_all_orders(symbol).await {
                            Ok(_) => {
                                if let Ok(mut st2) = state.lock() {
                                    st2.push_log(format!(
                                        "🧹 [LIVE] {} cancel_all (id yok, geniş iptal)", symbol,
                                    ));
                                }
                            }
                            Err(e) => {
                                if let Ok(mut st2) = state.lock() {
                                    st2.push_log(format!(
                                        "⚠️ [LIVE] {} cancel_all_orders hatası: {:?} (orphan SL/TP olabilir)",
                                        symbol, e,
                                    ));
                                }
                            }
                        }
                    }
                }
                // Eşlemeyi temizle (pozisyon artık yok).
                if let Ok(st2) = state.lock() {
                    if let Ok(mut map) = st2.finance.live_orders.write() {
                        map.remove(symbol);
                    }
                }
                // 2. Pozisyonu market emir ile kapat.
                match executor.close_position(symbol).await {
                    Ok(resp) => {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "💱 [LIVE] {} close ({:?}) ✓ order={}",
                                symbol, reason,
                                resp.get("orderId").map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
                            ));
                        }
                    }
                    Err(e) => {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "❌ [LIVE] {} close hatası: {:?} — paper tarafı yine de kapanacak",
                                symbol, e,
                            ));
                        }
                    }
                }
            }
        }

        let mut st = state.lock().unwrap();

        if let Some(pos) = target_pos {
            // Çıkış fiyatı: SL/TP/Trailing pos'taki seviye. StrategySignal'da ise
            // open_paper_position entry policy'siyle simetrik olmak için önce
            // fleet.live_price (REST 5sn snapshot), yoksa candles.last().close.
            //
            // Eski davranış: hep candles.last().close → DB mumu 15dk eskiyse
            // entry live, exit DB → asimetri sahte PnL üretiyordu (ScalpSwing
            // dispatch'i bu döngüyü saatlik fiyat farkına göre büyütmüştü).
            let fleet_live_price = st.fleet.live_price.read().ok()
                .and_then(|m| m.get(symbol).copied())
                .filter(|&v| v > 0.0);
            let exit_price = match reason {
                ExitReason::StopLoss | ExitReason::Breakeven => pos.stop_loss,
                ExitReason::TakeProfit                       => pos.take_profit,
                ExitReason::TrailingStop                     => pos.trailing_stop,
                ExitReason::StrategySignal => fleet_live_price.unwrap_or(last_candle.close),
            };
            let exit_price = if exit_price > 0.0 { exit_price } else { last_candle.close };

            // Phase C: TRAILING_STOP kapanışı sonrası 60sn olgunluk gözlemi için kuyruğa al.
            // Periyodik processor (spawn_trail_feedback_processor) bunu evalue eder ve
            // ParameterStore.record_trailing_outcome ile feedback uygular.
            if matches!(reason, ExitReason::TrailingStop) {
                let now_epoch = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                enqueue_trail_observation(crate::robot::parameters::PendingTrailObservation {
                    symbol:     symbol.to_string(),
                    strategy:   pos.trade_type.clone(),
                    is_long:    pos.is_long,
                    exit_price,
                    exit_epoch: now_epoch,
                });
            }

            let pnl_val = crate::core::math::calculate_pnl(pos.entry_price, exit_price, pos.qty, pos.is_long);
            // Çıkış komisyonu (0.1%) — exit notional üzerinden
            let exit_commission = (exit_price * pos.qty) * st.tuning.commission_rate;
            if let Ok(mut costs) = st.finance.live_execution_costs.write() {
                costs.commission_usd += exit_commission;
                costs.total_cost_usd += exit_commission;
            }
            st.finance.equity += pnl_val - exit_commission;
            if st.finance.equity > st.finance.peak_equity {
                st.finance.peak_equity = st.finance.equity;
            }
            // Tüm-zaman kapalı işlem sayacı (restart'a karşı korunur).
            st.finance.closed_trades_total.fetch_add(1, Ordering::Relaxed);
            // Re-entry cooldown mührü: open_paper_position bu zamanı okuyup
            // REENTRY_COOLDOWN_SECS içinde yeniden açılışı engeller (churn koruması).
            if let Ok(mut lc) = st.finance.last_close_at.write() {
                lc.insert(symbol.to_string(), std::time::Instant::now());
            }

            let pnl_pct_val = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                (pnl_val / (pos.entry_price * pos.qty)) * 100.0
            } else { 0.0 };

            let closed_trade = ClosedTradeModel {
                symbol: symbol.to_string(),
                is_long: pos.is_long,
                exit_reason: reason.as_str().to_string(),
                pnl: pnl_val,
                pnl_pct: pnl_pct_val,
                closed_at: chrono::Utc::now().to_rfc3339(),
                opened_at: pos.opened_at.clone(),
                leverage: pos.leverage,
            };

            // [DÜZELTME]: Arşiv listesine itme işlemi izole skopa alındı
            {
                if let Ok(mut closed_list) = st.finance.live_closed_trades.write() {
                    closed_list.push(closed_trade.clone());
                }
            }

            // ─── ScalpSwing A3: kanal-bazlı stats güncellemesi ──────────────
            // pos.kind None ise Regular akış (klasik strateji yolu) → no-op.
            // Some(Scalp/Swing) ise ilgili ScalpSwingStats kanal'ına pnl
            // kaydı geçer (wins/losses/total_pnl/streak otomatik).
            if let Some(kind) = pos.kind {
                if let Ok(mut tbl) = st.brain.scalp_swing_stats.write() {
                    tbl.record_close(kind, pnl_val);
                }
            }

            st.push_log(format!(
                "{} [{}-CLOSE/{}] {} kapatıldı @ {:.2} (entry={:.2}) | Net PnL: {:.2} USDT ({:+.2}%)",
                reason.emoji(), mode_tag, reason.as_str(), symbol, exit_price, pos.entry_price, pnl_val, pnl_pct_val,
            ));

            // ─── Faz 6 (Learn): IntelligenceHub.learn_from_exit ─────────────
            // track_trade'de açılışta hangi rejim/strateji ile mühürlediysek,
            // kazanç/kayıp uçtan uca o eşleştirmeye gider.
            let mut learn_recorded = false;
            if !pos.pos_id.is_empty() {
                let pid = crate::core::types::PositionId::from_str_or_new(&pos.pos_id);
                let mut hub_summary: Option<(usize, String)> = None;
                if let Ok(mut hub) = st.brain.intelligence_hub.write() {
                    hub.learn_from_exit(pid, pnl_pct_val);
                    hub_summary = Some((hub.controller.consecutive_failures, hub.controller.state.to_string()));
                    learn_recorded = true;
                }
                if let Some((cf, controller_state)) = hub_summary {
                    st.push_log(format!(
                        "🧠 Hub öğrendi: pos_id={}… pnl={:+.2}% · ardışık kayıp={} · controller={}",
                        &pos.pos_id[..pos.pos_id.len().min(8)],
                        pnl_pct_val, cf, controller_state,
                    ));
                }
            }
            // Learn mark — hub.write() gerçekten çalıştıysa Done; aksi halde Skipped
            // (eski/legacy pos_id eşleşmedi). Helper yerine inline yazıyoruz çünkü
            // `st` lock zaten elde; relock yapmak gereksiz. Skipped durumunda
            // helper'la aynı anomaly emit edilir (TUI Anomaliler paneline düşer).
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                use crate::robot::data_pipeline::{canon::PipelineStage, StepStatus,
                    AnomalyKind, AnomalySeverity};
                let status = if learn_recorded { StepStatus::Done } else { StepStatus::Skipped };
                pipe.mark_stage_completed(PipelineStage::Learn, status);
                if matches!(status, StepStatus::Skipped) {
                    pipe.push_anomaly(
                        AnomalySeverity::Warning,
                        AnomalyKind::RiskBreach,
                        format!("{} fazı atlandı: pos_id eşleşmedi (legacy pozisyon)", PipelineStage::Learn.label()),
                    );
                }
            }

            // 📝 Periyodik dosya logu: TRADE_CLOSE. Logger Arc'ını clone'la, IO için
            // mutex'i bırakmadan önce gerekli alanları kopyala.
            let logger_for_event = st.trading_logger.clone();
            let equity_now = st.finance.equity;
            let strategy_name = st.brain.live_strategy.read()
                .map(|s| s.clone()).unwrap_or_else(|_| "?".to_string());

            drop(st); // Q-Table alt işçisi çağrılmadan önce ana kilit tamamen imha edilir (Fail-Safe)

            if let Some(logger) = logger_for_event {
                let ev = crate::robot::infra::logger::TradeEvent::trade_close(
                    symbol, &strategy_name, pos.is_long, exit_price, pos.qty,
                    pnl_val, equity_now, reason.as_str(), pos.leverage,
                );
                let _ = logger.log_event(&ev);
            }
            Self::update_cognitive_memory(state, &closed_trade);

            // ─── Faz 3 c2: rejim-bazlı trade feedback rafinasyonu ───────────
            // Kapanış candles'tan anlık rejimi hesapla; ParameterStore'a pnl_pct'yi
            // bildir. Yeterli veri biriktiyse (WINDOW=10) ve win_rate eşiği (0.40)
            // altına düştüyse o rejim için patch otomatik sıkılaştırılır.
            let regime_at_close = Self::classify_regime(candles);
            let regime_key = regime_at_close.as_str().to_string();
            let tightened = {
                let st = state.lock().ok();
                let mut tightened = false;
                if let Some(st) = st {
                    if let Ok(mut params) = st.brain.parameters.write() {
                        tightened = params.apply_trade_feedback(&regime_key, pnl_pct_val);
                    }
                }
                tightened
            };
            if tightened {
                push_state_log(state, format!(
                    "🛡️ Adaptive: rejim '{}' düşük win-rate → patch sıkılaştırıldı",
                    regime_key,
                ));
            }

            // ─── Faz 5 (Execute): kapanış icrası tamamlandı ─────────────────
            // Açılış open_paper_position'da işaretleniyor; kapanış da bir Execute
            // adımı sayılır (cancel_orders + arşivleme + reward feedback).
            Self::mark_pipeline_stage(
                state,
                crate::robot::data_pipeline::canon::PipelineStage::Execute,
                crate::robot::data_pipeline::StepStatus::Done,
            );
        } else {
            // target_pos None: positions.remove(symbol) o anda sembolü bulamadı.
            // Bu yetim kapanış sinyali; sessizce yutulursa closed_trades muhasebe
            // boşluğu doğar. Hem push_log hem anomaly emit edilir.
            st.push_log(format!(
                "🧾 [CLOSE-NO-POS] {} kapanış istendi ama live_positions'da yok (yetim) — reason={}",
                symbol, reason.as_str(),
            ));
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity};
                pipe.push_anomaly(
                    AnomalySeverity::Warning,
                    AnomalyKind::Custom,
                    format!(
                        "Yetim kapanış: {} (reason={}) — live_positions'da kayıt yok",
                        symbol, reason.as_str(),
                    ),
                );
            }
            drop(st); // else dalında lock henüz canlıydı; persist'ten önce serbest bırak
        }

        // 💾 RECOVERY: pozisyon haritası değişti (kapanış sonrası). Boş harita
        // da snapshot'a yazılır → restart sonrası "tamamı kapalı" doğru yansır.
        // İki daldan sonra çağrılır: if-Some içinde st 2704'te drop edildi,
        // else dalında yukarıda drop edildi.
        Self::persist_open_positions_snapshot(state);
        // Equity + peak + closed sayacı kapanışta değişti → DB'ye yansıt.
        Self::persist_account_state(state);
        // Phase Executing göstergesi için: en son trade epoch'unu kaydet.
        Self::mark_execution_epoch(state);
    }


    /// 🧠 BİLİŞSEL HAFIZA: Q-Table ödül/ceza sistemi.
    pub fn update_cognitive_memory(state: &Arc<Mutex<AppState>>, last_trade: &ClosedTradeModel) {
        let mut st = state.lock().unwrap();
        let reward = crate::core::math::calculate_trade_reward(last_trade.pnl_pct, 0, 0.0);
        st.push_log(format!("🧠 Tecrübe Mühürlendi: {} | Ödül: {:.2}", last_trade.symbol, reward));
    }
}
