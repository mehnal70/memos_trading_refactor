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
            Constraint::Percentage(10), // Sembol
            Constraint::Percentage(9),  // Strateji
            Constraint::Percentage(5),  // TF (zaman dilimi)
            Constraint::Percentage(7),  // Yön
            Constraint::Percentage(6),  // Tür (spot/vadeli)
            Constraint::Percentage(6),  // Kald.
            Constraint::Percentage(9),  // Giriş
            Constraint::Percentage(9),  // Fiyat
            Constraint::Percentage(8),  // SL (stop-loss; XS'te "—")
            Constraint::Percentage(8),  // TP (take-profit; XS'te "—")
            Constraint::Percentage(8),  // TSL (trailing-stop; XS'te "—")
            Constraint::Percentage(8),  // PnL
            Constraint::Percentage(7),  // ROE%
        ])
        .header(
            Row::new(vec!["Sembol", "Strateji", "TF", "Yön", "Tür", "Kald.", "Giriş", "Fiyat", "SL", "TP", "TSL", "PnL", "ROE%"])
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
        AiBrainSnapshot, ChartSnapshot, FinanceSnapshot, MissionControl, PositionModel, TradeTypeStats,
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

    fn pos(symbol: &str, interval: &str) -> PositionModel {
        PositionModel {
            pos_id: String::new(), symbol: symbol.into(),
            entry_price: 100.0, current_price: 101.0, qty: 1.0, leverage: 1.0,
            market: "futures".into(), interval: interval.into(), is_long: true,
            trade_type: "BB".into(), opened_at: "2026-06-03T00:00:00Z".into(),
            stop_loss: 0.0, take_profit: 0.0, trailing_stop: 0.0,
            max_favorable_price: 101.0, breakeven_activated: false, kind: None,
            entry_commission: 0.0,
        }
    }

    /// Açık pozisyon tablosu TF kolonunu (başlık + pozisyonun interval'ı) göstermeli
    /// → operatör hangi sembolün hangi zaman diliminde işlendiğini görür (Item #3).
    #[test]
    fn position_table_shows_tf_column() {
        let mut snap = make_snap("Executing");
        snap.positions = vec![pos("BTCUSDT", "1d")];
        let r = render_string(&snap);
        assert!(r.contains(" TF "), "TF başlığı tabloda yok\n{}", r);
        assert!(r.contains("1d"), "pozisyonun TF'i (1d) gösterilmeli\n{}", r);
    }

    /// interval boş (eski snapshot) → TF hücresi "—" basar, panik yok.
    #[test]
    fn position_table_tf_empty_shows_dash() {
        let mut snap = make_snap("Executing");
        snap.positions = vec![pos("ETHUSDT", "")];
        let r = render_string(&snap);
        assert!(r.contains("—"), "boş TF tire ile gösterilmeli\n{}", r);
    }

    /// SL/TP/TSL kolonları: başlıklar + set edilmiş seviyeler (XS-dışı pozisyon) gösterilmeli.
    #[test]
    fn position_table_shows_sl_tp_tsl_columns() {
        let mut snap = make_snap("Executing");
        let mut p = pos("BTCUSDT", "1h");
        p.stop_loss = 95.0; p.take_profit = 112.0; p.trailing_stop = 98.0;
        snap.positions = vec![p];
        let r = render_string(&snap);
        assert!(r.contains(" SL "), "SL başlığı yok\n{}", r);
        assert!(r.contains(" TP "), "TP başlığı yok\n{}", r);
        assert!(r.contains("TSL"), "TSL başlığı yok\n{}", r);
        assert!(r.contains("95.0"), "SL seviyesi gösterilmeli\n{}", r);
        assert!(r.contains("112.0"), "TP seviyesi gösterilmeli\n{}", r);
    }

    /// XS (kesitsel) pozisyonu STOPSUZ (SL/TP/TSL=0) → üç hücre de "—" basar (rank-rebalance
    /// ile yönetilir, per-bacak stop yoktur — beklenen, bug değil).
    #[test]
    fn position_table_xs_no_stops_shows_dash() {
        let mut snap = make_snap("Executing");
        let mut p = pos("AVAXUSDT", "1d"); // interval dolu → "—" yalnız stop hücrelerinden gelir
        p.trade_type = "XS_MOMENTUM".into();
        // stop_loss/take_profit/trailing_stop pos() helper'ında zaten 0.0
        snap.positions = vec![p];
        let r = render_string(&snap);
        assert!(r.contains("—"), "XS stopsuz pozisyon → tire gösterilmeli\n{}", r);
    }
}
