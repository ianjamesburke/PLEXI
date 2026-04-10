/// Plexi Audio Player
///
/// A minimal audio playback app that speaks the Plexi app protocol.
/// Plexi spawns this binary with the audio file path as argv[1].
///
/// Thread layout:
///   main   — reads Plexi events from stdin, renders UI to stdout
///   audio  — owns the rodio OutputStream + Sink, handles play/pause/stop
///
/// The audio thread is intentionally separate so the audio clock never stalls
/// waiting on stdin I/O. The main thread sends control messages via a channel.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

// ── Protocol types (minimal subset — no dep on plexi crate) ──────────────────

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PlexiEvent {
    Init { width: f32, height: f32, #[serde(default)] pixels_per_point: f32 },
    Render { width: f32, height: f32 },
    Resize { width: f32, height: f32 },
    Key { key: String, #[serde(default)] modifiers: Modifiers },
    Command { text: String },
    Shutdown,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Debug, Default)]
struct Modifiers {
    #[serde(default)] shift: bool,
    #[serde(default)] ctrl: bool,
    #[serde(default)] alt: bool,
    #[serde(default)] cmd: bool,
}

// We write draw commands as plain JSON — no need for a full enum, just helpers.

fn send(obj: &impl Serialize) {
    let mut line = serde_json::to_string(obj).expect("serialize");
    line.push('\n');
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

macro_rules! rect {
    ($x:expr, $y:expr, $w:expr, $h:expr, $fill:expr) => {
        send(&serde_json::json!({"type":"rect","x":$x,"y":$y,"w":$w,"h":$h,"fill":$fill,"radius":0}))
    };
    ($x:expr, $y:expr, $w:expr, $h:expr, $fill:expr, $r:expr) => {
        send(&serde_json::json!({"type":"rect","x":$x,"y":$y,"w":$w,"h":$h,"fill":$fill,"radius":$r}))
    };
}

macro_rules! text {
    ($x:expr, $y:expr, $t:expr, $size:expr, $color:expr) => {
        send(&serde_json::json!({"type":"text","x":$x,"y":$y,"text":$t,"size":$size,"color":$color}))
    };
    ($x:expr, $y:expr, $t:expr, $size:expr, $color:expr, mono) => {
        send(&serde_json::json!({"type":"text","x":$x,"y":$y,"text":$t,"size":$size,"color":$color,"monospace":true}))
    };
}

// ── Audio thread ──────────────────────────────────────────────────────────────

enum AudioMsg {
    Load(PathBuf),
    Play,
    Pause,
    Stop,
    Shutdown,
}

#[derive(Clone, Copy, PartialEq)]
enum PlayState { Stopped, Playing, Paused }

fn audio_thread(
    rx: mpsc::Receiver<AudioMsg>,
    elapsed_ms: Arc<AtomicU64>,
    duration_ms: Arc<AtomicU64>,
) {
    // OutputStream must be kept alive for audio to play.
    let (_stream, stream_handle) = match rodio::OutputStream::try_default() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("audio: no output device: {e}");
            return;
        }
    };

    let mut sink: Option<rodio::Sink> = None;
    let mut state = PlayState::Stopped;
    let mut play_started: Option<Instant> = None;
    let mut accumulated_ms: u64 = 0;

    loop {
        // Update elapsed time each iteration.
        if state == PlayState::Playing {
            if let Some(started) = play_started {
                let ms = accumulated_ms + started.elapsed().as_millis() as u64;
                elapsed_ms.store(ms, Ordering::Relaxed);
            }
        }

        // Non-blocking receive with a short timeout so we keep updating elapsed.
        let msg = rx.recv_timeout(Duration::from_millis(16));

        match msg {
            Ok(AudioMsg::Load(path)) => {
                // Drop old sink to stop current playback.
                sink = None;
                accumulated_ms = 0;
                elapsed_ms.store(0, Ordering::Relaxed);
                duration_ms.store(0, Ordering::Relaxed);
                state = PlayState::Stopped;
                play_started = None;

                let file = match std::fs::File::open(&path) {
                    Ok(f) => f,
                    Err(e) => { eprintln!("audio: cannot open {:?}: {e}", path); continue; }
                };

                let source = match rodio::Decoder::new(std::io::BufReader::new(file)) {
                    Ok(s) => s,
                    Err(e) => { eprintln!("audio: decode error: {e}"); continue; }
                };

                // rodio's Decoder doesn't expose duration directly; we try to get it.
                // For wav files it's available; for mp3 it may not be.
                // We store 0 if unknown and update the UI accordingly.
                // rodio's Decoder may not know duration at decode time (mp3, ogg).
                // Store 0 = unknown; UI shows elapsed without a denominator.
                duration_ms.store(0, Ordering::Relaxed);

                let new_sink = match rodio::Sink::try_new(&stream_handle) {
                    Ok(s) => s,
                    Err(e) => { eprintln!("audio: sink error: {e}"); continue; }
                };
                new_sink.append(source);
                new_sink.play();
                sink = Some(new_sink);
                state = PlayState::Playing;
                play_started = Some(Instant::now());
                accumulated_ms = 0;
            }

            Ok(AudioMsg::Play) => {
                if let Some(s) = &sink {
                    s.play();
                    if state == PlayState::Paused {
                        play_started = Some(Instant::now());
                    }
                    state = PlayState::Playing;
                }
            }

            Ok(AudioMsg::Pause) => {
                if let Some(s) = &sink {
                    s.pause();
                    // Accumulate elapsed before pausing.
                    if let Some(started) = play_started.take() {
                        accumulated_ms += started.elapsed().as_millis() as u64;
                    }
                    state = PlayState::Paused;
                }
            }

            Ok(AudioMsg::Stop) => {
                sink = None;
                state = PlayState::Stopped;
                accumulated_ms = 0;
                play_started = None;
                elapsed_ms.store(0, Ordering::Relaxed);
            }

            Ok(AudioMsg::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Check if sink finished playing.
                if state == PlayState::Playing {
                    if let Some(s) = &sink {
                        if s.empty() {
                            state = PlayState::Stopped;
                            play_started = None;
                        }
                    }
                }
            }
        }
    }
}

