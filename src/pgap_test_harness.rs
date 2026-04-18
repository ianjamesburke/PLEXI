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
//! # Mock media
//!
//! Tests set `PLEXI_AUDIO=mock://...` and `PLEXI_VIDEO=mock://...` on the child
//! env so the media subsystem inside the Plexi host (if exercised) uses the
//! mock devices. The example apps themselves don't decode media — the host does
//! — but the env is propagated so any future app that does open media stays
//! deterministic.
//!
//! # Python requirement
//!
//! The example apps are Python 3. If `python3` is not resolvable on PATH, tests
//! skip with a logged warning instead of failing.

#![cfg(test)]

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

        Ok(Self { child: Some(child), stdin: Some(stdin), rx })
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

    /// Render one frame at the given rect and collect every command up to and
    /// including the matching `FrameDone`. Returns the draw commands in order.
    pub fn render_frame(&mut self, frame_id: u64, w: f32, h: f32) -> Vec<Value> {
        self.send(&serde_json::json!({
            "type": "render",
            "frame_id": frame_id,
            "rect": { "x": 0.0, "y": 0.0, "w": w, "h": h },
        }));

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut cmds = Vec::new();
        while Instant::now() < deadline {
            let Some(v) = self.recv(Duration::from_secs(5)) else { break };
            let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
            cmds.push(v.clone());
            if kind == "frame_done" {
                let got = v.get("frame_id").and_then(Value::as_u64).unwrap_or(0);
                assert_eq!(got, frame_id, "frame_done frame_id mismatch");
                return cmds;
            }
        }
        panic!("did not receive FrameDone({frame_id}) within timeout; got {} cmds", cmds.len());
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

// ── Helpers ──────────────────────────────────────────────────────────────────

fn python_available() -> bool {
    Command::new("python3").arg("--version").output().is_ok()
}

fn examples_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("examples")
}

fn mock_env() -> HashMap<String, String> {
    // Use absolute paths inside the OS temp dir so the child's CWD doesn't
    // matter and cleanup is automatic.
    let tmp = std::env::temp_dir();
    let in_wav = tmp.join("plexi-harness-in.wav").display().to_string();
    let out_wav = tmp.join("plexi-harness-out.wav").display().to_string();
    let mut m = HashMap::new();
    m.insert("PLEXI_AUDIO".to_string(), format!("mock://{in_wav},{out_wav}"));
    m.insert(
        "PLEXI_VIDEO".to_string(),
        "mock://fixture.mp4?duration=5000&w=320&h=180".to_string(),
    );
    m
}

fn python_harness(
    app_dir: &Path,
    entry: &str,
) -> Option<Harness> {
    if !python_available() {
        eprintln!("SKIP: python3 not on PATH");
        return None;
    }
    let bin = PathBuf::from("python3");
    let entry_path = app_dir.join(entry);
    let entry_str = entry_path.display().to_string();
    let args = [entry_str.as_str()];
    let env_map = mock_env();
    let env: Vec<(&str, &str)> = env_map.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    match Harness::spawn(&bin, &args, app_dir, &env) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("SKIP: failed to spawn {}: {e}", entry_path.display());
            None
        }
    }
}

