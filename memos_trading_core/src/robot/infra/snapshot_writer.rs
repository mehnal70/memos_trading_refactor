// src/robot/infra/snapshot_writer.rs
//
// Ortak Çıktı Katmanı (TUI / Headless / Android):
// AppState'in tüm bakanlık verisi `bridge::get_snapshot` ile MissionControl JSON'una
// dönüştürülür ve periyodik olarak disk'e atomik şekilde yazılır.
//
// Tüketici tarafı:
// - TUI bu dosyayı okumaz (kendi belleğinden çizer) ama dosya hala yazılır → Android & web
//   istemcileri ile **tek gerçek kaynak** sağlanır.
// - Headless mod: TUI olmasa bile dış görüntüleyiciler bu dosyadan beslenir.
// - Android: read-only bu JSON'ı poll eder.
//
// Atomicity: writer.rs::seal_config_to_disk geçici dosya + rename mantığı kullanır,
// kısmi yazma kaynaklı bozulma yaşanmaz.

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::robot::robotic_loop::AppState;
use crate::core::bridge;
use crate::persistence::writer::seal_config_to_disk;

/// Periyodik MissionControl snapshot yazıcısı.
/// `interval_secs` = 1 önerilir (Android polling ile uyumlu).
pub fn spawn_snapshot_writer(
    state: Arc<Mutex<AppState>>,
    path: String,
    interval_secs: u64,
) {
    tokio::spawn(async move {
        let interval = Duration::from_secs(interval_secs.max(1));
        let mut tick: u64 = 0;
        loop {
            // Çıkış kontrolü + snapshot üretimi tek kilit altında
            let snap_opt = {
                let st = match state.lock() {
                    Ok(g) => g,
                    Err(_) => break,
                };
                if st.app_stop_signal.load(Ordering::Relaxed) {
                    None
                } else {
                    Some(bridge::get_snapshot(&st))
                }
            };
            let snap = match snap_opt {
                Some(s) => s,
                None => break,
            };

            // Atomik mühürleme (.tmp → rename)
            if let Err(e) = seal_config_to_disk(&path, &snap) {
                // İlk hata kullanıcı görünür log'a düşsün, sonraki spam'leri sustur
                if tick == 0 {
                    log::warn!("snapshot_writer: {} dosyası yazılamadı: {:?}", path, e);
                    if let Ok(mut st) = state.lock() {
                        st.push_log(format!(
                            "⚠️ snapshot_writer: {} yazılamadı ({:?})",
                            path, e
                        ));
                    }
                }
            } else if tick == 0 {
                if let Ok(mut st) = state.lock() {
                    st.push_log(format!(
                        "📤 Snapshot yazıcı aktif: {} (her {}s)",
                        path, interval_secs
                    ));
                }
            }

            tick = tick.wrapping_add(1);
            tokio::time::sleep(interval).await;
        }
    });
}
