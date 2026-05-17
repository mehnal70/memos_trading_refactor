// distributed_worker.rs - Dağıtık Worker Pool ve Otonom Event İzleme

use tokio::sync::{mpsc, broadcast};
use tokio::task;
use chrono::Utc;
use std::sync::Arc;

/// İş tipi - Bellek dostu enum (String kopyalamayı azaltmak için Arc/Box kullanılabilir)
#[derive(Debug, Clone)]
pub enum WorkerTask {
    FetchData { symbol: String },
    GenerateSignal { symbol: String },
    PlaceOrder { symbol: String },
    ReportSummary,
}

/// Event tipi - Modern ve kategorize edilmiş bildirimler
#[derive(Debug, Clone)]
pub enum SystemEvent {
    Info(String),
    Warning(String),
    Error(String),
    Critical(String),
}

pub struct WorkerPool {
    pub task_sender: mpsc::Sender<WorkerTask>,
    pub event_sender: mpsc::Sender<SystemEvent>,
}

impl WorkerPool {
    pub fn new(worker_count: usize) -> Self {
        // Task kanalı: Buffer boyutu performans için 1024'e çıkarıldı
        let (task_tx, task_rx) = mpsc::channel::<WorkerTask>(1024);
        let (event_tx, mut event_rx) = mpsc::channel::<SystemEvent>(1024);
        
        // Alıcıyı (rx) paylaşılan bir yapıya sarıyoruz
        let shared_rx = Arc::new(tokio::sync::Mutex::new(task_rx));
        let event_tx_clone = event_tx.clone();

        // 1. WORKER'LARI BAŞLAT (Pool Construction)
        for id in 0..worker_count {
            let rx = Arc::clone(&shared_rx);
            let tx = event_tx.clone();

            task::spawn(async move {
                loop {
                    // Kilidi al ve bir görev bekle (Fair scheduling)
                    let task = {
                        let mut lock = rx.lock().await;
                        lock.recv().await
                    };

                    match task {
                        Some(t) => {
                            let now = Utc::now().format("%H:%M:%S");
                            let msg_prefix = format!("[{}][Worker {}]", now, id);

                            match t {
                                WorkerTask::FetchData { symbol } => {
                                    let _ = tx.send(SystemEvent::Info(format!("{} Veri çekiliyor: {}", msg_prefix, symbol))).await;
                                }
                                WorkerTask::GenerateSignal { symbol } => {
                                    let _ = tx.send(SystemEvent::Info(format!("{} Sinyal üretiliyor: {}", msg_prefix, symbol))).await;
                                }
                                WorkerTask::PlaceOrder { symbol } => {
                                    let _ = tx.send(SystemEvent::Info(format!("{} Emir iletiliyor: {}", msg_prefix, symbol))).await;
                                }
                                WorkerTask::ReportSummary => {
                                    let _ = tx.send(SystemEvent::Info(format!("{} Özet rapor hazırlanıyor", msg_prefix))).await;
                                }
                            }
                        }
                        None => break, // Kanal kapandıysa worker'ı bitir
                    }
                }
            });
        }

        // 2. EVENT DINLEYICI (Centralized Monitoring)
        task::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                match event {
                    SystemEvent::Info(msg) => println!("[INFO] {}", msg),
                    SystemEvent::Warning(msg) => eprintln!("[WARN] {}", msg),
                    SystemEvent::Error(msg) => eprintln!("[ERROR] {}", msg),
                    SystemEvent::Critical(msg) => {
                        eprintln!("[🚨 CRITICAL] {}", msg);
                        // Burada otonom aksiyonlar tetiklenebilir (e.g., kill switch)
                    }
                }
            }
        });

        Self { 
            task_sender: task_tx, 
            event_sender: event_tx_clone 
        }
    }
}
