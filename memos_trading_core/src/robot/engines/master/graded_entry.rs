// src/robot/engines/master/graded_entry.rs — KADEMELİ GİRİŞ (XS HARİÇ) saf çekirdeği + canlı infaz.
//
// Pozisyonu tek seferde değil N kademede kurar. Ek kademeler REJİME GÖRE açılır: trend rejimde
// PYRAMIDING (lehte hareket → kazananı besle, riski sınırla), ranging rejimde AVERAGING (aleyhte
// hareket → ortalama maliyet). Teyit: HTF trend hizası + yeterli hareket. Tüm karar/muhasebe SAF +
// testli; canlı infaz (try_add_graded_tranche) mevcut tek-nokta muhasebe disiplinini (equity/komisyon)
// birebir izler. XS sepeti HARİÇ (kesitsel mod kendi eşit-ağırlık/tek-fill sizing'ini kullanır;
// kademeli giriş turnover-düşmanı XS edge'ini bozar). [[feedback_autonomy_first]] [[project_xs_momentum]]
use super::*;
use crate::robot::parameters::GradedTriggerMode;

/// Ek kademe modu: rejimden türetilir. Pyramid = trend (lehte ekle), Average = ranging (aleyhte ekle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrancheMode {
    Pyramid,
    Average,
}

/// SAF: rejim → kademe modu. Trend rejimler → Pyramid; Ranging/LowVol → Average; HighVol/Unknown →
/// None (belirsiz/kriz → ek kademe AÇMA, mevcut pozisyonu olduğu gibi bırak). Tek-kaynak eşleme.
pub(crate) fn tranche_mode(regime: crate::evolution::MarketRegime) -> Option<TrancheMode> {
    use crate::evolution::MarketRegime::*;
    match regime {
        StrongUptrend | WeakUptrend | StrongDowntrend | WeakDowntrend => Some(TrancheMode::Pyramid),
        Ranging | LowVolatility => Some(TrancheMode::Average),
        HighVolatility | Unknown => None,
    }
}

/// SAF: pozisyonun LEHİNE hareket yüzdesi. Long: (current−entry)/entry·100; short: tersi. Pozitif =
/// lehte (kârda), negatif = aleyhte (zararda). entry<=0 → 0.
pub(crate) fn favorable_move_pct(is_long: bool, entry: f64, current: f64) -> f64 {
    if entry <= 0.0 {
        return 0.0;
    }
    let raw = (current - entry) / entry * 100.0;
    if is_long {
        raw
    } else {
        -raw
    }
}

/// SAF: pozisyonun LEHİNE mutlak fiyat mesafesi. ATR tabanlı tetik için kullanılır.
pub(crate) fn favorable_move_abs(is_long: bool, entry: f64, current: f64) -> f64 {
    if entry <= 0.0 {
        return 0.0;
    }
    let raw = if is_long {
        current - entry
    } else {
        entry - current
    };
    raw.max(0.0)
}

/// SAF: pozisyonun ALEYHİNE mutlak fiyat mesafesi. ATR-tabanlı averaging kapıları için kullanılır.
pub(crate) fn adverse_move_abs(is_long: bool, entry: f64, current: f64) -> f64 {
    if entry <= 0.0 {
        return 0.0;
    }
    let raw = if is_long {
        entry - current
    } else {
        current - entry
    };
    raw.max(0.0)
}

/// SAF: kademe ekleme → yeni (qty, ağırlıklı-ortalama entry). new_entry = (e0·q0 + p·dq)/(q0+dq).
/// Geçersiz (dq<=0 ya da toplam<=0) → değişiklik yok.
pub(crate) fn average_in(old_qty: f64, old_entry: f64, add_qty: f64, add_price: f64) -> (f64, f64) {
    if add_qty <= 0.0 {
        return (old_qty, old_entry);
    }
    let new_qty = old_qty + add_qty;
    if new_qty <= 0.0 {
        return (old_qty, old_entry);
    }
    let new_entry = (old_entry * old_qty + add_price * add_qty) / new_qty;
    (new_qty, new_entry)
}

