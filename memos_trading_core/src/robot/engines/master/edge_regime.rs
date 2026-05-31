// src/robot/engines/master/edge_regime.rs — Edge skoru + rejim sınıflandırma/drift/patch + ATR yardımcıları.
// Faz 2 modülerleştirme: loop_core.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

impl Engine {


    /// Edge skoru: son 20 mumun momentum gücü (ATR'ye göre normalize) ile ML confidence ortalaması.
    /// Sinyal yönü momentum ile uyumlu değilse ceza uygulanır.
    ///
    /// Momentum gücü = |ham getiri / ATR%|, 1.0'a clamp'lenir. Yani 20 mum içinde fiyatın ATR'nin
    /// en az 1 katı yön yapması "tam güç" sayılır. Ham getiriyi kullanmak yerine ATR normalizasyonu
    /// 1m gibi düşük volatilite timeframe'lerinde edge'in pratik olarak ölçülebilir kalmasını sağlar.
    pub(crate) fn compute_edge_score(candles: &[Candle], signal: &Signal, ml_confidence: f64) -> f64 {
        // Canlı yol sabit ters-momentum cezası 0.4 ile çağırır (davranış birebir).
        // Parametrik sürüm backtest A/B'si (#3) için ayrı eşik denemesine izin verir.
        Self::compute_edge_score_with(candles, signal, ml_confidence, 0.4)
    }

    /// `compute_edge_score`'un ters-momentum cezası parametrik sürümü. `reverse_penalty`:
    /// sinyal momentumla ters yöndeyse uygulanan dir_match çarpanı (canlı: 0.4).
    /// Düşürmek (örn. 0.2) ters girişleri daha çok bastırır; 1.0 cezayı kaldırır.
    pub(crate) fn compute_edge_score_with(
        candles: &[Candle], signal: &Signal, ml_confidence: f64, reverse_penalty: f64,
    ) -> f64 {
        if candles.len() < 20 { return 0.0; }
        let recent = &candles[candles.len() - 20..];
        let first = recent.first().map(|c| c.close).unwrap_or(0.0);
        let last  = recent.last().map(|c| c.close).unwrap_or(0.0);
        if first <= 0.0 || last <= 0.0 { return 0.0; }
        let mom = ((last - first) / first).clamp(-1.0, 1.0); // göreli getiri

        // Momentum'u ATR%'ye göre normalize et: kaç ATR yön yapıldı?
        let atr = Self::calc_atr(candles, 14);
        let atr_pct = if last > 0.0 { (atr / last).max(1e-6) } else { 1e-6 };
        let mom_strength = if atr_pct > 1e-6 {
            (mom.abs() / atr_pct).clamp(0.0, 1.0)
        } else {
            mom.abs().clamp(0.0, 1.0)
        };

        let dir_match = match signal {
            Signal::Buy  if mom > 0.0  => 1.0,
            Signal::Sell if mom < 0.0  => 1.0,
            Signal::Hold               => 0.0,
            _                          => reverse_penalty, // ters yön sinyali → ceza
        };
        let ml = ml_confidence.clamp(0.0, 1.0);
        // ML henüz hazır değilse (0.0) momentum tek başına baskın olsun.
        let ml_w = if ml < f64::EPSILON { 0.0 } else { 0.5 };
        let mom_w = 1.0 - ml_w;
        (dir_match * (mom_strength * mom_w + ml * ml_w)).clamp(0.0, 1.0)
    }

    /// Dinamik edge eşiği: ML modeli henüz hazır değilken (confidence ≈ 0) momentum tek başına
    /// taşıyıcı, bu yüzden daha gevşek eşik. ML hazırlandıkça daha katı bir filtreye geçilir.
    pub(crate) fn dynamic_edge_threshold(ml_confidence: f64) -> f64 {
        if ml_confidence < 0.05 { 0.20 }
        else if ml_confidence < 0.30 { 0.35 }
        else { 0.55 }
    }

