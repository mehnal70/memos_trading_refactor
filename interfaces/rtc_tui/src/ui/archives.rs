// interfaces/rtc_tui/src/ui/archives.rs

use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Block, Borders, List, ListItem, Row, Table, Paragraph};
use ratatui::style::{Color, Style, Modifier};
use memos_trading_core::core::model::MissionControl;

/// Olay Günlüğü (Tab 3) - Akıllı Kaydırma Destekli
pub fn draw_logs(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl, scroll: usize) {
    let log_items: Vec<ListItem> = snap.logs.iter()
        .rev() // En yeni en üstte
        .skip(scroll)
        .map(|log| {
            let color = match log.level.as_str() {
                "ERROR"  => Color::Red,
                "WARN"   => Color::Yellow,
                "SIGNAL" => Color::Cyan,
                _        => Color::Gray,
            };
            ListItem::new(format!(" {} | {} ", log.timestamp, log.message))
                .style(Style::default().fg(color))
        })
        .collect();

    let title = if scroll > 0 {
        format!(" 📋 Olay Günlüğü [Geçmişte: {} · Home: canlıya dön] ", scroll)
    } else {
        " 📋 Olay Günlüğü [Canlı · ↑/↓ geçmişi kaydır] ".to_string()
    };

    let list = List::new(log_items)
        .block(Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if scroll > 0 { Color::Yellow } else { Color::Blue })));
    
    f.render_widget(list, area);
}

/// Harekât Tarihçesi (Tab 4) - Kapanmış İşlemler Analizi
pub fn draw_trade_history(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    if snap.trade_history.is_empty() {
        let empty = Paragraph::new("\n\n  [○] Henüz kapanmış bir işlem kaydı bulunmuyor.")
            .block(Block::default().title(" 📜 Harekât Tarihçesi ").borders(Borders::ALL))
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty, area);
        return;
    }

    let rows: Vec<Row> = snap.trade_history.iter().map(|t| {
        let pnl_color = if t.pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
        Row::new(vec![
            format!("{}", t.closed_at),
            t.symbol.clone(),
            if t.is_long { "▲ LONG".into() } else { "▼ SHORT".into() },
            format!("{:+.2} USDT", t.pnl),
            format!("{:+.2}%", t.pnl_pct),
            t.exit_reason.clone(),
        ]).style(Style::default().fg(pnl_color))
    }).collect();

    let table = Table::new(rows, [
        Constraint::Length(20), // Zaman
        Constraint::Length(10), // Sembol
        Constraint::Length(10), // Yön
        Constraint::Length(15), // PnL
        Constraint::Length(10), // %
        Constraint::Min(15),    // Neden
    ])
    .header(Row::new(vec!["Kapanış", "Sembol", "Yön", "Net PnL", "ROE%", "Çıkış Nedeni"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
    .block(Block::default().title(" 📜 Kapanmış İşlemler (Son 50) ").borders(Borders::ALL));

    f.render_widget(table, area);
}
