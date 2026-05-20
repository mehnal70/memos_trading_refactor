// src/ui/pipeline.rs
//
// Pipeline sekmesi (TUI tuş 8): dört parçalı görünüm
//   1. 🌀 Kanonik Pipeline Timeline (7 faz, kanonik etiket "1."–"7." prefix'li).
//   2. ⚙️ Altyapı Adımları (price_poll, trigger:*, main_loop, vb.)
//   3. 🛡️ Aktif Anomaliler.
//   4. 🔧 Onarım Günlüğü.

use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Paragraph, Table, Row, Cell, List, ListItem};
use ratatui::style::{Color, Style, Modifier};
use memos_trading_core::core::model::{MissionControl, PipelineStep};

/// "1. Veri Akışı", "2. Özellik Çıkarımı", ... gibi kanonik fazları altyapı
/// step'lerinden ayırır. canon.rs `PipelineStage::label()` her zaman "N." ile
/// başlar (N = 1..7), ad-hoc record_step çağrıları (price_poll, trigger:* vb.)
/// böyle bir prefix taşımaz.
fn is_canonical_stage(s: &PipelineStep) -> bool {
    let label = s.label.trim_start();
    let mut chars = label.chars();
    matches!((chars.next(), chars.next()), (Some(c), Some('.')) if c.is_ascii_digit())
}

fn status_color(status: &str) -> Color {
    match status {
        "Ok" | "Healthy" | "Done" => Color::LightGreen,
        "Degraded" | "Running" => Color::Yellow,
        "Critical" | "Failed" | "Stalled" => Color::Red,
        "Skipped" => Color::DarkGray,
        _ => Color::Gray,
    }
}

fn make_row(s: &PipelineStep) -> Row<'static> {
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
        Cell::from(s.status.clone()).style(Style::default().fg(status_color(&s.status))),
        Cell::from(last_run_text),
        Cell::from(overdue_text).style(Style::default().fg(overdue_color)),
    ])
}

pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let (canon_steps, infra_steps): (Vec<&PipelineStep>, Vec<&PipelineStep>) = snap
        .pipeline_steps
        .iter()
        .partition(|s| is_canonical_stage(s));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // Kanonik Pipeline Timeline (7 satır + başlık)
            Constraint::Min(5),     // Altyapı Adımları
            Constraint::Length(8),  // Anomaliler
            Constraint::Min(5),     // Onarım Günlüğü
        ])
        .split(area);

    let col_widths = [
        Constraint::Percentage(35),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(25),
    ];

    // 1. Kanonik Pipeline Timeline
    let canon_rows: Vec<Row> = canon_steps.iter().map(|s| make_row(s)).collect();
    let canon_table = Table::new(canon_rows, col_widths)
        .header(Row::new(vec!["Faz", "Durum", "Son Çalışma", "Gecikme"])
            .style(Style::default().fg(Color::Yellow)))
        .block(Block::default()
            .title(format!(" 🌀 Pipeline Timeline ({}/{} faz) ", canon_steps.len(), 7))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)));
    f.render_widget(canon_table, chunks[0]);

    // 2. Altyapı Adımları
    let infra_rows: Vec<Row> = infra_steps.iter().map(|s| make_row(s)).collect();
    let infra_table = Table::new(infra_rows, col_widths)
        .header(Row::new(vec!["Altyapı", "Durum", "Son Çalışma", "Gecikme"])
            .style(Style::default().fg(Color::Yellow)))
        .block(Block::default()
            .title(format!(" ⚙️ Altyapı Adımları ({}) ", infra_steps.len()))
            .borders(Borders::ALL));
    f.render_widget(infra_table, chunks[1]);

    // 3. Anomali Listesi
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
    f.render_widget(anomalies, chunks[2]);

    // 4. Onarım Günlüğü
    let repair_items: Vec<ListItem> = snap.repair_log.iter().rev().take(10).map(|log| {
        ListItem::new(log.as_str()).style(Style::default().fg(Color::Cyan))
    }).collect();
    let repair_list = List::new(repair_items)
        .block(Block::default().title(" 🔧 Son Onarım İşlemleri ").borders(Borders::ALL));
    f.render_widget(repair_list, chunks[3]);
}
