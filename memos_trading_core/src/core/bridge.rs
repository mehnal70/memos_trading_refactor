// memos_trading_core/src/core/bridge.rs

use crate::robot::robotic_loop::AppState;
use crate::core::model::{
    MissionControl, FinanceSnapshot, PositionModel, WorkerModel,
    PipelineStep, AiBrainSnapshot, AnomalyModel, TradeTypeStats,
    ChartSnapshot, TradeDistribution, LogEntry, ClosedTradeModel,
    MarketAnalysisModel,
};
use chrono::Local;

/// Srivastava ATP - AppState'den saf bir Snapshot (Anlık Görüntü) çıkarır.
/// Bu fonksiyon robotun 'Adli Tercümanı'dır.
pub fn get_snapshot(st: &AppState) -> MissionControl {

    // 1. FİNANSAL HASAT (Anlık PnL Hesaplamaları)
    // TODO: SymbolOrchestrator + live_execution_costs yeni AppState'e bağlanınca
    //       open_pnl ve total_fees gerçek değerlerden hesaplanacak.
    let open_pnl: f64 = 0.0;
    let total_fees: f64 = 0.0;
    let finance = FinanceSnapshot {
        total_equity: st.finance.equity + open_pnl,
        realize_pnl: st.finance.equity - st.config.capital,
        open_pnl,
        starting_capital: st.config.capital,
        total_fees,
    };

    // 2. POZİSYON DÖNÜŞTÜRÜCÜ
    let positions: Vec<PositionModel> = st.finance.live_positions.read().ok().map(|m| {
        m.values().map(|p| PositionModel {
            symbol: p.symbol.clone(),
            entry_price: p.entry_price,
            current_price: p.current_price,
            qty: p.qty,
            leverage: p.leverage,
            is_long: p.is_long,
            trade_type: p.trade_type.clone(),
            opened_at: p.opened_at.clone(),
        }).collect()
    }).unwrap_or_default();

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
    let mut charts = ChartSnapshot { distributions: vec![], total_closed_pnl: 0.0, total_trade_count: 0 };

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
    // TODO: SymbolOrchestrator yeni AppState'e bağlanınca worker_status'tan doldurulacak.
    let fleet: Vec<WorkerModel> = vec![];

    // TODO: live_sr_zones yeni AppState'e taşınınca buradan beslenecek.
    let market_fleet: Vec<MarketAnalysisModel> = vec![];

    // 5. AI BEYİN, PİPELİNE VE ANOMALİLER
    // TODO: live_pipeline yeni AppState'e taşınınca chain_steps/anomalies buradan gelecek.
    let steps: Vec<PipelineStep> = vec![];
    let anomalies: Vec<AnomalyModel> = vec![];

    // total_trades alanı yeni AppState'de yok; kapanmış işlem sayısı en yakın kaynak.
    let trade_count = st.finance.live_closed_trades.read()
        .map(|t| t.len()).unwrap_or(0);

    let ai_brain = AiBrainSnapshot {
        genome_id: "Srivastava-Alpha-9".to_string(),
        fitness: finance.total_equity,
        win_rate: charts.distributions.iter().map(|d| d.win_rate).sum::<f64>() / charts.distributions.len().max(1) as f64,
        trade_count,
        gbt_score: Some(0.0),
        exploration_rate: 0.1,
        drift_score: 0.05,
        mc_ruin_prob: 0.01,
        is_evolution_active: true,
        next_evolution_secs: 300,
    };

    // 6. ÖZEL İSTATİSTİKLER (Placeholder - Lojik Engine'e taşınacak)
    let (scalp_stats, swing_stats) = (
        TradeTypeStats { label: "SCALP".into(), win_rate: 65.0, profit_factor: 1.8, avg_win: 12.0, avg_loss: 5.5, current_streak: 3 },
        TradeTypeStats { label: "SWING".into(), win_rate: 52.0, profit_factor: 2.1, avg_win: 45.0, avg_loss: 20.0, current_streak: -1 }
    );

    // anomalies aşağıda MissionControl'a move edilmeden önce sayıyı yakala.
    let active_anomalies = anomalies.len();
    // TODO: repair_log yeni AppState'de yok; Guardian altına eklenince buradan beslenecek.
    let repair_log: Vec<String> = vec![];

    MissionControl {
        finance,
        positions,
        ai_brain,
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
