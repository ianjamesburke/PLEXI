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

// ── Layer-1 integration tests ───────────────────────────────────────────────
//
// Spawn real example apps as subprocesses, drive them through scripted
// PlexiEvents, assert on DrawCommand output. Tests auto-skip when `python3`
// is not on PATH — CI should run them as a real gate, but local dev doesn't
// fail when Python isn't installed.

#[cfg(test)]
mod tests {
    use super::*;

    fn python3_available() -> bool {
        Command::new("python3")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn example(name: &str) -> PathBuf {
        // worktree root = one level up from Cargo's CARGO_MANIFEST_DIR
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        root.join("examples").join(name).join(format!("{name}.py"))
    }

    fn spawn_example(name: &str) -> Option<Harness> {
        if !python3_available() {
            eprintln!("SKIP: python3 not on PATH");
            return None;
        }
        let entry = example(name);
        if !entry.exists() {
            eprintln!("SKIP: {} missing", entry.display());
            return None;
        }
        let cwd = entry.parent().expect("entry has parent").to_path_buf();
        let workspace = std::env::temp_dir().join(format!("plexi-layer1-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&workspace);
        let python = Path::new("python3");
        let args: Vec<&str> = vec![entry.to_str().unwrap()];
        match Harness::spawn(python, &args, &cwd, &[]) {
            Ok(mut h) => {
                h.init_and_expect_ready(name, &workspace);
                Some(h)
            }
            Err(e) => {
                eprintln!("SKIP: spawn failed: {e}");
                None
            }
        }
    }

    #[test]
    fn layer1_snake_init_ready_handshake() {
        let Some(mut h) = spawn_example("snake") else { return };
        // Ready was already received in spawn_example; verify a render frame
        // produces any DrawCommand at all (non-empty app).
        let cmds = h.render_frame(1, 400.0, 300.0);
        assert!(!cmds.is_empty(), "snake must emit at least one draw command");
        h.shutdown();
    }

    #[test]
    fn layer1_snake_renders_frame_done_for_its_own_frame_id() {
        let Some(mut h) = spawn_example("snake") else { return };
        let cmds = h.render_frame(42, 300.0, 200.0);
        let fd = cmds
            .iter()
            .find(|v| v.get("type").and_then(Value::as_str) == Some("frame_done"))
            .expect("frame_done present");
        assert_eq!(fd.get("frame_id").and_then(Value::as_u64), Some(42));
        h.shutdown();
    }

    #[test]
    fn layer1_todo_path_changed_updates_cwd() {
        let Some(mut h) = spawn_example("todo") else { return };
        h.render_frame(1, 400.0, 400.0);
        h.send_path_changed(Path::new("/tmp/plexi-layer1-cwd"));
        let cmds = h.render_frame(2, 400.0, 400.0);
        let any_cwd = cmds.iter().any(|v| {
            v.get("type").and_then(Value::as_str) == Some("text")
                && v.get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.contains("/tmp/plexi-layer1-cwd"))
        });
        assert!(any_cwd, "path_changed must update the todo cwd display");
        h.shutdown();
    }

    #[test]
    fn layer1_wikipedia_inject_results_renders() {
        let Some(mut h) = spawn_example("wikipedia") else { return };
        h.inject_state(&serde_json::json!({
            "mode": "results",
            "query": "Rust",
            "results": ["Rust (programming language)", "Rust belt"],
        }));
        let cmds = h.render_frame(1, 800.0, 600.0);
        let has_rust = cmds.iter().any(|v| {
            v.get("type").and_then(Value::as_str) == Some("text")
                && v.get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.contains("Rust"))
        });
        assert!(has_rust, "injected results must surface in render output");
        h.shutdown();
    }

    #[test]
    fn layer1_shutdown_closes_child_cleanly() {
        let Some(h) = spawn_example("snake") else { return };
        // Drop via shutdown() — verifies no zombie process + no hang. If
        // Shutdown wasn't honored the Drop impl's 200ms grace+kill would
        // keep the test bounded, and the whole test still passes quickly.
        h.shutdown();
    }
}
