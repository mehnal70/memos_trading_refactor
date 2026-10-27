// src/robot/engines/master/positions_close.rs — Exit denetimi + pozisyon kapanışı + cognitive memory.
// Faz 2 modülerleştirme: positions.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

pub(crate) fn extract_fill_price(resp: &serde_json::Value) -> Option<f64> {
    let parse = |k: &str| {
        resp.get(k)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
    };
    if let Some(p) = parse("avgPrice") {
        if p > 0.0 {
            return Some(p);
        }
    }
    match (parse("cummulativeQuoteQty"), parse("executedQty")) {
        (Some(c), Some(e)) if c > 0.0 && e > 0.0 => return Some(c / e),
        _ => return None,
    }
}

impl Engine {
    pub fn check_exit_conditions(
        position: &mut PositionModel,
        last_price: f64,
        atr: f64,
        atr_trail_mult: f64,
        breakeven_rr: f64,
        exit_commission_rate: f64,
    ) -> Option<ExitReason> {
        return Self::check_exit_conditions_ohlc(
            position,
            last_price,
            last_price,
            last_price,
            atr,
            atr_trail_mult,
            breakeven_rr,
            exit_commission_rate,
        );
    }

    fn fee_protected_exit_price(
        position: &PositionModel,
        exit_commission_rate: f64,
    ) -> Option<f64> {
        if position.entry_price <= 0.0 || position.qty <= 0.0 {
            return None;
        }
        let entry_fee_per_unit = position.entry_commission / position.qty;
        if position.is_long {
            let denom = 1.0 - exit_commission_rate;
            if denom <= 0.0 {
                return None;
            }
            Some((position.entry_price + entry_fee_per_unit) / denom)
        } else {
            let denom = 1.0 + exit_commission_rate;
            if denom <= 0.0 {
                return None;
            }
            Some((position.entry_price - entry_fee_per_unit) / denom)
        }
    }

