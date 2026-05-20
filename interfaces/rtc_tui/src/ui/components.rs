// src/ui/components.rs
use ratatui::widgets::{Row, Cell, Block, Borders, Paragraph, Gauge};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Modifier};
use memos_trading_core::core::model::{PositionModel, FinanceSnapshot, PipelineStep};
use ratatui::text::{Line, Span};

/// Srivastava ATP - Evrensel Finansal Üst Bilgi.
///
/// Üç bölüm aynı satırda:
///   - **SERMAYE (gerçekleşen)**: starting_capital + realize_pnl. Açık pozisyon
///     kapanana kadar değişmez → "stabil tutamak" gözle hızlı okunur.
///   - **Açık PnL (kümüle)**: tüm açık pozisyonların mark-to-market PnL toplamı;
///     fiyatla anlık dalgalanır ama ayrı sütunda olduğu için sermaye flicker
///     yapmıyor görünür.
///   - **NET**: SERMAYE + Açık PnL = anlık portföy değeri.
///
/// `phase` boş geçilebilir; doluysa sağ tarafa renkli rozet düşer.
pub fn render_finance_header(area: Rect, f: &mut ratatui::Frame, snap: &FinanceSnapshot, phase: &str) {
    let stable_equity = snap.starting_capital + snap.realize_pnl;
    let open_color = if snap.open_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
    let net_color  = if snap.total_equity >= snap.starting_capital { Color::LightGreen } else { Color::LightRed };

    let mut spans = vec![
        Span::styled(
            format!(" 💰 SERMAYE: ${:.2}", stable_equity),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  ·  "),
        Span::styled(
            format!("Açık PnL (kümüle): {:+.2}", snap.open_pnl),
            Style::default().fg(open_color),
        ),
        Span::raw("  ·  "),
        Span::styled(
            format!("NET: ${:.2}", snap.total_equity),
            Style::default().fg(net_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
    ];

    let (phase_emoji, phase_color) = phase_badge(phase);
    if !phase.is_empty() {
        spans.push(Span::styled(
            format!("{} {}", phase_emoji, phase),
            Style::default().fg(phase_color).add_modifier(Modifier::BOLD),
        ));
    }

    let p = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::ALL).title(" Finansal Durum "));

    f.render_widget(p, area);
}

/// Phase string'i için emoji + renk eşlemesi (UI rozeti).
pub fn phase_badge(phase: &str) -> (&'static str, Color) {
    match phase {
        "Booting"    => ("🔌", Color::DarkGray),
        "Scanning"   => ("🔭", Color::LightCyan),
        "Executing"  => ("⚔️",  Color::LightGreen),
        "Recovering" => ("🛡️",  Color::LightYellow),
        "Stopped"    => ("🛑", Color::Red),
        "Idle" | ""  => ("○",  Color::DarkGray),
        _            => ("•",  Color::White),
    }
}

/// Tüm sekmelerde ortak kullanılan Pozisyon Tablo Satırı
pub fn render_position_row(p: &PositionModel) -> Row<'static> {
    let pnl = p.calculate_pnl();
    // roe() artık core/math.rs üzerinden hassas hesaplanıyor
    let pnl_pct = p.roe(); 
    let color = if pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };

    Row::new(vec![
        Cell::from(p.symbol.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(p.trade_type.clone()),
        Cell::from(if p.is_long { "▲ LONG" } else { "▼ SHORT" }).style(Style::default().fg(color)),
        Cell::from(format!("{:.4}", p.entry_price)),
        Cell::from(format!("{:.4}", p.current_price)),
        Cell::from(format!("{:+.2} USDT", pnl)).style(Style::default().fg(color)),
        Cell::from(format!("{:+.1}%", pnl_pct)).style(Style::default().fg(color)),
    ]).height(1)
}

/// Pipeline (İş Akışı) Durum Göstergesi
/// Robotun o anki hayati fonksiyonlarını (Backtest, ML, Data) küçük rozetler olarak basar.
pub fn render_pipeline_status(area: Rect, f: &mut ratatui::Frame, steps: &[PipelineStep]) {
    let mut spans = vec![];
    
    for step in steps {
        let (icon, color) = match step.status.as_str() {
            "Ok"      => ("✅", Color::Green),
            "Running" => ("⟳", Color::Cyan),
            "Stale"   => ("⚠️", Color::Yellow),
            _         => ("🚨", Color::Red),
        };
        
        spans.push(ratatui::text::Span::styled(
            format!(" {} {} ", icon, step.label),
            Style::default().fg(color)
        ));
    }

    let p = Paragraph::new(ratatui::text::Line::from(spans))
        .block(Block::default().borders(Borders::ALL).title(" ⚙️ Sistem Pipeline "));
    
    f.render_widget(p, area);
}

/// Sermaye Verimlilik Çubuğu (Equity Gauge).
///
/// Yüzde stable sermayeye (gerçekleşmiş = starting + realize_pnl) göre hesaplanır
/// → açık pozisyonun mark-to-market'i gauge'u titretmez. Anlık NET değer zaten
/// header'da görünüyor; burada sadece "gerçekleşen portföy sağlığı" izlenir.
pub fn render_equity_gauge(area: Rect, f: &mut ratatui::Frame, snap: &FinanceSnapshot) {
    let stable_equity = snap.starting_capital + snap.realize_pnl;
    let ratio = if snap.starting_capital > 0.0 {
        (stable_equity / snap.starting_capital) * 100.0
    } else { 0.0 };
    let pct_clamped = ratio.clamp(0.0, 100.0) as u16;

    let gauge = Gauge::default()
        .block(Block::default().title(" Portföy Sağlığı (gerçekleşen) ").borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::LightBlue).bg(Color::DarkGray))
        .percent(pct_clamped)
        .label(format!("%{:.1}  ·  Başlangıç ${:.2}", ratio, snap.starting_capital));

    f.render_widget(gauge, area);
}
