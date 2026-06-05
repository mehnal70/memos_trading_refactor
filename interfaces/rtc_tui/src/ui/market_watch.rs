// src/ui/market_watch.rs
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Table, Row, Cell, List, ListItem};
use ratatui::style::{Color, Style, Modifier};
use memos_trading_core::core::model::MissionControl;

/// `selected_index` Up/Down ile gezilen sembol; sol tablodaki ilgili satırı
/// highlight eder ve sağ panelde sadece o sembolün S/R bölgelerini gösterir.
pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl, selected_index: usize) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60), // Sol: Fiyat Tablosu
            Constraint::Percentage(40), // Sağ: Seçili Sembolün S/R Detayı
        ])
        .split(area);

    let selected_index = if snap.market_fleet.is_empty() {
        0
    } else {
        selected_index.min(snap.market_fleet.len() - 1)
    };

    // 1. Canlı Fiyat Tablosu — seçili satır vurgu rengiyle çizilir.
    let highlight = Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD);
    let rows: Vec<Row> = snap.market_fleet.iter().enumerate().map(|(i, m)| {
        let color = if m.change_24h >= 0.0 { Color::LightGreen } else { Color::LightRed };
        let marker = if i == selected_index { "▶" } else { " " };
        // Piyasa: futures/perp/coinm → Vadeli (sarı, kaldıraç riski), boş → "—", diğer → Spot (gri).
        let ml = m.market.to_lowercase();
        let (mkt_label, mkt_color) = if ml.contains("fut") || ml.contains("perp") || ml.contains("coinm") {
            ("Vadeli", Color::Yellow)
        } else if ml.is_empty() {
            ("—", Color::DarkGray)
        } else {
            ("Spot", Color::Gray)
        };
        let row = Row::new(vec![
            Cell::from(format!("{} {}", marker, m.symbol)).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(mkt_label).style(Style::default().fg(mkt_color)),
            Cell::from(format!("{:.4}", m.current_price)),
            Cell::from(format!("{:+.2}%", m.change_24h)).style(Style::default().fg(color)),
            Cell::from(m.nearest_support.map(|v| format!("{:.4}", v)).unwrap_or_else(|| "—".into())),
            Cell::from(m.nearest_resistance.map(|v| format!("{:.4}", v)).unwrap_or_else(|| "—".into())),
        ]);
        if i == selected_index { row.style(highlight) } else { row }
    }).collect();

    let table = Table::new(rows, [
        Constraint::Percentage(22), Constraint::Percentage(12),
        Constraint::Percentage(18), Constraint::Percentage(14),
        Constraint::Percentage(17), Constraint::Percentage(17),
    ])
    .header(Row::new(vec!["Sembol", "Piyasa", "Fiyat", "24h %", "Destek", "Direnç"]).style(Style::default().fg(Color::Yellow)))
    .block(Block::default()
        .title(" 🌐 Market Gözetimi  [↑/↓ sembol seç] ")
        .borders(Borders::ALL));

    f.render_widget(table, chunks[0]);

    // 2. S/R Bölge Detay Listesi (Sağ Panel) — seçili sembole bağlı.
    if let Some(selected) = snap.market_fleet.get(selected_index) {
        let items: Vec<ListItem> = selected.zones.iter().map(|z| {
            let z_color = if z.zone_type == "Support" { Color::LightGreen } else { Color::LightRed };
            ListItem::new(format!(
                " {} [{:.4} - {:.4}] Güç: {:.1} (x{})",
                if z.zone_type == "Support" { "▼" } else { "▲" },
                z.price_low, z.price_high, z.strength, z.touch_count
            )).style(Style::default().fg(z_color))
        }).collect();

        let list = List::new(items)
            .block(Block::default()
                .title(format!(" {} Teknik Bölgeler ({}) ", selected.symbol, selected.zones.len()))
                .borders(Borders::ALL));
        f.render_widget(list, chunks[1]);
    } else {
        // market_fleet boş — bilgilendirici placeholder.
        let empty = List::new(vec![
            ListItem::new(" (henüz S/R verisi yok — SR updater ilk turunu bekliyor)")
                .style(Style::default().fg(Color::DarkGray))
        ]).block(Block::default().title(" Teknik Bölgeler ").borders(Borders::ALL));
        f.render_widget(empty, chunks[1]);
    }
}
