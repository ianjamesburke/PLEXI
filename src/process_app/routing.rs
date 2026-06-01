//! DrawCommand routing — dispatches out-of-frame commands to subsystems.
//!
//! All visual draw commands stay in the frame pipeline; only control commands
//! (media, pipes, capabilities, secrets, runs, notifications) are routed here.

use crate::app_permissions::{check, is_blocked, Capability, PermissionCheck};
use crate::app_protocol::{AudioDeviceWire, AppRequest, MidiPortWire, PlexiEvent, StreamChannel};
use crate::app_trait::AppCommand;
use crate::audio::AudioCaptureRequest;
use crate::event_log::{self, HostEvent};
use crate::plexi_ai::broker::AiBrokerRequest;
use crate::typed_pipes::PipeDirection;
use std::io::Read;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use url::Url;

use super::{ProcessApp, StreamHandle};

impl ProcessApp {
    /// Route a v3 host command to the appropriate subsystem.
    /// Exhaustive match — no wildcard arm. The compiler enforces coverage.
    pub(super) fn route_command(&mut self, cmd: AppRequest) {
        match cmd {
            // ── Capability request ─────────────────────────────────────────
            AppRequest::CapabilityRequest {
                request_id,
                capability,
            } => {
                match Capability::try_from(capability.as_str()) {
                    Ok(cap) => {
                        if let PermissionCheck::Allowed = check(&self.permissions, cap) {
                            self.outbound_events
                                .push_back(PlexiEvent::CapabilityDecision {
                                    request_id,
                                    granted: true,
                                });
                        } else if is_blocked(&self.permissions, cap) {
                            log::info!(
                                "ProcessApp[{}]: {} permanently blocked — auto-denying",
                                self.type_id, cap
                            );
                            self.outbound_events
                                .push_back(PlexiEvent::CapabilityDecision {
                                    request_id,
                                    granted: false,
                                });
                        } else {
                            self.pending_prompts
                                .push_back(super::PendingPrompt::Capability {
                                    request_id,
                                    capability,
                                });
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "ProcessApp[{}]: CapabilityRequest with {e}; auto-denying",
                            self.type_id
                        );
                        self.outbound_events
                            .push_back(PlexiEvent::CapabilityDecision {
                                request_id,
                                granted: false,
                            });
                    }
                }
            }

            // ── Secret get ─────────────────────────────────────────────────
            AppRequest::SecretGet { key } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::SecretsGet)
                {
                    log::warn!("ProcessApp[{}]: SecretGet denied — {reason}", self.type_id);
                    event_log::emit(HostEvent::SecretDenied {
                        app_id: self.type_id.clone(),
                        key: key.clone(),
                        reason: format!("capability_denied: {reason}"),
                        timestamp: event_log::now_timestamp(),
                    });
                    self.outbound_events
                        .push_back(PlexiEvent::SecretValue { key, value: None });
                    return;
                }

                #[cfg(target_os = "macos")]
                {
                    use crate::workspace_secrets::{
                        resolve, MacKeychain, ResolveOutcome, WorkspaceConfig,
                        WorkspaceSecrets,
                    };
                    // Step 0: load workspace.toml + secrets.toml. Both must
                    // exist for routed lookup; missing either falls back to
                    // the legacy global-scoped secrets path so that apps
                    // running outside an initialized workspace still work
                    // until the user runs `plexi workspace init`.
                    let ws_cfg = WorkspaceConfig::load(&self.workspace_root)
                        .ok()
                        .flatten();
                    let router = WorkspaceSecrets::load(&self.workspace_root)
                        .map_err(|e| {
                            log::error!(
                                "ProcessApp[{}]: invalid {}/.plexi/secrets.toml: {e}",
                                self.type_id,
                                self.workspace_root.display()
                            );
                            e
                        })
                        .ok()
                        .flatten();
                    let store = MacKeychain::new();
                    match (ws_cfg, router) {
                        (Some(cfg), Some(r)) => match resolve(
                            &cfg.id,
                            &self.type_id,
                            &key,
                            &r,
                            &store,
                        ) {
                            ResolveOutcome::Found(value) => {
                                self.outbound_events.push_back(
                                    PlexiEvent::SecretValue {
                                        key,
                                        value: Some(value.to_string()),
                                    },
                                );
                            }
                            ResolveOutcome::HardMissing { reason } => {
                                log::warn!(
                                    "ProcessApp[{}]: SecretGet '{key}' hard-missing: {reason}",
                                    self.type_id
                                );
                                event_log::emit(HostEvent::SecretDenied {
                                    app_id: self.type_id.clone(),
                                    key: key.clone(),
                                    reason: format!("hard_missing: {reason}"),
                                    timestamp: event_log::now_timestamp(),
                                });
                                self.outbound_events.push_back(
                                    PlexiEvent::SecretValue { key, value: None },
                                );
                            }
                            ResolveOutcome::PromptUser => {
                                event_log::emit(HostEvent::SecretPrompted {
                                    app_id: self.type_id.clone(),
                                    key: key.clone(),
                                    timestamp: event_log::now_timestamp(),
                                });
                                self.pending_prompts.push_back(
                                    super::PendingPrompt::Secret { key },
                                );
                            }
                        },
                        _ => {
                            // No workspace config / router yet — fall back
                            // to the legacy v3.0 path (workspace_root-keyed
                            // single-namespace lookup). Logs the gap so the
                            // user knows to run `plexi workspace init`.
                            match crate::secrets::get_secret_scoped(
                                &key,
                                &self.type_id,
                                &self.workspace_root,
                            ) {
                                Some(value) => {
                                    self.outbound_events.push_back(
                                        PlexiEvent::SecretValue {
                                            key,
                                            value: Some(value.to_string()),
                                        },
                                    );
                                }
                                None => {
                                    event_log::emit(HostEvent::SecretPrompted {
                                        app_id: self.type_id.clone(),
                                        key: key.clone(),
                                        timestamp: event_log::now_timestamp(),
                                    });
                                    self.pending_prompts.push_back(
                                        super::PendingPrompt::Secret { key },
                                    );
                                }
                            }
                        }
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    log::warn!(
                        "ProcessApp[{}]: SecretGet not supported on this platform",
                        self.type_id
                    );
                    event_log::emit(HostEvent::SecretDenied {
                        app_id: self.type_id.clone(),
                        key: key.clone(),
                        reason: "unsupported_platform".to_string(),
                        timestamp: event_log::now_timestamp(),
                    });
                    self.outbound_events
                        .push_back(PlexiEvent::SecretValue { key, value: None });
                }
            }

            // ── Run get ────────────────────────────────────────────────────
            AppRequest::RunGet { intent, .. } => {
                let run_id = self.run_registry.allocate(&self.type_id);
                log::info!(
                    "ProcessApp[{}]: RunGet intent='{}' → run_id='{}'",
                    self.type_id,
                    intent,
                    run_id
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
            AppRequest::RunComplete { run_id, result } => {
                log::info!(
                    "ProcessApp[{}]: RunComplete run_id='{run_id}' result={result}",
                    self.type_id
                );
                let originator = self.run_registry.originator_of(&run_id).map(|s| s.to_string());
                self.run_registry.complete(&run_id);
                let update = PlexiEvent::RunUpdate {
                    run_id,
                    status: "completed".to_string(),
                    payload: result,
                };
                match originator.as_deref() {
                    Some(orig) if orig == self.type_id => {
                        self.outbound_events.push_back(update);
                    }
                    Some(orig_type_id) => {
                        self.pending_commands.push(AppCommand::DeliverRunUpdate {
                            originator_type_id: orig_type_id.to_string(),
                            event: update,
                        });
                    }
                    None => {
                        // No originator found (run already removed or never registered).
                        self.outbound_events.push_back(update);
                    }
                }
            }

            // ── Notify ─────────────────────────────────────────────────────
            AppRequest::Notify {
                level,
                title,
                body,
                kind,
                options,
                input_prompt,
                required,
                actions,
                notify_id,
                priority,
                image_inline,
                image_pipe_id,
                timeout_secs,
                on_dismiss,
                response_file: _,
                scope: _,
            } => {
                let notif_id = format!("{}-{}", self.type_id, event_log::now_timestamp());
                log::info!(
                    "ProcessApp[{}]: Notify [{level}] '{title}': {body}",
                    self.type_id
                );
                event_log::emit(HostEvent::NotificationPosted {
                    id: notif_id.clone(),
                    title: title.clone(),
                    urgency: level.clone(),
                    timestamp: event_log::now_timestamp(),
                });

                for action in &actions {
                    log::info!(
                        "ProcessApp[{}]: notify action action_type={} payload={}",
                        self.type_id,
                        action.action_type,
                        action.payload
                    );
                    match action.action_type.as_str() {
                        "resume_run" => {
                            if let Some(run_id) =
                                action.payload.get("run_id").and_then(|v| v.as_str())
                            {
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
                            if let Some(intent) =
                                action.payload.get("intent").and_then(|v| v.as_str())
                            {
                                self.pending_commands
                                    .push(AppCommand::Notify(format!("[intent] {intent}")));
                                event_log::emit(HostEvent::NotificationActionInvoked {
                                    id: notif_id.clone(),
                                    action: "open_intent".to_string(),
                                    timestamp: event_log::now_timestamp(),
                                });
                            }
                        }
                        "run_command" => {
                            if let Some(command) =
                                action.payload.get("command").and_then(|v| v.as_str())
                            {
                                self.pending_commands
                                    .push(AppCommand::Notify(format!("[run_command] {command}")));
                                event_log::emit(HostEvent::NotificationActionInvoked {
                                    id: notif_id.clone(),
                                    action: "run_command".to_string(),
                                    timestamp: event_log::now_timestamp(),
                                });
                            } else {
                                log::warn!(
                                    "ProcessApp[{}]: run_command action missing 'command' payload",
                                    self.type_id
                                );
                            }
                        }
                        other => {
                            log::warn!(
                                "ProcessApp[{}]: unknown notify action_type='{other}'",
                                self.type_id
                            );
                        }
                    }
                }
                // Always show in the notification panel. Use app-supplied notify_id if
                // provided (enables NotifyAction round-trips), otherwise use the auto-
                // generated notif_id so the panel entry is always created.
                let panel_id = notify_id.unwrap_or_else(|| notif_id.clone());
                // Mutually exclusive image attachments — inline wins. Loud
                // warn when both set so SDK users notice and clean up the
                // call site rather than silently shipping ambiguous data.
                let (image_inline, image_pipe_id) = match (image_inline, image_pipe_id) {
                    (Some(inline), Some(pipe_id)) => {
                        log::warn!(
                            "ProcessApp[{}]: Notify '{title}' has both image_inline and image_pipe_id='{pipe_id}'; inline wins, pipe ignored",
                            self.type_id
                        );
                        (Some(inline), None)
                    }
                    pair => pair,
                };
                self.pending_commands.push(AppCommand::ShowNotification {
                    notify_id: panel_id,
                    sender_pane_id: 0, // stamped by dispatch.rs with the real pane_id
                    source_context_id: 0, // stamped by dispatch.rs with the real context_id
                    level,
                    title,
                    body,
                    kind,
                    options,
                    input_prompt,
                    required,
                    priority,
                    // Scope placeholder — dispatch.rs overwrites with the
                    // manifest-declared default for this app's type_id.
                    // Apps never set scope; users control it via manifest.
                    scope: crate::app_protocol::NotifyScope::Context,
                    image_inline,
                    image_pipe_id,
                    timeout_secs,
                    on_dismiss,
                });
                // `actions` intentionally dropped: they were already processed
                // as server-side side effects above (resume_run / open_intent /
                // run_command). The modal surfaces options, not actions.
                let _ = actions;
            }

            // ── Pipe open ──────────────────────────────────────────────────
            AppRequest::PipeOpen {
                pipe_id,
                mode,
                direction,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::PipeOpen)
                {
                    log::warn!("ProcessApp[{}]: PipeOpen denied — {reason}", self.type_id);
                    return;
                }

                let dir = match direction.as_str() {
                    "in" => PipeDirection::In,
                    "out" => PipeDirection::Out,
                    _ => PipeDirection::Duplex,
                };

                if mode == "binary" {
                    match self
                        .pipe_registry
                        .lock()
                        .unwrap()
                        .open_binary(pipe_id.clone(), dir)
                    {
                        Ok(alloc) => {
                            log::info!(
                                "ProcessApp[{}]: opened binary pipe '{pipe_id}' → {}",
                                self.type_id,
                                alloc.socket_path
                            );
                            event_log::emit_pipe_opened(&self.type_id, &pipe_id, "binary");
                            self.outbound_events.push_back(PlexiEvent::PipeOpened {
                                pipe_id,
                                socket_path: alloc.socket_path,
                            });
                        }
                        Err(e) => {
                            log::warn!("ProcessApp[{}]: PipeOpen binary failed: {e}", self.type_id)
                        }
                    }
                } else {
                    match self
                        .pipe_registry
                        .lock()
                        .unwrap()
                        .open_json(pipe_id.clone(), dir)
                    {
                        Ok(()) => {
                            log::info!(
                                "ProcessApp[{}]: opened JSON pipe '{pipe_id}'",
                                self.type_id
                            );
                            event_log::emit_pipe_opened(&self.type_id, &pipe_id, "json");
                        }
                        Err(e) => {
                            log::warn!("ProcessApp[{}]: PipeOpen json failed: {e}", self.type_id)
                        }
                    }
                }
            }

            // ── Pipe open (directed) — #286 ────────────────────────────────
            // Inter-agent (or app↔agent) channel. Caller needs `pipe.open`
            // (same gate as undirected `PipeOpen`). Resolution lives in
            // `app/mod.rs::AppCommand::OpenDirectedPipe` because the target
            // pane is not in this process and has to be subscribed by the
            // host. We register the JSON pipe locally so subsequent
            // `PipeSend` calls succeed; `has_reader` returns true for the
            // sender side because the direction is duplex.
            AppRequest::PipeOpenDirected {
                pipe_id,
                target_pane_id,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::PipeOpen)
                {
                    log::warn!(
                        "ProcessApp[{}]: PipeOpenDirected denied — {reason}",
                        self.type_id
                    );
                    return;
                }
                match self
                    .pipe_registry
                    .lock()
                    .unwrap()
                    .open_json(pipe_id.clone(), PipeDirection::Duplex)
                {
                    Ok(()) => {
                        log::info!(
                            "ProcessApp[{}]: opened directed JSON pipe '{pipe_id}' → pane {target_pane_id}",
                            self.type_id
                        );
                        event_log::emit_pipe_opened(&self.type_id, &pipe_id, "json");
                        self.pending_commands.push(AppCommand::OpenDirectedPipe {
                            sender_pane_id: self.pane_id,
                            pipe_id,
                            target_pane_id,
                        });
                    }
                    Err(e) => log::warn!(
                        "ProcessApp[{}]: PipeOpenDirected failed: {e}",
                        self.type_id
                    ),
                }
            }

            // ── Pipe send ──────────────────────────────────────────────────
            AppRequest::PipeSend { pipe_id, payload } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::PipeOpen)
                {
                    log::warn!("ProcessApp[{}]: PipeSend denied — {reason}", self.type_id);
                    return;
                }
                match self
                    .pipe_registry
                    .lock()
                    .unwrap()
                    .send_json(&pipe_id, payload.clone())
                {
                    Ok(()) => {
                        self.pending_commands.push(AppCommand::DeliverPipeMessage {
                            sender_pane_id: self.pane_id,
                            pipe_id,
                            payload,
                        });
                    }
                    Err(e) => log::warn!("ProcessApp[{}]: PipeSend failed: {e}", self.type_id),
                }
            }

