/// DrawCommand routing — dispatches out-of-frame commands to subsystems.
///
/// All visual draw commands stay in the frame pipeline; only control commands
/// (media, pipes, capabilities, secrets, runs, notifications) are routed here.

use crate::app_permissions::{Capability, PermissionCheck, check};
use crate::app_protocol::{DrawCommand, PlexiEvent};
use crate::app_trait::AppCommand;
use crate::event_log::{self, HostEvent};
use crate::media::{audio_device, video_decoder, AudioSource, VideoSource, PlaybackState};
use crate::typed_pipes::PipeDirection;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use super::ProcessApp;

impl ProcessApp {
    /// Route a v3 out-of-frame draw command to the appropriate subsystem.
    /// Visual primitives must not reach this method — callers filter them upstream.
    pub(super) fn route_command(&mut self, cmd: DrawCommand) {
        match cmd {
            // ── Video ──────────────────────────────────────────────────────
            DrawCommand::VideoPlayer { source, x, y, w, h, state } => {
                if !self.video_handles.contains_key(&source) {
                    // Lazily create the decoder on first video open.
                    if self.video_decoder.is_none() {
                        self.video_decoder = Some(video_decoder());
                    }
                    if let Some(decoder) = self.video_decoder.as_mut() {
                        match decoder.open(VideoSource::File(PathBuf::from(&source))) {
                            Ok(handle) => {
                                log::info!(
                                    "ProcessApp[{}]: opened video '{}' {}x{} {}ms",
                                    self.type_id, source, handle.width, handle.height, handle.duration_ms
                                );
                                self.video_handles.insert(source.clone(), handle);
                            }
                            Err(e) => {
                                log::warn!("ProcessApp[{}]: failed to open video '{}': {e}", self.type_id, source);
                                return;
                            }
                        }
                    }
                }

                if let Some(handle) = self.video_handles.get(&source) {
                    let handle_id = handle.id;
                    let playback_state = if state == "play" {
                        Some(PlaybackState::Play)
                    } else if state == "pause" {
                        Some(PlaybackState::Pause)
                    } else if let Some(ms_str) = state.strip_prefix("seek:") {
                        ms_str.parse::<u64>().ok().map(PlaybackState::Seek)
                    } else {
                        None
                    };

                    if let Some(ps) = playback_state {
                        if let Some(decoder) = self.video_decoder.as_mut() {
                            decoder.set_state(handle_id, ps);
                        }
                    }

                    // Pull next frame and queue for upload+render in the render pass
                    // (egui texture upload requires a UI context, not available here).
                    if let Some(decoder) = self.video_decoder.as_mut() {
                        if let Some(frame) = decoder.next_frame(handle_id) {
                            log::debug!(
                                "ProcessApp[{}]: VideoFrame {}x{} pts={}ms source={source}",
                                self.type_id, frame.width, frame.height, frame.pts_ms
                            );
                            self.pending_video_frames.push((handle_id, frame, x, y, w, h));
                        }
                    }
                }
            }

            // ── Audio playback ─────────────────────────────────────────────
            DrawCommand::AudioPlay { source, volume, state } => {
                if state == "stop" {
                    if self.audio_playback_handles.remove(&source).is_some() {
                        log::info!("ProcessApp[{}]: stopped audio '{source}'", self.type_id);
                    }
                    return;
                }

                if !self.audio_playback_handles.contains_key(&source) && state == "play" {
                    let mut device = audio_device();
                    match device.start_playback(AudioSource::File(PathBuf::from(&source)), volume) {
                        Ok(handle) => {
                            log::info!("ProcessApp[{}]: started audio playback '{source}' vol={volume}", self.type_id);
                            self.audio_playback_handles.insert(source.clone(), handle);
                        }
                        Err(e) => {
                            log::warn!("ProcessApp[{}]: audio start_playback failed: {e}", self.type_id);
                        }
                    }
                }
            }

            // ── Audio capture ──────────────────────────────────────────────
            DrawCommand::AudioCapture { pipe_id, sample_rate, buffer_size } => {
                if let PermissionCheck::Denied(reason) = check(&self.permissions, Capability::AudioRecord) {
                    log::warn!("ProcessApp[{}]: AudioCapture denied — {reason}", self.type_id);
                    return;
                }

                if self.audio_capture_handles.contains_key(&pipe_id) {
                    log::warn!("ProcessApp[{}]: AudioCapture pipe '{}' already open", self.type_id, pipe_id);
                    return;
                }

                let alloc = match self.pipe_registry.lock().unwrap().open_binary(
                    pipe_id.clone(),
                    PipeDirection::Out,
                ) {
                    Ok(a) => a,
                    Err(e) => {
                        log::warn!("ProcessApp[{}]: AudioCapture pipe alloc failed: {e}", self.type_id);
                        return;
                    }
                };

                let mut device = audio_device();
                match device.start_capture(sample_rate, buffer_size) {
                    Ok(capture_handle) => {
                        let socket_path = alloc.socket_path.clone();

                        // Forwarding thread: read PCM frames and write to binary pipe.
                        // TypedPipeRegistry is behind Arc<Mutex<>> so this thread can
                        // call write_binary without blocking the main UI thread.
                        let pipe_id_fwd = pipe_id.clone();
                        let type_id_fwd = self.type_id.clone();
                        let registry_arc = Arc::clone(&self.pipe_registry);
                        thread::Builder::new()
                            .name(format!("audio-capture-{pipe_id_fwd}"))
                            .spawn(move || {
                                log::info!("ProcessApp[{type_id_fwd}]: audio capture thread started for pipe '{pipe_id_fwd}'");
                                loop {
                                    match capture_handle.receiver.recv() {
                                        Ok(samples) => {
                                            let bytes: Vec<u8> = samples
                                                .iter()
                                                .flat_map(|s| s.to_le_bytes())
                                                .collect();
                                            let mut reg = registry_arc.lock().unwrap();
                                            if let Err(e) = reg.write_binary(&pipe_id_fwd, &bytes) {
                                                log::warn!("ProcessApp[{type_id_fwd}]: audio write failed: {e}");
                                            }
                                        }
                                        Err(_) => break,
                                    }
                                }
                                log::info!("ProcessApp[{type_id_fwd}]: audio capture thread exiting for pipe '{pipe_id_fwd}'");
                            })
                            .ok();

                        event_log::emit_pipe_opened(&self.type_id, &pipe_id, "binary");
                        self.outbound_events.push_back(PlexiEvent::PipeOpened {
                            pipe_id: pipe_id.clone(),
                            socket_path,
                        });
                        log::info!(
                            "ProcessApp[{}]: AudioCapture started on pipe '{pipe_id}' {}hz buf={}",
                            self.type_id, sample_rate, buffer_size
                        );
                    }
                    Err(e) => {
                        log::warn!("ProcessApp[{}]: AudioCapture start_capture failed: {e}", self.type_id);
                        self.pipe_registry.lock().unwrap().close(&pipe_id);
                    }
                }
            }

            // ── Audio meter ────────────────────────────────────────────────
            DrawCommand::AudioMeter { x, y, w, h, pipe_id } => {
                if let Some(existing) = self.audio_meters.iter_mut().find(|m| m.pipe_id == pipe_id) {
                    existing.rect_x = x;
                    existing.rect_y = y;
                    existing.rect_w = w;
                    existing.rect_h = h;
                } else {
                    self.audio_meters.push(super::AudioMeterState {
                        rect_x: x, rect_y: y, rect_w: w, rect_h: h, pipe_id,
                    });
                }
            }

            // ── Capability request ─────────────────────────────────────────
            DrawCommand::CapabilityRequest { request_id, capability } => {
                let cap = Capability::from(capability.as_str());
                if let PermissionCheck::Allowed = check(&self.permissions, cap) {
                    self.outbound_events.push_back(PlexiEvent::CapabilityDecision {
                        request_id,
                        granted: true,
                    });
                } else {
                    self.pending_prompts.push_back(super::PendingPrompt::Capability {
                        request_id,
                        capability,
                    });
                }
            }

            // ── Secret get ─────────────────────────────────────────────────
            DrawCommand::SecretGet { key } => {
                if let PermissionCheck::Denied(reason) = check(&self.permissions, Capability::SecretsGet) {
                    log::warn!("ProcessApp[{}]: SecretGet denied — {reason}", self.type_id);
                    event_log::emit(HostEvent::SecretDenied {
                        app_id: self.type_id.clone(),
                        key: key.clone(),
                        reason: format!("capability_denied: {reason}"),
                        timestamp: event_log::now_timestamp(),
                    });
                    self.outbound_events.push_back(PlexiEvent::SecretValue { key, value: None });
                    return;
                }

                #[cfg(target_os = "macos")]
                {
                    match crate::secrets::get_secret_scoped(&key, &self.type_id, &self.workspace_root) {
                        Some(value) => {
                            self.outbound_events.push_back(PlexiEvent::SecretValue {
                                key,
                                value: Some(value.to_string()),
                            });
                        }
                        None => {
                            event_log::emit(HostEvent::SecretPrompted {
                                app_id: self.type_id.clone(),
                                key: key.clone(),
                                timestamp: event_log::now_timestamp(),
                            });
                            self.pending_prompts.push_back(super::PendingPrompt::Secret { key });
                        }
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    log::warn!("ProcessApp[{}]: SecretGet not supported on this platform", self.type_id);
                    event_log::emit(HostEvent::SecretDenied {
                        app_id: self.type_id.clone(),
                        key: key.clone(),
                        reason: "unsupported_platform".to_string(),
                        timestamp: event_log::now_timestamp(),
                    });
                    self.outbound_events.push_back(PlexiEvent::SecretValue { key, value: None });
                }
            }

            // ── Run get ────────────────────────────────────────────────────
            DrawCommand::RunGet { intent, payload } => {
                let run_id = self.run_registry.allocate(&self.type_id, &intent, payload.clone());
                log::info!(
                    "ProcessApp[{}]: RunGet intent='{}' → run_id='{}'",
                    self.type_id, intent, run_id
                );
                event_log::emit(HostEvent::RunStarted {
                    run_id: run_id.clone(),
                    app_id: self.type_id.clone(),
                    timestamp: event_log::now_timestamp(),
                });
                self.outbound_events.push_back(PlexiEvent::RunUpdate {
                    run_id,
                    status: "pending".to_string(),
                    payload: serde_json::Value::Null,
                });
            }

            // ── Run complete ───────────────────────────────────────────────
            DrawCommand::RunComplete { run_id, result } => {
                log::info!(
                    "ProcessApp[{}]: RunComplete run_id='{run_id}' result={result}",
                    self.type_id
                );
                self.run_registry.complete(&run_id);
            }

            // ── Notify ─────────────────────────────────────────────────────
            DrawCommand::Notify { level, title, body, actions } => {
                let notif_id = format!("{}-{}", self.type_id, event_log::now_timestamp());
                log::info!("ProcessApp[{}]: Notify [{level}] '{title}': {body}", self.type_id);
                event_log::emit(HostEvent::NotificationPosted {
                    id: notif_id.clone(),
                    title: title.clone(),
                    urgency: level.clone(),
                    timestamp: event_log::now_timestamp(),
                });

                for action in &actions {
                    log::info!(
                        "ProcessApp[{}]: notify action action_type={} payload={}",
                        self.type_id, action.action_type, action.payload
                    );
                    match action.action_type.as_str() {
                        "resume_run" => {
                            if let Some(run_id) = action.payload.get("run_id").and_then(|v| v.as_str()) {
                                self.run_registry.resume(run_id);
                                self.outbound_events.push_back(PlexiEvent::RunUpdate {
                                    run_id: run_id.to_string(),
                                    status: "pending".to_string(),
                                    payload: serde_json::Value::Null,
                                });
                                event_log::emit(HostEvent::NotificationActionInvoked {
                                    id: notif_id.clone(),
                                    action: "resume_run".to_string(),
                                    timestamp: event_log::now_timestamp(),
                                });
                            }
                        }
                        "open_intent" => {
                            if let Some(intent) = action.payload.get("intent").and_then(|v| v.as_str()) {
                                self.pending_commands.push(AppCommand::Notify(
                                    format!("[intent] {intent}")
                                ));
                                event_log::emit(HostEvent::NotificationActionInvoked {
                                    id: notif_id.clone(),
                                    action: "open_intent".to_string(),
                                    timestamp: event_log::now_timestamp(),
                                });
                            }
                        }
                        "run_command" => {
                            if let Some(command) = action.payload.get("command").and_then(|v| v.as_str()) {
                                self.pending_commands.push(AppCommand::RunInTerminal(command.to_string()));
                                event_log::emit(HostEvent::NotificationActionInvoked {
                                    id: notif_id.clone(),
                                    action: "run_command".to_string(),
                                    timestamp: event_log::now_timestamp(),
                                });
                            } else {
                                log::warn!("ProcessApp[{}]: run_command action missing 'command' payload", self.type_id);
                            }
                        }
                        other => {
                            log::warn!("ProcessApp[{}]: unknown notify action_type='{other}'", self.type_id);
                        }
                    }
                }
                self.pending_commands.push(AppCommand::Notify(
                    format!("[{level}] {title}: {body}")
                ));
            }

            // ── Pipe open ──────────────────────────────────────────────────
            DrawCommand::PipeOpen { pipe_id, mode, direction } => {
                if let PermissionCheck::Denied(reason) = check(&self.permissions, Capability::PipeOpen) {
                    log::warn!("ProcessApp[{}]: PipeOpen denied — {reason}", self.type_id);
                    return;
                }

                let dir = match direction.as_str() {
                    "in" => PipeDirection::In,
                    "out" => PipeDirection::Out,
                    _ => PipeDirection::Duplex,
                };

                if mode == "binary" {
                    match self.pipe_registry.lock().unwrap().open_binary(pipe_id.clone(), dir) {
                        Ok(alloc) => {
                            log::info!(
                                "ProcessApp[{}]: opened binary pipe '{pipe_id}' → {}",
                                self.type_id, alloc.socket_path
                            );
                            event_log::emit_pipe_opened(&self.type_id, &pipe_id, "binary");
                            self.outbound_events.push_back(PlexiEvent::PipeOpened {
                                pipe_id,
                                socket_path: alloc.socket_path,
                            });
                        }
                        Err(e) => log::warn!("ProcessApp[{}]: PipeOpen binary failed: {e}", self.type_id),
                    }
                } else {
                    match self.pipe_registry.lock().unwrap().open_json(pipe_id.clone(), dir) {
                        Ok(()) => {
                            log::info!("ProcessApp[{}]: opened JSON pipe '{pipe_id}'", self.type_id);
                            event_log::emit_pipe_opened(&self.type_id, &pipe_id, "json");
                        }
                        Err(e) => log::warn!("ProcessApp[{}]: PipeOpen json failed: {e}", self.type_id),
                    }
                }
            }

            // ── Pipe send ──────────────────────────────────────────────────
            DrawCommand::PipeSend { pipe_id, payload } => {
                match self.pipe_registry.lock().unwrap().send_json(&pipe_id, payload.clone()) {
                    Ok(()) => {
                        // TODO(layer-5): route PipeMessage to peer apps subscribed on this pipe_id
                        self.outbound_events.push_back(PlexiEvent::PipeMessage {
                            pipe_id,
                            payload,
                        });
                    }
                    Err(e) => log::warn!("ProcessApp[{}]: PipeSend failed: {e}", self.type_id),
                }
            }

            // ── Status summary ─────────────────────────────────────────────
            DrawCommand::StatusSummary { text } => {
                self.status_summary = Some(text);
            }

            // ── Spawn app ──────────────────────────────────────────────────
            DrawCommand::SpawnApp { type_id, layout, args } => {
                if let PermissionCheck::Denied(reason) = check(&self.permissions, Capability::SpawnApp) {
                    log::warn!("ProcessApp[{}]: SpawnApp denied — {reason}", self.type_id);
                    return;
                }
                log::info!(
                    "ProcessApp[{}]: SpawnApp type_id='{type_id}' layout={layout:?} args={args:?}",
                    self.type_id
                );
                self.pending_commands.push(AppCommand::SpawnApp { type_id, layout, args });
            }

            _ => unreachable!("route_command called with non-control command"),
        }
    }
}
