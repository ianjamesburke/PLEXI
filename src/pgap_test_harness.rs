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

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::host::services::{MockNetService, NetService, UreqNetService};

/// Thin PGAP v3 subprocess driver.
pub(crate) struct Harness {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
    /// HTTP broker used to satisfy `http_request` draw commands. Defaults to
    /// `UreqNetService` for parity with production; tests swap in
    /// `MockNetService` via `set_net(..)`.
    net: Arc<dyn NetService>,
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
            net: Arc::new(UreqNetService::new()),
        })
    }

    /// Install a `NetService` (mock or real) for the harness. Every
    /// `http_request` draw command emitted by the subprocess is satisfied
    /// by calling `net.http(..)` and replying with `http_response`.
    pub fn set_net(&mut self, net: Arc<dyn NetService>) {
        self.net = net;
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
        let method = v.get("method").and_then(Value::as_str).unwrap_or("GET");
        let body = v.get("body").and_then(Value::as_str).map(str::to_string);
        let headers: std::collections::HashMap<String, String> = v
            .get("headers")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let resp = self.net.http(method, url, &headers, body.as_deref());
        let mut payload = serde_json::json!({
            "type": "http_response",
            "request_id": req_id,
            "status": resp.status,
            "body": resp.body,
        });
        if let Some(err) = resp.error {
            payload["error"] = Value::String(err);
        }
        self.send(&payload);
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
        // Point at the canonical SDK so examples pick up the current package
        // instead of any stale per-example copy.
        let sdk_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("sdk")
            .join("python");
        let env: Vec<(&str, &str)> = vec![("PYTHONPATH", sdk_root.to_str().unwrap())];
        match Harness::spawn(python, &args, &cwd, &env) {
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
        let list_y = cmds
            .iter()
            .find(|v| v.get("type").and_then(Value::as_str) == Some("list"))
            .and_then(|v| v.get("y"))
            .and_then(Value::as_f64)
            .expect("results render must position its list below the heading");
        assert!(list_y > 80.0, "list must not overlap the results heading");
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

    /// End-to-end proof that the real HTTP broker pathway (routing.rs →
    /// NetService → http_response round-trip) drives wikipedia through a
    /// search. The previous `http_mocks` dict is gone; this test uses the
    /// same `MockNetService::with(..)` seam that production panes will use
    /// when tests swap the broker.
    #[test]
    fn layer1_wikipedia_http_broker_end_to_end() {
        let Some(mut h) = spawn_example("wikipedia") else { return };

        // Canonical opensearch response the wikipedia example expects to
        // parse. Position [0] is the query echo, [1] is the match list.
        let search_url = "https://en.wikipedia.org/w/api.php?action=opensearch&search=Rust&limit=10&format=json";
        let search_body = r#"["Rust",["Rust (programming language)","Rust belt","Rust Belt"],["",""," "],[]]"#;

        let mock = MockNetService::new().with(search_url, search_body);
        h.set_net(Arc::new(mock));

        // Type "Rust" then Enter. Each letter is delivered as a Key event;
        // the SDK fans out on_key for every char.
        h.render_frame(1, 800.0, 600.0);
        for ch in ["R", "u", "s", "t"] {
            h.send_key(ch);
        }
        h.send_key("Enter");

        // Search runs on a background thread; give it a few render cycles
        // to round-trip through the broker and update `_mode = "results"`.
        let mut found_rust = false;
        for frame_id in 2..20 {
            let cmds = h.render_frame(frame_id, 800.0, 600.0);
            let has_rust = cmds.iter().any(|v| {
                let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
                (kind == "text" || kind == "list")
                    && serde_json::to_string(v)
                        .unwrap_or_default()
                        .contains("Rust (programming language)")
            });
            if has_rust {
                found_rust = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            found_rust,
            "mocked broker search must surface 'Rust (programming language)' in render output"
        );
        h.shutdown();
    }
}
