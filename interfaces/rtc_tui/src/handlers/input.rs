// interfaces/rtc_tui/src/handlers/input.rs - TUI Input Yöneticisi
//
// Snapshot çek → ekrana çiz → klavye olaylarını dinle → AppState'in
// `fleet.triggers` HashMap'ı üzerinden komutları sızdır.

use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::io;
use ratatui::{Terminal, backend::CrosstermBackend};
use crossterm::event::{self, Event, KeyCode};

use memos_trading_core::core::bridge;
use memos_trading_core::core::commands::RobotCommand;
use memos_trading_core::robot::robotic_loop::AppState;
use crate::ui;

pub struct TuiManager {
    pub active_tab: usize,
    pub log_scroll: usize,
    pub settings_open: bool,
}

impl TuiManager {
    pub fn new() -> Self {
        Self { active_tab: 0, log_scroll: 0, settings_open: false }
    }

    pub async fn spawn_tui_loop(&mut self, state: Arc<Mutex<AppState>>) -> io::Result<()> {
        let stdout = io::stdout();
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

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
            terminal.draw(|f| {
                ui::render_main(f, &snapshot, self.active_tab, self.log_scroll);
            })?;

            // 3. INPUT İŞLEMLERİ (Event Poll)
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        // --- UI Navigasyon (Burada halledilir) ---
                        KeyCode::Char(c @ '1'..='9') => {
                            if let Some(digit) = c.to_digit(10) {
                                self.active_tab = (digit as usize).saturating_sub(1);
                            }
                        }
                        KeyCode::Up   => self.log_scroll = self.log_scroll.saturating_add(1),
                        KeyCode::Down => self.log_scroll = self.log_scroll.saturating_sub(1),
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
    fn dispatch_command(&self, cmd: RobotCommand, state: &Arc<Mutex<AppState>>) {
        let st = state.lock().unwrap();
        match cmd {
            RobotCommand::TriggerMl => {
                if let Some(t) = st.fleet.triggers.get("ml") {
                    t.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            RobotCommand::TriggerBacktest => {
                if let Some(t) = st.fleet.triggers.get("backtest") {
                    t.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            RobotCommand::StartDownload => {
                if let Some(t) = st.fleet.triggers.get("download") {
                    t.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            RobotCommand::ToggleAutoMode => {
                // Otonom mod geçiş mantığı brain üzerinden yönetilir (ileride wire'lanır).
            }
            RobotCommand::GracefulShutdown => {
                st.app_stop_signal.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            _ => {}
        }
    }
}
