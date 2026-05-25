// src/robot/engines/master/userdata.rs — Live user-data stream olayları + partial-fill
// Faz 1 modülerleştirme: master.rs'ten taşındı (davranış birebir korunur).
use super::*;

impl Engine {

    /// userDataStream'den gelen JSON event'i parse et: FILLED, PARTIALLY_FILLED,
    /// REJECTED, EXPIRED durumlarını ayrıştırıp ilgili işleyiciyi çağırır.
    /// NEW ve CANCELED sessizce yutulur (normal yaşam döngüsü).
    /// `pub` çünkü entegrasyon testlerinde gerçek JSON'la uçtan uca doğrulanır.
    pub async fn handle_user_data_event(state: &Arc<Mutex<AppState>>, raw: &str) {
        let v: serde_json::Value = match serde_json::from_str(raw) {
            Ok(v) => v, Err(_) => return,
        };

        // Spot: executionReport     → X=status, s=symbol, q=orig_qty, z=cum_qty, l=last_qty,
        //                             S=side, i=orderId, r=rejection reason ("NONE" yok ise)
        // Futures: ORDER_TRADE_UPDATE → o.X=status, o.s=symbol, o.q=orig_qty, o.z=cum_qty,
        //                             o.l=last_qty, o.S=side, o.i=orderId
        let event_type = v.get("e").and_then(|x| x.as_str()).unwrap_or("").to_owned();
        let parse_f = |o: &serde_json::Value, k: &str| -> f64 {
            o.get(k).and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0)
        };
        let parse_s = |o: &serde_json::Value, k: &str| -> String {
            o.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned()
        };
        let parse_id = |o: &serde_json::Value, k: &str| -> String {
            // orderId tipik olarak sayı; bazen string de gelir. İkisini de yakalayalım.
            o.get(k).map(|x| match x {
                serde_json::Value::String(s) => s.clone(),
                _ => x.to_string(),
            }).unwrap_or_default()
        };
        // L = bu event'te dolan kısmın ortalama fiyatı (last_filled_price).
        let (status, symbol, orig_qty, cum_qty, last_qty, last_price, side, order_id, reject_reason) =
            match event_type.as_str() {
                "executionReport" => (
                    parse_s(&v, "X"), parse_s(&v, "s"),
                    parse_f(&v, "q"), parse_f(&v, "z"), parse_f(&v, "l"), parse_f(&v, "L"),
                    parse_s(&v, "S"), parse_id(&v, "i"), parse_s(&v, "r"),
                ),
                "ORDER_TRADE_UPDATE" => {
                    let o = v.get("o").cloned().unwrap_or_default();
                    (
                        parse_s(&o, "X"), parse_s(&o, "s"),
                        parse_f(&o, "q"), parse_f(&o, "z"), parse_f(&o, "l"), parse_f(&o, "L"),
                        parse_s(&o, "S"), parse_id(&o, "i"), parse_s(&o, "r"),
                    )
                }
                _ => return, // diğer event'ler ignored (account update vb.)
            };