/// SAF: SL/TP/trailing seviyesini yeni ortalama entry'e göre ÖLÇEKLE (eski entry'e göreli %-mesafeyi
/// koru): level_new = new_entry · (level_old/old_entry). Böylece averaging sonrası stop "çok yakın"
/// kalmaz, pyramiding sonrası "çok uzak" kalmaz. level<=0 (devre dışı) ya da old_entry<=0 → değişmez.
pub(crate) fn rescale_level(level_old: f64, old_entry: f64, new_entry: f64) -> f64 {
    if level_old <= 0.0 || old_entry <= 0.0 {
        return level_old;
    }
    new_entry * (level_old / old_entry)
}

/// SAF: HTF kapanışları → trend yönü (+1 boğa / −1 ayı / 0 belirsiz). Son kapanış basit hareketli
/// ortalamanın üstündeyse +1, altındaysa −1, yetersiz veri/eşitlik → 0. Ucuz, look-ahead'siz.
pub(crate) fn htf_trend_sign(htf_closes: &[f64]) -> i8 {
    let n = htf_closes.len();
    if n < 2 {
        return 0;
    }
    let sma = htf_closes.iter().sum::<f64>() / n as f64;
    let last = htf_closes[n - 1];
    if last > sma {
        1
    } else if last < sma {
        -1
    } else {
        0
    }
}

/// SAF: HTF trend hizası kademe için uygun mu. Pyramid → trend pozisyonla AYNI yönde olmalı.
/// Average → trend pozisyona KARŞI olmamalı (nötr/lehte yeter; aleyhte trende ortalama = düşen bıçak).
pub(crate) fn htf_aligned(mode: TrancheMode, is_long: bool, htf_sign: i8) -> bool {
    let want: i8 = if is_long { 1 } else { -1 };
    match mode {
        TrancheMode::Pyramid => htf_sign == want,
        TrancheMode::Average => htf_sign != -want,
    }
}

/// SAF: tüm kapılardan geçti mi → bu cycle ek kademe açılsın mı. htf_gate çağıran tarafça hesaplanır
/// (require_htf_align kapalıysa daima true). Pyramid → lehte ≥ favorable_thr; Average → aleyhte ≥ adverse_thr.
pub(crate) fn should_add_tranche(_mode: TrancheMode, trigger_ok: bool, htf_gate: bool) -> bool {
    htf_gate && trigger_ok
}

/// SAF: pyramiding sonrası ortak güvenli çıkış seviyesi. Aggregate pozisyonun fee-protected
/// breakeven'ını baz alır; böylece ilk dalganın kârı ikinci dalganın ters dönüş zararını sıfırlayan
/// ortak bir taban oluşur.
pub(crate) fn fee_protected_breakeven_price(
    is_long: bool,
    entry_price: f64,
    qty: f64,
    entry_commission: f64,
    exit_commission_rate: f64,
) -> Option<f64> {
    if entry_price <= 0.0 || qty <= 0.0 {
        return None;
    }
    let entry_fee_per_unit = entry_commission / qty;
    if is_long {
        let denom = 1.0 - exit_commission_rate;
        if denom <= 0.0 {
            return None;
        }
        Some((entry_price + entry_fee_per_unit) / denom)
    } else {
        let denom = 1.0 + exit_commission_rate;
        if denom <= 0.0 {
            return None;
        }
        Some((entry_price - entry_fee_per_unit) / denom)
    }
}

