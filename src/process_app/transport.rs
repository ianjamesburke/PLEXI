//! PGAP process transport: stdin writer, stdout reader, stderr capture, reaper.

use super::lifecycle::LifecycleTracker;
use crate::app_protocol::DrawCommand;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStderr, ChildStdin, ChildStdout};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
    Arc, Mutex,
};
use std::thread;

/// Messages sent from the GUI thread to the stdin-writer background thread.
///
/// `Render` events are coalesced outside the writer by storing the latest JSON
/// payload in `render_slot` and sending one `FlushRender` token.
pub(crate) enum StdinItem {
    Event(String),
    FlushRender,
}

pub(crate) fn request_repaint_from_thread(repaint_ctx: &Arc<Mutex<Option<egui::Context>>>) {
    if let Ok(ctx) = repaint_ctx.lock() {
        if let Some(ctx) = ctx.as_ref() {
            ctx.request_repaint();
        }
    }
}

pub(crate) fn spawn_stdin_writer(
    type_id: String,
    stdin: ChildStdin,
    event_rx: std::sync::mpsc::Receiver<StdinItem>,
    render_slot: Arc<Mutex<Option<String>>>,
    render_in_queue: Arc<AtomicBool>,
) {
    thread::Builder::new()
        .name(format!("app-stdin-{type_id}"))
        .spawn(move || {
            let mut stdin = stdin;
            for item in event_rx {
                match item {
                    StdinItem::Event(line) => {
                        if stdin.write_all(line.as_bytes()).is_err() {
                            log::debug!(
                                "ProcessApp[{type_id}]: stdin write failed — writer thread exiting"
                            );
                            break;
                        }
                    }
                    StdinItem::FlushRender => {
                        render_in_queue.store(false, Ordering::Relaxed);
                        let line = render_slot.lock().unwrap().take();
                        if let Some(line) = line {
                            if stdin.write_all(line.as_bytes()).is_err() {
                                log::debug!(
                                    "ProcessApp[{type_id}]: stdin write failed — writer thread exiting"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        })
        .expect("failed to spawn app-stdin thread");
}

pub(crate) fn spawn_stderr_reader(
    type_id: String,
    stderr: ChildStderr,
    recent_stderr: Arc<Mutex<VecDeque<String>>>,
    lifecycle: Arc<LifecycleTracker>,
    repaint_ctx: Arc<Mutex<Option<egui::Context>>>,
) {
    thread::Builder::new()
        .name(format!("app-stderr-{type_id}"))
        .spawn(move || {
            const STDERR_RING_CAP: usize = 32;
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) if !l.trim().is_empty() => {
                        let target = format!("app::{type_id}");
                        log::warn!(target: &target, "stderr: {l}");
                        lifecycle.observe_stderr_line(&l);
                        request_repaint_from_thread(&repaint_ctx);
                        if let Ok(mut buf) = recent_stderr.lock() {
                            if buf.len() >= STDERR_RING_CAP {
                                buf.pop_front();
                            }
                            buf.push_back(l);
                        }
                    }
                    Err(_) => break,
                    _ => {}
                }
            }
        })
        .expect("failed to spawn app-stderr thread");
}

pub(crate) fn spawn_stdout_reader(
    type_id: String,
    stdout: ChildStdout,
    draw_tx: Sender<DrawCommand>,
    lifecycle: Arc<LifecycleTracker>,
    repaint_ctx: Arc<Mutex<Option<egui::Context>>>,
) {
    thread::Builder::new()
        .name(format!("app-stdout-{type_id}"))
        .spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) if !l.trim().is_empty() => match serde_json::from_str::<DrawCommand>(&l) {
                        Ok(cmd) => {
                            if draw_tx.send(cmd).is_err() {
                                break;
                            }
                            request_repaint_from_thread(&repaint_ctx);
                        }
                        Err(e) => {
                            log::warn!(
                                "ProcessApp[{type_id}]: malformed draw command: {e} — line: {l}"
                            );
                            if lifecycle.on_parse_error() {
                                log::error!(
                                    "ProcessApp[{type_id}]: protocol-error threshold reached — flipping pane state"
                                );
                                request_repaint_from_thread(&repaint_ctx);
                            }
                        }
                    },
                    Err(e) => {
                        log::debug!("ProcessApp[{type_id}] stdout closed: {e}");
                        lifecycle.on_stdout_closed();
                        request_repaint_from_thread(&repaint_ctx);
                        break;
                    }
                    _ => {}
                }
            }
            lifecycle.on_stdout_closed();
            request_repaint_from_thread(&repaint_ctx);
        })
        .expect("failed to spawn app-stdout thread");
}

pub(crate) fn spawn_reaper(
    type_id: String,
    pid: u32,
    lifecycle: Arc<LifecycleTracker>,
    repaint_ctx: Arc<Mutex<Option<egui::Context>>>,
) {
    thread::Builder::new()
        .name(format!("app-reaper-{type_id}"))
        .spawn(move || {
            let mut status = 0i32;
            // SAFETY: pid is a valid child PID obtained from Command::spawn().
            unsafe {
                libc::waitpid(pid as libc::pid_t, &mut status, 0);
            }
            log::info!("ProcessApp[{type_id}]: child exited — reaper signaling lifecycle");
            lifecycle.on_process_exited();
            request_repaint_from_thread(&repaint_ctx);
        })
        .expect("failed to spawn app-reaper thread");
}
