// interfaces/rtc_tui/src/ui/risk_center.rs - 🛡️ Risk & Anomali & Onarım Komuta Merkezi
//
// 4 bölümlü panel:
//   1. Drawdown & Equity sparkline (üst)
//   2. Aktif anomaliler tablosu (sol-alt)
//   3. Son onarım logu (sağ-üst alt)
//   4. Risk gate karar kütüğü (alt, guardian.log'dan filtrelenmiş)

use memos_trading_core::core::model::MissionControl;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Sparkline, Table};

pub fn draw(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    // Üst (9 satır) drawdown + sparkline, alt kalan kısım 3 satır.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),  // Üst bant: equity sparkline (header 3 + spark 6)
            Constraint::Min(8),     // Orta: anomaliler + onarım
            Constraint::Length(8),  // Alt: risk karar kütüğü
        ])
        .split(area);

    draw_equity_sparkline(f, outer[0], snap);

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(outer[1]);
    draw_active_anomalies(f, middle[0], snap);
    draw_repair_log(f, middle[1], snap);

    draw_risk_decisions(f, outer[2], snap);
}

fn draw_equity_sparkline(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    // Başlık satırı: equity, drawdown, peak
    let dd = snap.charts.current_drawdown_pct;
    let dd_color = if dd > 10.0 { Color::Red }
                   else if dd > 5.0 { Color::Yellow }
                   else { Color::LightGreen };
    // Equity = realized (starting + realize_pnl). Sparkline ile aynı seri →
    // açık pozisyonun mark-to-market'i header'ı titretmesin. Anlık NET değer
    // ana dashboard header'ında zaten görünür.
    let realized_equity = snap.finance.starting_capital + snap.finance.realize_pnl;
    let header = Line::from(vec![
        Span::styled(" Equity: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("${:.2}", realized_equity),
                     Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("Peak: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("${:.2}", snap.charts.peak_equity),
                     Style::default().fg(Color::LightBlue)),
        Span::raw("   "),
        Span::styled("Drawdown: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{:.2}%", dd),
                     Style::default().fg(dd_color).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("Başlangıç: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("${:.2}", snap.finance.starting_capital),
                     Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(
        Paragraph::new(header)
            .block(Block::default().title(" 📈 Varlık Performansı ").borders(Borders::ALL)),
        chunks[0],
    );

    // Sparkline: equity_series. u64 ölçek için scale.
    let series = &snap.charts.equity_series;
    if series.is_empty() {
        let placeholder = Paragraph::new("  Veri henüz yok (engine ısınıyor)…")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(placeholder, chunks[1]);
        return;
    }

    let min = series.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = series.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1.0);
    // 0..1000 u64 aralığına normalize
    let normalized: Vec<u64> = series.iter()
        .map(|v| (((*v - min) / span) * 1000.0).round() as u64)
        .collect();

    // Yön rengi: son nokta başlangıçtan büyük mü
    let trend_color = if series.last().copied().unwrap_or(0.0) >= series.first().copied().unwrap_or(0.0) {
        Color::Green
    } else {
        Color::Red
    };

    let sparkline = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(format!(
            " Equity Akışı ({} nokta · {:.2} → {:.2}) ",
            series.len(), min, max,
        )))
        .data(&normalized)
        .style(Style::default().fg(trend_color));
    f.render_widget(sparkline, chunks[1]);
}

fn draw_active_anomalies(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    if snap.anomalies.is_empty() {
        let msg = Paragraph::new("\n  ✅ Tüm sistemler nominal. Aktif anomali yok.")
            .style(Style::default().fg(Color::LightGreen))
            .block(Block::default().title(" 🛡️ Aktif Anomaliler ").borders(Borders::ALL));
        f.render_widget(msg, area);
        return;
    }

    let rows: Vec<Row> = snap.anomalies.iter().map(|a| {
        let sev_color = if a.severity.contains("Critical") { Color::Red } else { Color::Yellow };
        let fix_marker = if a.auto_fixed { "✅" } else { "🚨" };
        Row::new(vec![
            Cell::from(fix_marker),
            Cell::from(a.severity.clone()).style(Style::default().fg(sev_color)),
            Cell::from(a.kind.clone()).style(Style::default().fg(Color::LightCyan)),
            Cell::from(a.message.clone()),
        ])
    }).collect();

    // Tip-başı dağılım özetini başlığa ekle: "DataStall:32 ApiError:15 ..."
    // Boot anında 50 anomaly olduğunda hangi tipin baskın olduğunu tek bakışta
    // görmek için.
    let kind_summary = format_kind_summary(&snap.anomalies_by_kind);
    let title = if kind_summary.is_empty() {
        format!(" 🛡️ Aktif Anomaliler ({}) ", snap.anomalies.len())
    } else {
        format!(
            " 🛡️ Aktif Anomaliler ({}) · {} ",
            snap.anomalies.len(),
            kind_summary,
        )
    };

    let table = Table::new(rows, [
        Constraint::Length(2),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Min(20),
    ])
    .header(Row::new(vec!["", "Severity", "Kind", "Mesaj"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
    .block(Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red)));
    f.render_widget(table, area);
}

/// Anomaly kind sayımlarını başlık satırı için "Kind:N · Kind:N" formatına çevirir.
/// Çoktan aza göre sıralar.
fn format_kind_summary(by_kind: &std::collections::BTreeMap<String, usize>) -> String {
    if by_kind.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(&String, &usize)> = by_kind.iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    pairs.iter()
        .map(|(k, v)| format!("{}:{}", k, v))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn draw_repair_log(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    if snap.repair_log.is_empty() {
        let msg = Paragraph::new("\n  Henüz otonom onarım kaydı yok.")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().title(" 🔧 Son Onarım İşleri ").borders(Borders::ALL));
        f.render_widget(msg, area);
        return;
    }
    let items: Vec<ListItem> = snap.repair_log.iter().take(20).map(|entry| {
        ListItem::new(entry.as_str()).style(Style::default().fg(Color::Cyan))
    }).collect();
    let list = List::new(items)
        .block(Block::default()
            .title(format!(" 🔧 Son Onarım İşleri ({}) ", snap.repair_log.len()))
            .borders(Borders::ALL));
    f.render_widget(list, area);
}

fn draw_risk_decisions(f: &mut ratatui::Frame, area: Rect, snap: &MissionControl) {
    // Risk kararları guardian.log'da emoji desenleriyle yazıyor:
    //   🛡️  ⇒ RiskManager veto
    //   📊  ⇒ edge filtresi (kabul/red)
    //   🔄  ⇒ pozisyon kapanış
    //   🚨  ⇒ anomali / adli uyarı
    // bridge en yeni 100 log'u zaten "rev" sırada veriyor.
    let filtered: Vec<&memos_trading_core::core::model::LogEntry> = snap.logs.iter()
        .filter(|l| {
            let m = &l.message;
            m.contains("🛡️") || m.contains("📊") || m.contains("🔄") || m.contains("🚨")
        })
        .take(15)
        .collect();

    if filtered.is_empty() {
        let msg = Paragraph::new("\n  Henüz risk gate kararı yok (sinyal bekleniyor)…")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().title(" ⚖️ Risk Gate Karar Kütüğü ").borders(Borders::ALL));
        f.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = filtered.iter().map(|l| {
        let style = if l.message.contains("VETO") { Style::default().fg(Color::Red) }
                    else if l.message.contains("REDDEDİLDİ") { Style::default().fg(Color::Yellow) }
                    else if l.message.contains("AÇILIYOR") || l.message.contains("KAPANIŞ") {
                        Style::default().fg(Color::LightGreen)
                    }
                    else { Style::default().fg(Color::White) };
        ListItem::new(l.message.as_str()).style(style)
    }).collect();

    let list = List::new(items).block(Block::default()
        .title(" ⚖️ Risk Gate Karar Kütüğü (son 15) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta)));
    f.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_trading_core::core::model::{
        AiBrainSnapshot, AnomalyModel, ChartSnapshot, FinanceSnapshot, LogEntry,
        MissionControl, TradeTypeStats,
    };
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_snap_with_data() -> MissionControl {
        MissionControl {
            finance: FinanceSnapshot {
                total_equity: 10250.0,
                realize_pnl: 250.0,
                open_pnl: 0.0,
                starting_capital: 10000.0,
                total_fees: 12.5,
            },
            positions: vec![],
            fleet: vec![],
            phase: "Scanning".into(),
            pipeline_steps: vec![],
            ai_brain: AiBrainSnapshot {
                genome_id: "test".into(), fitness: 0.0, win_rate: 0.0, trade_count: 0,
                gbt_score: Some(0.0), exploration_rate: 0.1, drift_score: 0.0,
                mc_ruin_prob: 0.0, is_evolution_active: false, next_evolution_secs: 0,
                live_strategy: String::new(), controller_state: String::new(),
                controller_cycle: 0, consecutive_failures: 0, pending_trades: 0,
                drift_series: vec![], best_tp_pct: 0.0, best_sl_pct: 0.0,
                best_position_size: 0.0,
            },
            market_fleet: vec![],
            logs: vec![
                LogEntry { timestamp: "10:00".into(), level: "INFO".into(),
                    message: "📊 BTCUSDT BUY edge=0.71 ✓ + risk ✓ ⇒ POZİSYON AÇILIYOR".into() },
                LogEntry { timestamp: "10:01".into(), level: "WARN".into(),
                    message: "🛡️ ETHUSDT BUY edge=0.62 ✓ ama RiskManager VETO etti".into() },
            ],
            trade_history: vec![],
            charts: ChartSnapshot {
                distributions: vec![],
                total_closed_pnl: 250.0,
                total_trade_count: 5,
                equity_series: vec![10000.0, 10050.0, 10100.0, 10250.0],
                current_drawdown_pct: 0.0,
                peak_equity: 10250.0,
            },
            anomalies: vec![
                AnomalyModel {
                    severity: "Warning".into(),
                    kind: "DataStall".into(),
                    message: "main_loop gecikti: +6s".into(),
                    fix_hint: String::new(),
                    auto_fixed: false,
                },
            ],
            repair_log: vec!["[10:02:15] auto-fix: ml-retrain dispatched (anomaly_count=1)".into()],
            scalp_stats: TradeTypeStats { label: "SCALP".into(), win_rate: 0.0, profit_factor: 0.0,
                avg_win: 0.0, avg_loss: 0.0, current_streak: 0 },
            swing_stats: TradeTypeStats { label: "SWING".into(), win_rate: 0.0, profit_factor: 0.0,
                avg_win: 0.0, avg_loss: 0.0, current_streak: 0 },
            active_anomalies: 1,
            anomalies_by_kind: {
                let mut m = std::collections::BTreeMap::new();
                m.insert("DataStall".to_string(), 1);
                m
            },
        }
    }

    fn buffer_to_string(t: &Terminal<TestBackend>) -> String {
        let buf = t.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf.get(x, y).symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn risk_center_renders_all_four_widgets() {
        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let snap = make_snap_with_data();
        terminal.draw(|f| draw(f, f.size(), &snap)).unwrap();

        let rendered = buffer_to_string(&terminal);

        // 1. Varlık Performansı başlığı + equity sayıları
        assert!(rendered.contains("Varlık Performansı"),
            "üst varlık paneli başlığı yok\n{}", rendered);
        assert!(rendered.contains("10250"),
            "total_equity render edilmemiş\n{}", rendered);
        assert!(rendered.contains("Drawdown"),
            "drawdown etiketi yok\n{}", rendered);

        // 2. Aktif anomaliler tablosu
        assert!(rendered.contains("Aktif Anomaliler"),
            "anomali paneli başlığı yok\n{}", rendered);
        assert!(rendered.contains("DataStall"),
            "anomali kind render edilmemiş\n{}", rendered);

        // 3. Onarım logu
        assert!(rendered.contains("Onarım"),
            "onarım paneli başlığı yok\n{}", rendered);
        assert!(rendered.contains("ml-retrain dispatched"),
            "repair_log içeriği render edilmemiş\n{}", rendered);

        // 4. Risk gate karar kütüğü
        assert!(rendered.contains("Risk Gate"),
            "risk gate paneli başlığı yok\n{}", rendered);
        assert!(rendered.contains("AÇILIYOR") || rendered.contains("VETO"),
            "risk gate karar log'u render edilmemiş\n{}", rendered);
    }

    #[test]
    fn anomaly_kind_summary_appears_in_title_when_many_kinds() {
        let backend = TestBackend::new(160, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut snap = make_snap_with_data();
        // Boot anomaly senaryosu: birden çok tip aynı snapshot içinde.
        snap.anomalies.clear();
        for _ in 0..32 {
            snap.anomalies.push(AnomalyModel {
                severity: "Warning".into(), kind: "DataStall".into(),
                message: "stall".into(), fix_hint: String::new(), auto_fixed: false,
            });
        }
        for _ in 0..12 {
            snap.anomalies.push(AnomalyModel {
                severity: "Warning".into(), kind: "ApiError".into(),
                message: "err".into(), fix_hint: String::new(), auto_fixed: false,
            });
        }
        snap.active_anomalies = snap.anomalies.len();
        snap.anomalies_by_kind = {
            let mut m = std::collections::BTreeMap::new();
            m.insert("DataStall".into(), 32);
            m.insert("ApiError".into(), 12);
            m
        };

        terminal.draw(|f| draw(f, f.size(), &snap)).unwrap();
        let rendered = buffer_to_string(&terminal);
        // Başlıkta tip-başı dağılım görünmeli (DataStall çoğunlukta → önce).
        assert!(
            rendered.contains("DataStall:32"),
            "DataStall:32 başlık özetinde yok\n{}", rendered,
        );
        assert!(
            rendered.contains("ApiError:12"),
            "ApiError:12 başlık özetinde yok\n{}", rendered,
        );
    }

    #[test]
    fn format_kind_summary_sorts_descending_by_count() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("ApiError".into(), 5);
        m.insert("DataStall".into(), 32);
        m.insert("Drift".into(), 1);
        let out = format_kind_summary(&m);
        // En çok olan başta: DataStall:32 → ApiError:5 → Drift:1
        assert_eq!(out, "DataStall:32 · ApiError:5 · Drift:1");
    }

    #[test]
    fn format_kind_summary_empty_returns_blank() {
        let m: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        assert_eq!(format_kind_summary(&m), "");
    }

    #[test]
    fn risk_center_empty_state_is_friendly() {
        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut snap = make_snap_with_data();
        snap.anomalies.clear();
        snap.repair_log.clear();
        snap.logs.clear();
        snap.charts.equity_series.clear();

        terminal.draw(|f| draw(f, f.size(), &snap)).unwrap();
        let rendered = buffer_to_string(&terminal);

        assert!(rendered.contains("nominal"),
            "anomali boş durum mesajı yok\n{}", rendered);
        assert!(rendered.contains("Henüz otonom onarım"),
            "repair_log boş durum mesajı yok\n{}", rendered);
        assert!(rendered.contains("sinyal bekleniyor"),
            "risk gate boş durum mesajı yok\n{}", rendered);
    }
}