impl Engine {
    /// Açık (XS-DIŞI) pozisyona kademeli giriş ek-kademesi dener. Kademeli giriş kapalı / pozisyon
    /// yok / tüm kademeler dolu / rejim belirsiz / eşik karşılanmadı → no-op. process_symbol_cycle
    /// exit denetiminden SONRA, yeni-açılış üretiminden ÖNCE çağrılır (açık pozisyon yolu). Muhasebe
    /// tek-lock: qty/entry ortalama, SL/TP/trailing yeniden ölçek, equity'den komisyon, kademe sayacı++.
    pub(crate) async fn try_add_graded_tranche(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        candles: &[Candle],
        db_path: &str,
    ) {
        // 1) Parametreler + kademe durumu + pozisyon görüntüsü (tek lock'ta oku).
        let snap = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            let g = match st.brain.parameters.read() {
                Ok(p) => p.graded_entry_for_symbol(symbol),
                Err(_) => return,
            };
            if !g.enabled || g.tranche_count() < 2 {
                return;
            }
            // XS sepeti HARİÇ: kesitsel mod kendi sizing'ini kullanır (kademeli giriş onu bozar).
            let is_xs = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| p.xs_live.enabled && p.xs_live.symbols.iter().any(|s| s == symbol))
                .unwrap_or(false);
            if is_xs {
                return;
            }
            let gstate = match st
                .finance
                .graded_tranches
                .read()
                .ok()
                .and_then(|m| m.get(symbol).copied())
            {
                Some(gs) if (gs.tranches_filled as usize) < g.tranche_count() => gs,
                _ => return, // kademe durumu yok (legacy/tam) ya da tüm kademeler dolu
            };
            let pos = match st
                .finance
                .live_positions
                .read()
                .ok()
                .and_then(|p| p.get(symbol).cloned())
            {
                Some(p) => p,
                None => return,
            };
            // Mark fiyatı: fleet live_price (taze) > candle close (entry/exit ile simetrik).
            let live = st
                .fleet
                .live_price
                .read()
                .ok()
                .and_then(|m| m.get(symbol).copied())
                .filter(|&v| v > 0.0);
            let commission_rate = st.tuning.commission_rate;
            (
                g,
                gstate,
                pos,
                live,
                commission_rate,
                st.config.market.clone(),
                st.config.interval.clone(),
                std::sync::Arc::clone(&st.venue_registry),
            )
        };
        let (g, gstate, pos, live, commission_rate, market, base_interval, venue_registry) = snap;
        let venue_profile = super::venue_profile::VenueProfile::for_symbol(symbol, &venue_registry);
        let max_tranches = g.tranche_count().min(venue_profile.max_pyramid_tranches);
        if max_tranches < 2 || gstate.tranches_filled as usize >= max_tranches {
            return;
        }
        let add_price = live
            .or_else(|| candles.last().map(|c| c.close))
            .unwrap_or(0.0);
        if add_price <= 0.0 {
            return;
        }

        let regime = Self::classify_regime(candles);
        if g.pyramiding_only
            && !matches!(
                regime,
                crate::evolution::MarketRegime::StrongUptrend
                    | crate::evolution::MarketRegime::WeakUptrend
                    | crate::evolution::MarketRegime::StrongDowntrend
                    | crate::evolution::MarketRegime::WeakDowntrend
            )
        {
            return;
        }

        // 2) Rejim → kademe modu (HighVol/Unknown → ek kademe yok).
        let mode = match tranche_mode(regime) {
            Some(m) => m,
            None => return,
        };
        if g.pyramiding_only && !matches!(mode, TrancheMode::Pyramid) {
            return;
        }
        if matches!(mode, TrancheMode::Pyramid) && !venue_profile.allows_pyramid(candles) {
            return;
        }

        let atr_abs = crate::core::indicators::calculate_atr(candles, 14)
            .last()
            .copied()
            .unwrap_or(0.0);

        // 3) HTF hiza kapısı (require_htf_align): üst-TF trend yönü pozisyonla uyumlu mu.
        let htf_gate = if g.require_htf_align {
            let account_profile = state
                .lock()
                .ok()
                .map(|st| st.config.venue_profile.as_str().to_string())
                .unwrap_or_else(|| "default".to_string());
            let htf = match Self::resolve_read_ns(state, symbol) {
                Some((ex, mk, bare)) => {
                    crate::robot::data_pipeline::load_htf_candles_ns_for_profile(
                        db_path,
                        &bare,
                        &base_interval,
                        &ex,
                        &mk,
                        &account_profile,
                        50,
                    )
                }
                None => crate::robot::data_pipeline::load_htf_candles(
                    db_path,
                    symbol,
                    &base_interval,
                    &market,
                    50,
                ),
            };
            let sign = htf_trend_sign(&htf.iter().map(|c| c.close).collect::<Vec<f64>>());
            htf_aligned(mode, pos.is_long, sign)
        } else {
            true
        };

        // 4) Hareket eşiği + HTF kapısı → ek kademe açılsın mı.
        let fav = favorable_move_pct(pos.is_long, pos.entry_price, add_price);
        let adverse_abs = adverse_move_abs(pos.is_long, pos.entry_price, add_price);
        let directional_ok = match mode {
            TrancheMode::Pyramid => {
                let pct_hit = fav >= g.favorable_move_pct;
                let atr_hit = atr_abs > 0.0
                    && favorable_move_abs(pos.is_long, pos.entry_price, add_price)
                        >= atr_abs * g.trigger_atr_mult.max(0.0);
                match g.trigger_mode {
                    GradedTriggerMode::Percent => pct_hit,
                    GradedTriggerMode::Atr => atr_hit,
                    GradedTriggerMode::Hybrid => pct_hit || atr_hit,
                }
            }
            TrancheMode::Average => {
                let pct_hit = fav <= -g.adverse_move_pct;
                let atr_hit = atr_abs > 0.0 && adverse_abs >= atr_abs * g.trigger_atr_mult.max(0.0);
                match g.trigger_mode {
                    GradedTriggerMode::Percent => pct_hit,
                    GradedTriggerMode::Atr => atr_hit,
                    GradedTriggerMode::Hybrid => pct_hit || atr_hit,
                }
            }
        };
        if !should_add_tranche(mode, directional_ok, htf_gate) {
            return;
        }

        // 5) İNFAZ (tek lock): ek kademe = target_capital · weight[k]; qty/entry ortalama; SL/TP/trailing
        //    yeniden ölçek; komisyon equity'den düş; kademe sayacı++.
        let k = gstate.tranches_filled as usize;
        let add_capital = (gstate.target_capital * g.weight_at(k)).max(0.0);
        let add_qty = (add_capital / add_price).max(0.0);
        if add_qty <= 0.0 {
            return;
        }
        let mut st = match state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        // Pozisyon hâlâ açık mı (lock arası kapanmış olabilir) + yön değişmemiş mi → güvenli güncelle.
        let (old_qty, old_entry, is_long) = {
            let positions = match st.finance.live_positions.read() {
                Ok(p) => p,
                Err(_) => return,
            };
            match positions.get(symbol) {
                Some(p) => (p.qty, p.entry_price, p.is_long),
                None => return,
            }
        };
        let (new_qty, new_entry) = average_in(old_qty, old_entry, add_qty, add_price);
        let add_commission = (add_price * add_qty) * commission_rate;
        if let Ok(mut positions) = st.finance.live_positions.write() {
            if let Some(p) = positions.get_mut(symbol) {
                let rescaled_sl = rescale_level(p.stop_loss, old_entry, new_entry);
                p.take_profit = rescale_level(p.take_profit, old_entry, new_entry);
                p.trailing_stop = rescale_level(p.trailing_stop, old_entry, new_entry);
                p.qty = new_qty;
                p.entry_price = new_entry;
                p.entry_commission += add_commission;
                let safe_stop = fee_protected_breakeven_price(
                    p.is_long,
                    new_entry,
                    new_qty,
                    p.entry_commission,
                    commission_rate,
                );
                p.stop_loss = match (p.is_long, safe_stop) {
                    (true, Some(floor)) => rescaled_sl.max(floor),
                    (false, Some(ceiling)) => rescaled_sl.min(ceiling),
                    _ => rescaled_sl,
                };
                p.max_favorable_price = p.max_favorable_price.max(add_price);
            }
        }
        st.finance.equity -= add_commission;
        if let Ok(mut costs) = st.finance.live_execution_costs.write() {
            costs.commission_usd += add_commission;
            costs.total_cost_usd += add_commission;
        }
        if let Ok(mut m) = st.finance.graded_tranches.write() {
            if let Some(gs) = m.get_mut(symbol) {
                gs.tranches_filled = gs.tranches_filled.saturating_add(1);
            }
        }
        let mode_str = if matches!(mode, TrancheMode::Pyramid) {
            "pyramiding"
        } else {
            "averaging"
        };
        let filled = k + 1;
        st.push_log(format!(
            "🪜 {} kademe {}/{} ({}): +{:.6} @ {:.4} (hareket {:+.2}% | ATR {:.4}) → qty={:.6} avg_entry={:.4} (SL ortak güvenli tabana taşındı, kom {:.4})",
            symbol, filled, max_tranches, mode_str, add_qty, add_price, fav, atr_abs, new_qty, new_entry, add_commission,
        ));
        let _ = is_long;
    }
}