fn extract_types(cmds: &[Value]) -> Vec<String> {
    cmds.iter()
        .filter_map(|v| v.get("type").and_then(Value::as_str).map(str::to_string))
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn snake_handshake_and_frame() {
    let Some(mut h) = python_harness(&examples_dir().join("snake"), "snake.py") else {
        return;
    };
    let workspace_root = std::env::temp_dir();
    let ready = h.init_and_expect_ready("snake", &workspace_root);
    assert_eq!(ready["type"], "ready");
    assert!(ready["sdk"].as_str().unwrap_or("").starts_with("plexi-sdk-py/"));

    let cmds = h.render_frame(1, 640.0, 480.0);
    let types = extract_types(&cmds);
    assert!(types.contains(&"rect".to_string()), "snake should draw at least one rect, got: {types:?}");
    assert_eq!(types.last().map(String::as_str), Some("frame_done"));
    h.shutdown();
}

#[test]
fn todo_handshake_renders_cwd_in_header() {
    let Some(mut h) = python_harness(&examples_dir().join("todo"), "todo.py") else {
        return;
    };
    let workspace_root = std::env::temp_dir();
    h.init_and_expect_ready("todo", &workspace_root);

    let cmds = h.render_frame(1, 800.0, 600.0);
    let has_todo_header = cmds.iter().any(|v| {
        v.get("type").and_then(Value::as_str) == Some("text")
            && v.get("text").and_then(Value::as_str) == Some("Todo")
    });
    assert!(has_todo_header, "todo app should draw its 'Todo' header text");
    h.shutdown();
}

#[test]
fn todo_reacts_to_path_changed() {
    let Some(mut h) = python_harness(&examples_dir().join("todo"), "todo.py") else {
        return;
    };
    let workspace_root = std::env::temp_dir();
    h.init_and_expect_ready("todo", &workspace_root);

    // Baseline render — header should contain the workspace_root path.
    let first = h.render_frame(1, 800.0, 600.0);
    let baseline_cwd_text = first.iter().find_map(|v| {
        if v.get("type").and_then(Value::as_str) != Some("text") {
            return None;
        }
        // Second text cmd in the header is the cwd display (monospace=true).
        if v.get("monospace").and_then(Value::as_bool) == Some(true)
            && v.get("y").and_then(Value::as_f64).map(|y| (y - 18.0).abs() < 0.5).unwrap_or(false)
        {
            v.get("text").and_then(Value::as_str).map(str::to_string)
        } else {
            None
        }
    });
    assert!(baseline_cwd_text.is_some(), "todo should render the cwd in its header");

    // Push a PathChanged → a different directory under temp.
    let new_cwd = std::env::temp_dir().join("plexi-harness-pathchanged");
    std::fs::create_dir_all(&new_cwd).expect("mkdir new_cwd");
    h.send_path_changed(&new_cwd);

    // Next frame — header should now reflect the new cwd.
    let second = h.render_frame(2, 800.0, 600.0);
    let new_cwd_str = new_cwd.display().to_string();
    let shows_new_cwd = second.iter().any(|v| {
        v.get("type").and_then(Value::as_str) == Some("text")
            && v.get("text").and_then(Value::as_str) == Some(new_cwd_str.as_str())
    });
    assert!(
        shows_new_cwd,
        "after PathChanged, todo header should show {new_cwd_str:?}. \
         saw text cmds: {:?}",
        second.iter().filter(|v| v.get("type").and_then(Value::as_str) == Some("text"))
            .map(|v| v.get("text").and_then(Value::as_str).unwrap_or(""))
            .collect::<Vec<_>>()
    );
    h.shutdown();
}

#[test]
fn wikipedia_handshake() {
    let Some(mut h) = python_harness(&examples_dir().join("wikipedia"), "wikipedia.py") else {
        return;
    };
    let workspace_root = std::env::temp_dir();
    h.init_and_expect_ready("wikipedia", &workspace_root);
    let cmds = h.render_frame(1, 800.0, 600.0);
    let types = extract_types(&cmds);
    assert!(types.contains(&"frame_done".to_string()));
    h.shutdown();
}

#[test]
fn quick_note_handshake() {
    let Some(mut h) = python_harness(&examples_dir().join("quick-note"), "quick_note.py") else {
        return;
    };
    let workspace_root = std::env::temp_dir();
    h.init_and_expect_ready("quick-note", &workspace_root);
    let cmds = h.render_frame(1, 800.0, 600.0);
    let types = extract_types(&cmds);
    assert!(types.contains(&"frame_done".to_string()));
    h.shutdown();
}

#[test]
fn audio_recorder_handshake() {
    let Some(mut h) = python_harness(&examples_dir().join("audio-recorder"), "audio_recorder.py") else {
        return;
    };
    let workspace_root = std::env::temp_dir();
    h.init_and_expect_ready("audio-recorder", &workspace_root);
    let cmds = h.render_frame(1, 800.0, 600.0);
    let types = extract_types(&cmds);
    assert!(types.contains(&"frame_done".to_string()));
    h.shutdown();
}

#[test]
fn video_player_handshake() {
    let Some(mut h) = python_harness(&examples_dir().join("video-player"), "video_player.py") else {
        return;
    };
    let workspace_root = std::env::temp_dir();
    h.init_and_expect_ready("video-player", &workspace_root);
    let cmds = h.render_frame(1, 800.0, 600.0);
    let types = extract_types(&cmds);
    assert!(types.contains(&"frame_done".to_string()));
    h.shutdown();
}

// ── Media subsystem: exercise mock audio + mock video directly ───────────────

/// Write a tiny WAV file suitable for MockAudioDevice capture input.
fn write_minimal_wav(path: &Path, sample_count: usize) -> std::io::Result<()> {
    use hound::{SampleFormat, WavSpec, WavWriter};
    let spec = WavSpec {
        channels: 1,
        sample_rate: 48000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).map_err(std::io::Error::other)?;
    for i in 0..sample_count {
        let v: i16 = ((i as i32 * 123) % i16::MAX as i32) as i16;
        writer.write_sample(v).map_err(std::io::Error::other)?;
    }
    writer.finalize().map_err(std::io::Error::other)?;
    Ok(())
}

#[test]
fn mock_audio_device_captures_wav_file() {
    use crate::media::{audio_device, AudioSource};
    let tmp = std::env::temp_dir();
    let in_wav = tmp.join("plexi-harness-mock-in.wav");
    let out_wav = tmp.join("plexi-harness-mock-out.wav");
    write_minimal_wav(&in_wav, 4800).expect("write mock capture wav");

    let prev_audio = std::env::var("PLEXI_AUDIO").ok();
    std::env::set_var(
        "PLEXI_AUDIO",
        format!("mock://{},{}", in_wav.display(), out_wav.display()),
    );

    let mut dev = audio_device();

    // Capture: pull at least one buffer off the channel.
    let handle = dev
        .start_capture(48000, 512)
        .expect("mock start_capture");
    let buf = handle
        .receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("mock capture produced a buffer");
    assert!(!buf.is_empty(), "mock capture buffer should be non-empty");

    // Playback: write the source WAV into out_wav via the mock device.
    let play = dev
        .start_playback(AudioSource::File(in_wav.clone()), 1.0)
        .expect("mock start_playback");
    // Give the background thread time to finalize the WAV file.
    thread::sleep(Duration::from_millis(500));
    drop(play);
    thread::sleep(Duration::from_millis(200));

    assert!(out_wav.exists(), "mock playback should create {out_wav:?}");
    let meta = std::fs::metadata(&out_wav).expect("stat out_wav");
    assert!(meta.len() > 44, "output WAV should have more than just a header");

    match prev_audio {
        Some(v) => std::env::set_var("PLEXI_AUDIO", v),
        None => std::env::remove_var("PLEXI_AUDIO"),
    }
}

#[test]
fn mock_video_decoder_yields_frames() {
    use crate::media::{video_decoder, PlaybackState, VideoSource};

    let prev = std::env::var("PLEXI_VIDEO").ok();
    std::env::set_var(
        "PLEXI_VIDEO",
        "mock://fixture.mp4?duration=1000&w=32&h=32",
    );

    let mut dec = video_decoder();
    let handle = dec
        .open(VideoSource::File(PathBuf::from("fixture.mp4")))
        .expect("mock open");
    assert_eq!(handle.width, 32);
    assert_eq!(handle.height, 32);

    dec.set_state(handle.id, PlaybackState::Play);

    // MockVideoDecoder emits a frame roughly every 33ms; wait > one frame.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut frames = 0;
    while Instant::now() < deadline {
        if let Some(frame) = dec.next_frame(handle.id) {
            assert_eq!(frame.width, 32);
            assert_eq!(frame.height, 32);
            assert_eq!(frame.rgba.len(), 32 * 32 * 4);
            frames += 1;
            if frames >= 3 {
                break;
            }
        }
        thread::sleep(Duration::from_millis(40));
    }
    assert!(frames >= 3, "mock decoder should yield ≥3 frames, got {frames}");

    dec.close(handle.id);

    match prev {
        Some(v) => std::env::set_var("PLEXI_VIDEO", v),
        None => std::env::remove_var("PLEXI_VIDEO"),
    }
}
