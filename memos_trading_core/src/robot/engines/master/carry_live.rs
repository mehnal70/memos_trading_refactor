// src/robot/engines/master/carry_live.rs — FUNDING-CARRY adanmış mod (ince sarmalayıcı).
//
// CARRY_LIVE_ENABLED iken sepet sembolleri funding-carry kitabıyla (market-nötr long/short) yönetilir:
// yüksek-funding SHORT (funding alır) / düşük/negatif-funding LONG → perp taşıma SPREAD'ini hasat.
// Edge fiyat hareketinde DEĞİL, funding ödemelerinde (yapısal dik; ρ≈0.11 momentum'la). WF-OOS
// Newey-West p=0.032 doğrulandı. Tüm kitap makinesi `book_core` ORTAK motorunda (xs⊕carry DRY):
// burada yalnız carry'ye özgü kısım — sinyal (−trailing funding), funding-tazelik kapısı, KADANS
// (rebalance_bars≥14, fee'ye dayanıklı iki-haftalık turnover ŞART). [[project_funding_carry]]
// [[feedback_modular_dry_perf]] [[project_maker_limit_entry]]
use super::*;
use crate::persistence::reader::read_funding_market;

/// Carry kitabı pozisyonlarının strateji/trade_type etiketi — açılışta mühürlenir, kapanış + komisyon
/// muhasebesi bununla carry pozisyonunu tanır (maker icra: USE_LIMIT_ENTRY iken maker oranı, momentum'la
/// simetrik). Funding-carry düşük-turnover + maker icra ister → net edge yalnız o senaryoda doğrulandı.
pub(crate) const CARRY_STRATEGY_TAG: &str = "FUNDING_CARRY";

/// Funding-carry sinyali yalnız futures'ta anlamlı (perp funding). Sabit (operatör-ayarı değil; funding
/// başka markette yok → koda gömülü doğru, env'e açılmaz [[feedback_market_agnostic]]).
const CARRY_MARKET: &str = "futures";

/// SAF: trailing funding penceresinin carry sinyali = −ortalama(funding). Yüksek funding → DÜŞÜK skor
/// → SHORT bacak (book funding'i alır); negatif funding → YÜKSEK skor → LONG bacak (funding'i alır).
/// Sıralama-tabanlı kitap için ölçek-değişmez (sabit çarpan tüm sembollerde aynı → rank korunur).
/// Boş → None (funding yok → kitaba girmez). Testli.
pub(crate) fn latest_carry_signal(rates: &[f64]) -> Option<f64> {
    if rates.is_empty() {
        return None;
    }
    let mean = rates.iter().sum::<f64>() / rates.len() as f64;
    Some(-mean)
}

impl Engine {
    /// FUNDING-CARRY adanmış mod cycle adımı: `book_core::process_book` ortak motorunu carry sinyali
    /// (−trailing funding) + iki-haftalık kadans (rebalance_bars) + funding-tazelik kapısı ile çağırır.
    /// Mod kapalı/sepet yetersiz → no-op. Momentum'la ÇAKIŞMA: Faz 1'de carry+momentum aynı sepette açık
    /// koşmamalı (tek-pozisyon/sembol invariantı; loop_core sepeti normal döngüden + birbirinden ayrı ele).
    pub(crate) async fn process_carry_book(state: &Arc<Mutex<AppState>>) {
        let (cfg, tuning, db_path, lookback, funding_max_age_secs, funding_limit) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let cr = match st.brain.parameters.read().ok().map(|p| p.carry_live.clone()) {
                Some(x) if x.enabled && x.top_k >= 1 && x.symbols.len() >= 2 * x.top_k => x,
                _ => return,
            };
            let cfg = super::book_core::BookConfig {
                symbols: cr.symbols.clone(),
                interval: cr.interval.clone(),
                lookback: cr.lookback,
                top_k: cr.top_k,
                exit_buffer: cr.exit_buffer,
                // Carry sinyali zaten negatiflenmiş (−funding) → en YÜKSEK skor = en düşük funding = LONG.
                // xs_target_book "momentum=true" (en yüksek skoru long) bu yönü doğru ifade eder.
                momentum: true,
                position_pct: cr.position_pct,
                leverage: cr.leverage,
                regime_gate: cr.regime_gate,
                max_drawdown_pct: cr.max_drawdown_pct,
                cb_cooldown_secs: cr.cb_cooldown_secs,
                take_profit_pct: cr.take_profit_pct,
                tp_cooldown_secs: cr.tp_cooldown_secs,
                // ⏱️ KADANS: carry düşük-turnover ŞART (günlük rebalance fee'ye yenik) → ≥14 bar.
                rebalance_min_bars: cr.rebalance_bars.max(1),
            };
            (cfg, Arc::clone(&st.tuning), st.config.db_path.clone(),
             cr.lookback, cr.funding_max_age_secs, cr.funding_limit)
        };