#[cfg(test)]
mod graded_entry_tests {
    use super::*;

    #[test]
    fn favorable_move_long_short() {
        // Long: fiyat yükseldi → lehte pozitif.
        assert!((favorable_move_pct(true, 100.0, 102.0) - 2.0).abs() < 1e-9);
        // Short: fiyat yükseldi → aleyhte negatif.
        assert!((favorable_move_pct(false, 100.0, 102.0) - (-2.0)).abs() < 1e-9);
        // Short: fiyat düştü → lehte pozitif.
        assert!((favorable_move_pct(false, 100.0, 98.0) - 2.0).abs() < 1e-9);
        assert_eq!(favorable_move_pct(true, 0.0, 5.0), 0.0, "entry 0 → koruma");
    }

    #[test]
    fn average_in_weighted() {
        // 1@100 + 1@110 → 2@105.
        let (q, e) = average_in(1.0, 100.0, 1.0, 110.0);
        assert!((q - 2.0).abs() < 1e-9 && (e - 105.0).abs() < 1e-9);
        // add_qty<=0 → değişmez.
        assert_eq!(average_in(2.0, 100.0, 0.0, 110.0), (2.0, 100.0));
    }

    #[test]
    fn rescale_preserves_relative_distance() {
        // SL entry'nin %5 altı (95@100). Entry 110'a kayınca SL 104.5 (yine %5 alt).
        let sl = rescale_level(95.0, 100.0, 110.0);
        assert!((sl - 104.5).abs() < 1e-9);
        assert_eq!(
            rescale_level(0.0, 100.0, 110.0),
            0.0,
            "devre dışı seviye değişmez"
        );
    }