        match status.as_str() {
            "FILLED" => Self::process_user_fill_status(state, &status, &symbol).await,
            "PARTIALLY_FILLED" =>
                Self::process_partial_fill(
                    state, &symbol, &side, orig_qty, cum_qty, last_qty, last_price,
                ).await,
            "REJECTED" | "EXPIRED" =>
                Self::process_order_anomaly(
                    state, &status, &symbol, &side, &order_id, orig_qty, &reject_reason,
                ).await,
            _ => {} // NEW, CANCELED, TRADE → sessiz (normal yaşam döngüsü)
        }
    }

    /// 🛑 REJECTED / EXPIRED — emir borsada açılamadı/iptal oldu.
    /// Sebep çoğunlukla LOT_SIZE / MIN_NOTIONAL / INSUFFICIENT_BALANCE / GTX-as-taker.
    /// `apply_filters` ön kontrolüyle önlenmesi gerekiyordu; yine de düşerse hem
    /// push_log hem repair_log'a yazılır (kullanıcı görsün ve operatör doğrulasın).
    pub(crate) async fn process_order_anomaly(
        state: &Arc<Mutex<AppState>>,
        status: &str,
        symbol: &str,
        side: &str,
        order_id: &str,
        orig_qty: f64,
        reject_reason: &str,
    ) {
        if symbol.is_empty() { return; }
        // Spot'ta `r="NONE"` gelirse sebep yok demektir. Boş veya NONE olan değeri gizle.
        let reason_part = if reject_reason.is_empty() || reject_reason == "NONE" {
            String::new()
        } else {
            format!(" · sebep={}", reject_reason)
        };
        let side_part = if side.is_empty() { String::new() } else { format!(" {}", side) };
        let id_part = if order_id.is_empty() { String::new() } else { format!(" order={}", order_id) };

        if let Ok(mut st) = state.lock() {
            // Telegram: REJECTED → Critical, EXPIRED → Warning. Throttle key sembol+status.
            let severity = if status == "REJECTED" {
                crate::robot::infra::telegram_notifier::Severity::Critical
            } else {
                crate::robot::infra::telegram_notifier::Severity::Warning
            };
            let key = format!("WS-{}-{}", status, symbol);
            st.push_alert(
                &key,
                severity,
                format!(
                    "[WS-{}] {}{} qty={:.4}{}{}",
                    status, symbol, side_part, orig_qty, id_part, reason_part,
                ),
            );
            st.guardian.repair_log.push_back(format!(
                "[{}] {}: {}{} qty={:.4}{}{}",
                chrono::Local::now().format("%H:%M:%S"),
                status, symbol, side_part, orig_qty, id_part, reason_part,
            ));
            while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
        }
    }

    /// 🌓 PARTIAL fill — emirin bir bölümü dolu. İki tür var:
    ///
    /// **ENTRY partial** (side pozisyonun yönüyle aynı: LONG için BUY, SHORT için SELL):
    ///   - Local qty `cum_qty`'e hizalanır (gerçekte bu kadar tutuyoruz).
    ///   - Sadece komisyon equity'den düşülür; realize PnL yok.
    ///
    /// **CLOSE partial** (side pozisyonu kapatıyor: LONG için SELL, SHORT için BUY):
    ///   - Local qty bu event'te kapanan kadar (`last_qty`) azalır.
    ///   - Realize PnL = (last_price − entry_price) × last_qty × yön; equity'e işlenir.
    ///   - Komisyon ayrıca düşülür; live_execution_costs.commission_usd büyür.
    ///
    /// `pub` çünkü entegrasyon testleri (partial fill PnL muhasebesi) bunu doğrular.
    pub async fn process_partial_fill(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        side: &str,
        orig_qty: f64,
        cum_qty: f64,
        last_qty: f64,
        last_price: f64,
    ) {
        if symbol.is_empty() || orig_qty <= 0.0 || last_qty <= 0.0 { return; }
        let fill_pct = (cum_qty / orig_qty * 100.0).clamp(0.0, 100.0);
        const COMMISSION_RATE: f64 = 0.001; // 0.1% — open/close ile aynı

        // 1. Pozisyonu oku, entry vs close sınıflandır.
        let (is_long, entry_price, current_price, local_qty_before) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let positions = match st.finance.live_positions.read() { Ok(p) => p, Err(_) => return };
            match positions.get(symbol) {
                Some(pos) => (pos.is_long, pos.entry_price, pos.current_price, pos.qty),
                None => return, // bot bilmediği bir sembol için event aldı
            }
        };
        let is_closing = (is_long && side == "SELL") || (!is_long && side == "BUY");

        // 2. Fiyat 0 ise (executor pratikte 0 dönmez ama defensive guard) realize PnL
        //    hesaplayamayız; o zaman sadece qty güncelle ve log at — komisyon da
        //    notional 0 olur. Çağıran zaten WS payload'unu doğrudan veriyor.
        let trade_notional = last_qty * last_price;
        let commission = trade_notional * COMMISSION_RATE;
        let realized_pnl = if is_closing && last_price > 0.0 {
            crate::core::math::calculate_pnl(entry_price, last_price, last_qty, is_long)
        } else { 0.0 };

        // 3. State mutation — pozisyon qty + equity + execution_costs.
        let new_qty: Option<f64> = if let Ok(mut st) = state.lock() {
            let mutated = if let Ok(mut positions) = st.finance.live_positions.write() {
                positions.get_mut(symbol).map(|pos| {
                    if is_closing {
                        pos.qty = (pos.qty - last_qty).max(0.0);
                    } else {
                        // Entry partial: cum kadar gerçekten tutuyoruz
                        pos.qty = cum_qty;
                    }
                    pos.qty
                })
            } else { None };

            if mutated.is_some() {
                if let Ok(mut costs) = st.finance.live_execution_costs.write() {
                    costs.commission_usd += commission;
                    costs.total_cost_usd += commission;
                }
                // Realize PnL sadece kapanış partial'inde; ENTRY partial'de equity
                // sadece komisyon kadar azalır (notional henüz realize değil).
                if is_closing {
                    st.finance.equity += realized_pnl - commission;
                } else {
                    st.finance.equity -= commission;
                }
            }
            mutated
        } else { None };

        // 4. Log + audit.
        if let Some(new_q) = new_qty {
            let kind_tag = if is_closing { "CLOSE" } else { "ENTRY" };
            let pnl_part = if is_closing {
                format!(" · pnl=${:+.2}", realized_pnl)
            } else { String::new() };
            push_state_log(state, format!(
                "🌓 [WS-PARTIAL-{}] {} %{:.1} ({} {:.4} @ {:.4}) · qty {:.4} → {:.4}{} · fee=${:.4}",
                kind_tag, symbol, fill_pct, side, last_qty, last_price,
                local_qty_before, new_q, pnl_part, commission,
            ));

            // 5. Anomali tespiti → Telegram push_alert.
            //    Üç kriter; her biri farklı throttle anahtarına bağlandı, sembol başına
            //    bağımsız cooldown takip eder (BTCUSDT'nin uyarısı ETHUSDT'yi susturmaz).
            Self::detect_partial_fill_anomalies(
                state, symbol, side, fill_pct, orig_qty, cum_qty,
                last_qty, last_price, local_qty_before, entry_price,
                current_price, is_closing, is_long,
            );
        }
    }

    /// Partial fill anomalilerini değerlendirir; eşik aşıldığında push_alert atar.
    ///
    /// 3 kriter:
    ///   - OVERFILL (Critical): `last_qty > local_qty_before * 1.001`. Borsa local
    ///     pozisyondan fazla doldurmuş → bot ↔ borsa qty ayrışması. equity ve risk
    ///     hesabı bozulur; muhasebe için kritik.
    ///   - CUM_INCONSISTENT (Warning): `cum_qty > orig_qty * 1.001`. Borsa toplam
    ///     fill'i emrin orig_qty'sinden büyük raporladı; payload tutarsızlığı.
    ///   - SLIPPAGE (Warning): adverse fiyat sapması eşiği aştı. Beklenen referans
    ///     CLOSE partial'de pozisyonun `current_price`'ı, ENTRY partial'de
    ///     `entry_price`. Eşik env `PARTIAL_FILL_MAX_SLIPPAGE_PCT` (default 1.0%).
    ///     `side` bot tarafından bakılır: BUY → daha pahalıya alındı, SELL → daha
    ///     ucuza satıldı negatif.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn detect_partial_fill_anomalies(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        side: &str,
        fill_pct: f64,
        orig_qty: f64,
        cum_qty: f64,
        last_qty: f64,
        last_price: f64,
        local_qty_before: f64,
        entry_price: f64,
        current_price: f64,
        is_closing: bool,
        is_long: bool,
    ) {
        use crate::robot::infra::telegram_notifier::Severity;
        // Faz 2: sabit eşikler yerine ParameterStore'dan oku (HyperOpt/manuel update
        // runtime'da değişiklik yapabilsin). Lock alınamazsa legacy default fallback.
        let pf = state.lock().ok()
            .and_then(|st| st.brain.parameters.read().ok().map(|p| p.partial_fill))
            .unwrap_or_default();

        // 1) OVERFILL: borsa pozisyondan fazla doldurmuş (close partial için anlamlı).
        //    Entry partial'de cum henüz local'in üstüne çıkamaz tanım gereği, ama yine
        //    de defensive olarak kontrol ediyoruz.
        if local_qty_before > 0.0
            && last_qty > local_qty_before * (1.0 + pf.overfill_tolerance)
        {
            let key = format!("PARTIAL-ANOMALY-OVERFILL-{}", symbol);
            let msg = format!(
                "[PARTIAL-ANOMALY-OVERFILL] {} side={} last_qty={:.6} > local_qty={:.6} \
                 (cum={:.6}/orig={:.6}) — bot↔borsa qty ayrışması",
                symbol, side, last_qty, local_qty_before, cum_qty, orig_qty,
            );
            if let Ok(mut st) = state.lock() {
                st.push_alert(&key, Severity::Critical, msg);
            }
        }

        // 2) CUM tutarsız: borsa cum'u emrin orig_qty'sinden büyük raporladı.
        if cum_qty > orig_qty * (1.0 + pf.cum_tolerance) {
            let key = format!("PARTIAL-ANOMALY-CUM-{}", symbol);
            let msg = format!(
                "[PARTIAL-ANOMALY-CUM] {} cum={:.6} > orig={:.6} (%{:.1}) — borsa payload tutarsız",
                symbol, cum_qty, orig_qty, fill_pct,
            );
            if let Ok(mut st) = state.lock() {
                st.push_alert(&key, Severity::Warning, msg);
            }
        }

        // 3) SLIPPAGE: bot tarafına göre adverse fiyat sapması.
        if last_price > 0.0 {
            let expected = if is_closing { current_price } else { entry_price };
            if expected > 0.0 {
                let adverse_pct = match side {
                    "BUY"  => (last_price - expected) / expected * 100.0,
                    "SELL" => (expected - last_price) / expected * 100.0,
                    _ => 0.0,
                };
                let threshold_pct = pf.max_slippage_pct;
                if adverse_pct > threshold_pct {
                    let kind = if is_closing { "CLOSE" } else { "ENTRY" };
                    let key = format!("PARTIAL-ANOMALY-SLIPPAGE-{}-{}", kind, symbol);
                    let dir = if is_long { "LONG" } else { "SHORT" };
                    let msg = format!(
                        "[PARTIAL-ANOMALY-SLIPPAGE] {} {} {} side={} fill@{:.6} \
                         vs beklenen {:.6} → adverse %{:.3} (eşik %{:.2})",
                        symbol, dir, kind, side, last_price, expected, adverse_pct, threshold_pct,
                    );
                    if let Ok(mut st) = state.lock() {
                        st.push_alert(&key, Severity::Warning, msg);
                    }
                }
            }
        }
    }

    /// FILLED event'inin tek/uniform işleyicisi (spot + futures için ortak).
    pub(crate) async fn process_user_fill_status(state: &Arc<Mutex<AppState>>, status: &str, symbol: &str) {
        if status != "FILLED" || symbol.is_empty() { return; }

        let (executor, db_path, interval, has_local) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let has = st.finance.live_positions.read().map(|p| p.contains_key(symbol)).unwrap_or(false);
            (st.live_executor.clone(), st.config.db_path.clone(),
             st.config.interval.clone(), has)
        };
        if !has_local { return; } // bot bilmediği bir sembol için event aldı

        if let Some(exec) = executor {
            let _ = exec.cancel_all_orders(symbol).await;
            push_state_log(state, format!(
                "🛰️ [WS-FILL] {} FILLED yakalandı → orphan emirler temizlendi, local pozisyon kapatılıyor",
                symbol,
            ));
        }

        if let Ok(candles) = crate::persistence::reader::read_candles(&db_path, symbol, &interval, 5) {
            if !candles.is_empty() {
                Self::close_paper_position(state, symbol, &candles, ExitReason::TrailingStop).await;
            }
        }
    }
}
