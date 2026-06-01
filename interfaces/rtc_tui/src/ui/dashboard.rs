// interfaces/rtc_tui/src/ui/dashboard.rs

use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Table, Row, Paragraph}; // Paragraph eklendi
use ratatui::style::{Color, Style, Modifier};
// DİKKAT: crate:: yerine artık kütüphane adını kullanıyoruz (Workspace uyumu)
use memos_trading_core::core::model::MissionControl; 
use crate::ui::components;

/// Srivastava ATP - Ana Panel Render Motoru
pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Finansal Özet
            Constraint::Length(3), // Portföy Gauge
            Constraint::Min(10),   // Pozisyonlar Tablosu
            Constraint::Length(3), // Sistem Pipeline
        ])
        .split(area);

    // 1. Üst Bilgi ve Gauge — phase rozeti dahil
    components::render_finance_header(chunks[0], f, &snap.finance, &snap.phase);
    components::render_equity_gauge(chunks[1], f, &snap.finance);

    // 2. Orta Bölüm: Aktif Pozisyonlar Lojiği
    if snap.positions.is_empty() {
        // Pozisyon yoksa tablo yerine bilgilendirme mesajı bas (UX İyileştirmesi)
        let empty_msg = Paragraph::new("\n\n  [○] Şu an aktif operasyon yok. Sinyal bekleniyor...")
            .block(Block::default().title(" ⚔️ Aktif Operasyonlar ").borders(Borders::ALL))
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty_msg, chunks[2]);
    } else {
        let rows: Vec<Row> = snap.positions.iter()
            .map(components::render_position_row)
            .collect();

        let table = Table::new(rows, [
            Constraint::Percentage(13), // Sembol
            Constraint::Percentage(11), // Strateji
            Constraint::Percentage(8),  // Yön
            Constraint::Percentage(8),  // Tür (spot/vadeli)
            Constraint::Percentage(8),  // Kald.
            Constraint::Percentage(12), // Giriş
            Constraint::Percentage(12), // Fiyat
            Constraint::Percentage(14), // PnL
            Constraint::Percentage(14), // ROE%
        ])
        .header(
            Row::new(vec!["Sembol", "Strateji", "Yön", "Tür", "Kald.", "Giriş", "Fiyat", "PnL", "ROE%"])
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        )
        .block(
            Block::default()
                .title(format!(" ⚔️ Aktif Operasyonlar ({}) ", snap.positions.len()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
        );

        f.render_widget(table, chunks[2]);
    }

    // 3. Alt Bilgi: Pipeline Durumu
    components::render_pipeline_status(chunks[3], f, &snap.pipeline_steps);
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_trading_core::core::model::{
        AiBrainSnapshot, ChartSnapshot, FinanceSnapshot, MissionControl, TradeTypeStats,
    };
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_snap(phase: &str) -> MissionControl {
        MissionControl {
            finance: FinanceSnapshot {
                total_equity: 10000.0, realize_pnl: 0.0, open_pnl: 0.0,
                starting_capital: 10000.0, total_fees: 0.0,
            },
            positions: vec![], fleet: vec![], phase: phase.into(),
            pipeline_steps: vec![],
            ai_brain: AiBrainSnapshot { genome_id: "t".into(), fitness: 0.0, win_rate: 0.0,
                trade_count: 0, gbt_score: Some(0.0), exploration_rate: 0.0,
                drift_score: 0.0, mc_ruin_prob: 0.0, is_evolution_active: false,
                next_evolution_secs: 0,
                live_strategy: String::new(), controller_state: String::new(),
                controller_cycle: 0, consecutive_failures: 0, pending_trades: 0,
                drift_series: vec![], best_tp_pct: 0.0, best_sl_pct: 0.0,
                best_position_size: 0.0 },
            market_fleet: vec![], logs: vec![], trade_history: vec![],
            charts: ChartSnapshot { distributions: vec![], total_closed_pnl: 0.0,
                total_trade_count: 0, equity_series: vec![], current_drawdown_pct: 0.0,
                peak_equity: 0.0 },
            anomalies: vec![], repair_log: vec![],
            scalp_stats: TradeTypeStats { label: "S".into(), win_rate: 0.0, profit_factor: 0.0,
                avg_win: 0.0, avg_loss: 0.0, current_streak: 0 },
            swing_stats: TradeTypeStats { label: "W".into(), win_rate: 0.0, profit_factor: 0.0,
                avg_win: 0.0, avg_loss: 0.0, current_streak: 0 },
            active_anomalies: 0,
            anomalies_by_kind: std::collections::BTreeMap::new(),
        }
    }

    fn render_string(snap: &MissionControl) -> String {
        let backend = TestBackend::new(140, 25);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, f.size(), snap)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width { s.push_str(buf.get(x, y).symbol()); }
            s.push('\n');
        }
        s
    }

    #[test]
    fn phase_badge_renders_scanning() {
        let r = render_string(&make_snap("Scanning"));
        assert!(r.contains("Scanning"), "Scanning rozeti header'da yok\n{}", r);
    }

    #[test]
    fn phase_badge_renders_executing() {
        let r = render_string(&make_snap("Executing"));
        assert!(r.contains("Executing"), "Executing rozeti header'da yok\n{}", r);
    }

    #[test]
    fn phase_badge_empty_does_not_crash() {
        // Phase boş geçilirse rozet basılmaz ama ekran bozulmamalı.
        let r = render_string(&make_snap(""));
        assert!(r.contains("SERMAYE"), "header gövdesi yine basılmalı\n{}", r);
    }
}
