// src/ui/pipeline.rs
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, Cell};
use ratatui::style::{Color, Style, Modifier};
use memos_trading_core::core::model::MissionControl;

pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // Anomaliler
            Constraint::Min(5),    // Onarım Günlüğü
        ])
        .split(area);

    // 1. Anomali Listesi
    let lines: Vec<ratatui::text::Line> = if snap.anomalies.is_empty() {
        vec![ratatui::text::Line::from(ratatui::text::Span::styled(" ✅ Tüm sistemler nominal.", Style::default().fg(Color::LightGreen)))]
    } else {
        snap.anomalies.iter().map(|a| {
            let color = if a.severity == "Critical" { Color::Red } else { Color::Yellow };
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(format!(" {} {} ", if a.auto_fixed { "✅" } else { "🚨" }, a.kind), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                ratatui::text::Span::raw(a.message.clone()),
            ])
        }).collect()
    };

    let p1 = Paragraph::new(lines).block(Block::default().title(" 🛡️ Aktif Anomaliler ").borders(Borders::ALL));
    f.render_widget(p1, chunks[0]);

    // 2. Onarım Günlüğü
    let repair_lines: Vec<ratatui::widgets::ListItem> = snap.repair_log.iter().rev().take(10).map(|log| {
        ratatui::widgets::ListItem::new(log.as_str()).style(Style::default().fg(Color::Cyan))
    }).collect();

    let l1 = ratatui::widgets::List::new(repair_lines).block(Block::default().title(" 🔧 Son Onarım İşlemleri ").borders(Borders::ALL));
    f.render_widget(l1, chunks[1]);
}