// ── UI rendering ──────────────────────────────────────────────────────────────

fn format_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

fn render_ui(
    width: f32,
    height: f32,
    filename: &str,
    state_label: &str,
    elapsed_ms: u64,
    duration_ms: u64,
) {
    // Background
    rect!(0, 0, width, height, "#1e1e2e");

    // Header
    rect!(0, 0, width, 52, "#181825");
    text!(20, 14, filename, 13.0, "#89b4fa");

    let state_color = match state_label {
        "PLAYING" => "#a6e3a1",
        "PAUSED"  => "#f9e2af",
        _         => "#6c7086",
    };
    text!(20, 32, state_label, 10.0, state_color);

    // Time display
    let elapsed_str = format_ms(elapsed_ms);
    let duration_str = if duration_ms > 0 {
        format!("/ {}", format_ms(duration_ms))
    } else {
        String::new()
    };
    let time_str = format!("{} {}", elapsed_str, duration_str);
    text!(width - 120.0, 32, time_str, 10.0, "#cdd6f4", mono);

    // Progress bar
    let bar_y = 60.0;
    let bar_h = 4.0;
    let bar_margin = 20.0;
    let bar_w = width - bar_margin * 2.0;
    rect!(bar_margin, bar_y, bar_w, bar_h, "#313244", 2.0);
    if duration_ms > 0 && elapsed_ms > 0 {
        let frac = (elapsed_ms as f32 / duration_ms as f32).min(1.0);
        let filled_w = (bar_w * frac).max(4.0);
        rect!(bar_margin, bar_y, filled_w, bar_h, "#89b4fa", 2.0);
    }

    // Controls hint
    let hint = "Space  play/pause    S  stop    Esc  close";
    text!(20, height - 22.0, hint, 9.0, "#45475a");

    send(&serde_json::json!({"type": "frame_done"}));
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let file_path: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);

    // Shared state between main and audio threads.
    let elapsed_ms = Arc::new(AtomicU64::new(0));
    let duration_ms = Arc::new(AtomicU64::new(0));

    let (audio_tx, audio_rx) = mpsc::channel::<AudioMsg>();

    // Spawn audio thread.
    let elapsed_clone = Arc::clone(&elapsed_ms);
    let duration_clone = Arc::clone(&duration_ms);
    std::thread::spawn(move || {
        audio_thread(audio_rx, elapsed_clone, duration_clone);
    });

    // If a file was passed, load it immediately.
    if let Some(ref path) = file_path {
        let _ = audio_tx.send(AudioMsg::Load(path.clone()));
    }

    let filename = file_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "No file".into());

    let mut width: f32 = 800.0;
    let mut height: f32 = 400.0;

    // State label is approximated from elapsed/sink — audio thread stores this.
    // We keep a local copy updated from atomic reads each Render.
    let mut state_label = if file_path.is_some() { "PLAYING" } else { "STOPPED" };

    let stdin = std::io::stdin();
    for line in BufReader::new(stdin).lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() { continue; }

        let event: PlexiEvent = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        match event {
            PlexiEvent::Init { width: w, height: h, .. } => {
                width = w;
                height = h;
            }

            PlexiEvent::Render { width: w, height: h } => {
                width = w;
                height = h;
                let elapsed = elapsed_ms.load(Ordering::Relaxed);
                let duration = duration_ms.load(Ordering::Relaxed);
                render_ui(width, height, &filename, state_label, elapsed, duration);
            }

            PlexiEvent::Resize { width: w, height: h } => {
                width = w;
                height = h;
            }

            PlexiEvent::Key { key, .. } => {
                match key.as_str() {
                    "Space" => {
                        if state_label == "PLAYING" {
                            let _ = audio_tx.send(AudioMsg::Pause);
                            state_label = "PAUSED";
                        } else {
                            let _ = audio_tx.send(AudioMsg::Play);
                            state_label = "PLAYING";
                        }
                    }
                    "S" | "s" => {
                        let _ = audio_tx.send(AudioMsg::Stop);
                        state_label = "STOPPED";
                    }
                    _ => {}
                }
            }

            PlexiEvent::Command { text } => {
                let cmd = text.trim().to_lowercase();
                match cmd.as_str() {
                    "play" => {
                        let _ = audio_tx.send(AudioMsg::Play);
                        state_label = "PLAYING";
                    }
                    "pause" => {
                        let _ = audio_tx.send(AudioMsg::Pause);
                        state_label = "PAUSED";
                    }
                    "stop" => {
                        let _ = audio_tx.send(AudioMsg::Stop);
                        state_label = "STOPPED";
                    }
                    other if other.starts_with("load ") => {
                        let path = PathBuf::from(&other[5..].trim());
                        let _ = audio_tx.send(AudioMsg::Load(path));
                        state_label = "PLAYING";
                    }
                    _ => {}
                }
            }

            PlexiEvent::Shutdown | PlexiEvent::Unknown => {
                let _ = audio_tx.send(AudioMsg::Shutdown);
                break;
            }
        }
    }
}
