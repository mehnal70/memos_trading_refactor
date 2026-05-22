// src/ui/ai_center.rs - 🧠 İSTİHBARAT BAŞKANLIĞI (Canlı IntelligenceHub Görüntüleyici)
//
// 5 bölümlü panel:
//   1. Üst başlık: aktif strateji + controller state + cycle + pending_trades
//   2. Drift skoru + sparkline (son 60 nokta)
//   3. Best Params tablosu (TP/SL/PS + Sharpe)
//   4. ML karar destek (GBT, win rate, trade count, exploration)
//   5. Risk validasyonu (consecutive failures, MC ruin, evolution status)

use memos_trading_core::core::model::MissionControl;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Sparkline, Table};

pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // 1. Strateji başlık satırı
            Constraint::Length(7),  // 2. Drift sparkline
            Constraint::Length(7),  // 3. Best Params tablosu
            Constraint::Min(5),     // 4+5. Karar destek + risk yan yana
        ])
        .split(area);

    draw_strategy_header(f, outer[0], snap);
    draw_drift_sparkline(f, outer[1], snap);
    draw_best_params(f, outer[2], snap);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(outer[3]);
    draw_decision_support(f, bottom[0], snap);
    draw_risk_validation(f, bottom[1], snap);
}

fn draw_strategy_header(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let brain = &snap.ai_brain;
    let ctrl_color = match brain.controller_state.as_str() {
        "Trade"    => Color::LightGreen,
        "Observe"  => Color::LightCyan,
        "Optimize" => Color::LightYellow,
        "SafeMode" => Color::Yellow,
        "Halted"   => Color::Red,
        _          => Color::DarkGray,
    };
    let strategy_display = if brain.live_strategy.is_empty() || brain.live_strategy == "—" {
        "AUTO (regime-based)".to_string()
    } else { brain.live_strategy.clone() };

    let line = Line::from(vec![
        Span::styled(" 🎯 Aktif Strateji: ", Style::default().fg(Color::DarkGray)),
        Span::styled(strategy_display, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("Controller: ", Style::default().fg(Color::DarkGray)),
        Span::styled(brain.controller_state.clone(),
                     Style::default().fg(ctrl_color).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("Cycle: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{}", brain.controller_cycle), Style::default().fg(Color::Cyan)),
        Span::raw("   "),
        Span::styled("Bekleyen: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{}", brain.pending_trades),
                     Style::default().fg(if brain.pending_trades > 0 { Color::LightGreen } else { Color::DarkGray })),
    ]);
    let p = Paragraph::new(line)
        .block(Block::default().borders(Borders::ALL).title(" 🧬 İstihbarat Başkanlığı "));
    f.render_widget(p, area);
}

fn draw_drift_sparkline(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let brain = &snap.ai_brain;
    // Drift eşiği: 0.35 (DriftDetector default). Renk drift'in eşik üstünde olup olmadığına göre.
    let drift_color = if brain.drift_score >= 0.35 { Color::Red }
                      else if brain.drift_score >= 0.20 { Color::Yellow }
                      else { Color::Green };

    if brain.drift_series.is_empty() {
        let msg = Paragraph::new(format!(
            "  Drift skoru: {:.3} (henüz tarihçe yok, hub ısınıyor…)",
            brain.drift_score,
        ))
        .style(Style::default().fg(drift_color))
        .block(Block::default().title(" 📉 Drift İzleyici ").borders(Borders::ALL));
        f.render_widget(msg, area);
        return;
    }

    // Sparkline: 0..1000 u64'a normalize et (drift 0..1 aralığında).
    let normalized: Vec<u64> = brain.drift_series.iter()
        .map(|d| (d.clamp(0.0, 1.0) * 1000.0) as u64)
        .collect();
    let max_drift = brain.drift_series.iter().cloned().fold(0.0_f64, f64::max);
    let title = format!(
        " 📉 Drift Akışı (anlık: {:.3} · max: {:.3} · eşik: 0.35 · {} nokta) ",
        brain.drift_score, max_drift, brain.drift_series.len(),
    );
    let sparkline = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .data(&normalized)
        .style(Style::default().fg(drift_color));
    f.render_widget(sparkline, area);
}

