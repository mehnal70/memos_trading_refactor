// src/robot/engines/master/fleet_tuners.rs — Otonom periyodik tuner task'ları:
// scalp/swing tuner, trail-feedback processor, S/R updater.
// Faz 2 modülerleştirme: infra_fleet.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

impl Engine {
    /// ScalpSwing A4: periyodik tuner task. scalp_swing_stats'i okuyup
    /// `auto_tune(stats, Scalp, cfg)` ve `auto_tune(stats, Swing, cfg)`
    /// çağırır; değişiklikleri config'e yazar + log push. Stop signal
    /// veya SCALP_SWING_TUNE_DISABLE=1 ile devre dışı bırakılır.
    pub(crate) fn spawn_scalp_swing_tuner(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            if std::env::var("SCALP_SWING_TUNE_DISABLE").ok().as_deref() == Some("1") {
                push_state_log(&state, "🎚️ ScalpSwing tuner: DISABLE=1, task pasif".into());
                return;
            }
            let every_secs: u64 = env_parse("SCALP_SWING_TUNE_EVERY_SECS", 300);

            // Tuner devrede olduğunu boot'ta operatöre bildir — aksi halde
            // "tuner çalışmadı mı?" şüphesi oluşuyordu (summary boş olunca
            // hiç log atılmıyor → görünürlük kayboluyordu).
            push_state_log(&state, format!(
                "🎚️ ScalpSwing tuner devrede (periyot={}sn, min trade=5)",
                every_secs,
            ));

            // İlk turda warmup için kısa bekle — boot anında stats boş olur.
            tokio::time::sleep(std::time::Duration::from_secs(every_secs.min(30))).await;

            loop {
                let stop = state.lock()
                    .map(|s| s.app_stop_signal.load(Ordering::Relaxed))
                    .unwrap_or(true);
                if stop { break; }

                // 1) Stats + cfg snapshot (tek kısa mutex; sleep'ten önce drop).
                let cfg_stats = {
                    if let Ok(st) = state.lock() {
                        let cfg = st.brain.scalp_swing_config.read().ok().map(|c| c.clone());
                        let stats = st.brain.scalp_swing_stats.read().ok().map(|t| t.clone());
                        cfg.zip(stats)
                    } else { None }
                };

                // 2) auto_tune çağrıları + yazma (mutex'ler block içinde).
                let mut summary: Vec<String> = Vec::new();
                if let Some((mut cfg, stats)) = cfg_stats {
                    if cfg.autonomous_tuning {
                        if stats.scalp.total_closed >= 5 {
                            summary.extend(crate::robot::scalp_swing::auto_tune(
                                &stats.scalp,
                                crate::robot::scalp_swing::TradeType::Scalp,
                                &mut cfg,
                            ));
                        }
                        if stats.swing.total_closed >= 5 {
                            summary.extend(crate::robot::scalp_swing::auto_tune(
                                &stats.swing,
                                crate::robot::scalp_swing::TradeType::Swing,
                                &mut cfg,
                            ));
                        }
                        // A6: Otonom kanal-kapama. 20+ trade'lik yeterli sample
                        // varsa ve win_rate < 0.30 ise kanal kalıcı olarak
                        // pasifleştirilir → ScalpSwing fırsatı üretmeyi durur.
                        // Operatör override: cfg.scalp_enabled/swing_enabled
                        // manuel re-enable (UI veya config dosyası).
                        if cfg.scalp_enabled
                            && stats.scalp.total_closed >= 20
                            && stats.scalp.win_rate() < 0.30
                        {
                            cfg.scalp_enabled = false;
                            summary.push(format!(
                                "SCP Auto-Disabled (wr={:.2}, n={})",
                                stats.scalp.win_rate(), stats.scalp.total_closed,
                            ));
                        }
                        if cfg.swing_enabled
                            && stats.swing.total_closed >= 20
                            && stats.swing.win_rate() < 0.30
                        {
                            cfg.swing_enabled = false;
                            summary.push(format!(
                                "SWG Auto-Disabled (wr={:.2}, n={})",
                                stats.swing.win_rate(), stats.swing.total_closed,
                            ));
                        }
                        if !summary.is_empty() {
                            if let Ok(st) = state.lock() {
                                if let Ok(mut w) = st.brain.scalp_swing_config.write() {
                                    *w = cfg;
                                }
                            }
                        }
                    }
                }

                // 3) Log (yine kısa scope).
                if !summary.is_empty() {
                    push_state_log(&state, format!(
                        "🎚️ ScalpSwing tuner: {} ayar uygulandı → {}",
                        summary.len(), summary.join(", "),
                    ));
                }

                tokio::time::sleep(std::time::Duration::from_secs(every_secs)).await;
            }
        });
    }

    /// Phase C processor: olgunlaşmış trailing observation'ları evalue edip
    /// ParameterStore'a feedback yansıtır. Her 10sn'de bir tarama yapar — ne çok
    /// agresif (kuyruk hızla büyür mü kontrol), ne de çok az (60sn olgunluk için yeter).
    pub(crate) fn spawn_trail_feedback_processor(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            const MATURE_SECS: u64 = 60;
            const POLL_SECS:   u64 = 10;
            const STALE_SECS:  u64 = 300; // 5dk: live_price yoksa gözlem düşer

            loop {
                let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                // Olgun + stale ayrımı yapıp tek scope'lu queue manipülasyonu.
                let now = crate::core::time::now_epoch_secs();
                let mature: Vec<crate::robot::parameters::PendingTrailObservation> = {
                    if let Ok(mut q) = trail_obs_queue().lock() {
                        let mut keep = std::collections::VecDeque::with_capacity(q.len());
                        let mut take = Vec::new();
                        while let Some(o) = q.pop_front() {
                            if now.saturating_sub(o.exit_epoch) >= STALE_SECS {
                                continue; // 5dk üstü → düşür
                            }
                            if o.is_mature(MATURE_SECS) { take.push(o); }
                            else { keep.push_back(o); }
                        }
                        *q = keep;
                        take
                    } else { Vec::new() }
                };

                // Evalue + record — tek state.lock altında batch.
                if !mature.is_empty() {
                    if let Ok(st) = state.lock() {
                        let live_price = st.fleet.live_price.read().ok().map(|g| g.clone()).unwrap_or_default();
                        if let Ok(mut params) = st.brain.parameters.write() {
                            for obs in &mature {
                                let cur = match live_price.get(&obs.symbol).copied() {
                                    Some(v) if v > 0.0 => v,
                                    _ => continue, // fiyat yok → atla (kuyruktan zaten alındı, drop edilir)
                                };
                                let was_early = obs.evaluate(cur);
                                let changed = params.record_trailing_outcome(&obs.symbol, &obs.strategy, was_early);
                                if let Some(new_target) = changed {
                                    log::info!(
                                        "🎯 Trail feedback patch: {} ({}) → target={:.2}%",
                                        obs.symbol, obs.strategy, new_target,
                                    );
                                }
                            }
                        }
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
            }
        });
    }

    /// 📐 Periyodik S/R updater — aktif sembol setini gezer, son 200 candle üzerinden
    /// `SrDetector::detect` çağırıp `fleet.live_sr_zones` HashMap'ini günceller.
    ///
    /// Aktif sembol seti: `config.symbol` + `config.pinned_symbols` + orchestrator
    /// worker'ları (yinelemeler atılır). DB'de yeterli candle yoksa sembol atlanır.
    /// İlk turda warmup yok — bot ilk açıldığında TUI hemen dolu görünür.
    pub(crate) fn spawn_sr_updater(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            if std::env::var("SR_UPDATER_DISABLE").ok().as_deref() == Some("1") {
                push_state_log(&state, "📐 SR updater: SR_UPDATER_DISABLE=1, task pasif".into());
                return;
            }
            // Faz 2: interval ParameterStore'dan okunur. SR_UPDATE_EVERY_SECS env'i
            // store.from_env'de boot anında zaten alındı; runtime'da brain.parameters
            // güncellenirse bu task da sonraki turda yeni aralığı görür.
            let interval_secs: u64 = state.lock().ok()
                .and_then(|st| st.brain.parameters.read().ok().map(|p| p.sr_update_every_secs))
                .unwrap_or(30);
            let detector = crate::robot::sr_detector::SrDetector::new(
                crate::robot::sr_detector::SrDetectorConfig::default()
            );
            let mut first_run_logged = false;

            loop {
                let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                // 1) Aktif sembolleri topla. Canlı feed'i olmayan borsa sembolleri
                // (örn. eski BIST mumları DB'de durur) SR/Market Gözetimi'ne girmesin →
                // market-agnostik tek nokta: RuntimeTuning.symbol_eligible_for_live.
                let (db_path, interval, market, symbols) = {
                    let st = match state.lock() { Ok(s) => s, Err(_) => break };
                    let tuning = Arc::clone(&st.tuning);
                    let eligible = |s: &str| tuning.symbol_eligible_for_live(s);

                    let mut symbols: Vec<String> = vec![];
                    if eligible(&st.config.symbol)
                        && !st.config.symbol.is_empty()
                        && !symbols.contains(&st.config.symbol)
                    {
                        symbols.push(st.config.symbol.clone());
                    }
                    for s in &st.config.pinned_symbols {
                        if !eligible(s) { continue; }
                        if !symbols.contains(s) { symbols.push(s.clone()); }
                    }
                    if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                        for w in orch.get_worker_status() {
                            if !eligible(&w.symbol) { continue; }
                            if !symbols.contains(&w.symbol) { symbols.push(w.symbol); }
                        }
                    }
                    (st.config.db_path.clone(), st.config.interval.clone(), st.config.market.clone(), symbols)
                };

                // 2) Her sembol için candles oku, SR detect — IO/CPU lock dışında yapılır.
                let mut zones_map: std::collections::HashMap<String, Vec<crate::robot::sr_detector::SrZone>>
                    = Default::default();
                // Market Gözetimi "24h %" sütunu için ~24h önceki referans fiyat (SR'dan bağımsız;
                // candles varsa hesaplanır). bridge displayed live_price ile % değişimi türetir.
                let mut ref_map: std::collections::HashMap<String, f64> = Default::default();
                let mut total_zones = 0usize;
                for sym in &symbols {
                    if let Ok(candles) = crate::persistence::reader::read_candles_market(&db_path, sym, &interval, &market, 200) {
                        let r = reference_price_24h_ago(&candles);
                        if r > 0.0 {
                            ref_map.insert(sym.clone(), r);
                        }
                        // Detect lookback=5 default; en az ~11 candle gerekir, güvenli alt sınır 20.
                        if candles.len() >= 20 {
                            let zones = detector.detect(&candles);
                            if !zones.is_empty() {
                                total_zones += zones.len();
                                zones_map.insert(sym.clone(), zones);
                            }
                        }
                    }
                }

                // 3) Yaz — kısa scope'lu write lock (zones + 24h referans).
                if let Ok(st) = state.lock() {
                    if let Ok(mut guard) = st.fleet.live_sr_zones.write() {
                        *guard = zones_map;
                    }
                    if let Ok(mut rg) = st.fleet.live_ref_price_24h.write() {
                        *rg = ref_map;
                    }
                }

                if !first_run_logged {
                    push_state_log(&state, format!(
                        "📐 SR updater: {} sembol, {} bölge, her {}sn",
                        symbols.len(), total_zones, interval_secs,
                    ));
                    first_run_logged = true;
                }

                sleep(Duration::from_secs(interval_secs)).await;
            }
        });
    }

}

