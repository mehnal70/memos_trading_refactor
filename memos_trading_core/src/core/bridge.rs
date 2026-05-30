// memos_trading_core/src/core/bridge.rs

use crate::robot::robotic_loop::AppState;
use crate::core::model::{
    MissionControl, FinanceSnapshot, PositionModel, WorkerModel,
    PipelineStep, AiBrainSnapshot, AnomalyModel, TradeTypeStats,
    ChartSnapshot, TradeDistribution, LogEntry, ClosedTradeModel,
    MarketAnalysisModel, SrZoneModel,
};

/// Srivastava ATP - AppState'den saf bir Snapshot (Anlık Görüntü) çıkarır.
/// Bu fonksiyon robotun 'Adli Tercümanı'dır.
pub fn get_snapshot(st: &AppState) -> MissionControl {

    // 1. FİNANSAL HASAT (Anlık PnL Hesaplamaları)
    let open_pnl: f64 = st.fleet.symbol_orchestrator.read().ok()
        .map(|orch| orch.total_open_pnl(None))
        .unwrap_or(0.0);
    let total_fees: f64 = st.finance.live_execution_costs.read().ok()
        .map(|c| c.total_cost_usd)
        .unwrap_or(0.0);
    let finance = FinanceSnapshot {
        total_equity: st.finance.equity + open_pnl,
        realize_pnl: st.finance.equity - st.config.capital,
        open_pnl,
        starting_capital: st.config.capital,
        total_fees,
    };

    // 2. POZİSYON DÖNÜŞTÜRÜCÜ — clone yeterli (struct serialize-friendly)
    let positions: Vec<PositionModel> = st.finance.live_positions.read().ok()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default();

    // 3. ADLİ LOG VE ARŞİV HASADI (Son 100 log ve 50 işlem)
    // Timestamp log satırının kendisinde mühürlü (push_log "[HH:MM:SS] msg"
    // formatında yazıyor); burada PARSE edilir. Daha önce Local::now()
    // çağrısıyla her snapshot'ta yeniden üretiliyordu → log paneli her render'da
    // "değişti" görünüyor, TUI'de görsel flicker yaratıyordu.
    let logs: Vec<LogEntry> = st.guardian.log.iter().rev().take(100).map(|line| {
        let level = if line.contains("ERROR") { "ERROR" }
                    else if line.contains("WARN") { "WARN" }
                    else if line.contains("SIGNAL") { "SIGNAL" }
                    else { "INFO" };
        let (timestamp, message) = parse_log_prefix(line);
        LogEntry { timestamp, message, level: level.to_string() }
    }).collect();

    let mut trade_history: Vec<ClosedTradeModel> = vec![];
    let equity_series: Vec<f64> = st.finance.equity_history.read()
        .map(|h| h.iter().copied().collect()).unwrap_or_default();
    let peak = st.finance.peak_equity.max(st.finance.equity);
    let current_dd = if peak > 0.0 {
        ((peak - st.finance.equity) / peak * 100.0).max(0.0)
    } else { 0.0 };
    let mut charts = ChartSnapshot {
        distributions: vec![],
        total_closed_pnl: 0.0,
        total_trade_count: 0,
        equity_series,
        current_drawdown_pct: current_dd,
        peak_equity: peak,
    };

    if let Ok(trades) = st.finance.live_closed_trades.read() {
        trade_history = trades.iter().rev().take(50).map(|t| {
            ClosedTradeModel {
                symbol: t.symbol.clone(),
                is_long: t.is_long,
                pnl: t.pnl,
                pnl_pct: t.pnl_pct,
                exit_reason: t.exit_reason.clone(),
                closed_at: t.closed_at.clone(),
                opened_at: t.opened_at.clone(),
                leverage: t.leverage,
            }
        }).collect();

        // Pasta Grafiği (Trade Dağılımı) Lojiği
        let mut dist_map = std::collections::HashMap::new();
        for t in trades.iter() {
            let entry = dist_map.entry(t.symbol.clone()).or_insert((0.0_f64, 0u32, 0u32));
            entry.0 += t.pnl;
            entry.1 += 1;
            if t.pnl > 0.0 { entry.2 += 1; }
        }
        charts.total_closed_pnl = trades.iter().map(|t| t.pnl).sum();
        charts.total_trade_count = trades.len();
        charts.distributions = dist_map.into_iter().map(|(sym, (pnl, cnt, wins))| {
            TradeDistribution {
                symbol: sym,
                pnl,
                trade_count: cnt,
                win_rate: if cnt > 0 { (wins as f64 / cnt as f64) * 100.0 } else { 0.0 },
            }
        }).collect();
    }

    // 4. FİLO VE PAZAR ALGISI (Worker Status + S/R Zones)
    let live_price_map = st.fleet.live_price.read().ok().map(|g| g.clone()).unwrap_or_default();
    let fleet: Vec<WorkerModel> = st.fleet.symbol_orchestrator.read().ok().map(|orch| {
        orch.get_worker_status().into_iter().map(|w| {
            let price = live_price_map.get(&w.symbol).copied().unwrap_or(0.0);
            WorkerModel {
                symbol: w.symbol,
                market: w.market,
                interval: w.interval,
                price,
                change_pct: 0.0, // change_pct ileride live_price snapshot'tan beslenecek
                uptime_secs: w.uptime_secs,
                is_paused: w.paused,
                score: 0.0, // skor ml_engine'den geldiğinde wire'lanır
            }
        }).collect()
    }).unwrap_or_default();

    let market_fleet: Vec<MarketAnalysisModel> = st.fleet.live_sr_zones.read().ok().map(|zones_map| {
        zones_map.iter().map(|(sym, zones)| {
            let current_price = live_price_map.get(sym).copied().unwrap_or(0.0);
            // En yakın destek: midpoint ≤ price (mevcut fiyatın altındaki en yüksek destek).
            // En yakın direnç: midpoint ≥ price (mevcut fiyatın üstündeki en düşük direnç).
            // Fiyat 0 ise (henüz live price yok) en yüksek destek / en düşük direnç fallback.
            let nearest_support = zones.iter()
                .filter(|z| matches!(z.zone_type, crate::robot::sr_detector::ZoneType::Support)
                    && (current_price <= 0.0 || z.midpoint <= current_price))
                .max_by(|a, b| a.midpoint.partial_cmp(&b.midpoint).unwrap_or(std::cmp::Ordering::Equal))
                .map(|z| z.midpoint);
            let nearest_resistance = zones.iter()
                .filter(|z| matches!(z.zone_type, crate::robot::sr_detector::ZoneType::Resistance)
                    && (current_price <= 0.0 || z.midpoint >= current_price))
                .min_by(|a, b| a.midpoint.partial_cmp(&b.midpoint).unwrap_or(std::cmp::Ordering::Equal))
                .map(|z| z.midpoint);
            let zones_converted = zones.iter().map(|z| SrZoneModel {
                zone_type:   format!("{:?}", z.zone_type),
                price_low:   z.price_low,
                price_high:  z.price_high,
                strength:    z.strength,
                touch_count: z.touch_count,
            }).collect();
            MarketAnalysisModel {
                symbol: sym.clone(),
                current_price,
                change_24h: 0.0,
                zones: zones_converted,
                nearest_support,
                nearest_resistance,
            }
        }).collect()
    }).unwrap_or_default();

    // 5. AI BEYİN, PİPELİNE VE ANOMALİLER
    // step.last_run_secs aslında Unix epoch saniyesi (record_step çağrılarına
    // last_tick = SystemTime::now()...as_secs() geçiriliyor). UI'da "X saniye önce"
    // yaşı göstermek için şimdiyle farkı alıyoruz; 0 → henüz hiç koşulmadı.
    let now_epoch_secs: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (steps, anomalies): (Vec<PipelineStep>, Vec<AnomalyModel>) = st.guardian.live_pipeline.read().ok()
        .map(|ph| {
            let s = ph.chain_steps.iter().map(|step| PipelineStep {
                label:             step.label.clone(),
                status:            format!("{:?}", step.status),
                last_run_age_secs: if step.last_run_secs == 0 { 0 }
                                   else { now_epoch_secs.saturating_sub(step.last_run_secs) },
                overdue_secs:      step.overdue_secs as i64,
            }).collect();
            let a = ph.anomalies.iter().map(|anom| AnomalyModel {
                severity:   format!("{:?}", anom.severity),
                kind:       format!("{:?}", anom.kind),
                message:    anom.message.clone(),
                fix_hint:   anom.fix_hint.clone().unwrap_or_default(),
                auto_fixed: anom.auto_fixed,
            }).collect();
            (s, a)
        })
        .unwrap_or_else(|| (vec![], vec![]));

    // total_trades alanı yeni AppState'de yok; kapanmış işlem sayısı en yakın kaynak.
    let trade_count = st.finance.live_closed_trades.read()
        .map(|t| t.len()).unwrap_or(0);

    // IntelligenceHub'tan canlı veriler — kullanılamıyorsa muhafazakar varsayılana düş.
    let (hub_drift, hub_pending, hub_cycle, hub_state, hub_evolution_active,
         hub_failures, hub_drift_series) =
        st.brain.intelligence_hub.read().map(|h| (
            h.drift_detector.drift_score,
            h.pending_trades.len(),
            h.controller.cycle_id,
            h.controller.state.to_string(),
            h.controller.evolution_enabled,
            h.controller.consecutive_failures as u32,
            // Drift tarihçesinin son 60 noktası (AI Center sparkline için).
            h.drift_history.iter().rev().take(60).rev().cloned().collect::<Vec<f64>>(),
        )).unwrap_or((0.0, 0, 0, "Unknown".into(), false, 0, vec![]));

    // brain.live_strategy "Default"/"Auto" iken motor zaten her cycle'da rejime göre
    // sembol başına dinamik strateji seçiyor (StrategySelector::select_best); tek bir
    // statik isim yansıtmak yanıltıcı olur. Tek-nokta normalize için
    // core::model::normalize_strategy_label kullanılır.
    let live_strategy_name = st.brain.live_strategy.read()
        .map(|s| crate::core::model::normalize_strategy_label(&s))
        .unwrap_or_else(|_| "—".to_string());
    let best_tp_pct = st.brain.best_params.get("take_profit_pct").copied().unwrap_or(0.0);
    let best_sl_pct = st.brain.best_params.get("stop_loss_pct").copied().unwrap_or(0.0);
    let best_position_size = st.brain.best_params.get("max_position_size").copied().unwrap_or(0.0);

    let ai_brain = AiBrainSnapshot {
        genome_id: format!("Srivastava-Alpha-9 [cycle={} · ctrl={}]", hub_cycle, hub_state),
        fitness: finance.total_equity,
        win_rate: charts.distributions.iter().map(|d| d.win_rate).sum::<f64>() / charts.distributions.len().max(1) as f64,
        trade_count,
        gbt_score: Some(st.brain.hyperopt_score),
        exploration_rate: 0.1,
        drift_score: hub_drift,
        mc_ruin_prob: 0.01,
        is_evolution_active: hub_evolution_active,
        next_evolution_secs: 300_u64.saturating_sub(hub_cycle % 300),
        live_strategy: live_strategy_name,
        controller_state: hub_state,
        controller_cycle: hub_cycle,
        consecutive_failures: hub_failures,
        pending_trades: hub_pending,
        drift_series: hub_drift_series,
        best_tp_pct,
        best_sl_pct,
        best_position_size,
    };

    // 6. ÖZEL İSTATİSTİKLER — closed_trades'ten holding period'a göre Scalp/Swing ayrımı.
    //    Eşik Faz 2 ParameterStore'dan okunuyor (önce env, runtime'da HyperOpt güncelleyebilir).
    //    opened_at/closed_at parse edilemeyen veya boş kayıtlar atlanır.
    let scalp_swing_threshold_min = st.brain.parameters.read()
        .map(|p| p.scalp_swing_threshold_min).unwrap_or(60);
    let (scalp_stats, swing_stats) = st.finance.live_closed_trades.read().ok()
        .map(|trades| compute_scalp_swing_stats(&trades, scalp_swing_threshold_min))
        .unwrap_or_else(|| (
            TradeTypeStats { label: "SCALP".into(), win_rate: 0.0, profit_factor: 0.0, avg_win: 0.0, avg_loss: 0.0, current_streak: 0 },
            TradeTypeStats { label: "SWING".into(), win_rate: 0.0, profit_factor: 0.0, avg_win: 0.0, avg_loss: 0.0, current_streak: 0 },
        ));

    // anomalies aşağıda MissionControl'a move edilmeden önce sayıyı + tip
    // dağılımını yakala. boot anomaly 50 olduğunda hangi tipin baskın olduğu
    // görünür (TUI Risk Center).
    let active_anomalies = anomalies.len();
    let mut anomalies_by_kind: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for a in &anomalies {
        *anomalies_by_kind.entry(a.kind.clone()).or_insert(0) += 1;
    }
    let repair_log: Vec<String> = st.guardian.repair_log.iter().rev().take(50).cloned().collect();

    MissionControl {
        finance,
        positions,
        ai_brain,
        phase: st.fleet.phase.clone(),
        pipeline_steps: steps,
        anomalies,
        repair_log,
        scalp_stats,
        swing_stats,
        logs,
        trade_history,
        market_fleet,
        charts,
        fleet,
        active_anomalies,
        anomalies_by_kind,
    }
}