            // ── Status summary ─────────────────────────────────────────────
            AppRequest::StatusSummary { text } => {
                self.status_summary = Some(text);
            }

            // ── Navigation stack ───────────────────────────────────────────
            AppRequest::PushNav { view_id, title } => {
                self.nav_stack.push(crate::process_app::NavEntry { view_id, title });
                log::debug!(
                    "ProcessApp[{}]: PushNav → depth={} title={:?}",
                    self.type_id,
                    self.nav_stack.len(),
                    self.nav_stack.last().map(|e| &e.title),
                );
            }
            AppRequest::PopNav {} => {
                self.nav_stack.pop();
                log::debug!(
                    "ProcessApp[{}]: PopNav → depth={}",
                    self.type_id,
                    self.nav_stack.len(),
                );
            }

            // ── Process streaming (#358) ───────────────────────────────────
            AppRequest::StreamProcess {
                correlation_id,
                terminal_pane_id: _,
                command,
                channel,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::TerminalBindings)
                {
                    log::warn!(
                        "ProcessApp[{}]: StreamProcess {correlation_id} denied — {reason}",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::StreamEnd {
                        correlation_id,
                        exit_code: 1,
                    });
                    return;
                }
                log::info!(
                    "ProcessApp[{}]: StreamProcess {correlation_id} — command={command:?} channel={channel:?}",
                    self.type_id
                );
                let mut child = match std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!(
                            "ProcessApp[{}]: StreamProcess {correlation_id} — spawn failed: {e}",
                            self.type_id
                        );
                        self.outbound_events.push_back(PlexiEvent::StreamEnd {
                            correlation_id,
                            exit_code: 1,
                        });
                        return;
                    }
                };
                let pid = child.id();
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.stream_handles.insert(
                    correlation_id.clone(),
                    StreamHandle { cancel: Arc::clone(&cancel), pid },
                );
                // Take the appropriate pipe before moving child into the thread.
                let pipe: Box<dyn Read + Send> = match channel {
                    StreamChannel::Stdout | StreamChannel::Structured => {
                        Box::new(child.stdout.take().expect("stdout piped"))
                    }
                    StreamChannel::Stderr => {
                        Box::new(child.stderr.take().expect("stderr piped"))
                    }
                };
                let tx = self.http_tx.clone();
                let type_id = self.type_id.clone();
                let corr_id = correlation_id.clone();
                const MAX_STREAM_THREADS: usize = 32;
                let active = Arc::clone(&self.active_stream_threads);
                let prior = active.fetch_add(1, Ordering::Relaxed);
                if prior >= MAX_STREAM_THREADS {
                    active.fetch_sub(1, Ordering::Relaxed);
                    log::warn!(
                        "ProcessApp[{}]: StreamProcess {correlation_id} rejected — stream thread limit ({MAX_STREAM_THREADS}) reached",
                        self.type_id
                    );
                    let _ = self.http_tx.send(PlexiEvent::StreamEnd {
                        correlation_id,
                        exit_code: -1,
                    });
                    return;
                }
                let thread_name = format!("stream-reader-{}-{}", type_id, corr_id);
                std::thread::Builder::new()
                    .name(thread_name)
                    .spawn(move || {
                    let mut buf = vec![0u8; 4096];
                    let mut pipe = pipe;
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        match pipe.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                let _ = tx.send(PlexiEvent::StreamChunk {
                                    correlation_id: corr_id.clone(),
                                    channel,
                                    bytes: buf[..n].to_vec(),
                                });
                            }
                            Err(_) => break,
                        }
                    }
                    // Drop the pipe so the child's pipe descriptor is closed,
                    // then reap the child to avoid zombies.
                    drop(pipe);
                    let exit_code = child
                        .wait()
                        .map(|s| s.code().unwrap_or(-1))
                        .unwrap_or(-1);
                    log::info!(
                        "ProcessApp[{type_id}]: StreamProcess {corr_id} ended — exit_code={exit_code}"
                    );
                    let _ = tx.send(PlexiEvent::StreamEnd {
                        correlation_id: corr_id,
                        exit_code,
                    });
                    active.fetch_sub(1, Ordering::Relaxed);
                }).expect("failed to spawn stream-reader thread");
            }

            AppRequest::CancelProcess { correlation_id } => {
                if let Some(handle) = self.stream_handles.remove(&correlation_id) {
                    log::info!(
                        "ProcessApp[{}]: CancelProcess {correlation_id} — pid={}",
                        self.type_id,
                        handle.pid
                    );
                    handle.cancel.store(true, Ordering::Relaxed);
                    // SIGTERM first; reader thread exits when the pipe closes.
                    unsafe {
                        libc::kill(handle.pid as libc::pid_t, libc::SIGTERM);
                    }
                    // Escalate to SIGKILL after a 1s grace period on a
                    // background thread so the UI never blocks.
                    let pid = handle.pid;
                    std::thread::Builder::new()
                        .name(format!("sigkill-{pid}"))
                        .spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        unsafe {
                            libc::kill(pid as libc::pid_t, libc::SIGKILL);
                        }
                    }).expect("failed to spawn sigkill thread");
                }
                // else: stream already ended — no-op, no error event.
            }

            // ── File picker (#514) ─────────────────────────────────────────
            AppRequest::OpenFilePicker { request_id, filter, multiple } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::FsPick)
                {
                    log::warn!(
                        "ProcessApp[{}]: OpenFilePicker {request_id} denied — {reason}",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::FilePickCancelled { request_id });
                    return;
                }
                log::debug!(
                    "ProcessApp[{}]: OpenFilePicker {request_id} filter={filter:?} multiple={multiple}",
                    self.type_id
                );
                let tx = self.file_picker_tx.clone();
                let type_id = self.type_id.clone();
                std::thread::Builder::new()
                    .name(format!("file-picker-{type_id}"))
                    .spawn(move || {
                    let result = pick_files(&filter, multiple);
                    log::debug!("ProcessApp[{type_id}]: OpenFilePicker {request_id} → {result:?}");
                    let event = match result {
                        Some(paths) if !paths.is_empty() => {
                            PlexiEvent::FilePicked { request_id, paths }
                        }
                        _ => PlexiEvent::FilePickCancelled { request_id },
                    };
                    let _ = tx.send(event);
                }).expect("failed to spawn file-picker thread");
            }

            // ── Mouse tracking toggle ──────────────────────────────────────
            AppRequest::SetMouseTracking { enabled } => {
                self.mouse_tracking_enabled = enabled;
                log::debug!(
                    "ProcessApp[{}]: SetMouseTracking → {}",
                    self.type_id,
                    enabled
                );
            }

            // ── Spawn app ──────────────────────────────────────────────────
            AppRequest::SpawnApp {
                type_id,
                layout,
                args,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::SpawnApp)
                {
                    log::warn!("ProcessApp[{}]: SpawnApp denied — {reason}", self.type_id);
                    return;
                }
                log::info!(
                    "ProcessApp[{}]: SpawnApp type_id='{type_id}' layout={layout:?} args={args:?}",
                    self.type_id
                );
                self.pending_commands.push(AppCommand::SpawnApp {
                    type_id,
                    layout,
                    args,
                });
            }

            // ── Spawn pane (#592) ──────────────────────────────────────────────────────
            AppRequest::SpawnPane {
                type_id,
                layout,
                args,
                pipe_id,
                from_pane_id,
                request_id,
                response_file: _,
                ephemeral: _,
                cwd: _,
                no_focus: _,
                path: _,
                workspace_root: _,
                target_context,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::PanesSpawn)
                {
                    log::warn!("ProcessApp[{}]: SpawnPane denied — {reason}", self.type_id);
                    self.outbound_events
                        .push_back(PlexiEvent::PaneSpawnError { reason, request_id });
                    return;
                }
                let layout_str = layout.unwrap_or_else(|| "overlay".to_string());
                log::info!(
                    "ProcessApp[{}]: SpawnPane type_id='{type_id}' layout='{layout_str}' args={args:?} pipe_id={pipe_id:?} from_pane_id={from_pane_id:?} request_id={request_id:?} target_context={target_context:?}",
                    self.type_id
                );
                self.pending_commands.push(AppCommand::SpawnPane {
                    type_id,
                    layout: layout_str,
                    args,
                    pipe_id,
                    from_pane_id,
                    request_id,
                    target_context,
                });
            }

            // ── HTTP request (broker via HostServices::net) ───────────────
            AppRequest::HttpRequest {
                request_id,
                url,
                method,
                headers,
                body,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::NetHttp)
                {
                    log::warn!(
                        "ProcessApp[{}]: HttpRequest {request_id} denied — {reason}",
                        self.type_id
                    );
                    self.outbound_events
                        .push_back(PlexiEvent::HttpResponse {
                            request_id,
                            status: 403,
                            body: String::new(),
                            error: Some(format!("capability_denied: {reason}")),
                        });
                    return;
                }

                // Validate and normalize the initial URL host.
                let initial_host = match extract_host_normalized(&url) {
                    Some(h) => h,
                    None => {
                        log::warn!(
                            "ProcessApp[{}]: HttpRequest {request_id} denied — malformed URL or unsupported scheme '{url}'",
                            self.type_id
                        );
                        self.outbound_events.push_back(PlexiEvent::HttpResponse {
                            request_id,
                            status: 400,
                            body: String::new(),
                            error: Some("invalid_url: malformed URL or unsupported scheme".to_string()),
                        });
                        return;
                    }
                };

                // Pre-normalize allowed_hosts patterns once for initial check and all redirect hops.
                let normalized_allowed_hosts: Vec<String> = self.permissions.allowed_hosts.iter()
                    .map(|p| p.to_lowercase().trim_end_matches('.').to_string())
                    .collect();

                // Check initial URL against allowed_hosts.
                if !normalized_allowed_hosts.is_empty() {
                    let allowed = normalized_allowed_hosts.iter().any(|pattern| host_matches(&initial_host, pattern));
                    if !allowed {
                        log::warn!(
                            "ProcessApp[{}]: HttpRequest {request_id} denied — host '{}' not in allowed_hosts",
                            self.type_id, initial_host
                        );
                        self.outbound_events.push_back(PlexiEvent::HttpResponse {
                            request_id,
                            status: 403,
                            body: String::new(),
                            error: Some(format!("host_not_allowed: '{}' is not in this app's allowed_hosts list", initial_host)),
                        });
                        return;
                    }
                }

                log::info!(
                    "ProcessApp[{}]: HttpRequest {request_id} {method} {url}",
                    self.type_id
                );

                let allowed_hosts = normalized_allowed_hosts;
                let net = std::sync::Arc::clone(&self.net);
                let tx = self.http_tx.clone();
                let type_id = self.type_id.clone();

                std::thread::Builder::new()
                    .name(format!("http-{type_id}-{request_id}"))
                    .spawn(move || {
                        let mut current_url = url.clone();
                        let mut current_method = method.clone();
                        let mut current_body = body.clone();
                        const MAX_REDIRECTS: u8 = 5;

                        for hop in 0..=MAX_REDIRECTS {
                            let resp = net.http(&current_method, &current_url, &headers, current_body.as_deref());

                            // Check if this is a redirect (3xx)
                            let is_redirect = resp.status >= 300 && resp.status < 400;
                            if !is_redirect || hop == MAX_REDIRECTS {
                                log::debug!(
                                    "ProcessApp[{type_id}]: HttpRequest {request_id} → {} (hop {hop})",
                                    resp.status
                                );
                                let _ = tx.send(PlexiEvent::HttpResponse {
                                    request_id,
                                    status: resp.status,
                                    body: resp.body,
                                    error: resp.error,
                                });
                                return;
                            }

                            // Extract Location header from redirect (take first value)
                            let location = resp.response_headers
                                .get("location")
                                .and_then(|v| v.first())
                                .cloned();

                            let Some(next_url) = location else {
                                log::warn!(
                                    "ProcessApp[{type_id}]: HttpRequest {request_id} — redirect {} with no Location header",
                                    resp.status
                                );
                                let _ = tx.send(PlexiEvent::HttpResponse {
                                    request_id,
                                    status: resp.status,
                                    body: resp.body,
                                    error: resp.error,
                                });
                                return;
                            };

                            // Resolve the redirect URL (handles both absolute and relative)
                            // Url::join handles absolute URLs correctly too, so no special-casing needed.
                            let next_url = match Url::parse(&current_url).and_then(|base| base.join(&next_url)) {
                                Ok(resolved) => resolved.to_string(),
                                Err(_) => {
                                    log::warn!(
                                        "ProcessApp[{type_id}]: HttpRequest {request_id} — cannot resolve redirect URL '{next_url}'"
                                    );
                                    let _ = tx.send(PlexiEvent::HttpResponse {
                                        request_id,
                                        status: 400,
                                        body: String::new(),
                                        error: Some("invalid_redirect: cannot resolve redirect URL".to_string()),
                                    });
                                    return;
                                }
                            };

                            // Re-validate the redirect destination against allowed_hosts
                            let redirect_host = match extract_host_normalized(&next_url) {
                                Some(h) => h,
                                None => {
                                    log::warn!(
                                        "ProcessApp[{type_id}]: HttpRequest {request_id} — redirect to malformed URL '{next_url}'"
                                    );
                                    let _ = tx.send(PlexiEvent::HttpResponse {
                                        request_id,
                                        status: 403,
                                        body: String::new(),
                                        error: Some("invalid_redirect: malformed redirect URL".to_string()),
                                    });
                                    return;
                                }
                            };

                            if !allowed_hosts.is_empty() {
                                let allowed = allowed_hosts.iter().any(|pattern| host_matches(&redirect_host, pattern));
                                if !allowed {
                                    log::warn!(
                                        "ProcessApp[{type_id}]: HttpRequest {request_id} — redirect to '{}' denied, not in allowed_hosts",
                                        redirect_host
                                    );
                                    let _ = tx.send(PlexiEvent::HttpResponse {
                                        request_id,
                                        status: 403,
                                        body: String::new(),
                                        error: Some(format!("host_not_allowed: redirect destination '{}' is not in allowed_hosts", redirect_host)),
                                    });
                                    return;
                                }
                            }

                            log::info!(
                                "ProcessApp[{type_id}]: HttpRequest {request_id} — redirect {}/{MAX_REDIRECTS} to '{next_url}' (hop {hop})",
                                hop + 1
                            );

                            // For 303 See Other, always use GET with no body
                            if resp.status == 303 {
                                current_method = "GET".to_string();
                                current_body = None;
                            }
                            current_url = next_url;
                        }
                    }).expect("failed to spawn http thread");
            }
            // ── ai.query broker (#284) ─────────────────────────────────────
            AppRequest::AiQuery {
                request_id,
                model_tier,
                system,
                messages,
                tools,
            } => {
                if let PermissionCheck::Denied(_reason) =
                    check(&self.permissions, Capability::AiQuery)
                {
                    if is_blocked(&self.permissions, Capability::AiQuery) {
                        // Permanently blocked by user — deny immediately.
                        log::warn!(
                            "ProcessApp[{}]: AiQuery {request_id} denied — ai.query permanently blocked",
                            self.type_id
                        );
                        self.outbound_events.push_back(PlexiEvent::AiResponse {
                            request_id,
                            content: None,
                            tokens_in: 0,
                            tokens_out: 0,
                            error: Some("capability denied: ai.query blocked".to_string()),
                        });
                    } else {
                        // Withheld — capability declared but first-run consent not yet given.
                        // Defer the query and trigger the consent modal.
                        // Cap the deferred queue to prevent thread exhaustion when consent
                        // is slow or the app fires many queries before the modal is shown.
                        if self.deferred_ai_queries.len() >= 16 {
                            log::warn!(
                                "ProcessApp[{}]: AiQuery {request_id} dropped — deferred queue full (>=16 pending consent)",
                                self.type_id
                            );
                            self.outbound_events.push_back(PlexiEvent::AiResponse {
                                request_id,
                                content: None,
                                tokens_in: 0,
                                tokens_out: 0,
                                error: Some(
                                    "capability withheld: too many deferred queries pending consent"
                                        .to_string(),
                                ),
                            });
                            return;
                        }
                        log::info!(
                            "ProcessApp[{}]: AiQuery {request_id} withheld — queuing for ai.query consent",
                            self.type_id
                        );
                        self.deferred_ai_queries.push_back(super::DeferredAiQuery {
                            request_id,
                            model_tier,
                            system,
                            messages,
                            tools,
                        });
                        // Deduplicate: only push a prompt if one isn't already pending.
                        let already_pending = self.pending_prompts.iter().any(|p| {
                            matches!(p, super::PendingPrompt::Capability { capability, .. }
                                if capability == "ai.query")
                        });
                        if !already_pending {
                            self.pending_prompts.push_back(super::PendingPrompt::Capability {
                                request_id: uuid::Uuid::new_v4().to_string(),
                                capability: "ai.query".to_string(),
                            });
                        }
                    }
                    return;
                }

                log::debug!(
                    "ProcessApp[{}]: AiQuery {request_id} tier={:?} messages={} tools={}",
                    self.type_id,
                    model_tier,
                    messages.len(),
                    tools.len()
                );

                let broker = self.ai_broker.clone();
                let app_id = self.type_id.clone();
                let tx = self.http_tx.clone();
                let workspace_root = self.workspace_root.clone();
                let open_panes = crate::plexi_ai::broker::get_pane_snapshot();
                let tool_dispatcher = std::sync::Arc::new(
                    crate::plexi_ai::tool_dispatch::ToolDispatcher::from_registry(
                        self.pane_id,
                        self.type_id.clone(),
                        workspace_root.clone(),
                    ),
                );
                std::thread::Builder::new()
                    .name(format!("ai-query-{app_id}-{request_id}"))
                    .spawn(move || {
                    let resp = broker.dispatch(AiBrokerRequest {
                        app_id,
                        model_tier,
                        system,
                        messages,
                        tools,
                        workspace_root: Some(workspace_root),
                        open_panes,
                        tool_dispatcher: Some(tool_dispatcher),
                    });
                    // Send incremental stream chunks before the final AiResponse
                    // so the app can display tokens as they arrive.
                    let n_chunks = resp.stream_deltas.len();
                    for (i, delta) in resp.stream_deltas.into_iter().enumerate() {
                        let done = i + 1 == n_chunks;
                        let chunk = PlexiEvent::AiStreamChunk {
                            request_id: request_id.clone(),
                            delta,
                            done,
                        };
                        if let Err(e) = tx.send(chunk) {
                            log::warn!("ai broker: stream chunk receiver dropped: {e}");
                            return;
                        }
                    }
                    let event = PlexiEvent::AiResponse {
                        request_id,
                        content: resp.content,
                        tokens_in: resp.tokens_in,
                        tokens_out: resp.tokens_out,
                        error: resp.error,
                    };
                    if let Err(e) = tx.send(event) {
                        log::warn!("ai broker: response receiver dropped: {e}");
                    }
                }).expect("failed to spawn ai-query thread");
            }

            AppRequest::AudioPlay {
                source,
                pipe_id,
                volume,
                state,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::AudioPlayback)
                {
                    log::warn!(
                        "ProcessApp[{}]: AudioPlay denied — {reason}",
                        self.type_id
                    );
                    return;
                }

                log::info!(
                    "ProcessApp[{}]: AudioPlay state={state} source={source:?} pipe_id={pipe_id:?} volume={volume}",
                    self.type_id
                );

                match state.as_str() {
                    "playing" => {
                        if let Some(src) = source.as_deref().filter(|s| !s.is_empty()) {
                            // If a session already exists for this source (paused), resume it.
                            if let Some(session) = self.audio_playback_sessions.get(src) {
                                session.set_volume(volume);
                                session.resume();
                                log::info!(
                                    "ProcessApp[{}]: AudioPlay resumed: source={src}",
                                    self.type_id
                                );
                            } else {
                            let req = crate::audio::PlaybackRequest {
                                source: src.to_owned(),
                                volume,
                            };
                            match crate::audio::start_playback(req) {
                                Ok(session) => {
                                    log::info!(
                                        "ProcessApp[{}]: AudioPlay started: source={src}",
                                        self.type_id
                                    );
                                    self.audio_playback_sessions.insert(src.to_owned(), session);
                                }
                                Err(e) => {
                                    log::warn!(
                                        "ProcessApp[{}]: AudioPlay start failed for {src:?}: {e}",
                                        self.type_id
                                    );
                                }
                            }
                            }
                        } else if pipe_id.is_some() {
                            log::warn!(
                                "ProcessApp[{}]: AudioPlay pipe_id playback not yet implemented",
                                self.type_id
                            );
                        } else {
                            log::warn!(
                                "ProcessApp[{}]: AudioPlay: neither source nor pipe_id provided",
                                self.type_id
                            );
                        }
                    }
                    "paused" => {
                        if let Some(src) = source.as_deref().filter(|s| !s.is_empty()) {
                            if let Some(session) = self.audio_playback_sessions.get(src) {
                                session.pause();
                                log::info!(
                                    "ProcessApp[{}]: AudioPlay paused: source={src}",
                                    self.type_id
                                );
                            } else {
                                log::warn!(
                                    "ProcessApp[{}]: AudioPlay pause: no session for source={src:?}",
                                    self.type_id
                                );
                            }
                        }
                    }
                    "stopped" => {
                        let key = source.as_deref().unwrap_or("").to_owned();
                        if self.audio_playback_sessions.remove(&key).is_some() {
                            log::info!(
                                "ProcessApp[{}]: AudioPlay stopped: source={key:?}",
                                self.type_id
                            );
                        } else {
                            log::warn!(
                                "ProcessApp[{}]: AudioPlay stop: no session for source={key:?}",
                                self.type_id
                            );
                        }
                    }
                    other => {
                        log::warn!(
                            "ProcessApp[{}]: AudioPlay: unknown state {other:?} (expected playing|paused|stopped)",
                            self.type_id
                        );
                    }
                }
            }
            AppRequest::AudioCapture {
                pipe_id,
                device_id,
                sample_rate,
                buffer_size,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::AudioRecord)
                {
                    log::warn!(
                        "ProcessApp[{}]: AudioCapture denied — {reason}",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::AudioCaptureError {
                        pipe_id,
                        error: format!("capability denied: {reason}"),
                    });
                    return;
                }
                self.start_audio_capture(pipe_id, device_id, sample_rate, buffer_size);
            }
            AppRequest::ListAudioDevices { request_id } => {
                // Enumeration is not gated — device names are already visible
                // to any macOS app via System Information. The per-device
                // capture call goes through the `AudioRecord` gate.
                // Offload to a background thread: cpal device enumeration can
                // block for hundreds of milliseconds on macOS (Bluetooth scan).
                let audio = std::sync::Arc::clone(&self.audio_device);
                let tx = self.http_tx.clone();
                let type_id = self.type_id.clone();
                std::thread::Builder::new()
                    .name(format!("audio-devices-{type_id}"))
                    .spawn(move || {
                    let inputs = audio
                        .list_input_devices()
                        .into_iter()
                        .map(AudioDeviceWire::from)
                        .collect();
                    let outputs = audio
                        .list_output_devices()
                        .into_iter()
                        .map(AudioDeviceWire::from)
                        .collect();
                    log::info!("ProcessApp[{type_id}]: ListAudioDevices complete");
                    let _ = tx.send(PlexiEvent::AudioDevicesListed {
                        request_id,
                        inputs,
                        outputs,
                        error: None,
                    });
                }).expect("failed to spawn audio-devices thread");
            }
            AppRequest::ListMidiDevices { request_id } => {
                // MIDI enumeration mirrors audio: not gated. Port names are
                // already visible in Audio MIDI Setup.app, no privacy gate.
                // Offload to a background thread for the same reason as audio.
                let midi = std::sync::Arc::clone(&self.midi_device);
                let tx = self.http_tx.clone();
                let type_id = self.type_id.clone();
                std::thread::Builder::new()
                    .name(format!("midi-devices-{type_id}"))
                    .spawn(move || {
                    let inputs = midi
                        .list_input_ports()
                        .into_iter()
                        .map(MidiPortWire::from)
                        .collect();
                    let outputs = midi
                        .list_output_ports()
                        .into_iter()
                        .map(MidiPortWire::from)
                        .collect();
                    log::info!("ProcessApp[{type_id}]: ListMidiDevices complete");
                    let _ = tx.send(PlexiEvent::MidiDevicesListed {
                        request_id,
                        inputs,
                        outputs,
                        error: None,
                    });
                }).expect("failed to spawn midi-devices thread");
            }
            AppRequest::OpenMidiInput { port_id, pipe_id } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::MidiIn)
                {
                    log::warn!(
                        "ProcessApp[{}]: OpenMidiInput denied — {reason}",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::MidiInputError {
                        pipe_id,
                        error: format!("capability denied: {reason}"),
                    });
                    return;
                }
                self.start_midi_input(port_id, pipe_id);
            }
            AppRequest::CloseMidiInput { port_id } => {
                // Drop the session — its Drop impl disconnects from CoreMIDI.
                // No response event; closing is fire-and-forget.
                if self.midi_input_sessions.remove(&port_id).is_some() {
                    log::info!(
                        "app::{} midi.input closed: port_id={port_id}",
                        self.type_id
                    );
                } else {
                    log::debug!(
                        "ProcessApp[{}]: CloseMidiInput on inactive port_id={port_id} (no-op)",
                        self.type_id
                    );
                }
            }
            AppRequest::SendMidi { port_id, bytes } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::MidiOut)
                {
                    log::warn!(
                        "ProcessApp[{}]: SendMidi denied — {reason}",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::MidiSendError {
                        port_id,
                        error: format!("capability denied: {reason}"),
                    });
                    return;
                }
                self.send_midi(port_id, bytes);
            }

            AppRequest::OpenVideo {
                request_id,
                source,
                pipe_id,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::VideoPlayback)
                {
                    log::warn!(
                        "ProcessApp[{}]: OpenVideo denied — {reason}",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::VideoOpenError {
                        request_id,
                        error: format!("capability denied: {reason}"),
                    });
                    return;
                }
                self.start_video(request_id, source, pipe_id);
            }
            AppRequest::SetVideoState { handle_id, state } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::VideoPlayback)
                {
                    log::warn!(
                        "ProcessApp[{}]: SetVideoState denied — {reason}",
                        self.type_id
                    );
                    return;
                }
                match self.video_handles.get_mut(&handle_id) {
                    Some(h) => {
                        if let Err(e) = h.set_state(state) {
                            log::warn!(
                                "ProcessApp[{}]: SetVideoState handle_id={handle_id} failed: {e}",
                                self.type_id
                            );
                        }
                    }
                    None => {
                        log::warn!(
                            "ProcessApp[{}]: SetVideoState on unknown handle_id={handle_id}",
                            self.type_id
                        );
                    }
                }
            }
            AppRequest::CloseVideo { handle_id } => {
                // Drop the handle (its Drop impl tears down the worker /
                // decoder) and close the associated binary pipe so the
                // drain thread exits. Fire-and-forget — no response event.
                if let Some(h) = self.video_handles.remove(&handle_id) {
                    drop(h);
                    if let Some(pipe_id) = self.video_pipe_ids.remove(&handle_id) {
                        self.pipe_registry
                            .lock()
                            .expect("pipe_registry poisoned")
                            .close(&pipe_id);
                    }
                    log::info!(
                        "app::{} video.close: handle_id={handle_id}",
                        self.type_id
                    );
                } else {
                    log::debug!(
                        "ProcessApp[{}]: CloseVideo on inactive handle_id={handle_id} (no-op)",
                        self.type_id
                    );
                }
            }


            // ── Cd request ─────────────────────────────────────────────────
            AppRequest::CdRequest { cwd } => {
                log::info!("ProcessApp[{}]: CdRequest cwd='{cwd}'", self.type_id);
                self.pending_commands.push(AppCommand::CdRequest { cwd, sender_pane_id: self.pane_id });
            }

            // ── Async image load ───────────────────────────────────────────
            AppRequest::LoadImage { handle, src } => {
                let net_http_granted = self.permissions.is_builtin
                    || self.permissions.capabilities.contains(&Capability::NetHttp);
                log::info!(
                    "ProcessApp[{}]: LoadImage handle={handle} src={src}",
                    self.type_id
                );
                self.image_cache.request_by_handle(&handle, &src, net_http_granted);
                // Immediate error completions (e.g. capability denied) were pushed to
                // image_cache.immediate_completions and will be drained in the next poll().
            }

            // ── Set timer ──────────────────────────────────────────────────
            AppRequest::SetTimer { timer_id, after_ms } => {
                if let PermissionCheck::Denied(reason) = check(&self.permissions, Capability::Timer) {
                    log::warn!("ProcessApp[{}]: SetTimer {timer_id} denied — {reason}", self.type_id);
                    return;
                }
                let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.pending_timers.insert(timer_id.clone(), std::sync::Arc::clone(&cancelled));
                let tx = self.http_tx.clone();
                let type_id = self.type_id.clone();
                std::thread::Builder::new()
                    .name(format!("timer-{type_id}-{timer_id}"))
                    .spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(after_ms));
                    if !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        log::debug!("ProcessApp[{type_id}]: timer {timer_id} fired");
                        let _ = tx.send(PlexiEvent::Timer { timer_id });
                    }
                }).expect("failed to spawn timer thread");
            }

            // ── Cancel timer ───────────────────────────────────────────────
            AppRequest::CancelTimer { timer_id } => {
                if let Some(flag) = self.pending_timers.remove(&timer_id) {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    log::debug!("ProcessApp[{}]: timer {timer_id} cancelled", self.type_id);
                }
            }

            // ── Canvas Terminal Binding Primitives (#78) ───────────────────
            // All five gate on `terminal.bindings`. The host-side state
            // (creating the terminal pane, finding the linked terminal,
            // injecting bytes into its PTY) lives at the `PlexiApp`
            // dispatch site — the routing layer just enforces the
            // capability gate, logs, and forwards an `AppCommand`.
            AppRequest::RequestLinkedTerminal { request_id, cwd, label } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::TerminalBindings)
                {
                    log::warn!(
                        "ProcessApp[{}]: RequestLinkedTerminal {request_id} denied — {reason}",
                        self.type_id
                    );
                    // No event-shape on the wire for "denied" on this call —
                    // mirror the iq.query precedent: respond with an explicit
                    // error event so the SDK's blocking helper unblocks.
                    self.outbound_events.push_back(
                        PlexiEvent::LinkedTerminalReady {
                            request_id,
                            // pane_id 0 sentinel = "no terminal opened". The
                            // SDK helper checks for 0 and raises
                            // CapabilityDeniedError.
                            terminal_pane_id: 0,
                        },
                    );
                    return;
                }
                self.pending_commands.push(AppCommand::RequestLinkedTerminal {
                    sender_pane_id: self.pane_id,
                    request_id,
                    cwd,
                    label,
                });
            }
            AppRequest::RunInLinkedTerminal { terminal_pane_id, command, echo } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::TerminalBindings)
                {
                    log::warn!(
                        "ProcessApp[{}]: RunInLinkedTerminal denied — {reason}",
                        self.type_id
                    );
                    return;
                }
                self.pending_commands.push(AppCommand::RunInLinkedTerminal {
                    sender_pane_id: self.pane_id,
                    terminal_pane_id,
                    command,
                    echo,
                });
            }
            AppRequest::InsertPathToken { terminal_pane_id, path, mode } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::TerminalBindings)
                {
                    log::warn!(
                        "ProcessApp[{}]: InsertPathToken denied — {reason}",
                        self.type_id
                    );
                    return;
                }
                self.pending_commands.push(AppCommand::InsertPathToken {
                    sender_pane_id: self.pane_id,
                    terminal_pane_id,
                    path,
                    mode,
                });
            }
            AppRequest::RequestCommandPreview { request_id, terminal_pane_id, command } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::TerminalBindings)
                {
                    log::warn!(
                        "ProcessApp[{}]: RequestCommandPreview {request_id} denied — {reason}",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::CommandPreview {
                        request_id,
                        command,
                        would_run_in_cwd: String::new(),
                    });
                    return;
                }
                self.pending_commands.push(AppCommand::RequestCommandPreview {
                    sender_pane_id: self.pane_id,
                    request_id,
                    terminal_pane_id,
                    command,
                });
            }
            AppRequest::OpenArtifact { path, mode } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::TerminalBindings)
                {
                    log::warn!(
                        "ProcessApp[{}]: OpenArtifact denied — {reason}",
                        self.type_id
                    );
                    return;
                }
                self.pending_commands.push(AppCommand::OpenArtifact {
                    sender_pane_id: self.pane_id,
                    path,
                    mode,
                });
            }

            // ── Tool protocol (#398, #399) ─────────────────────────────────

            // App declares its callable tools to the host.
            AppRequest::ExposeTools { tools } => {
                log::debug!(
                    "ProcessApp[{}]: ExposeTools {} tool(s)",
                    self.type_id,
                    tools.len(),
                );
                self.exposed_tools = tools.clone();
                // Defer registration until set_pane_id assigns the real id.
                // Registering under pane_id=0 (the default before set_pane_id)
                // causes a race when multiple apps launch simultaneously.
                if self.pane_id != 0 {
                    if let Some(sender) = self.make_app_event_sender() {
                        crate::plexi_ai::tool_dispatch::register(
                            self.pane_id,
                            tools,
                            sender,
                            self.workspace_root.clone(),
                        );
                    } else {
                        log::warn!(
                            "ProcessApp[{}]: ExposeTools — stdin writer gone, skipping registration",
                            self.type_id
                        );
                    }
                }
            }

            // App returns the result of a ToolCall from the broker.
            AppRequest::ToolResult {
                call_id,
                output_json,
                error,
            } => {
                log::debug!(
                    "ProcessApp[{}]: ToolResult call_id={call_id:?} ok={}",
                    self.type_id,
                    output_json.is_some(),
                );
                crate::plexi_ai::tool_dispatch::resolve_pending(
                    &call_id,
                    crate::plexi_ai::tool_dispatch::ToolCallResult {
                        output_json,
                        error,
                    },
                );
            }

            // App returns the result of an McpToolCall from an external MCP client.
            AppRequest::McpToolResult {
                call_id,
                result,
                error,
            } => {
                log::info!(
                    "ProcessApp[{}]: McpToolResult call_id={call_id:?} ok={}",
                    self.type_id,
                    result.is_some(),
                );
                if let Some(tx) = self.mcp_pending.remove(&call_id) {
                    let _ = tx.send(crate::process_app::mcp_server::McpToolResponse {
                        result,
                        error,
                    });
                } else {
                    log::warn!(
                        "ProcessApp[{}]: McpToolResult for unknown call_id={call_id:?}",
                        self.type_id
                    );
                }
            }

            // SetPaneTitle is only valid over PLEXI_SOCKET; an app sending it
            // via PGAP is a protocol error — log and drop.
            AppRequest::SetPaneTitle { pane_id, .. } => {
                log::warn!(
                    "ProcessApp[{}]: received SetPaneTitle for pane_id={pane_id} over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }

            // Context commands are only valid over PLEXI_SOCKET; log and drop
            // when sent via PGAP.
            AppRequest::CreateContext { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received CreateContext over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::FocusContext { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received FocusContext over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::SetContextRoot { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received SetContextRoot over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::SetContextDescription { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received SetContextDescription over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::ZoomIntoContext { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received ZoomIntoContext over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::ZoomOutOfContext => {
                log::warn!(
                    "ProcessApp[{}]: received ZoomOutOfContext over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::ListPanes { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received ListPanes over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::FocusPane { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received FocusPane over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::ClosePane { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received ClosePane over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::SendToPane { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received SendToPane over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }
            AppRequest::KeyPane { .. } => {
                log::warn!(
                    "ProcessApp[{}]: received KeyPane over PGAP — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }

            // ── App state save ─────────────────────────────────────────────
            AppRequest::SaveAppState { payload } => {
                let filename = format!("{}.json", self.type_id);
                let workspace_toml = self.workspace_root.join(".plexi").join("workspace.toml");
                let state_path = if workspace_toml.exists() {
                    self.workspace_root.join(".plexi").join("app_states").join(&filename)
                } else {
                    crate::config::config_dir().join("app_states").join(&filename)
                };
                if let Some(dir) = state_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(dir) {
                        log::warn!("ProcessApp[{}]: SaveAppState: mkdir failed: {e}", self.type_id);
                        return;
                    }
                }
                match serde_json::to_vec_pretty(&payload) {
                    Ok(bytes) => {
                        if let Err(e) = std::fs::write(&state_path, &bytes) {
                            log::warn!("ProcessApp[{}]: SaveAppState: write failed: {e}", self.type_id);
                        } else {
                            log::info!(
                                "ProcessApp[{}]: saved app state to {}",
                                self.type_id,
                                state_path.display()
                            );
                        }
                    }
                    Err(e) => {
                        log::warn!("ProcessApp[{}]: SaveAppState: serialize failed: {e}", self.type_id);
                    }
                }
            }
            AppRequest::GetPaneInfo { pane_id, .. } => {
                log::warn!(
                    "ProcessApp[{}]: GetPaneInfo pane_id={pane_id} received in app routing — ignored (host-only command)",
                    self.type_id
                );
            }
            AppRequest::CapturePane { pane_id, .. } => {
                log::warn!(
                    "ProcessApp[{}]: CapturePane pane_id={pane_id} received in app routing — ignored (use PLEXI_SOCKET instead)",
                    self.type_id
                );
            }

            // ── Query context state (#1518) ───────────────────────────────
            AppRequest::QueryContextState { context_id } => {
                log::info!(
                    "ProcessApp[{}]: QueryContextState context_id={context_id}",
                    self.type_id
                );
                self.pending_commands.push(AppCommand::QueryContextState {
                    sender_pane_id: self.pane_id,
                    context_id,
                });
            }
        }
    }

    /// Allocate a binary pipe for `pipe_id`, spin up the audio capture
    /// stream, and wire the cpal callback to push f32 PCM frames into the
    /// pipe ring. On any failure emits `PlexiEvent::AudioCaptureError` and
    /// frees the pipe so the app's `pipe_open` queue stays consistent.
    pub(super) fn start_audio_capture(
        &mut self,
        pipe_id: String,
        device_id: Option<String>,
        sample_rate: u32,
        buffer_size: u32,
    ) {
        if self.audio_capture_sessions.remove(&pipe_id).is_some() {
            // Stale session — pipe broke without a clean stop (e.g. EBADF on
            // the app side). Drop the old CaptureSession (stops the cpal
            // stream) and close the pipe before re-opening.
            log::warn!(
                "ProcessApp[{}]: AudioCapture pipe_id={pipe_id} stale session — cleaning up and restarting",
                self.type_id
            );
            self.pipe_registry
                .lock()
                .expect("pipe_registry poisoned")
                .close(&pipe_id);
            if let Ok(mut m) = self.audio_peak_meters.lock() {
                m.remove(&pipe_id);
            }
        }

        // Allocate the binary pipe first — without a destination the cpal
        // callback would have nowhere to push frames. The pipe-allocation
        // failure path returns early so we never start a stream we can't
        // drain.
        let socket_path = match self
            .pipe_registry
            .lock()
            .expect("pipe_registry poisoned")
            .open_binary(pipe_id.clone(), PipeDirection::In)
        {
            Ok(alloc) => alloc.socket_path,
            Err(e) => {
                log::warn!(
                    "ProcessApp[{}]: AudioCapture pipe alloc failed for {pipe_id}: {e}",
                    self.type_id
                );
                self.outbound_events.push_back(PlexiEvent::AudioCaptureError {
                    pipe_id,
                    error: format!("pipe alloc failed: {e}"),
                });
                return;
            }
        };

        // Grab a clone of the ring so the cpal callback can push without
        // touching the registry mutex on the audio thread.
        let ring = match self
            .pipe_registry
            .lock()
            .expect("pipe_registry poisoned")
            .binary_ring(&pipe_id)
        {
            Some(r) => r,
            None => {
                // Should not happen — we just opened it. Defensive cleanup.
                self.pipe_registry
                    .lock()
                    .expect("pipe_registry poisoned")
                    .close(&pipe_id);
                self.outbound_events.push_back(PlexiEvent::AudioCaptureError {
                    pipe_id,
                    error: "pipe ring missing after open".to_owned(),
                });
                return;
            }
        };

        // Notify the app that the pipe is open BEFORE starting the stream
        // — apps connect to the unix socket on `pipe_opened`, and we want
        // them ready before the first cpal callback fires.
        self.outbound_events.push_back(PlexiEvent::PipeOpened {
            pipe_id: pipe_id.clone(),
            socket_path,
        });

        // Build the frame sink. Each callback invocation gets a fresh
        // `Vec<u8>` (4 × frames × channels) and tries to push it into the
        // ring. A full ring drops the frame; we don't emit a `PipeOverrun`
        // event from the audio thread to avoid contending on the outbound
        // queue mutex from a real-time context.
        let ring_for_cb = std::sync::Arc::clone(&ring);
        // Clone the Arc for peak tracking so the cpal thread can update
        // without touching the ProcessApp mutex.
        let peaks_for_meter = std::sync::Arc::clone(&self.audio_peak_meters);
        let pipe_id_for_meter = pipe_id.clone();
        let sink: crate::audio::FrameSink = std::sync::Arc::new(move |frames: &[f32]| {
            let mut buf = Vec::with_capacity(frames.len() * 4);
            for &s in frames {
                buf.extend_from_slice(&s.to_le_bytes());
            }
            // Push fails when the ring is full — drop the frame, signal
            // backpressure to the producer (cpal). cpal handles `Err(())`
            // by stopping the stream; we want to keep streaming through a
            // brief congestion, so always return Ok.
            let _ = ring_for_cb.push(buf);
            // Compute and store peak amplitude for AudioMeter rendering (#341).
            let peak = frames.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
            if let Ok(mut map) = peaks_for_meter.lock() {
                map.insert(pipe_id_for_meter.clone(), peak);
            }
            Ok(())
        });

        let request = AudioCaptureRequest {
            device_id,
            requested_sample_rate: sample_rate,
            requested_buffer_size: buffer_size,
        };

        match self.audio_device.start_capture(request, sink) {
            Ok(session) => {
                let neg = session.negotiated.clone();
                log::info!(
                    "app::{} audio.capture started: device={}, rate={}, channels={}, buffer={}",
                    self.type_id,
                    neg.device_name,
                    neg.sample_rate,
                    neg.channels,
                    neg.buffer_size,
                );
                self.audio_capture_sessions.insert(pipe_id.clone(), session);
                self.outbound_events
                    .push_back(PlexiEvent::AudioCaptureStarted {
                        pipe_id,
                        sample_rate: neg.sample_rate,
                        channels: neg.channels,
                        buffer_size: neg.buffer_size,
                        device_name: neg.device_name,
                    });
            }
            Err(e) => {
                log::warn!(
                    "ProcessApp[{}]: AudioCapture start failed for {pipe_id}: {e}",
                    self.type_id
                );
                // Capture failed — close the pipe so the app's bookkeeping
                // and the drain thread don't hang waiting for frames.
                self.pipe_registry
                    .lock()
                    .expect("pipe_registry poisoned")
                    .close(&pipe_id);
                self.outbound_events.push_back(PlexiEvent::AudioCaptureError {
                    pipe_id,
                    error: format!("{e}"),
                });
            }
        }
    }

    /// Allocate a binary pipe for `pipe_id`, open the requested CoreMIDI input
    /// port, and wire the CoreMIDI callback to push raw MIDI 1.0 byte streams
    /// into the pipe ring. On any failure emits `PlexiEvent::MidiInputError`
    /// and frees the pipe so the app's `pipe_open` queue stays consistent.
    pub(super) fn start_midi_input(&mut self, port_id: String, pipe_id: String) {
        if self.midi_input_sessions.contains_key(&port_id) {
            log::warn!(
                "ProcessApp[{}]: OpenMidiInput port_id={port_id} already open",
                self.type_id
            );
            self.outbound_events.push_back(PlexiEvent::MidiInputError {
                pipe_id,
                error: format!("port_id={port_id} already open"),
            });
            return;
        }

        // Allocate the binary pipe first — without a destination the CoreMIDI
        // callback would have nowhere to push frames.
        let socket_path = match self
            .pipe_registry
            .lock()
            .expect("pipe_registry poisoned")
            .open_binary(pipe_id.clone(), PipeDirection::In)
        {
            Ok(alloc) => alloc.socket_path,
            Err(e) => {
                log::warn!(
                    "ProcessApp[{}]: OpenMidiInput pipe alloc failed for {pipe_id}: {e}",
                    self.type_id
                );
                self.outbound_events.push_back(PlexiEvent::MidiInputError {
                    pipe_id,
                    error: format!("pipe alloc failed: {e}"),
                });
                return;
            }
        };

        let ring = match self
            .pipe_registry
            .lock()
            .expect("pipe_registry poisoned")
            .binary_ring(&pipe_id)
        {
            Some(r) => r,
            None => {
                self.pipe_registry
                    .lock()
                    .expect("pipe_registry poisoned")
                    .close(&pipe_id);
                self.outbound_events.push_back(PlexiEvent::MidiInputError {
                    pipe_id,
                    error: "pipe ring missing after open".to_owned(),
                });
                return;
            }
        };

        // Notify the app that the pipe is open BEFORE opening the CoreMIDI
        // source so the app can connect the unix socket before the first
        // MIDI byte arrives.
        self.outbound_events.push_back(PlexiEvent::PipeOpened {
            pipe_id: pipe_id.clone(),
            socket_path,
        });

        // Build the packet sink. CoreMIDI delivers callbacks on a real-time
        // thread; we copy each MIDI 1.0 byte stream into a fresh `Vec<u8>`
        // and push it into the ring. A full ring drops the frame.
        let ring_for_cb = std::sync::Arc::clone(&ring);
        let sink: crate::midi::MidiPacketSink =
            std::sync::Arc::new(move |bytes: &[u8]| {
                // Each MIDI message is its own pipe frame so the consumer
                // sees message boundaries — no ambiguity decoding back-to-back
                // 3-byte messages.
                let _ = ring_for_cb.push(bytes.to_vec());
                Ok(())
            });

        match self.midi_device.open_input(&port_id, sink) {
            Ok(session) => {
                let port_name = session.port_name.clone();
                log::info!(
                    "app::{} midi.input opened: port_id={port_id}, port_name={port_name}, pipe_id={pipe_id}",
                    self.type_id
                );
                self.midi_input_sessions.insert(port_id.clone(), session);
                self.outbound_events.push_back(PlexiEvent::MidiInputOpened {
                    pipe_id,
                    port_id,
                    port_name,
                });
            }
            Err(e) => {
                log::warn!(
                    "ProcessApp[{}]: OpenMidiInput failed for port_id={port_id}: {e}",
                    self.type_id
                );
                self.pipe_registry
                    .lock()
                    .expect("pipe_registry poisoned")
                    .close(&pipe_id);
                self.outbound_events.push_back(PlexiEvent::MidiInputError {
                    pipe_id,
                    error: format!("{e}"),
                });
            }
        }
    }

    /// Send one MIDI byte stream to `port_id`, opening the output handle
    /// lazily on the first send. Successful sends produce no event;
    /// failures emit `PlexiEvent::MidiSendError`.
    pub(super) fn send_midi(&mut self, port_id: String, bytes: Vec<u8>) {
        if !self.midi_output_handles.contains_key(&port_id) {
            match self.midi_device.open_output(&port_id) {
                Ok(handle) => {
                    log::info!(
                        "app::{} midi.output opened: port_id={port_id}, port_name={}",
                        self.type_id,
                        handle.port_name
                    );
                    self.midi_output_handles.insert(port_id.clone(), handle);
                }
                Err(e) => {
                    log::warn!(
                        "ProcessApp[{}]: SendMidi open_output failed for port_id={port_id}: {e}",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::MidiSendError {
                        port_id,
                        error: format!("{e}"),
                    });
                    return;
                }
            }
        }

        let handle = self
            .midi_output_handles
            .get_mut(&port_id)
            .expect("midi output handle just inserted");
        if let Err(e) = handle.send(&bytes) {
            log::warn!(
                "ProcessApp[{}]: SendMidi send failed for port_id={port_id}: {e}",
                self.type_id
            );
            self.outbound_events.push_back(PlexiEvent::MidiSendError {
                port_id,
                error: format!("{e}"),
            });
        }
    }

    /// Allocate a binary pipe for `pipe_id`, open the video decoder against
    /// `source`, and wire decoded RGBA8 frames into the pipe ring (#345).
    /// On any failure emits `PlexiEvent::VideoOpenError` and frees the pipe
    /// so the app's `pipe_open` queue stays consistent.
    pub(super) fn start_video(
        &mut self,
        request_id: String,
        source: String,
        pipe_id: String,
    ) {
        // Allocate the binary pipe first — without a destination the decoder
        // would have nowhere to push frames. Mirrors the audio / MIDI flow.
        let socket_path = match self
            .pipe_registry
            .lock()
            .expect("pipe_registry poisoned")
            .open_binary(pipe_id.clone(), PipeDirection::In)
        {
            Ok(alloc) => alloc.socket_path,
            Err(e) => {
                log::warn!(
                    "ProcessApp[{}]: OpenVideo pipe alloc failed for {pipe_id}: {e}",
                    self.type_id
                );
                self.outbound_events.push_back(PlexiEvent::VideoOpenError {
                    request_id,
                    error: format!("pipe alloc failed: {e}"),
                });
                return;
            }
        };

        let ring = match self
            .pipe_registry
            .lock()
            .expect("pipe_registry poisoned")
            .binary_ring(&pipe_id)
        {
            Some(r) => r,
            None => {
                self.pipe_registry
                    .lock()
                    .expect("pipe_registry poisoned")
                    .close(&pipe_id);
                self.outbound_events.push_back(PlexiEvent::VideoOpenError {
                    request_id,
                    error: "pipe ring missing after open".to_owned(),
                });
                return;
            }
        };

        // Notify the app that the pipe is open BEFORE the decoder starts so
        // the app can connect to the unix socket before the first frame.
        self.outbound_events.push_back(PlexiEvent::PipeOpened {
            pipe_id: pipe_id.clone(),
            socket_path,
        });

        match self.video_device.open(&source, std::sync::Arc::clone(&ring)) {
            Ok((ack, handle)) => {
                log::info!(
                    "app::{} video.open: handle_id={}, source={source:?}, {}x{} @ {} fps, duration_ms={}",
                    self.type_id,
                    ack.handle_id,
                    ack.width,
                    ack.height,
                    ack.fps,
                    ack.duration_ms,
                );
                self.video_handles.insert(ack.handle_id, handle);
                self.video_pipe_ids.insert(ack.handle_id, pipe_id);
                self.outbound_events.push_back(PlexiEvent::VideoOpenAck {
                    request_id,
                    handle_id: ack.handle_id,
                    width: ack.width,
                    height: ack.height,
                    fps: ack.fps,
                    duration_ms: ack.duration_ms,
                });
            }
            Err(e) => {
                log::warn!(
                    "ProcessApp[{}]: OpenVideo failed for source={source:?}: {e}",
                    self.type_id
                );
                self.pipe_registry
                    .lock()
                    .expect("pipe_registry poisoned")
                    .close(&pipe_id);
                self.outbound_events.push_back(PlexiEvent::VideoOpenError {
                    request_id,
                    error: format!("{e}"),
                });
            }
        }
    }
}

/// Extract and normalize the hostname from a URL string.
/// Returns `None` if the URL is malformed, uses an unsupported scheme,
/// or has no resolvable host.
fn extract_host_normalized(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    // Only http and https are supported
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return None,
    }
    // Use host() enum to get a bracket-free representation for IPv6 addresses.
    // host_str() includes brackets for IPv6 in url 2.x.
    let raw = match parsed.host()? {
        url::Host::Domain(d) => d.to_string(),
        url::Host::Ipv4(addr) => addr.to_string(),
        url::Host::Ipv6(addr) => addr.to_string(),
    };
    if raw.is_empty() {
        return None;
    }
    // Normalize: lowercase + strip trailing dot (DNS FQDN notation)
    let normalized = raw.to_lowercase();
    let normalized = normalized.trim_end_matches('.');
    if normalized.is_empty() {
        return None;
    }
    Some(normalized.to_string())
}

/// Check if `host` matches `pattern`.
/// `host` must already be normalized (lowercase, no trailing dot).
/// Supports exact match and `*.domain.com` wildcards.
/// `*.example.com` matches `sub.example.com` but NOT bare `example.com`.
fn host_matches(host: &str, pattern: &str) -> bool {
    let pattern_norm = pattern.to_lowercase();
    let pattern_norm = pattern_norm.trim_end_matches('.');
    if let Some(suffix) = pattern_norm.strip_prefix("*.") {
        // Wildcard: must have a dot-separated subdomain prefix
        // e.g. "sub.example.com" ends_with ".example.com"
        host.ends_with(&format!(".{suffix}"))
    } else {
        host == pattern_norm
    }
}

/// Show a native file picker dialog on macOS using rfd's async API.
/// Blocks the calling (background) thread; must NOT be called on the main thread.
/// Returns `Some(paths)` with the selected paths, or `None` if cancelled.
fn pick_files(filter: &[String], multiple: bool) -> Option<Vec<String>> {
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Wake, Waker};
    use std::pin::pin;

    // Minimal block_on that parks the current thread while a future is pending.
    // Works correctly with rfd::AsyncFileDialog, which resolves its future via
    // AppKit's completion handler fired on the main thread.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        struct Signal(Arc<(Mutex<bool>, Condvar)>);
        impl Wake for Signal {
            fn wake(self: Arc<Self>) { self.signal(); }
            fn wake_by_ref(self: &Arc<Self>) { self.signal(); }
        }
        impl Signal {
            fn signal(&self) {
                let (lock, cvar) = &*self.0;
                *lock.lock().unwrap() = true;
                cvar.notify_one();
            }
        }
        let waker: Waker = Arc::new(Signal(Arc::clone(&signal))).into();
        let mut cx = Context::from_waker(&waker);
        let mut f = pin!(f);
        loop {
            match f.as_mut().poll(&mut cx) {
                Poll::Ready(val) => return val,
                Poll::Pending => {
                    let (lock, cvar) = &*signal;
                    let mut ready = lock.lock().unwrap();
                    while !*ready { ready = cvar.wait(ready).unwrap(); }
                    *ready = false;
                }
            }
        }
    }

    let mut dialog = rfd::AsyncFileDialog::new();
    // Group all extensions under a single "Files" label.
    if !filter.is_empty() {
        let refs: Vec<&str> = filter.iter().map(|s| s.as_str()).collect();
        dialog = dialog.add_filter("Files", &refs);
    }

    if multiple {
        let handles = block_on(dialog.pick_files())?;
        Some(handles.into_iter().map(|h| h.path().to_string_lossy().into_owned()).collect())
    } else {
        let handle = block_on(dialog.pick_file())?;
        Some(vec![handle.path().to_string_lossy().into_owned()])
    }
}

#[cfg(test)]
mod allowed_hosts_tests {
    use super::*;

    #[test]
    fn extract_host_basic() {
        assert_eq!(extract_host_normalized("https://example.com/path"), Some("example.com".to_string()));
        assert_eq!(extract_host_normalized("http://example.com:8080/"), Some("example.com".to_string()));
    }

    #[test]
    fn extract_host_normalizes_case() {
        assert_eq!(extract_host_normalized("https://EXAMPLE.COM/"), Some("example.com".to_string()));
        assert_eq!(extract_host_normalized("https://Example.Com/path?q=1"), Some("example.com".to_string()));
    }

    #[test]
    fn extract_host_strips_trailing_dot() {
        assert_eq!(extract_host_normalized("https://example.com./"), Some("example.com".to_string()));
    }

    #[test]
    fn extract_host_rejects_credentials() {
        // Credential injection: http://user:pass@evil.com/ should still give evil.com
        assert_eq!(extract_host_normalized("http://user:pass@evil.com/"), Some("evil.com".to_string()));
    }

    #[test]
    fn extract_host_ipv6() {
        assert_eq!(extract_host_normalized("http://[::1]:8080/"), Some("::1".to_string()));
        assert_eq!(extract_host_normalized("https://[2001:db8::1]/path"), Some("2001:db8::1".to_string()));
    }

    #[test]
    fn extract_host_rejects_unsupported_schemes() {
        assert_eq!(extract_host_normalized("ftp://example.com/"), None);
        assert_eq!(extract_host_normalized("file:///etc/passwd"), None);
        assert_eq!(extract_host_normalized("javascript:alert(1)"), None);
        assert_eq!(extract_host_normalized("data:text/html,<h1>hi</h1>"), None);
    }

    #[test]
    fn extract_host_rejects_malformed() {
        assert_eq!(extract_host_normalized("not-a-url"), None);
        assert_eq!(extract_host_normalized(""), None);
        assert_eq!(extract_host_normalized("https://"), None);
    }

    #[test]
    fn host_matches_exact() {
        assert!(host_matches("example.com", "example.com"));
        assert!(!host_matches("evil.com", "example.com"));
        assert!(!host_matches("sub.example.com", "example.com"));
    }

    #[test]
    fn host_matches_wildcard() {
        assert!(host_matches("sub.example.com", "*.example.com"));
        assert!(host_matches("a.b.example.com", "*.example.com"));
        // Wildcard must NOT match bare domain
        assert!(!host_matches("example.com", "*.example.com"));
        // Wildcard must NOT match adjacent domain
        assert!(!host_matches("badexample.com", "*.example.com"));
        assert!(!host_matches("notexample.com", "*.example.com"));
    }

    #[test]
    fn host_matches_normalizes_pattern() {
        // Pattern case folding
        assert!(host_matches("example.com", "EXAMPLE.COM"));
        assert!(host_matches("sub.example.com", "*.EXAMPLE.COM"));
        // Pattern trailing dot
        assert!(host_matches("example.com", "example.com."));
    }

    #[test]
    fn host_matches_ip_literals() {
        assert!(host_matches("127.0.0.1", "127.0.0.1"));
        assert!(!host_matches("127.0.0.1", "example.com"));
        // localhost must be explicit
        assert!(!host_matches("localhost", "127.0.0.1"));
        assert!(host_matches("localhost", "localhost"));
    }
}
