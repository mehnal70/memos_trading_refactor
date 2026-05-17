// src/ui/ai_center.rs
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::style::{Color, Style, Modifier};
use memos_trading_core::core::model::MissionControl;

pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10), // Genom ve Evrim Özeti
            Constraint::Length(8),  // ML Model Durumu (GBT)
            Constraint::Min(5),     // Monte Carlo / Risk Analizi
        ])
        .split(area);

    let brain = &snap.ai_brain;

    // 1. Evrimsel Durum Paneli
    let evo_status = if brain.is_evolution_active { "● AKTİF" } else { "○ PASİF" };
    let evo_color = if brain.is_evolution_active { Color::LightGreen } else { Color::Red };

    let info = vec![
        format!(" Aktif Genom : {} ", brain.genome_id),
        format!(" Fitness     : {:.4} ", brain.fitness),
        format!(" Kazanma Oranı: {:.1}% ", brain.win_rate),
        format!(" Keşif (Expl) : {:.1}% ", brain.exploration_rate),
        format!(" Durum       : {} ", evo_status),
    ].join("\n");

    let p1 = Paragraph::new(info)
        .block(Block::default().title(" 🧬 Evrimsel AI Motoru ").borders(Borders::ALL).border_style(Style::default().fg(evo_color)));
    
    f.render_widget(p1, chunks[0]);

    // 2. ML Tahmin ve GBT Paneli
    let gbt_val = brain.gbt_score.unwrap_or(0.0);
    let gbt_color = if gbt_val > 0.1 { Color::Green } else if gbt_val < -0.1 { Color::Red } else { Color::Yellow };
    
    let gbt_p = Paragraph::new(format!(" GBT Bias: {:.4} | Drift: {:.3}", gbt_val, brain.drift_score))
        .block(Block::default().title(" 🧠 ML Karar Destek ").borders(Borders::ALL))
        .style(Style::default().fg(gbt_color));
    
    f.render_widget(gbt_p, chunks[1]);

    // 3. Monte Carlo / Validasyon Paneli
    let mc_color = if brain.mc_ruin_prob < 5.0 { Color::Green } else { Color::Red };
    let mc_p = Paragraph::new(format!(" MC İflas Olasılığı: {:.2}% (Risk: {})", brain.mc_ruin_prob, if brain.mc_ruin_prob < 5.0 { "DÜŞÜK" } else { "YÜKSEK" }))
        .block(Block::default().title(" 🎲 Risk Validasyonu ").borders(Borders::ALL))
        .style(Style::default().fg(mc_color));
        
    f.render_widget(mc_p, chunks[2]);
}