/// Log satırının başındaki `[HH:MM:SS]` prefix'ini ayırır. Prefix yoksa
/// timestamp boş döner, message tüm satırı kapsar. Saf, side-effect yok.
fn parse_log_prefix(line: &str) -> (String, String) {
    // Beklenen format: "[HH:MM:SS] message" (push_log böyle yazıyor).
    let bytes = line.as_bytes();
    if bytes.len() >= 11 && bytes[0] == b'[' && bytes[9] == b']' && bytes[10] == b' ' {
        let ts = &line[1..9];
        if ts.bytes().all(|b| b.is_ascii_digit() || b == b':') {
            return (ts.to_string(), line[11..].to_string());
        }
    }
    (String::new(), line.to_string())
}

/// Closed trades listesini holding period'a göre Scalp/Swing'e ayırıp her grup için
/// (win_rate, profit_factor, avg_win, avg_loss, current_streak) hesaplar.
///
/// - `threshold_min`: Scalp/Swing ayrım eşiği (dakika). Holding < eşik → Scalp.
///   Faz 2 öncesi env `SCALP_SWING_THRESHOLD_MIN`'den okunuyordu; artık ParameterStore'dan
///   geliyor, ama unit testler kolay olsun diye parametre olarak geçiyor.
/// - opened_at/closed_at boş veya parse edilemeyen kayıtlar atlanır (eski şema toleransı).
/// - current_streak: en son kayıttan başlayarak ardışık aynı yön (kâr/zarar) sayısı,
///   kâr → pozitif, zarar → negatif. Yön değişince durur.
fn compute_scalp_swing_stats(
    trades: &[ClosedTradeModel],
    threshold_min: i64,
) -> (TradeTypeStats, TradeTypeStats) {
    let threshold_secs = threshold_min.saturating_mul(60);

    // Holding period'u hesapla, geçersiz kayıtları atla.
    let with_holding: Vec<(&ClosedTradeModel, i64)> = trades.iter().filter_map(|t| {
        if t.opened_at.is_empty() || t.closed_at.is_empty() { return None; }
        let o = chrono::DateTime::parse_from_rfc3339(&t.opened_at).ok()?;
        let c = chrono::DateTime::parse_from_rfc3339(&t.closed_at).ok()?;
        let secs = (c - o).num_seconds();
        if secs < 0 { return None; } // bozuk sıralama
        Some((t, secs))
    }).collect();

    let mut scalp: Vec<&ClosedTradeModel> = vec![];
    let mut swing: Vec<&ClosedTradeModel> = vec![];
    for (t, h) in &with_holding {
        if *h < threshold_secs { scalp.push(*t); } else { swing.push(*t); }
    }
    (
        stats_for_group("SCALP", &scalp),
        stats_for_group("SWING", &swing),
    )
}