    pub fn check_exit_conditions_ohlc(
        position: &mut PositionModel,
        high: f64,
        low: f64,
        close: f64,
        atr: f64,
        atr_trail_mult: f64,
        breakeven_rr: f64,
        exit_commission_rate: f64,
    ) -> Option<ExitReason> {
        if close <= 0.0 {
            return None;
        }

        let (favorable, adverse) = if position.is_long {
            (high, low)
        } else {
            (low, high)
        };

        if position.is_long {
            if favorable > position.max_favorable_price {
                position.max_favorable_price = favorable;
            }
        } else if position.max_favorable_price == 0.0 || favorable < position.max_favorable_price {
            position.max_favorable_price = favorable;
        }

        if position.stop_loss > 0.0 {
            let hit = if position.is_long {
                adverse <= position.stop_loss
            } else {
                adverse >= position.stop_loss
            };
            if hit {
                return Some(if position.breakeven_activated {
                    ExitReason::Breakeven
                } else {
                    ExitReason::StopLoss
                });
            }
        }

        if position.take_profit > 0.0 {
            let hit = if position.is_long {
                favorable >= position.take_profit
            } else {
                favorable <= position.take_profit
            };
            if hit {
                return Some(ExitReason::TakeProfit);
            }
        }

        if !position.breakeven_activated && position.entry_price > 0.0 && position.stop_loss > 0.0 {
            let risk = (position.entry_price - position.stop_loss).abs();
            if risk > 0.0 {
                let gain = if position.is_long {
                    close - position.entry_price
                } else {
                    position.entry_price - close
                };
                if gain >= risk * breakeven_rr {
                    position.breakeven_activated = true;
                    position.stop_loss =
                        Self::fee_protected_exit_price(position, exit_commission_rate)
                            .unwrap_or(position.entry_price);
                }
            }
        }

        if atr > 0.0 && atr_trail_mult > 0.0 {
            let delta = atr * atr_trail_mult;
            let fee_guard = Self::fee_protected_exit_price(position, exit_commission_rate);
            // Trailing, pozisyon lehine en az bir ATR ilerlemeden silahlanmaz.
            // Aksi halde açılışta görülen ilk tick/candle hareketi hemen kapanışa
            // dönüşebilir; gerçek dönüşü beklemek için sabit SL/TP çalışmaya devam eder.
            let trail_armed = if position.is_long {
                position.max_favorable_price >= position.entry_price + delta
            } else {
                position.max_favorable_price <= position.entry_price - delta
            };
            if !trail_armed {
                return None;
            }
            if position.is_long {
                let new_trail = position.max_favorable_price - delta;
                let new_trail = match fee_guard {
                    Some(floor) => new_trail.max(floor),
                    None => new_trail,
                };
                if new_trail > position.trailing_stop {
                    position.trailing_stop = new_trail;
                }
                if position.trailing_stop > 0.0 && adverse <= position.trailing_stop {
                    return Some(ExitReason::TrailingStop);
                }
            } else {
                let new_trail = position.max_favorable_price + delta;
                let new_trail = match fee_guard {
                    Some(ceiling) => new_trail.min(ceiling),
                    None => new_trail,
                };
                if position.trailing_stop == 0.0 || new_trail < position.trailing_stop {
                    position.trailing_stop = new_trail;
                }
                if position.trailing_stop > 0.0 && adverse >= position.trailing_stop {
                    return Some(ExitReason::TrailingStop);
                }
            }
        }

        return None;
    }
    pub(crate) async fn close_paper_position(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        candles: &[Candle],
        reason: ExitReason,
    ) {
        let last_candle = match candles.last() {
            Some(c) => c,
            None => return,
        };

        if matches!(reason, ExitReason::StrategySignal) {
            let (min_hold_secs, opened_at_str) = state
                .lock()
                .ok()
                .map(|st| {
                    (
                        st.tuning.min_holding_secs_strategy,
                        st.finance
                            .live_positions
                            .read()
                            .ok()
                            .and_then(|p| p.get(symbol).map(|pos| pos.opened_at.clone())),
                    )
                })
                .unwrap_or((RuntimeTuning::default().min_holding_secs_strategy, None));
            if let Some(s) = opened_at_str {
                if let Ok(opened) = chrono::DateTime::parse_from_rfc3339(&s) {
                    let age_secs =
                        (chrono::Utc::now() - opened.with_timezone(&chrono::Utc)).num_seconds();
                    if age_secs < min_hold_secs {
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

        let (target_pos, live_executor, live_dry_run, mode_tag) = {
            let mut st = lock_state_recover(state);
            st.fleet.phase = "Executing".into();
            let pending_key = st
                .finance
                .live_positions
                .read()
                .ok()
                .and_then(|positions| {
                    positions
                        .get(symbol)
                        .and_then(|position| position.canonical_key())
                })
                .or_else(|| {
                    st.venue_registry.for_symbol(symbol).map(|venue| {
                        memos_venue_api::CanonicalPositionKey::new(
                            venue.venue_key(),
                            symbol.to_string(),
                        )
                    })
                });
            let close_pending = pending_key
                .as_ref()
                .and_then(|key| {
                    st.finance
                        .pending_closes
                        .read()
                        .ok()
                        .map(|pending| pending.contains_key(key))
                })
                .unwrap_or(false);
            if close_pending {
                st.push_log(format!(
                    "⏳ [CLOSE-PENDING] {} broker kapanışı henüz reconcile ile doğrulanmadı",
                    symbol
                ));
                return;
            }
            // Bu branch, tuning içindeki kaldırılmış sembol-gate metoduna bağlı kalmadan
            // yalnız trading mode üzerinden canlı kapanış yolunu belirler.
            let live_path_enabled = matches!(
                st.config.trading_mode,
                crate::core::model::TradingMode::Live
            );
            let routed_venue = st.venue_registry.route(symbol).map(|(venue, _)| venue);
            let exec = if live_path_enabled {
                routed_venue
                    .as_ref()
                    .filter(|venue| {
                        crate::robot::robotic_loop::live_execution_is_armed()
                            && venue.execution_engine().is_some()
                    })
                    .map(|venue| std::sync::Arc::clone(*venue))
            } else {
                None
            };
            if live_path_enabled && routed_venue.is_some() && exec.is_none() {
                st.push_log(format!(
                    "🛑 [LIVE-READ-ONLY] {} close ({:?}) reddedildi — venue execution kapalı; local closed trade oluşturulmadı",
                    symbol, reason,
                ));
                return;
            }
            let target_pos = st.remove_live_position(symbol);
            let dry = st.live_dry_run;
            let tag = if exec.is_some() && !dry {
                "LIVE"
            } else if exec.is_some() && dry {
                "DRY-RUN"
            } else if live_path_enabled {
                "LIVE-FALLBACK-PAPER"
            } else {
                "PAPER"
            };
            (target_pos, exec, dry, tag)
        };

        let mut live_fill_price: Option<f64> = None;
        if let Some(executor) = live_executor.as_ref() {
            if live_dry_run {
                if let Ok(mut st2) = state.lock() {
                    st2.push_log(format!(
                        "🟡 [LIVE-DRY-RUN] {} close ({:?}) → emir gönderilmedi",
                        symbol, reason,
                    ));
                    if let Some(pos) = target_pos.as_ref() {
                        st2.insert_live_position(symbol.to_string(), pos.clone());
                    }
                }
                Self::persist_open_positions_snapshot(state);
                return;
            } else {
                let refs = state.lock().ok().and_then(|s| {
                    s.finance
                        .live_orders
                        .read()
                        .ok()
                        .and_then(|m| m.get(symbol).cloned())
                });

                let cancel_result = if let Some(refs) = refs {
                    let mut summary: Vec<String> = Vec::new();
                    if let Some(sl_id_str) = refs.sl_order_id.as_deref() {
                        match executor.cancel_order(symbol, sl_id_str).await {
                            Ok(_) => summary.push(format!("SL#{} ✓", sl_id_str)),
                            Err(e) => summary.push(format!("SL#{} ❌ {:?}", sl_id_str, e)),
                        }
                    }
                    if let Some(tp_id_str) = refs.tp_order_id.as_deref() {
                        match executor.cancel_order(symbol, tp_id_str).await {
                            Ok(_) => summary.push(format!("TP#{} ✓", tp_id_str)),
                            Err(e) => summary.push(format!("TP#{} ❌ {:?}", tp_id_str, e)),
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
                                "🧹 [LIVE] {} hedefli iptal: {}",
                                symbol,
                                summary.join(" · "),
                            ));
                        }
                    }
                    _ => match executor.cancel_all(symbol).await {
                        Ok(_) => {
                            if let Ok(mut st2) = state.lock() {
                                st2.push_log(format!(
                                    "🧹 [LIVE] {} cancel_all (id yok, geniş iptal)",
                                    symbol,
                                ));
                            }
                        }
                        Err(e) => {
                            if let Ok(mut st2) = state.lock() {
                                st2.push_log(format!(
                                        "🧹 [LIVE] {} cancel_all_orders hatası: {:?} (orphan SL/TP olabilir)",
                                        symbol, e,
                                    ));
                                st2.push_log(format!(
                                        "⚠️ [LIVE] {} cancel_all_orders hatası: {:?} (orphan SL/TP olabilir)",
                                        symbol, e,
                                    ));
                            }
                        }
                    },
                }
                if let Ok(st2) = state.lock() {
                    if let Ok(mut map) = st2.finance.live_orders.write() {
                        let _ = map.remove(symbol);
                    }
                }
                match executor.close_position(symbol).await {
                    Ok(resp) => {
                        live_fill_price = (resp.avg_price > 0.0).then_some(resp.avg_price);
                        let mut pending_persist = None;
                        if let Ok(mut st2) = state.lock() {
                            if let Ok(mut pending) = st2.finance.pending_closes.write() {
                                if let Some(key) = target_pos
                                    .as_ref()
                                    .and_then(|position| position.canonical_key())
                                    .or_else(|| {
                                        st2.venue_registry.for_symbol(symbol).map(|venue| {
                                            memos_venue_api::CanonicalPositionKey::new(
                                                venue.venue_key(),
                                                symbol.to_string(),
                                            )
                                        })
                                    })
                                {
                                    let broker_order_id = resp
                                        .venue_order_id
                                        .clone()
                                        .unwrap_or_else(|| "acknowledged".to_string());
                                    pending.insert(key.clone(), broker_order_id.clone());
                                    pending_persist = Some((key, broker_order_id));
                                }
                            }
                            st2.push_log(format!(
                                "💱 [LIVE] {} close ({:?}) ✓ order={} fill={}",
                                symbol,
                                reason,
                                resp.venue_order_id.as_deref().unwrap_or("?"),
                                live_fill_price
                                    .map(|p| format!("{:.4}", p))
                                    .unwrap_or_else(|| "?".into()),
                            ));
                        }
                        if let Some((key, broker_order_id)) = pending_persist {
                            Self::persist_pending_close(state, key, broker_order_id).await;
                        }
                        if let Some(pos) = target_pos.as_ref() {
                            if let Ok(st2) = state.lock() {
                                st2.insert_live_position(symbol.to_string(), pos.clone());
                            }
                        }
                        Self::persist_open_positions_snapshot(state);
                        return;
                    }
                    Err(e) => {
                        if let Ok(mut st2) = state.lock() {
                            st2.push_log(format!(
                                "❌ [LIVE] {} close hatası: {:?} — pozisyon local'de korunuyor",
                                symbol, e,
                            ));
                        }
                        if let Some(pos) = target_pos.as_ref() {
                            if let Ok(st2) = state.lock() {
                                st2.insert_live_position(symbol.to_string(), pos.clone());
                            }
                        }
                        Self::persist_open_positions_snapshot(state);
                        return;
                    }
                }
            }
        }

        let mut st = lock_state_recover(state);

        if let Some(pos) = target_pos {
            // Aynı yerel pozisyon ikinci kez reconcile edilirse finansal state ve
            // TRADE_CLOSE kaydı tekrar üretilmemeli.
            if !pos.pos_id.is_empty()
                && st
                    .finance
                    .live_closed_trades
                    .read()
                    .ok()
                    .map(|closed| {
                        closed.iter().any(|existing| {
                            (!pos.pos_id.is_empty() && existing.exchange_ref == pos.pos_id)
                                || (pos.pos_id.is_empty()
                                    && existing.symbol == symbol
                                    && existing.opened_at == pos.opened_at
                                    && (existing.entry_price - pos.entry_price).abs() < 1e-8)
                        })
                    })
                    .unwrap_or(false)
            {
                return;
            }
            let fleet_live_price = st
                .fleet
                .live_price
                .read()
                .ok()
                .and_then(|m| m.get(symbol).copied())
                .filter(|&v| v > 0.0);
            let exit_price = match reason {
                ExitReason::StopLoss | ExitReason::Breakeven => pos.stop_loss,
                ExitReason::TakeProfit => pos.take_profit,
                ExitReason::TrailingStop => pos.trailing_stop,
                // Borsa kapattı: gerçek fill fiyatı bilinmiyor → son bilinen canlı fiyat.
                ExitReason::ExchangeClosed => fleet_live_price.unwrap_or(last_candle.close),
                ExitReason::StrategySignal => fleet_live_price.unwrap_or(last_candle.close),
            };
            let exit_price = if exit_price > 0.0 {
                exit_price
            } else {
                last_candle.close
            };
            let exit_price = live_fill_price.filter(|&p| p > 0.0).unwrap_or(exit_price);

            if matches!(reason, ExitReason::TrailingStop) {
                let now_epoch = crate::core::time::now_epoch_secs();
                enqueue_trail_observation(crate::robot::parameters::PendingTrailObservation {
                    symbol: symbol.to_string(),
                    strategy: pos.trade_type.clone(),
                    is_long: pos.is_long,
                    exit_price,
                    exit_epoch: now_epoch,
                });
            }
            let pnl_val =
                crate::core::math::calculate_pnl(pos.entry_price, exit_price, pos.qty, pos.is_long);
            let exit_rate = if (pos.trade_type
                == crate::robot::engines::master::xs_live::XS_STRATEGY_TAG
                || pos.trade_type == crate::robot::engines::master::carry_live::CARRY_STRATEGY_TAG
                || pos.trade_type == crate::robot::engines::master::blend_live::BLEND_STRATEGY_TAG)
                && st.tuning.use_limit_entry
            {
                st.tuning.maker_commission_rate
            } else {
                st.tuning.commission_rate
            };
            let exit_commission = (exit_price * pos.qty) * exit_rate;
            if let Ok(mut costs) = st.finance.live_execution_costs.write() {
                costs.commission_usd += exit_commission;
                costs.total_cost_usd += exit_commission;
            }
            st.finance.equity += pnl_val - exit_commission;
            if st.finance.equity > st.finance.peak_equity {
                st.finance.peak_equity = st.finance.equity;
            }
            st.finance
                .closed_trades_total
                .fetch_add(1, Ordering::Relaxed);
            if let Ok(mut lc) = st.finance.last_close_at.write() {
                let _ = lc.insert(symbol.to_string(), std::time::Instant::now());
            }
            let pyramid_tranches = st
                .finance
                .graded_tranches
                .read()
                .ok()
                .and_then(|tranches| tranches.get(symbol).map(|state| state.tranches_filled))
                .unwrap_or(1);
            if let Ok(mut gm) = st.finance.graded_tranches.write() {
                let _ = gm.remove(symbol);
            }

            let pnl_pct_val = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                (pnl_val / (pos.entry_price * pos.qty)) * 100.0
            } else {
                0.0
            };

            let round_trip_commission = pos.entry_commission + exit_commission;
            let net_pnl_val = pnl_val - round_trip_commission;
            let net_pnl_pct_val = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                (net_pnl_val / (pos.entry_price * pos.qty)) * 100.0
            } else {
                0.0
            };
            let venue = pos
                .venue
                .map(|key| key.exchange.as_str().to_string())
                .unwrap_or_else(|| {
                    crate::robot::venue::registry::split_venue_tag(symbol)
                        .1
                        .unwrap_or_else(|| crate::core::types::Exchange::classify(symbol))
                        .as_str()
                        .to_string()
                });

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
                entry_price: pos.entry_price,
                exit_price,
                strategy: pos.trade_type.clone(),
                venue,
                pyramid_tranches,
                exchange_ref: pos.pos_id.clone(),
            };

            {
                if let Ok(mut closed_list) = st.finance.live_closed_trades.write() {
                    if !closed_list
                        .iter()
                        .any(|existing| existing.same_identity(&closed_trade))
                    {
                        closed_list.push(closed_trade.clone());
                    }
                }
            }

            if let Some(n) = st.notifier.as_ref() {
                let mark = if net_pnl_val >= 0.0 { "🟢" } else { "🔴" };
                let _ = n.notify(
                    &format!("close-{symbol}"),
                    crate::robot::infra::telegram_notifier::Severity::Info,
                    &format!(
                        "{mark} {symbol} {} kapandı · net ${:.2} ({:+.2}%) · {} · {}",
                        if pos.is_long { "LONG" } else { "SHORT" },
                        net_pnl_val,
                        net_pnl_pct_val,
                        reason.as_str(),
                        pos.trade_type
                    ),
                );
            }

            if let Some(kind) = pos.kind {
                if let Ok(mut tbl) = st.brain.scalp_swing_stats.write() {
                    tbl.record_close(kind, pnl_val);
                }
            }

            st.push_log(format!(
                "{} [{}-CLOSE/{}] {} kapatıldı @ {:.2} (entry={:.2}) | Net PnL: {:.2} USDT ({:+.2}%)",
                reason.emoji(), mode_tag, reason.as_str(), symbol, exit_price, pos.entry_price, pnl_val, pnl_pct_val,
            ));

            let mut learn_recorded = false;
            if !pos.pos_id.is_empty() {
                let pid = crate::core::types::PositionId::from_str_or_new(&pos.pos_id);
                let mut hub_summary: Option<(usize, String)> = None;
                if let Ok(mut hub) = st.brain.intelligence_hub.write() {
                    hub.learn_from_exit(pid, pnl_pct_val);
                    hub_summary = Some((
                        hub.controller.consecutive_failures,
                        hub.controller.state.to_string(),
                    ));
                    learn_recorded = true;
                }
                if let Some((cf, controller_state)) = hub_summary {
                    st.push_log(format!(
                        "🧠 Hub öğrendi: pos_id={}… pnl={:+.2}% · ardışık kayıp={} · controller={}",
                        &pos.pos_id[..pos.pos_id.len().min(8)],
                        pnl_pct_val,
                        cf,
                        controller_state,
                    ));
                }
            }
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                use crate::robot::data_pipeline::{
                    canon::PipelineStage, AnomalyKind, AnomalySeverity, StepStatus,
                };
                let status = if learn_recorded {
                    StepStatus::Done
                } else {
                    StepStatus::Skipped
                };
                pipe.mark_stage_completed(PipelineStage::Learn, status);
                if matches!(status, StepStatus::Skipped) {
                    let _ = pipe.push_anomaly(
                        AnomalySeverity::Warning,
                        AnomalyKind::RiskBreach,
                        format!(
                            "{} fazı atlandı: pos_id eşleşmedi (legacy pozisyon)",
                            PipelineStage::Learn.label()
                        ),
                    );
                }
            }

