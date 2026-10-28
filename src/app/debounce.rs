//! Shared trailing debounce loop for filesystem watcher notifications.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Coalesce events by key until each has been quiet for `delay`.
/// Stop when cancelled, the input disconnects, or delivery fails.
pub(crate) fn run<K: Clone + Eq + Hash>(
    rx: Receiver<K>,
    cancel: Arc<Mutex<bool>>,
    delay: Duration,
    mut deliver: impl FnMut(K) -> bool,
) {
    let mut pending = HashMap::new();
    loop {
        if cancel.lock().map(|guard| *guard).unwrap_or(true) {
            return;
        }
        match rx.try_recv() {
            Ok(key) => {
                pending.insert(key, Instant::now());
            }
            Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {
                let due: Vec<K> = pending
                    .iter()
                    .filter(|(_, timestamp)| timestamp.elapsed() >= delay)
                    .map(|(key, _)| key.clone())
                    .collect();
                for key in due {
                    pending.remove(&key);
                    if !deliver(key) {
                        return;
                    }
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn coalesces_each_key_independently_and_stops_on_drop() {
        let (input, rx) = mpsc::channel();
        let (output, result) = mpsc::channel();
        let cancel = Arc::new(Mutex::new(false));
        let worker = thread::spawn(move || {
            run(rx, cancel, Duration::from_millis(30), |key| {
                output.send(key).is_ok()
            });
        });
        input.send(1).unwrap();
        input.send(1).unwrap();
        input.send(2).unwrap();
        let mut got = vec![
            result.recv_timeout(Duration::from_secs(3)).unwrap(),
            result.recv_timeout(Duration::from_secs(3)).unwrap(),
        ];
        got.sort();
        assert_eq!(got, [1, 2]);
        assert!(result.recv_timeout(Duration::from_millis(120)).is_err());
        drop(input);
        worker.join().unwrap();
    }
}
