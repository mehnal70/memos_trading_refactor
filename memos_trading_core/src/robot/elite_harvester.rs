// src/robot/elite_harvester.rs — Otonom Kalburüstü Sembol Hasat ve Altyapı Enjektör Motoru.
use crate::core::types::{Exchange, Market};
use crate::robot::robotic_loop::AppState;
use crate::robot::venue::adapter::ExchangeCapability;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub struct EliteSymbolHarvester;

impl EliteSymbolHarvester {
    /// 🌾 ELİT HASAT MOTORU: Aktif borsaların hacim ve gürültü kabiliyetlerini tarayarak,
    /// en kalburüstü sembolleri otonom keşfeder ve sistem konfigürasyon omurgasına işler.
    pub async fn execute_harvest_and_inject(state: &Arc<Mutex<AppState>>) -> Result<(), String> {
        let (venue_registry, db_path, current_interval, profile) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            (
                Arc::clone(&st.venue_registry),
                st.config.db_path.clone(),
                st.config.interval.clone(),
                st.config.venue_profile.as_str().to_string(),
            )
        };

        let mut final_elite_symbols: HashSet<String> = HashSet::new();

        // 1. Adım: Sistemde kayıtlı aktif borsaları otonom geziyoruz
        for exchange in venue_registry.exchanges() {
            let Some(venue) = venue_registry.get(exchange) else {
                continue;
            };
            let caps = venue.capabilities();

            // 2. Adım: Borsanın piyasa tipine göre otonom kalburüstü kriteri belirliyoruz
            if caps.contains(&ExchangeCapability::SwapRates) {
                // FOREX/CFD (MT5) RELEFKSİ: En likit ve makas aralığı (spread) en kararlı majör pariteler
                let forex_majors = vec![
                    "EURUSD@mt5".to_string(),
                    "GBPUSD@mt5".to_string(),
                    "USDJPY@mt5".to_string(),
                    "XAUUSD@mt5".to_string(),
                    "AUDUSD@mt5".to_string(),
                    "USDCAD@mt5".to_string(),
                ];
                for bare_sym in &forex_majors {
                    let dynamic_tag = format!("{}@{}", bare_sym, venue.exchange().as_str());
                    final_elite_symbols.insert(dynamic_tag);

                    // 🔌 🛠️ OTONOM BOOTSTRAP ÇEKİRDEĞİ: Operatörün komut koşturmasına gerek kalmadan
                    // veritabanındaki son kayıt zamanını yoklar. Eğer kayıt yoksa barda tohumlama yapar.
                    let last_ts = crate::persistence::reader::last_candle_ts_for_profile(
                        &db_path,
                        bare_sym,
                        &current_interval,
                        &"mt5".to_string(),
                        &profile,
                    );
                    if last_ts.is_none() {
                        log::info!("🔌 [AUTO-BOOTSTRAP] {} için yerel veri yok. Otonom geçmiş tamamlama başlatılıyor...", bare_sym);

                        let venue_clone = Arc::clone(&venue);
                        let sym_clone = bare_sym.clone();
                        let iv_clone = current_interval.clone();
                        let db_path_clone = db_path.clone();
                        let profile_clone = profile.clone();

                        // Hot-path döngüsünü kilitlememek için indirme ve yazma işini arka plana asenkron spawn ediyoruz
                        tokio::spawn(async move {
                            // Borsadan son 1000 mumu otonom talep et
                            if let Ok(candles) =
                                venue_clone.fetch_candles(&sym_clone, &iv_clone, 1000).await
                            {
                                if !candles.is_empty() {
                                    let n_written =
                                        tokio::task::spawn_blocking(move || -> usize {
                                            let Ok(conn) =
                                                crate::persistence::open_db(&db_path_clone)
                                            else {
                                                return 0;
                                            };
                                            let _ = conn
                                                .busy_timeout(std::time::Duration::from_secs(5));
                                            let mut w = 0usize;
                                            for c in &candles {
                                                // bootstrap bacağı standart 'mt5' namespace'ine mühürlenir
                                                if crate::persistence::writer::save_candle_for_profile(
                                                    &conn, &profile_clone, "mt5", "mt5", c,
                                                )
                                                .is_ok()
                                                {
                                                    w += 1;
                                                }
                                            }
                                            return w;
                                        })
                                        .await
                                        .unwrap_or(0);

                                    log::info!("🔌 [AUTO-BOOTSTRAP] ✅ {} için {} adet geçmiş mum otonom başarıyla yazıldı.", sym_clone, n_written);
                                }
                            }
                        });
                    }
                }
            }
            // for sym in forex_majors { final_elite_symbols.insert(sym); }

            // elite_harvester.rs içindeki Level2OrderBook (Kripto) bloğunu şu şekilde güncelleyin:
            if caps.contains(&ExchangeCapability::Level2OrderBook) {
                if let Ok(db_symbols) = crate::persistence::reader::list_symbols_for_market(
                    &db_path,
                    Some("futures"),
                    Some(&current_interval),
                ) {
                    // OTONOM GENİŞLETME: Kısıtlayıcı 8 koin barajı kaldırıldı.
                    // Operatör env tanımı yapmadıysa varsayılan tavan 40 seçkin koine esnetildi.
                    let max_crypto: usize = std::env::var("HARVEST_MAX_CRYPTO_COUNT")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(40);

                    let mut crypto_count = 0;
                    for sym in db_symbols {
                        if crypto_count >= max_crypto {
                            break;
                        }
                        if !sym.contains('@') {
                            let _ =
                                final_elite_symbols.insert(format!("{}@binance", sym).to_string());
                            crypto_count += 1;
                        }
                    }
                }
            }

            // ─── 🔌 OTONOM MT5 BOOTSTRAP ENTEGRASYON HALKASI ───────────────────
            // Manuel olarak `./memos mt5-bootstrap` ile beslenen barları borsa kabiliyetlerine
            // göre otonom keşfediyor ve hiçbir konfigürasyon dosyasına bakmadan ana döngüye bağlıyor.
            // ─── 🔌 OTONOM MT5 BOOTSTRAP ENTEGRASYON HALKASI (KESİN ÇÖZÜM) ───────
            if caps.contains(&ExchangeCapability::SwapRates) {
                if let Ok(db_mt5_symbols) = crate::persistence::reader::list_symbols_for_market(
                    &db_path,
                    Some("mt5"),
                    Some(&current_interval),
                ) {
                    for sym in db_mt5_symbols {
                        // ANOMALİ ÇÖZÜMÜ: Eğer sembolün sonunda borsa etiketi yoksa ("EURUSD"),
                        // sonuna açıkça "@mt5" mühürünü ekliyoruz. Sonundaki sahte "T" (EURUSDT) ekini temizliyoruz.
                        let clean_sym = sym.trim_end_matches('T').to_string();
                        let dynamic_tag = if clean_sym.contains('@') {
                            clean_sym.clone()
                        } else {
                            format!("{}@mt5", clean_sym).to_string()
                        };
                        final_elite_symbols.insert(dynamic_tag);
                    }
                }
            }
            // ─────────────────────────────────────────────────────────────────────────────────
        }