fn draw_best_params(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let brain = &snap.ai_brain;
    let tp = brain.best_tp_pct;
    let sl = brain.best_sl_pct;
    let ps = brain.best_position_size;
    let rr = if sl > 0.0 { tp / sl } else { 0.0 };

    let rows = vec![
        Row::new(vec![
            Cell::from("Take Profit %"),
            Cell::from(format!("{:.2}", tp)).style(Style::default().fg(Color::LightGreen)),
            Cell::from("ML retrain optimize ediyor"),
        ]),
        Row::new(vec![
            Cell::from("Stop Loss %"),
            Cell::from(format!("{:.2}", sl)).style(Style::default().fg(Color::LightRed)),
            Cell::from("Pozisyon SL seviyesi"),
        ]),
        Row::new(vec![
            Cell::from("Position Size"),
            Cell::from(format!("{:.2}", ps)).style(Style::default().fg(Color::LightCyan)),
            Cell::from("Equity oranı (Kelly çarpan)"),
        ]),
        Row::new(vec![
            Cell::from("Risk/Reward"),
            Cell::from(format!("{:.2}", rr)).style(Style::default().fg(
                if rr >= 1.5 { Color::LightGreen }
                else if rr >= 1.0 { Color::Yellow }
                else { Color::LightRed }
            )),
            Cell::from(format!("Sharpe (hyperopt): {:.2}", brain.gbt_score.unwrap_or(0.0))),
        ]),
    ];

    let table = Table::new(rows, [
        Constraint::Length(16),
        Constraint::Length(10),
        Constraint::Min(20),
    ])
    .header(Row::new(vec!["Parametre", "Değer", "Açıklama"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
    .block(Block::default()
        .title(" 🎛️ Best Params (ML Retrain ürünü) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan)));
    f.render_widget(table, area);
}

fn draw_decision_support(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let brain = &snap.ai_brain;
    let gbt = brain.gbt_score.unwrap_or(0.0);
    let gbt_color = if gbt > 0.5 { Color::LightGreen }
                    else if gbt > 0.0 { Color::Green }
                    else if gbt < -0.1 { Color::Red }
                    else { Color::Yellow };

    let items = vec![
        ListItem::new(Line::from(vec![
            Span::styled("Hyperopt Skoru: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:+.4}", gbt), Style::default().fg(gbt_color).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("Win Rate: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:.1}%", brain.win_rate),
                         Style::default().fg(if brain.win_rate >= 50.0 { Color::Green } else { Color::Yellow })),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("Trade Sayısı: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", brain.trade_count),
                         Style::default().fg(Color::LightCyan)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("Exploration: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:.1}%", brain.exploration_rate * 100.0),
                         Style::default().fg(Color::Magenta)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("Fitness (Equity): ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("${:.2}", brain.fitness),
                         Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ])),
    ];
    let list = List::new(items).block(Block::default()
        .title(" 🧠 ML Karar Destek ")
        .borders(Borders::ALL));
    f.render_widget(list, area);
}

fn draw_risk_validation(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let brain = &snap.ai_brain;
    let cf_color = if brain.consecutive_failures >= 5 { Color::Red }
                   else if brain.consecutive_failures >= 3 { Color::Yellow }
                   else { Color::Green };
    let mc_color = if brain.mc_ruin_prob >= 5.0 { Color::Red } else { Color::Green };
    let evo_marker = if brain.is_evolution_active { "● AKTİF" } else { "○ PASİF" };
    let evo_color = if brain.is_evolution_active { Color::LightGreen } else { Color::Red };

    let items = vec![
        ListItem::new(Line::from(vec![
            Span::styled("Ardışık Kayıp: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", brain.consecutive_failures),
                         Style::default().fg(cf_color).add_modifier(Modifier::BOLD)),
            Span::styled(
                if brain.consecutive_failures >= 5 { " ⚠️ SafeMode" } else { "" },
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("MC İflas: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:.2}%", brain.mc_ruin_prob), Style::default().fg(mc_color)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("Evrim Motoru: ", Style::default().fg(Color::DarkGray)),
            Span::styled(evo_marker, Style::default().fg(evo_color).add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("Sonraki Evrim: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} sn", brain.next_evolution_secs),
                         Style::default().fg(Color::LightYellow)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("Genom: ", Style::default().fg(Color::DarkGray)),
            Span::styled(brain.genome_id.clone(),
                         Style::default().fg(Color::White)),
        ])),
    ];
    let list = List::new(items).block(Block::default()
        .title(" 🎲 Risk Validasyonu ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if brain.consecutive_failures >= 5 { Color::Red } else { Color::Magenta })));
    f.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_trading_core::core::model::{
        AiBrainSnapshot, ChartSnapshot, FinanceSnapshot, MissionControl, TradeTypeStats,
    };
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_snap() -> MissionControl {
        MissionControl {
            finance: FinanceSnapshot {
                total_equity: 10250.0, realize_pnl: 250.0, open_pnl: 0.0,
                starting_capital: 10000.0, total_fees: 10.0,
            },
            positions: vec![], fleet: vec![], phase: "Scanning".into(),
            pipeline_steps: vec![],
            ai_brain: AiBrainSnapshot {
                genome_id: "Srivastava-Alpha-9 [cycle=42 · ctrl=Trade]".into(),
                fitness: 10250.0, win_rate: 65.0, trade_count: 17,
                gbt_score: Some(1.85), exploration_rate: 0.1,
                drift_score: 0.18, mc_ruin_prob: 1.2,
                is_evolution_active: true, next_evolution_secs: 180,
                live_strategy: "SUPERTREND".into(),
                controller_state: "Trade".into(),
                controller_cycle: 42,
                consecutive_failures: 2,
                pending_trades: 3,
                drift_series: vec![0.10, 0.12, 0.15, 0.18, 0.20, 0.18, 0.16, 0.15],
                best_tp_pct: 4.5, best_sl_pct: 2.0, best_position_size: 0.3,
            },
            market_fleet: vec![], logs: vec![], trade_history: vec![],
            charts: ChartSnapshot { distributions: vec![], total_closed_pnl: 250.0,
                total_trade_count: 17, equity_series: vec![], current_drawdown_pct: 0.0,
                peak_equity: 10250.0 },
            anomalies: vec![], repair_log: vec![],
            scalp_stats: TradeTypeStats { label: "S".into(), win_rate: 0.0, profit_factor: 0.0,
                avg_win: 0.0, avg_loss: 0.0, current_streak: 0 },
            swing_stats: TradeTypeStats { label: "W".into(), win_rate: 0.0, profit_factor: 0.0,
                avg_win: 0.0, avg_loss: 0.0, current_streak: 0 },
            active_anomalies: 0,
            anomalies_by_kind: std::collections::BTreeMap::new(),
        }
    }

    fn render_string(snap: &MissionControl) -> String {
        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, f.size(), snap)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width { s.push_str(buf.get(x, y).symbol()); }
            s.push('\n');
        }
        s
    }

    #[test]
    fn ai_center_renders_all_five_widgets() {
        let r = render_string(&make_snap());

        // 1. Strateji başlık
        assert!(r.contains("Aktif Strateji"), "strateji başlığı yok\n{}", r);
        assert!(r.contains("SUPERTREND"), "live_strategy değeri yok\n{}", r);
        assert!(r.contains("Trade"), "controller state yok\n{}", r);
        assert!(r.contains("42"), "cycle sayacı yok\n{}", r);

        // 2. Drift sparkline başlığı
        assert!(r.contains("Drift"), "drift başlığı yok\n{}", r);
        assert!(r.contains("0.18") || r.contains("0.180"), "drift skoru yok\n{}", r);

        // 3. Best params tablosu
        assert!(r.contains("Best Params"), "best params başlığı yok\n{}", r);
        assert!(r.contains("Take Profit"), "TP satırı yok\n{}", r);
        assert!(r.contains("4.50"), "TP değeri yok\n{}", r);
        assert!(r.contains("Stop Loss"), "SL satırı yok\n{}", r);

        // 4. Karar destek
        assert!(r.contains("Karar Destek"), "karar destek başlığı yok\n{}", r);
        assert!(r.contains("65.0%"), "win rate yok\n{}", r);
        assert!(r.contains("Hyperopt"), "hyperopt skoru yok\n{}", r);

        // 5. Risk validasyonu
        assert!(r.contains("Risk Valid"), "risk validasyonu başlığı yok\n{}", r);
        assert!(r.contains("Ardışık Kayıp"), "ardışık kayıp yok\n{}", r);
    }

    #[test]
    fn ai_center_safe_mode_warning_appears_at_5_failures() {
        let mut snap = make_snap();
        snap.ai_brain.consecutive_failures = 5;
        let r = render_string(&snap);
        assert!(r.contains("SafeMode"),
            "5 ardışık kayıpta SafeMode uyarısı görünmeli\n{}", r);
    }

    #[test]
    fn ai_center_empty_drift_history_is_friendly() {
        let mut snap = make_snap();
        snap.ai_brain.drift_series.clear();
        let r = render_string(&snap);
        assert!(r.contains("henüz tarihçe yok"),
            "boş drift tarihçesinde dostça mesaj yok\n{}", r);
    }
}
