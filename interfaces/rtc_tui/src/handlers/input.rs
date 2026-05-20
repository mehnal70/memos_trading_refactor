// interfaces/rtc_tui/src/handlers/input.rs - TUI Input Yöneticisi
//
// Snapshot çek → ekrana çiz → klavye olaylarını dinle → AppState'in
// `fleet.triggers` HashMap'ı üzerinden komutları sızdır.

use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::io;
use ratatui::{Terminal, backend::CrosstermBackend};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use memos_trading_core::core::bridge;
use memos_trading_core::core::commands::RobotCommand;
use memos_trading_core::robot::robotic_loop::AppState;
use crate::ui;

pub struct TuiManager {
    pub active_tab: usize,
    pub log_scroll: usize,
    /// Market Gözetimi sekmesinde (tab 4) seçili sembolün index'i. Up/Down ile değişir,
    /// market_fleet uzunluğuna her render'da clamp edilir.
    pub market_symbol_index: usize,
    pub settings_open: bool,
}

impl TuiManager {
    pub fn new() -> Self {
        // Başlangıç sekmesi: TUI_INITIAL_TAB env'i 0..=8 aralığında bir sayı olabilir.
        // Demo/Test'te belirli sekmeden açmak için kullanılır.
        let initial = std::env::var("TUI_INITIAL_TAB").ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map(|n| n.min(8))
            .unwrap_or(0);
        Self { active_tab: initial, log_scroll: 0, market_symbol_index: 0, settings_open: false }
    }

    pub async fn spawn_tui_loop(&mut self, state: Arc<Mutex<AppState>>) -> io::Result<()> {
        // Terminal'i TUI moduna al: raw mode + alternate screen → temiz, geri dönüşlü
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Hata olsa bile terminal'i temiz bırakacak iç çalıştırıcı
        let result = self.run_inner(&mut terminal, state).await;

        // Cleanup: hangi sonuçla bitilirse bitilsin terminal eski haline döner
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
        terminal.show_cursor().ok();
        result
    }

    async fn run_inner(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        state: Arc<Mutex<AppState>>,
    ) -> io::Result<()> {
        loop {
            // 1. SNAPSHOT AL (Kilit süresi minimumda)
            let snapshot = {
                let st_guard = state.lock().unwrap();
                if st_guard.app_stop_signal.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                bridge::get_snapshot(&st_guard)
            };

            // 2. ÇİZİM YAP (Kilit yok, tam performans)
            // Market Gözetimi seçim index'i her render öncesi fleet uzunluğuna clamp.
            let fleet_len = snapshot.market_fleet.len();
            if fleet_len > 0 && self.market_symbol_index >= fleet_len {
                self.market_symbol_index = fleet_len - 1;
            }
            terminal.draw(|f| {
                ui::render_main(f, &snapshot, self.active_tab, self.log_scroll, self.market_symbol_index);
            })?;

            // 3. INPUT İŞLEMLERİ (Event Poll)
            // 200ms poll: tuş gecikmesi insanca (göz fark etmez), snapshot+render
            // yenileme hızı 5 fps'e iner → sermaye/PnL alanlarında mark-to-market
            // dalgalanması artık göze "rakam altında belirip kayboluyor" flicker
            // üretmez. Daha agresif (50ms) snapshot+lock çevrim maliyetiyle gelirdi.
            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        // --- UI Navigasyon (Burada halledilir) ---
                        KeyCode::Char(c @ '1'..='9') => {
                            if let Some(digit) = c.to_digit(10) {
                                self.active_tab = (digit as usize).saturating_sub(1);
                            }
                        }
                        // Up/Down sekme-aware: Market Gözetimi (tab 4) iken sembol seçer;
                        // diğer sekmelerde log scroll'unu hareket ettirir.
                        KeyCode::Up => {
                            if self.active_tab == 4 {
                                self.market_symbol_index = self.market_symbol_index.saturating_sub(1);
                            } else {
                                self.log_scroll = self.log_scroll.saturating_add(1);
                            }
                        }
                        KeyCode::Down => {
                            if self.active_tab == 4 {
                                let n = snapshot.market_fleet.len();
                                if n > 0 {
                                    self.market_symbol_index =
                                        (self.market_symbol_index + 1).min(n - 1);
                                }
                            } else {
                                self.log_scroll = self.log_scroll.saturating_sub(1);
                            }
                        }
                        // Home: olay günlüğünde canlı kuyruğa hızlı dönüş (scroll=0).
                        KeyCode::Home => self.log_scroll = 0,
                        KeyCode::Esc  => self.settings_open = false,

                        // --- Operasyonel Komutlar (AppState'e iletilir) ---
                        k => {
                            let cmd = match k {
                                KeyCode::Char('m') | KeyCode::Char('M') => Some(RobotCommand::TriggerMl),
                                KeyCode::Char('b') | KeyCode::Char('B') => Some(RobotCommand::TriggerBacktest),
                                KeyCode::Char('s') | KeyCode::Char('S') => Some(RobotCommand::ToggleAutoMode),
                                KeyCode::Char('d') | KeyCode::Char('D') => Some(RobotCommand::StartDownload),
                                KeyCode::Char('q') | KeyCode::Char('Q') => Some(RobotCommand::GracefulShutdown),
                                _ => None,
                            };

                            if let Some(command) = cmd {
                                self.dispatch_command(command, &state);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Komutu yeni mimari'ye uygun olarak FleetCommand.triggers HashMap'ına sızdırır.
    /// Kullanıcı tuşu da guardian.log'a yazılır → TUI archives panelinde bağlam görünür.
    fn dispatch_command(&self, cmd: RobotCommand, state: &Arc<Mutex<AppState>>) {
        let mut st = state.lock().unwrap();
        match cmd {
            RobotCommand::TriggerMl => {
                if let Some(t) = st.fleet.triggers.get("ml") {
                    t.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                st.push_log("⌨️ Kullanıcı tuşu [m] ⇒ ml trigger gönderildi".into());
            }
            RobotCommand::TriggerBacktest => {
                if let Some(t) = st.fleet.triggers.get("backtest") {
                    t.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                st.push_log("⌨️ Kullanıcı tuşu [b] ⇒ backtest trigger gönderildi".into());
            }
            RobotCommand::StartDownload => {
                if let Some(t) = st.fleet.triggers.get("download") {
                    t.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                st.push_log("⌨️ Kullanıcı tuşu [d] ⇒ download trigger gönderildi".into());
            }
            RobotCommand::ToggleAutoMode => {
                // Otonom mod geçiş mantığı brain üzerinden yönetilir (ileride wire'lanır).
                st.push_log("⌨️ Kullanıcı tuşu [s] ⇒ otonom mod geçişi (henüz uygulanmadı)".into());
            }
            RobotCommand::GracefulShutdown => {
                st.app_stop_signal.store(true, std::sync::atomic::Ordering::Relaxed);
                st.push_log("⌨️ Kullanıcı tuşu [q] ⇒ graceful shutdown başlatıldı".into());
            }
            _ => {}
        }
    }
}