        if final_elite_symbols.is_empty() {
            return Ok(());
        }

        let elite_vec: Vec<String> = final_elite_symbols.into_iter().collect();

        // 3. Adım: Keşfedilen kalburüstü sembolleri anlık olarak altyapı katmanına enjekte ediyoruz
        let (orchestrator, current_market) = match state.lock() {
            Ok(mut st) => {
                st.config.pinned_symbols = elite_vec.clone();
                (
                    Arc::clone(&st.fleet.symbol_orchestrator),
                    st.config.market.clone(),
                )
            }
            Err(e) => return Err(format!("state lock: {e}")),
        };

        // State kilidini bırakmadan önce yalnızca kısa bellek güncellemesini tamamla.
        // Orkestratör ve disk I/O ayrı tutulur; böylece lock sırası ve bekleme zinciri
        // pipeline'ın geri kalanına yayılmaz.
        if let Ok(mut orch) = orchestrator.write() {
            for sym in &elite_vec {
                orch.register(sym, &current_market, &current_interval);
            }
        }

        if let Err(e) = Self::flush_to_launch_conf(&elite_vec) {
            log::warn!(
                "⚠️ Hasat edilen semboller .launch.conf dosyasına yazılırken hata: {}",
                e
            );
        }

        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🌾 [HARVEST] Otonom Elit Sembol Keşfi Tamamlandı! {} Kalburüstü Sembol Enjekte Edildi: {}",
                elite_vec.len(), elite_vec.join(", ")
            ));
        }

        return Ok(());
    }
    /// Keşfedilen kalburüstü sembolleri scripts/.launch.conf dosyasına dinamik işler,
    /// böylece bot restart olsa bile otonom kazanım kalıcı olarak korunur.
    fn flush_to_launch_conf(symbols: &[String]) -> std::io::Result<()> {
        let launch_path = "scripts/.launch.conf";
        let content = match std::fs::read_to_string(launch_path) {
            Ok(c) => c,
            Err(_) => return Ok(()), // Dosya yoksa sessiz geç (sandbox/test koruması)
        };

        let csv_symbols = symbols.join(",");
        let mut lines: Vec<String> = Vec::new();
        let mut pinned_exist = false;

        for line in content.lines() {
            if line.starts_with("PINNED_SYMBOLS=") {
                lines.push(format!("PINNED_SYMBOLS={}", csv_symbols));
                pinned_exist = true;
            } else {
                lines.push(line.to_string());
            }
        }

        if !pinned_exist {
            lines.push(format!("PINNED_SYMBOLS={}", csv_symbols));
        }

        std::fs::write(launch_path, lines.join("\n") + "\n")?;
        return Ok(());
    }
}