fn stats_for_group(label: &str, group: &[&ClosedTradeModel]) -> TradeTypeStats {
    let n = group.len();
    if n == 0 {
        return TradeTypeStats {
            label: label.into(), win_rate: 0.0, profit_factor: 0.0,
            avg_win: 0.0, avg_loss: 0.0, current_streak: 0,
        };
    }
    let wins: Vec<f64> = group.iter().filter(|t| t.pnl > 0.0).map(|t| t.pnl).collect();
    let losses: Vec<f64> = group.iter().filter(|t| t.pnl < 0.0).map(|t| t.pnl.abs()).collect();
    let sum_wins: f64 = wins.iter().sum();
    let sum_losses: f64 = losses.iter().sum();

    let win_rate = wins.len() as f64 / n as f64 * 100.0;
    // Loss yokken sonsuz olur; UI/JSON'da NaN/inf üretmemek için 100.0'a cap'liyoruz.
    let profit_factor = if sum_losses > 0.0 { (sum_wins / sum_losses).min(100.0) }
                        else if sum_wins > 0.0 { 100.0 }
                        else { 0.0 };
    let avg_win = if !wins.is_empty() { sum_wins / wins.len() as f64 } else { 0.0 };
    let avg_loss = if !losses.is_empty() { sum_losses / losses.len() as f64 } else { 0.0 };

    // Streak: en son trade'den geriye, aynı yönde gittiği sürece say.
    let mut streak: i32 = 0;
    if let Some(last) = group.last() {
        let last_sign = last.pnl.signum();
        if last_sign != 0.0 {
            for t in group.iter().rev() {
                if t.pnl.signum() == last_sign {
                    streak += if last_sign > 0.0 { 1 } else { -1 };
                } else { break; }
            }
        }
    }

    TradeTypeStats { label: label.into(), win_rate, profit_factor, avg_win, avg_loss, current_streak: streak }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_log_prefix_extracts_timestamp_and_strips_brackets() {
        let (ts, msg) = parse_log_prefix("[12:34:56] 🚀 Devriye başladı");
        assert_eq!(ts, "12:34:56");
        assert_eq!(msg, "🚀 Devriye başladı");
    }

    #[test]
    fn parse_log_prefix_without_bracket_returns_empty_timestamp() {
        let (ts, msg) = parse_log_prefix("plain line without prefix");
        assert_eq!(ts, "");
        assert_eq!(msg, "plain line without prefix");
    }

    #[test]
    fn parse_log_prefix_rejects_non_digit_inside_brackets() {
        // "[ABCDEFGH]" yapısal eşleşse de digit/`:` değil → fallback yol.
        let (ts, msg) = parse_log_prefix("[ABCDEFGH] hello");
        assert_eq!(ts, "");
        assert_eq!(msg, "[ABCDEFGH] hello");
    }

    #[test]
    fn parse_log_prefix_handles_empty_line() {
        let (ts, msg) = parse_log_prefix("");
        assert_eq!(ts, "");
        assert_eq!(msg, "");
    }

    fn t(opened: &str, closed: &str, pnl: f64) -> ClosedTradeModel {
        ClosedTradeModel {
            symbol: "BTCUSDT".into(),
            is_long: true,
            pnl,
            pnl_pct: 0.0,
            exit_reason: "TP".into(),
            opened_at: opened.into(),
            closed_at: closed.into(),
            leverage: 1.0,
        }
    }

    /// Default eşik (60 dk) ile compute_scalp_swing_stats çağırır — Faz 2 öncesi
    /// env-tabanlı davranışın test ergonomisini korur.
    fn call_default(trades: &[ClosedTradeModel]) -> (TradeTypeStats, TradeTypeStats) {
        compute_scalp_swing_stats(trades, 60)
    }

    #[test]
    fn empty_trades_yield_zero_stats() {
        let (s, w) = call_default(&[]);
        assert_eq!(s.win_rate, 0.0);
        assert_eq!(w.win_rate, 0.0);
        assert_eq!(s.label, "SCALP");
        assert_eq!(w.label, "SWING");
    }

    #[test]
    fn short_holding_goes_to_scalp() {
        // 10 dakikalık holding → SCALP grubu (default eşik 60 dk).
        let trades = vec![
            t("2026-05-20T10:00:00+00:00", "2026-05-20T10:10:00+00:00", 5.0),
            t("2026-05-20T11:00:00+00:00", "2026-05-20T11:10:00+00:00", -2.0),
        ];
        let (s, w) = call_default(&trades);
        assert!((s.win_rate - 50.0).abs() < 1e-9);
        assert!((s.profit_factor - 2.5).abs() < 1e-9, "5/2=2.5; got={}", s.profit_factor);
        assert_eq!(w.win_rate, 0.0, "Swing grubu boş kalmalı");
    }

    #[test]
    fn long_holding_goes_to_swing() {
        // 2 saatlik holding → SWING grubu.
        let trades = vec![
            t("2026-05-20T10:00:00+00:00", "2026-05-20T12:00:00+00:00", 10.0),
        ];
        let (s, w) = call_default(&trades);
        assert_eq!(s.win_rate, 0.0);
        assert!((w.win_rate - 100.0).abs() < 1e-9);
        assert!((w.profit_factor - 100.0).abs() < 1e-9, "loss yok → cap 100; got={}", w.profit_factor);
    }

    #[test]
    fn current_streak_counts_consecutive_same_sign() {
        // Son üç trade kazanç → streak = +3
        let trades = vec![
            t("2026-05-20T10:00:00+00:00", "2026-05-20T10:05:00+00:00", -1.0),
            t("2026-05-20T11:00:00+00:00", "2026-05-20T11:05:00+00:00",  2.0),
            t("2026-05-20T12:00:00+00:00", "2026-05-20T12:05:00+00:00",  3.0),
            t("2026-05-20T13:00:00+00:00", "2026-05-20T13:05:00+00:00",  1.0),
        ];
        let (s, _) = call_default(&trades);
        assert_eq!(s.current_streak, 3);
    }

    #[test]
    fn current_streak_negative_when_last_trades_are_losses() {
        let trades = vec![
            t("2026-05-20T10:00:00+00:00", "2026-05-20T10:05:00+00:00",  5.0),
            t("2026-05-20T11:00:00+00:00", "2026-05-20T11:05:00+00:00", -2.0),
            t("2026-05-20T12:00:00+00:00", "2026-05-20T12:05:00+00:00", -1.0),
        ];
        let (s, _) = call_default(&trades);
        assert_eq!(s.current_streak, -2);
    }

    #[test]
    fn explicit_threshold_moves_trades_between_groups() {
        // Eşik 5 dk → 10 dk holding artık SWING'e düşer.
        let trades = vec![
            t("2026-05-20T10:00:00+00:00", "2026-05-20T10:10:00+00:00", 7.0),
        ];
        let (s, w) = compute_scalp_swing_stats(&trades, 5);
        assert_eq!(s.win_rate, 0.0);
        assert!((w.win_rate - 100.0).abs() < 1e-9);
    }

    #[test]
    fn missing_opened_at_is_skipped() {
        let trades = vec![
            t("", "2026-05-20T10:10:00+00:00", 5.0),         // opened_at boş → skip
            t("2026-05-20T11:00:00+00:00", "", -2.0),         // closed_at boş → skip
            t("2026-05-20T12:00:00+00:00", "2026-05-20T12:05:00+00:00", 3.0),
        ];
        let (s, _) = call_default(&trades);
        // Sadece son trade dahil → win_rate 100
        assert!((s.win_rate - 100.0).abs() < 1e-9);
    }
}