        let interval_secs = crate::robot::data_pipeline::DataNormalizer::parse_interval(&cfg.interval)
            .max(1) as i64;
        let now_ms = crate::core::time::now_epoch_millis() as i64;
        // Trailing pencere: son `lookback` barlık zaman aralığındaki funding ödemeleri (mum bucket'ı
        // yerine zaman-penceresi → tek sembol için hafif yol; ortalama-funding ekonomik olarak aynı,
        // rank-değişmez). Backtest bar-bucket'lı; cross-sectional rank ikisinde de bit-aynı sıralar.
        let cutoff = now_ms - (lookback as i64) * interval_secs * 1000;
        let db_path_sig = db_path.clone();

        // 💰 Carry sinyali: trailing funding ortalaması (−). 🧊 FUNDING-TAZELİK KAPISI: son funding
        // `funding_max_age_secs`'ten eskiyse (delisted/feed durdu, örn. MKR) → None → kitaba girmez
        // (mum stale-feed kapısının funding ikizi; phantom carry önler). [[project_symbol_status_registry]]
        let signal_fn = move |sym: &str, _c: &[Candle]| {
            let funding = read_funding_market(&db_path_sig, sym, CARRY_MARKET, funding_limit)
                .unwrap_or_default();
            if let Some((last_t, _)) = funding.last() {
                if funding_max_age_secs > 0 && now_ms - *last_t > funding_max_age_secs * 1000 {
                    let age_h = (now_ms - *last_t) / 3_600_000;
                    log::debug!("💰 carry: {} funding bayat ({}sa > {}sa) → sinyalden dışlandı (phantom carry koruması)",
                        sym, age_h, funding_max_age_secs / 3600);
                    return None;
                }
            } else {
                return None; // funding hiç indirilmemiş → kitaba girmez
            }
            let rates: Vec<f64> = funding.iter().filter(|(t, _)| *t >= cutoff).map(|(_, r)| *r).collect();
            latest_carry_signal(&rates)
        };

        Self::process_book(
            state, &cfg, super::book_core::BookKind::Carry, &tuning, &db_path, signal_fn,
        ).await;
    }
}

#[cfg(test)]
mod carry_live_tests {
    use super::*;

    #[test]
    fn carry_signal_negates_mean_funding() {
        // Yüksek pozitif funding → negatif skor (short eğilimi).
        assert!((latest_carry_signal(&[0.01, 0.03]).unwrap() - (-0.02)).abs() < 1e-12);
        // Negatif funding → pozitif skor (long eğilimi).
        assert!((latest_carry_signal(&[-0.02, -0.04]).unwrap() - 0.03).abs() < 1e-12);
        assert_eq!(latest_carry_signal(&[]), None, "funding yok → None");
    }

    #[test]
    fn carry_signal_rank_orders_short_high_funding() {
        // İki sembol: A yüksek-funding (short adayı), B negatif-funding (long adayı).
        let sig_a = latest_carry_signal(&[0.05, 0.05]).unwrap(); // −0.05
        let sig_b = latest_carry_signal(&[-0.01, -0.01]).unwrap(); // +0.01
        assert!(sig_b > sig_a, "negatif-funding B, yüksek-funding A'dan daha LONG skorlu olmalı");
    }
}
