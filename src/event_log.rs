use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::fs::OpenOptions;
use std::io::Write;
use crate::app_protocol::{BusEvent, BusEventKind, RunScope};

pub struct EventLog {
    tx: std::sync::mpsc::SyncSender<BusEvent>,
    subscribers: Arc<std::sync::Mutex<Vec<std::sync::mpsc::SyncSender<BusEvent>>>>,
    counter: Arc<AtomicU64>,
}

impl EventLog {
    pub fn new(log_path: PathBuf) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<BusEvent>(4096);
        let subscribers: Arc<std::sync::Mutex<Vec<std::sync::mpsc::SyncSender<BusEvent>>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscribers_writer = subscribers.clone();
        let counter = Arc::new(AtomicU64::new(0));

        std::thread::spawn(move || {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .map_err(|e| {
                    log::warn!("event_log: failed to open {:?}: {e}", log_path);
                    e
                })
                .ok();

            while let Ok(event) = rx.recv() {
                if let Some(ref mut f) = file {
                    if let Ok(line) = serde_json::to_string(&event) {
                        let _ = writeln!(f, "{}", line);
                        let _ = f.flush();
                    }
                }
                // Fan out to subscribers; remove dead ones.
                let mut subs = subscribers_writer.lock().unwrap();
                subs.retain(|sub| sub.try_send(event.clone()).is_ok());
            }
        });

        Self {
            tx,
            subscribers,
            counter,
        }
    }

    pub fn emit(&self, kind: BusEventKind) {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let event = BusEvent {
            id,
            ts,
            scope: RunScope::Global,
            kind,
            caller: None,
        };
        let _ = self.tx.try_send(event);
    }

    /// Subscribe to bus events. Returns a receiver that gets a copy of each event.
    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<BusEvent> {
        let (sub_tx, sub_rx) = std::sync::mpsc::sync_channel::<BusEvent>(256);
        if let Ok(mut subs) = self.subscribers.lock() {
            subs.push(sub_tx);
        }
        sub_rx
    }
}
