// interfaces/rtc_tui/src/ui/mod.rs

pub mod components;
pub mod dashboard;
pub mod ai_center;
pub mod market_watch;
pub mod archives;
pub mod charts;
pub mod pipeline;
pub mod special_trades;

// DİKKAT: Artık modellerimizi kütüphane üzerinden çağırıyoruz
use memos_trading_core::core::model::MissionControl;
use ratatui::Frame;

/// Srivastava ATP - Tüm TUI sekmelerini yöneten ana orkestratör
pub fn render_main(f: &mut Frame, snap: &MissionControl, active_tab: usize, log_scroll: usize) {
    let area = f.size();
    
    match active_tab {
        0 => dashboard::draw(f, area, snap),
        1 => ai_center::draw(f, area, snap),
        2 => archives::draw_logs(f, area, snap, log_scroll),
        3 => archives::draw_trade_history(f, area, snap),
        4 => market_watch::draw(f, area, snap),
        // 5 => htf_watch::draw(f, area, snap), // İleride eklenecek HTF modülü
        6 => charts::draw(f, area, snap),
        7 => pipeline::draw(f, area, snap),
        8 => special_trades::draw(f, area, snap),
        // Fallback: Tanımsız bir sekme istenirse güvenli liman olan Dashboard'a dön
        _ => dashboard::draw(f, area, snap),
    }
}
