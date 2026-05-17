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

    // 1. Üst Bilgi ve Gauge
    components::render_finance_header(chunks[0], f, &snap.finance);
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
            Constraint::Percentage(15), 
            Constraint::Percentage(10), 
            Constraint::Percentage(10), 
            Constraint::Percentage(15), 
            Constraint::Percentage(15), 
            Constraint::Percentage(15), 
            Constraint::Percentage(20), 
        ])
        .header(
            Row::new(vec!["Sembol", "Tip", "Yön", "Giriş", "Fiyat", "PnL", "ROE%"])
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
