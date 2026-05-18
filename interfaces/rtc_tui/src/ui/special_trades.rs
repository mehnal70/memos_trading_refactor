// src/ui/special_trades.rs
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::style::{Color, Style};
use memos_trading_core::core::model::{MissionControl, TradeTypeStats};

fn draw_stat_box(area: Rect, f: &mut ratatui::Frame, stats: &TradeTypeStats, color: Color) {
    let _streak_color = if stats.current_streak >= 0 { Color::Green } else { Color::Red };
    let info = vec![
        format!(" Kazanma: {:.1}% ", stats.win_rate),
        format!(" Kâr Faktörü: {:.2} ", stats.profit_factor),
        format!(" Ort. Kazanç: ${:.2} ", stats.avg_win),
        format!(" Ort. Kayıp: -${:.2} ", stats.avg_loss),
        format!(" Güncel Seri: {} ", stats.current_streak),
    ].join("\n");

    let p = Paragraph::new(info)
        .block(Block::default().title(format!(" {} İstatistikleri ", stats.label)).borders(Borders::ALL).border_style(Style::default().fg(color)));
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
