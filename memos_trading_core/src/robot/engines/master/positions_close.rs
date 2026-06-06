// src/robot/engines/master/positions_close.rs — Exit denetimi + pozisyon kapanışı + cognitive memory.
// Faz 2 modülerleştirme: positions.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

impl Engine {

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
                let now_epoch = crate::core::time::now_epoch_secs();
                enqueue_trail_observation(crate::robot::parameters::PendingTrailObservation {
                    symbol:     symbol.to_string(),
                    strategy:   pos.trade_type.clone(),
                    is_long:    pos.is_long,
                    exit_price,
                    exit_epoch: now_epoch,
                });
            }

            let pnl_val = crate::core::math::calculate_pnl(pos.entry_price, exit_price, pos.qty, pos.is_long);
            // Çıkış komisyonu — exit notional üzerinden. KESİTSEL maker icra: XS pozisyonu (trade_type=XS tag)
            // + USE_LIMIT_ENTRY → maker oranı (açılışla simetrik; net edge maker'da doğrulandı).
            let exit_rate = if pos.trade_type == crate::robot::engines::master::xs_live::XS_STRATEGY_TAG
                && st.tuning.use_limit_entry { st.tuning.maker_commission_rate }
                else { st.tuning.commission_rate };
            let exit_commission = (exit_price * pos.qty) * exit_rate;
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
            // 🪜 Kademeli giriş durumunu temizle: pozisyon kapandı → kademe sayacı/hedef sıfırlanır
            // (sonraki açılış taze 1. kademeden başlar). Kayıt yoksa no-op.
            if let Ok(mut gm) = st.finance.graded_tranches.write() {
                gm.remove(symbol);
            }

            let pnl_pct_val = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                (pnl_val / (pos.entry_price * pos.qty)) * 100.0
            } else { 0.0 };

            // NET P&L (dürüst gösterim): gross − round-trip komisyon. entry_commission açılışta
            // pozisyona mühürlendi (open_paper_position), exit_commission yukarıda hesaplandı. Bir
            // BREAKEVEN gross 0 olsa da net round-trip fee'yi yansıtır → "+0.00" yanıltması biter.
            // `pnl`/`pnl_pct` (gross) win-rate/skorlama tüketicileri için DEĞİŞMEDEN kalır.
            let round_trip_commission = pos.entry_commission + exit_commission;
            let net_pnl_val = pnl_val - round_trip_commission;
            let net_pnl_pct_val = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                (net_pnl_val / (pos.entry_price * pos.qty)) * 100.0
            } else { 0.0 };

            let closed_trade = ClosedTradeModel {
                symbol: symbol.to_string(),
                is_long: pos.is_long,
                exit_reason: reason.as_str().to_string(),
                pnl: pnl_val,
                pnl_pct: pnl_pct_val,
                net_pnl: net_pnl_val,
                net_pnl_pct: net_pnl_pct_val,
                commission: round_trip_commission,
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
            // Strateji etiketi: pozisyonun KENDİ mührü (pos.trade_type) — açılış dosya-logu
            // (TradeEvent::trade_open, strategy_name) ile SİMETRİK. Global live_strategy ("AUTO"
            // → "Otonom") DEĞİL: aksi halde XS_MOMENTUM/ScalpSwing açılışları kapanışta tek bir
            // "Otonom" kovasına karışır → strateji-bazlı realize P&L (uzun paper izlemenin temeli)
            // ölçülemezdi. trade_type açılışta her zaman strategy_name ile mühürlenir. [[project_xs_momentum]]
            let strategy_name = if pos.trade_type.is_empty() {
                "?".to_string()
            } else {
                pos.trade_type.clone()
            };

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
