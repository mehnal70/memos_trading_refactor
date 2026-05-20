// src/ui/pipeline.rs
//
// Pipeline sekmesi (TUI tuş 8): üç parçalı görünüm
//   1. Pipeline Timeline tablosu — her step için son çalışma yaşı ve overdue durumu.
//   2. Aktif Anomaliler listesi.
//   3. Onarım Günlüğü.

use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Paragraph, Table, Row, Cell, List, ListItem};
use ratatui::style::{Color, Style, Modifier};
use memos_trading_core::core::model::MissionControl;

pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(7),    // Pipeline Timeline
            Constraint::Length(8), // Anomaliler
            Constraint::Min(5),    // Onarım Günlüğü
        ])
        .split(area);

    // 1. Pipeline Timeline — bridge.rs `pipeline_steps`'i last_run_age_secs + overdue_secs ile doluyor.
    let rows: Vec<Row> = snap.pipeline_steps.iter().map(|s| {
        let status_color = match s.status.as_str() {
            "Ok" | "Healthy" => Color::LightGreen,
            "Degraded" => Color::Yellow,
            "Critical" | "Failed" | "Stalled" => Color::Red,
            _ => Color::Gray,
        };
        let overdue_text = if s.overdue_secs > 0 {
            format!("+{}s gecikme", s.overdue_secs)
        } else if s.overdue_secs < 0 {
            format!("{}s pay", s.overdue_secs)
        } else {
            "—".to_string()
        };
        let overdue_color = if s.overdue_secs > 0 { Color::Red } else { Color::DarkGray };
        let last_run_text = if s.last_run_age_secs == 0 {
            "henüz".to_string()
        } else {
            format!("{}s önce", s.last_run_age_secs)
        };
        Row::new(vec![
            Cell::from(s.label.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(s.status.clone()).style(Style::default().fg(status_color)),
            Cell::from(last_run_text),
            Cell::from(overdue_text).style(Style::default().fg(overdue_color)),
        ])
    }).collect();

    let timeline = Table::new(rows, [
        Constraint::Percentage(35),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(25),
    ])
    .header(Row::new(vec!["Adım", "Durum", "Son Çalışma", "Gecikme"])
        .style(Style::default().fg(Color::Yellow)))
    .block(Block::default()
        .title(format!(" 🌀 Pipeline Timeline ({} adım) ", snap.pipeline_steps.len()))
        .borders(Borders::ALL));
    f.render_widget(timeline, chunks[0]);

    // 2. Anomali Listesi
    let lines: Vec<ratatui::text::Line> = if snap.anomalies.is_empty() {
        vec![ratatui::text::Line::from(ratatui::text::Span::styled(
            " ✅ Tüm sistemler nominal.",
            Style::default().fg(Color::LightGreen),
        ))]
    } else {
        snap.anomalies.iter().map(|a| {
            let color = if a.severity == "Critical" { Color::Red } else { Color::Yellow };
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(
                    format!(" {} {} ", if a.auto_fixed { "✅" } else { "🚨" }, a.kind),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                ratatui::text::Span::raw(a.message.clone()),
            ])
        }).collect()
    };
    let anomalies = Paragraph::new(lines)
        .block(Block::default().title(" 🛡️ Aktif Anomaliler ").borders(Borders::ALL));
    f.render_widget(anomalies, chunks[1]);

    // 3. Onarım Günlüğü
    let repair_items: Vec<ListItem> = snap.repair_log.iter().rev().take(10).map(|log| {
        ListItem::new(log.as_str()).style(Style::default().fg(Color::Cyan))
    }).collect();
    let repair_list = List::new(repair_items)
        .block(Block::default().title(" 🔧 Son Onarım İşlemleri ").borders(Borders::ALL));
    f.render_widget(repair_list, chunks[2]);
}
