// src/ui/special_trades.rs
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::style::{Color, Style, Modifier};
use memos_trading_core::core::model::{MissionControl, TradeTypeStats};

/// Bir grupta gerçek veri var mı? Tüm alanlar sıfırsa henüz hiç kapanmış trade gelmemiş
/// demektir (compute_scalp_swing_stats boş grup için sıfır döndürüyor).
fn is_empty(stats: &TradeTypeStats) -> bool {
    stats.win_rate == 0.0 && stats.profit_factor == 0.0
        && stats.avg_win == 0.0 && stats.avg_loss == 0.0
        && stats.current_streak == 0
}

fn draw_stat_box(area: Rect, f: &mut ratatui::Frame, stats: &TradeTypeStats, color: Color) {
    if is_empty(stats) {
        let p = Paragraph::new("\n  [○] Bu grupta henüz kapanmış işlem yok.\n      İlk trade kapandığında doldurulur.")
            .block(Block::default()
                .title(format!(" {} İstatistikleri ", stats.label))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)))
            .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC));
        f.render_widget(p, area);
        return;
    }
    let info = vec![
        format!(" Kazanma: {:.1}% ", stats.win_rate),
        format!(" Kâr Faktörü: {:.2} ", stats.profit_factor),
        format!(" Ort. Kazanç: ${:.2} ", stats.avg_win),
        format!(" Ort. Kayıp: -${:.2} ", stats.avg_loss),
        format!(" Güncel Seri: {} ", stats.current_streak),
    ].join("\n");

    let p = Paragraph::new(info)
        .block(Block::default()
            .title(format!(" {} İstatistikleri ", stats.label))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color)));
    f.render_widget(p, area);
}

pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    draw_stat_box(chunks[0], f, &snap.scalp_stats, Color::Magenta);
    draw_stat_box(chunks[1], f, &snap.swing_stats, Color::Cyan);
}
