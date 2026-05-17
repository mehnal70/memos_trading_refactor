// src/ui/charts.rs
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Paragraph, canvas::{Canvas, Points}};
use ratatui::style::{Color, Style, Modifier};
use ratatui::symbols::Marker;
use std::f64::consts::PI;
use memos_trading_core::core::model::MissionControl;

const CHART_COLORS: [Color; 6] = [Color::Cyan, Color::Green, Color::Yellow, Color::Magenta, Color::Red, Color::Blue];

pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // 1. Donut Pasta Grafiği (Canvas)
    let distributions = &snap.charts.distributions;
    let total_count: u32 = distributions.iter().map(|d| d.trade_count).sum();

    if total_count > 0 {
        let canvas = Canvas::default()
            .block(Block::default().title(" 📊 Sembol Dağılımı (İşlem Sayısı) ").borders(Borders::ALL))
            .marker(Marker::Braille)
            .x_bounds([-5.0, 5.0])
            .y_bounds([-5.0, 5.0])
            .paint(|ctx| {
                let mut current_angle = 0.0;
                for (i, dist) in distributions.iter().enumerate() {
                    let share = dist.trade_count as f64 / total_count as f64;
                    let angle_step = share * 2.0 * PI;
                    let color = CHART_COLORS[i % CHART_COLORS.len()];
                    
                    // Donut dilim noktalarını hesapla
                    for a_idx in 0..100 {
                        let angle = current_angle + (a_idx as f64 / 100.0) * angle_step;
                        // Halka kalınlığı için 2 farklı yarıçap
                        for r_step in 0..5 {
                            let r = 3.0 + (r_step as f64 * 0.2);
                            let x = r * angle.cos();
                            let y = r * angle.sin();
                            ctx.draw(&Points { coords: &[(x, y)], color });
                        }
                    }
                    current_angle += angle_step;
                }
            });
        f.render_widget(canvas, chunks[0]);
    }

    // 2. Performans Özeti (Sağ Panel)
    let stats_lines: Vec<ratatui::text::Line> = distributions.iter().enumerate().map(|(i, d)| {
        let color = CHART_COLORS[i % CHART_COLORS.len()];
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("█ ", Style::default().fg(color)),
            ratatui::text::Span::raw(format!("{:<10} | PnL: {:+>8.2}$ | WR: {:.0}%", d.symbol, d.pnl, d.win_rate)),
        ])
    }).collect();

    let p = Paragraph::new(stats_lines)
        .block(Block::default().title(" 📈 Varlık Performans Analizi ").borders(Borders::ALL));
    f.render_widget(p, chunks[1]);
}