    /// Faz 3 c3: rejim drift gözlemi. Önceki cycle'dan farklı bir rejime
    /// geçildiyse store kendi içinde patch'i bir basamak daha sıkılaştırır;
    /// burada push_alert ile kullanıcıya bildirim gönderiyoruz (Telegram + UI log).
    /// İlk gözlem değişim sayılmaz (cold start).
    pub(crate) fn observe_regime_drift(state: &Arc<Mutex<AppState>>, regime: &str) {
        let drift = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            st.brain.parameters.write().ok().map(|mut p| p.observe_regime(regime)).unwrap_or(false)
        };
        if !drift { return; }
        if let Ok(mut st) = state.lock() {
            let key = format!("REGIME-DRIFT-{}", regime);
            let msg = format!(
                "🌪️ Rejim drift: '{}' → patch sıkılaştırıldı (savunmacı duruş)",
                regime,
            );
            st.push_alert(
                &key,
                crate::robot::infra::telegram_notifier::Severity::Warning,
                msg,
            );
        }
    }

    /// Faz 3 c1: ilk kez görülen rejim için ParameterStore'da otomatik heuristic
    /// patch yerleştirir. Patch zaten varsa (manuel ya da önceki cycle'da yazıldı)
    /// dokunmaz — HyperOpt veya manuel override'ın ezilmesini önler.
    /// Boş heuristic patch ise (Weak* / Unknown) hiç yazılmaz.
    pub(crate) fn ensure_regime_patch(state: &Arc<Mutex<AppState>>, regime: &str) {
        let needs_patch = state.lock().ok()
            .and_then(|st| st.brain.parameters.read().ok()
                .map(|p| !p.regime_overrides.contains_key(regime)))
            .unwrap_or(false);
        if !needs_patch { return; }

        let patch = crate::robot::parameters::adaptive::default_patch_for_regime(regime);
        if patch.is_empty() { return; }

        if let Ok(mut st) = state.lock() {
            if let Ok(mut params) = st.brain.parameters.write() {
                params.set_regime_patch(regime, patch);
            }
            st.push_log(format!(
                "📐 Rejim '{}' ilk kez görüldü → adaptive patch uygulandı (Faz 3)",
                regime,
            ));
        }
    }

    /// Pipeline canon aşamasını "bitti" olarak işaretler ve Failed/Skipped
    /// durumlarda otomatik bir Anomaly emit eder (TUI Pipeline sekmesindeki
    /// "🛡️ Aktif Anomaliler" panelinde gözükür).
    ///
    /// - `Done` → sadece chain_steps güncellenir.
    /// - `Failed` → Critical anomaly + chain_steps.
    /// - `Skipped` → Warning anomaly + chain_steps (örn. RiskGate Deny; bot
    ///   sinyali bilerek reddetti, kullanıcının görmesi faydalı).
    /// - diğer (Idle/Running) → sadece chain_steps; anomaly üretilmez.
    ///
    /// state.lock() + live_pipeline.write() kısa scope'lu; lock contention yok.
    pub(crate) fn mark_pipeline_stage(
        state: &Arc<Mutex<AppState>>,
        stage: crate::robot::data_pipeline::canon::PipelineStage,
        status: crate::robot::data_pipeline::StepStatus,
    ) {
        use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity, StepStatus};
        if let Ok(st) = state.lock() {
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                pipe.mark_stage_completed(stage, status);
                let (severity, kind, msg) = match status {
                    StepStatus::Failed => (
                        AnomalySeverity::Critical,
                        AnomalyKind::DataStall,
                        format!("{} fazı başarısız: cycle bu aşamada koptu", stage.label()),
                    ),
                    StepStatus::Skipped => (
                        AnomalySeverity::Warning,
                        AnomalyKind::RiskBreach,
                        format!("{} fazı atlandı: koruma/red akışı tetiklendi", stage.label()),
                    ),
                    _ => return, // Done / Idle / Running anomaly üretmez
                };
                pipe.push_anomaly(severity, kind, msg);
            }
        }
    }

    /// 🌐 Mum dizisinden evolution::MarketRegime çıkar (IntelligenceHub'a yöne duyarlı sinyal).
    /// Tek-kaynak: mantık `logic::market_regime::classify_market_regime`'de (RegimeContext
    /// detektörü ve backtest agregasyonu da aynı fn'i kullanır). Bu metot ince delege.
    pub(crate) fn classify_regime(candles: &[Candle]) -> crate::evolution::MarketRegime {
        crate::robot::logic::market_regime::classify_market_regime(candles)
    }

    /// 🌐 ADIM 1 — Rejim bağlamı (cache'li, HTF-tercihli, seyrek). Cycle hot-path bunu
    /// `classify_regime` yerine çağırır: TTL içinde cache'ten OKUR (her 500ms yeniden
    /// hesaplamaz); bayat/yok ise pluggable dedektörle (`default_regime_detector()`:
    /// math→onnx) yeniden üretir ve cache'e yazar. HTF dilimi yeterliyse (≥20 mum)
    /// rejim ONDAN üretilir (hedef: AI/regime geniş TF'de seyrek çalışsın); yoksa base
    /// mumlardan (cold-start = eski `classify_regime` ile birebir, aynı tek-kaynak fn).
    /// `ttl_secs == 0` → cache bypass, her çağrı yeniden hesaplar (legacy davranış).
    pub(crate) fn regime_for_cycle(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        base_candles: &[Candle],
        base_interval: &str,
        htf_slice: Option<&[Candle]>,
        ttl_secs: u64,
        gbt_enabled: bool,
        regime_adaptive_pctl: Option<f64>,
    ) -> crate::evolution::MarketRegime {
        use crate::robot::logic::regime_context::{build_context, default_regime_detector, RegimeContext};
        use crate::robot::logic::market_regime::{
            adaptive_thresholds, classify_market_regime_with,
            compute_adx_from_candles, compute_atr_pct,
        };
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64).unwrap_or(0);
        let ttl_ms = ttl_secs.saturating_mul(1000);

        // 1) Taze cache var mı? (kısa read-lock)
        if ttl_ms > 0 {
            if let Ok(st) = state.lock() {
                if let Ok(cache) = st.brain.regime_context.read() {
                    if let Some(ctx) = cache.get(symbol) {
                        if ctx.is_fresh(now_ms, ttl_ms) {
                            return ctx.regime;
                        }
                    }
                }
            }
        }

        // 2) Bayat/yok/bypass → yeniden üret (SEYREK: TTL'de bir / cold-start). Yapı
        //    (Ranging/Volatile/Trending) HTF yeterliyse ondan (geniş TF), değilse base.
        let (det_candles, src) = match htf_slice {
            Some(h) if h.len() >= 20 => (
                h,
                crate::robot::data_pipeline::orchestrator::DataPipeline::get_htf_interval(base_interval),
            ),
            _ => (base_candles, base_interval),
        };

        // Adım 1 — GBT YÖN skoru: yalnız burada (refresh, seyrek), edge hot-path'inde
        // DEĞİL. GBT EĞİTİLDİĞİ TF'deki mumlarla beslenir (train/infer tutarlılığı):
        // HTF'de eğitildiyse (hedef mimari) ve det HTF ise det_candles; base eğitimliyse
        // base_candles; eşleşme yoksa (HTF eğitimli ama HTF mum yok vb.) None → momentum
        // yönü. Trending rejimin yönünü besler; eğitilmemiş/kapalı → None.
        let dir_score: Option<f64> = if gbt_enabled {
            state.lock().ok().and_then(|st| {
                st.brain.intelligence_hub.read().ok().and_then(|hub| {
                    let input: Option<&[Candle]> = match hub.gbt_trained_interval.as_deref() {
                        Some(ti) if ti == src           => Some(det_candles),
                        Some(ti) if ti == base_interval => Some(base_candles),
                        _ => None,
                    };
                    input.and_then(|c| hub.regime_direction_score(c))
                })
            })
        } else {
            None
        };

        // Adaptif Volatile eşiği (opt-in): set ise det_candles'ın kendi ATR% dağılımından
        // türetilir → sınıflandırma sembol-relatif Volatile sınırı kullanır. None →
        // sabit (Default) eşik = mevcut davranış birebir (parite korunur).
        let thr = match regime_adaptive_pctl {
            Some(p) => adaptive_thresholds(det_candles, p),
            None => Default::default(),
        };

        // dir_score varsa GBT-zenginleştirilmiş; yoksa pluggable detector (math; ileride
        // onnx). Adaptif eşik kapalıyken (None) eski yollar birebir korunur (parite);
        // açıkken ikisi de tek-kaynak classify_market_regime_with'e iner (eşik-farkında).
        let ctx = match (dir_score, regime_adaptive_pctl) {
            (Some(s), _) => RegimeContext {
                regime: classify_market_regime_with(det_candles, Some(s), &thr),
                adx: compute_adx_from_candles(det_candles),
                atr_pct: compute_atr_pct(det_candles),
                source_interval: src.to_string(),
                computed_at_ms: now_ms,
                detector: "gbt",
            },
            (None, Some(_)) => RegimeContext {
                regime: classify_market_regime_with(det_candles, None, &thr),
                adx: compute_adx_from_candles(det_candles),
                atr_pct: compute_atr_pct(det_candles),
                source_interval: src.to_string(),
                computed_at_ms: now_ms,
                detector: "math-adaptive",
            },
            (None, None) => {
                let detector = default_regime_detector();
                build_context(detector.as_ref(), det_candles, src, now_ms)
            }
        };
        let regime = ctx.regime;
        if ttl_ms > 0 {
            if let Ok(st) = state.lock() {
                if let Ok(mut cache) = st.brain.regime_context.write() {
                    cache.insert(symbol.to_string(), ctx);
                }
            }
        }
        regime
    }

    /// 📏 ATR (Average True Range) — son N mum üzerinde Wilder-style.
    /// Tarihçe yetersizse 0.0 döner (trailing devre dışı sayılır).
    pub(crate) fn calc_atr(candles: &[Candle], period: usize) -> f64 {
        let n = candles.len();
        if n < period + 1 { return 0.0; }
        let slice = &candles[n - period - 1..];
        let mut sum = 0.0;
        for w in slice.windows(2) {
            let prev = &w[0]; let cur = &w[1];
            let h_l = cur.high - cur.low;
            let h_pc = (cur.high - prev.close).abs();
            let l_pc = (cur.low  - prev.close).abs();
            sum += h_l.max(h_pc).max(l_pc);
        }
        sum / period as f64
    }
}