/// SAF: candles → son mum timestamp'inden ~24h önceki referans close fiyatı (Market Gözetimi
/// "24h %" sütununun paydası). Tam 24h öncesine eşit/önceki en yeni mumu seçer; pencere 24h'i
/// kapsamıyorsa (örn. 1m·200=3.3h) en eski mevcut muma düşer (kısmi pencere). Boş → 0. Testli.
pub(crate) fn reference_price_24h_ago(candles: &[Candle]) -> f64 {
    let last = match candles.last() {
        Some(c) => c,
        None => return 0.0,
    };
    let cutoff = last.timestamp - chrono::Duration::hours(24);
    candles.iter().rev()
        .find(|c| c.timestamp <= cutoff)
        .map(|c| c.close)
        .unwrap_or_else(|| candles[0].close)
}

#[cfg(test)]
mod fleet_tuners_tests {
    use super::*;

    fn candle(ts_offset_h: i64, close: f64) -> Candle {
        Candle {
            timestamp: chrono::Utc::now() - chrono::Duration::hours(ts_offset_h.max(0)),
            close,
            ..Default::default()
        }
    }

    #[test]
    fn reference_24h_picks_bar_at_or_before_cutoff() {
        // Son mum şimdi (0h), 24h önce close=100, 12h önce close=110, şimdi close=120.
        // 24h sınırına eşit/önceki en yeni mum = 24h'lik (100).
        let candles = vec![candle(48, 90.0), candle(24, 100.0), candle(12, 110.0), candle(0, 120.0)];
        assert!((reference_price_24h_ago(&candles) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn reference_24h_partial_window_falls_back_to_oldest() {
        // Pencere yalnız 3h (1m benzeri kısa) → 24h öncesi yok → en eski mum referans.
        let candles = vec![candle(3, 200.0), candle(2, 205.0), candle(1, 210.0), candle(0, 215.0)];
        assert!((reference_price_24h_ago(&candles) - 200.0).abs() < 1e-9);
        assert_eq!(reference_price_24h_ago(&[]), 0.0, "boş → 0");
    }
}
