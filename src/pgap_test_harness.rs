//! PGAP v3 protocol test harness — spawns each example app as a subprocess,
//! drives it through a scripted `PlexiEvent` stream, and asserts on the
//! `DrawCommand` output.
//!
//! This is the v3.0 "CI gate" described in `STATE_OF_PLEXI.md` step #13.
//!
//! # How it works
//!
//! 1. `Harness::spawn(bin, args, env)` starts the subprocess with piped stdio.
//! 2. A background thread reads stdout lines and forwards them on an mpsc channel.
//! 3. `send(event)` writes one newline-delimited JSON line to stdin.
//! 4. `expect_ready(timeout)` drains lines until the first `type:"ready"`.
//! 5. `render_frame(frame_id, rect)` sends a Render and collects every command
//!    up to the matching `FrameDone`.
//! 6. `shutdown()` sends Shutdown and waits for the child to exit.
//!
//!
//! # Python requirement
//!
//! The example apps are Python 3. If `python3` is not resolvable on PATH, tests
//! skip with a logged warning instead of failing.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Thin PGAP v3 subprocess driver.
pub(crate) struct Harness {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
    /// Mock HTTP responses: url → body. Matched exactly against `http_request` url fields.
    http_mocks: HashMap<String, String>,
}

impl Harness {
    pub fn spawn(
        bin: &Path,
        args: &[&str],
        cwd: &Path,
        env: &[(&str, &str)],
    ) -> std::io::Result<Self> {
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let (tx, rx) = mpsc::channel::<String>();
        let tx_err = tx.clone();

        // Stdout forwarder.
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Stderr forwarder — prefix with "STDERR: " so tests can see Python
        // tracebacks when they fail. Logged via eprintln (captured by cargo).
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        eprintln!("STDERR: {l}");
                        // Route stderr into the same channel tagged so tests can
                        // match on it when asserting on errors.
                        let _ = tx_err.send(format!("__stderr__: {l}"));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            rx,
            http_mocks: HashMap::new(),
        })
    }

    /// Send a raw JSON value as one NDJSON line.
    pub fn send(&mut self, value: &Value) {
        let Some(stdin) = self.stdin.as_mut() else {
            panic!("stdin already closed");
        };
        let line = serde_json::to_string(value).expect("serialize event");
        stdin.write_all(line.as_bytes()).expect("write event");
        stdin.write_all(b"\n").expect("write newline");
        stdin.flush().expect("flush stdin");
    }

    /// Read one JSON line from the app, skipping stderr lines. Returns None on
    /// timeout.
    pub fn recv(&self, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match self.rx.recv_timeout(remaining) {
                Ok(line) => {
                    if line.starts_with("__stderr__") || line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Value>(&line) {
                        Ok(v) => return Some(v),
                        Err(_) => {
                            eprintln!("malformed app output: {line}");
                            continue;
                        }
                    }
                }
                Err(_) => return None,
            }
        }
    }

    /// Send `Init` and drain lines until a `Ready` JSON value is received.
    pub fn init_and_expect_ready(&mut self, app_id: &str, workspace_root: &Path) -> Value {
        self.send(&serde_json::json!({
            "type": "init",
            "protocol": "pgap/3",
            "app_id": app_id,
            "workspace_root": workspace_root,
            "capabilities": [],
            "feature_flags": ["media_v1", "pane_groups_v1"],
        }));

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match self.recv(Duration::from_secs(5)) {
                Some(v) if v.get("type").and_then(Value::as_str) == Some("ready") => return v,
                Some(_) => continue,
                None => break,
            }
        }
        panic!("did not receive Ready within timeout");
    }

    fn reply_http_request(&mut self, v: &Value) {
        let req_id = v.get("request_id").and_then(Value::as_str).unwrap_or("");
        let url = v.get("url").and_then(Value::as_str).unwrap_or("");
        let resp = if let Some(body) = self.http_mocks.get(url) {
            serde_json::json!({"type":"http_response","request_id":req_id,"status":200,"body":body})
        } else {
            serde_json::json!({"type":"http_response","request_id":req_id,"status":404,"body":"","error":format!("no mock for {url}")})
        };
        self.send(&resp);
    }

    fn pre_drain_http(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(line) => {
                    if line.starts_with("__stderr__") || line.trim().is_empty() { continue; }
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        if v.get("type").and_then(Value::as_str) == Some("http_request") {
                            let vc = v.clone();
                            self.reply_http_request(&vc);
                            thread::sleep(Duration::from_millis(20));
                        }
                    }
                }
                Err(_) => break,
            }
        }
    }

    /// Render one frame at the given rect and collect every command up to and
    /// including the matching `FrameDone`. Returns the draw commands in order.
    /// Pre-drains buffered `http_request`s before sending render to avoid races
    /// with background threads that call `emit.http_get()`.
    pub fn render_frame(&mut self, frame_id: u64, w: f32, h: f32) -> Vec<Value> {
        self.pre_drain_http();

        self.send(&serde_json::json!({
            "type": "render",
            "frame_id": frame_id,
            "rect": { "x": 0.0, "y": 0.0, "w": w, "h": h },
        }));

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut cmds = Vec::new();
        while Instant::now() < deadline {
            let Some(v) = self.recv(Duration::from_secs(5)) else {
                break;
            };
            let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
            if kind == "http_request" {
                let vc = v.clone();
                self.reply_http_request(&vc);
                continue;
            }
            cmds.push(v.clone());
            if kind == "frame_done" {
                let got = v.get("frame_id").and_then(Value::as_u64).unwrap_or(0);
                assert_eq!(got, frame_id, "frame_done frame_id mismatch");
                return cmds;
            }
        }
        panic!(
            "did not receive FrameDone({frame_id}) within timeout; got {} cmds",
            cmds.len()
        );
    }

    pub fn inject_state(&mut self, payload: &Value) {
        self.send(&serde_json::json!({"type":"inject_state","payload":payload}));
        thread::sleep(Duration::from_millis(30));
    }

    pub fn mock_http(&mut self, url: &str, body: &str) {
        self.http_mocks.insert(url.to_string(), body.to_string());
    }

    pub fn render_to_png(&mut self, frame_id: u64, w: f32, h: f32) -> Vec<u8> {
        let cmds = self.render_frame(frame_id, w, h);
        crate::headless_renderer::HeadlessRenderer::new()
            .render_pgap_frame(&cmds, w as u32, h as u32)
    }

    /// Send a Key event.
    pub fn send_key(&mut self, key: &str) {
        self.send(&serde_json::json!({
            "type": "key",
            "key": key,
            "modifiers": {"shift": false, "ctrl": false, "alt": false, "cmd": false},
        }));
    }

    /// Send a PathChanged event.
    pub fn send_path_changed(&mut self, cwd: &Path) {
        self.send(&serde_json::json!({
            "type": "path_changed",
            "cwd": cwd,
        }));
    }

    pub fn shutdown(mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = writeln!(stdin, "{}", serde_json::json!({ "type": "shutdown" }));
            let _ = stdin.flush();
        }
        if let Some(mut child) = self.child.take() {
            // Give the app a second to exit cleanly on Shutdown.
            thread::sleep(Duration::from_millis(200));
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
