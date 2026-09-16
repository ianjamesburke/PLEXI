use crate::app::ui_mailbox::{EguiWake, UiMailbox};
use crate::host::event_subscriptions::ConsentChoice;
use crate::testing::HostHarness;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

struct Connection {
    socket: UnixStream,
    lines: mpsc::Receiver<String>,
    server: Option<std::thread::JoinHandle<()>>,
}

impl Connection {
    fn open(h: &mut HostHarness, request: Value) -> Self {
        let wake = Arc::new(EguiWake::new(h.app.ctx.clone()));
        let (subscribe, rx) = UiMailbox::channel(wake.clone(), "consumer-test");
        h.app.event_subscribe_rx = rx;
        let (publish, rx) = UiMailbox::channel(wake, "consumer-publish-test");
        h.app.event_publish_rx = rx;
        let ipc = h.ipc_tx.clone();
        let (mut socket, server) = UnixStream::pair().unwrap();
        let thread = std::thread::spawn(move || {
            crate::app::handle_socket_connection(server, ipc, subscribe, publish, None);
        });
        writeln!(socket, "{request}").unwrap();
        let input = socket.try_clone().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(input).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            socket,
            lines,
            server: Some(thread),
        }
    }

    fn next(&self, h: &mut HostHarness) -> Value {
        let until = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(5));
        loop {
            h.hidden_frame();
            while let Some(consent) = h.app.pending_event_consents.pop_front() {
                h.app.host_subscriptions.resolve_consent(
                    consent,
                    ConsentChoice::AllowOnce,
                    &crate::config::config_dir(),
                );
            }
            if let Ok(line) = self.lines.try_recv() {
                return serde_json::from_str(&line).unwrap();
            }
            assert!(
                Instant::now() < until,
                "host did not return a consumer record"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        if let Some(thread) = self.server.take() {
            thread.join().unwrap();
        }
    }
}

fn report(h: &mut HostHarness, pane: u64, state: &str, event: &str) {
    h.inject_ipc(
        serde_json::from_value(json!({
            "type":"set_agent_state", "pane_id":pane, "agent":"test-provider",
            "state":state, "event":event,
        }))
        .unwrap(),
    );
    h.hidden_frame();
}

#[test]
fn pane_consumer_wait_matches_current_idle_while_hidden() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    report(&mut h, pane, "idle", "Stop");
    let connection = Connection::open(
        &mut h,
        json!({
            "type":"pane_lifecycle_wait", "pane_id":pane, "until":"idle",
            "timeout":1.0, "from_pane_id":pane,
        }),
    );
    let reply = connection.next(&mut h);
    assert_eq!(
        reply["type"], "event",
        "wait must return the matched event: {reply}"
    );
    assert_eq!(reply["payload"]["kind"], "agent_idle");
    assert_eq!(reply["payload"]["provenance"]["raw_event"], "Stop");
}

#[test]
fn pane_consumer_cli_surfaces_parse() {
    use clap::Parser;
    for args in [
        vec![
            "plexi",
            "pane",
            "wait",
            "7",
            "--until",
            "idle",
            "--timeout",
            "1",
        ],
        vec!["plexi", "pane", "events", "--follow", "--pane", "7"],
    ] {
        assert!(
            crate::cli::args::Cli::try_parse_from(&args).is_ok(),
            "missing CLI surface: {args:?}"
        );
    }
}