            let logger_for_event = st.trading_logger.clone();
            let equity_now = st.finance.equity;
            let strategy_name = if pos.trade_type.is_empty() {
                "?".to_string()
            } else {
                pos.trade_type.clone()
            };

            if pnl_val > 0.0 {
                Self::bump_indicator_counters(&mut st, &strategy_name, "win_contrib");
            } else if pnl_val < 0.0 {
                Self::bump_indicator_counters(&mut st, &strategy_name, "loss_contrib");
            }
            let exit_suffix = format!("exit_{}", reason.as_str().to_ascii_lowercase());
            Self::bump_indicator_counters(&mut st, &strategy_name, &exit_suffix);

            drop(st);

            if let Some(logger) = logger_for_event {
                let ev = crate::robot::infra::logger::TradeEvent::trade_close(
                    symbol,
                    &strategy_name,
                    pos.is_long,
                    exit_price,
                    pos.qty,
                    pnl_val,
                    equity_now,
                    reason.as_str(),
                    pos.leverage,
                );
                let _ = logger.log_event(&ev);
            }
            Self::update_cognitive_memory(state, &closed_trade);

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
                push_state_log(
                    state,
                    format!(
                        "🛡️ Adaptive: rejim '{}' düşük win-rate → patch sıkılaştırıldı",
                        regime_key,
                    ),
                );
            }

            Self::mark_pipeline_stage(
                state,
                crate::robot::data_pipeline::canon::PipelineStage::Execute,
                crate::robot::data_pipeline::StepStatus::Done,
            );
        } else {
            st.push_log(format!(
                "🧾 [CLOSE-NO-POS] {} kapanış istendi ama live_positions'da yok (yetim) — reason={}",
                symbol, reason.as_str(),
            ));
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity};
                let _ = pipe.push_anomaly(
                    AnomalySeverity::Warning,
                    AnomalyKind::Custom,
                    format!(
                        "Yetim kapanış: {} (reason={}) — live_positions'da kayıt yok",
                        symbol,
                        reason.as_str(),
                    ),
                );
            }
            drop(st);
        }

        Self::persist_open_positions_snapshot(state);
        Self::persist_account_state(state);
        Self::mark_execution_epoch(state);
    }

    pub fn update_cognitive_memory(state: &Arc<Mutex<AppState>>, last_trade: &ClosedTradeModel) {
        let mut st = lock_state_recover(state);
        let reward = crate::core::math::calculate_trade_reward(last_trade.pnl_pct, 0, 0.0);
        st.push_log(format!(
            "🧠 Tecrübe Mühürlendi: {} | Ödül: {:.2}",
            last_trade.symbol, reward
        ));
    }
}
