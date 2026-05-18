// memos_trading_core/src/core/bridge.rs

use crate::robot::robotic_loop::AppState;
use crate::core::model::{
    MissionControl, FinanceSnapshot, PositionModel, WorkerModel,
    PipelineStep, AiBrainSnapshot, AnomalyModel, TradeTypeStats,
    ChartSnapshot, TradeDistribution, LogEntry, ClosedTradeModel,
    MarketAnalysisModel, SrZoneModel,
};
use chrono::Local;

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
    let logs: Vec<LogEntry> = st.guardian.log.iter().rev().take(100).map(|line| {
        let level = if line.contains("ERROR") { "ERROR" }
                    else if line.contains("WARN") { "WARN" }
                    else if line.contains("SIGNAL") { "SIGNAL" }
                    else { "INFO" };
        LogEntry {
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            message: line.clone(),
            level: level.to_string(),
        }
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
            let zones_converted = zones.iter().map(|z| SrZoneModel {
                zone_type:   format!("{:?}", z.zone_type),
                price_low:   z.price_low,
                price_high:  z.price_high,
                strength:    z.strength,
                touch_count: z.touch_count,
            }).collect();
            MarketAnalysisModel {
                symbol: sym.clone(),
                current_price: live_price_map.get(sym).copied().unwrap_or(0.0),
                change_24h: 0.0,
                zones: zones_converted,
                nearest_support: None,
                nearest_resistance: None,
            }
        }).collect()
    }).unwrap_or_default();

    // 5. AI BEYİN, PİPELİNE VE ANOMALİLER
    let (steps, anomalies): (Vec<PipelineStep>, Vec<AnomalyModel>) = st.guardian.live_pipeline.read().ok()
        .map(|ph| {
            let s = ph.chain_steps.iter().map(|step| PipelineStep {
                label:             step.label.clone(),
                status:            format!("{:?}", step.status),
                last_run_age_secs: step.last_run_secs,
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

    let live_strategy_name = st.brain.live_strategy.read()
        .map(|s| s.clone()).unwrap_or_else(|_| "—".to_string());
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
        next_evolution_secs: 300_u64.saturating_sub((hub_cycle % 300) as u64),
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

    // 6. ÖZEL İSTATİSTİKLER (Placeholder - Lojik Engine'e taşınacak)
    let (scalp_stats, swing_stats) = (
        TradeTypeStats { label: "SCALP".into(), win_rate: 65.0, profit_factor: 1.8, avg_win: 12.0, avg_loss: 5.5, current_streak: 3 },
        TradeTypeStats { label: "SWING".into(), win_rate: 52.0, profit_factor: 2.1, avg_win: 45.0, avg_loss: 20.0, current_streak: -1 }
    );

    // anomalies aşağıda MissionControl'a move edilmeden önce sayıyı yakala.
    let active_anomalies = anomalies.len();
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
    }
}
