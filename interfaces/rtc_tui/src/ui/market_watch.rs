// src/ui/market_watch.rs
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Table, Row, Cell, List, ListItem};
use ratatui::style::{Color, Style, Modifier};
use memos_trading_core::core::model::MissionControl;

pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60), // Sol: Fiyat Tablosu
            Constraint::Percentage(40), // Sağ: Seçili Sembolün S/R Detayı
        ])
        .split(area);

    // 1. Canlı Fiyat Tablosu
    let rows: Vec<Row> = snap.market_fleet.iter().map(|m| {
        let color = if m.change_24h >= 0.0 { Color::LightGreen } else { Color::LightRed };
        Row::new(vec![
            Cell::from(m.symbol.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(format!("{:.4}", m.current_price)),
            Cell::from(format!("{:+.2}%", m.change_24h)).style(Style::default().fg(color)),
            Cell::from(m.nearest_support.map(|v| format!("{:.4}", v)).unwrap_or_else(|| "—".into())),
            Cell::from(m.nearest_resistance.map(|v| format!("{:.4}", v)).unwrap_or_else(|| "—".into())),
        ])
    }).collect();

    let table = Table::new(rows, [
        Constraint::Percentage(25), Constraint::Percentage(20),
        Constraint::Percentage(15), Constraint::Percentage(20),
        Constraint::Percentage(20),
    ])
    .header(Row::new(vec!["Sembol", "Fiyat", "24h %", "Destek", "Direnç"]).style(Style::default().fg(Color::Yellow)))
    .block(Block::default().title(" 🌐 Market Gözetimi ").borders(Borders::ALL));

    f.render_widget(table, chunks[0]);

    // 2. S/R Bölge Detay Listesi (Sağ Panel)
    if let Some(selected) = snap.market_fleet.first() { // Şimdilik ilk sembolü gösteriyoruz
        let items: Vec<ListItem> = selected.zones.iter().map(|z| {
            let z_color = if z.zone_type == "Support" { Color::LightGreen } else { Color::LightRed };
            ListItem::new(format!(
                " {} [{:.4} - {:.4}] Güç: {:.1} (x{})",
                if z.zone_type == "Support" { "▼" } else { "▲" },
                z.price_low, z.price_high, z.strength, z.touch_count
            )).style(Style::default().fg(z_color))
        }).collect();

        let list = List::new(items)
            .block(Block::default().title(format!(" {} Teknik Bölgeler ", selected.symbol)).borders(Borders::ALL));
        f.render_widget(list, chunks[1]);
    }
}