    #[test]
    fn tranche_mode_maps_regime() {
        use crate::evolution::MarketRegime::*;
        assert_eq!(tranche_mode(StrongUptrend), Some(TrancheMode::Pyramid));
        assert_eq!(tranche_mode(WeakDowntrend), Some(TrancheMode::Pyramid));
        assert_eq!(tranche_mode(Ranging), Some(TrancheMode::Average));
        assert_eq!(tranche_mode(LowVolatility), Some(TrancheMode::Average));
        assert_eq!(tranche_mode(HighVolatility), None, "kriz → ek kademe yok");
        assert_eq!(tranche_mode(Unknown), None);
    }

    #[test]
    fn htf_sign_and_alignment() {
        assert_eq!(htf_trend_sign(&[10.0, 11.0, 12.0]), 1, "yükselen → boğa");
        assert_eq!(htf_trend_sign(&[12.0, 11.0, 10.0]), -1, "düşen → ayı");
        assert_eq!(htf_trend_sign(&[10.0]), 0, "yetersiz veri");
        // Pyramid long: HTF boğa olmalı.
        assert!(htf_aligned(TrancheMode::Pyramid, true, 1));
        assert!(!htf_aligned(TrancheMode::Pyramid, true, -1));
        // Average long: HTF ayı OLMAMALI (nötr/boğa yeter).
        assert!(htf_aligned(TrancheMode::Average, true, 0));
        assert!(htf_aligned(TrancheMode::Average, true, 1));
        assert!(
            !htf_aligned(TrancheMode::Average, true, -1),
            "düşen trende long averaging = düşen bıçak"
        );
    }

    #[test]
    fn should_add_tranche_gates() {
        assert!(should_add_tranche(TrancheMode::Pyramid, true, true));
        assert!(
            !should_add_tranche(TrancheMode::Pyramid, true, false),
            "HTF kapısı kapalı"
        );
        assert!(
            !should_add_tranche(TrancheMode::Average, false, true),
            "trigger yoksa kapalı"
        );
    }

    #[test]
    fn adverse_move_abs_tracks_against_position() {
        assert!((adverse_move_abs(true, 100.0, 95.0) - 5.0).abs() < 1e-9);
        assert!((adverse_move_abs(false, 100.0, 105.0) - 5.0).abs() < 1e-9);
    }
}
