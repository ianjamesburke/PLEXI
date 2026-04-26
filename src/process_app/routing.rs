//! DrawCommand routing — dispatches out-of-frame commands to subsystems.
//!
//! All visual draw commands stay in the frame pipeline; only control commands
//! (media, pipes, capabilities, secrets, runs, notifications) are routed here.

use crate::app_permissions::{check, Capability, PermissionCheck};
use crate::app_protocol::{AudioDeviceWire, DrawCommand, MidiPortWire, PlexiEvent};
use crate::app_trait::AppCommand;
use crate::audio::AudioCaptureRequest;
use crate::event_log::{self, HostEvent};
use crate::plexi_iq::broker::IqBrokerRequest;
use crate::typed_pipes::PipeDirection;

use super::ProcessApp;

impl ProcessApp {
    /// Route a v3 out-of-frame draw command to the appropriate subsystem.
    /// Visual primitives must not reach this method — callers filter them upstream.
    pub(super) fn route_command(&mut self, cmd: DrawCommand) {
        match cmd {
            // ── Capability request ─────────────────────────────────────────
            DrawCommand::CapabilityRequest {
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
            DrawCommand::SecretGet { key } => {
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
            DrawCommand::RunGet { intent, .. } => {
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
            DrawCommand::RunComplete { run_id, result } => {
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
            DrawCommand::Notify {
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
                self.pending_commands.push(AppCommand::ShowNotification {
                    notify_id: panel_id,
                    sender_pane_id: 0, // stamped by dispatch.rs with the real pane_id
                    source_context: 0, // stamped by dispatch.rs with the real ctx_idx
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
                });
                // `actions` intentionally dropped: they were already processed
                // as server-side side effects above (resume_run / open_intent /
                // run_command). The modal surfaces options, not actions.
                let _ = actions;
            }

            // ── Pipe open ──────────────────────────────────────────────────
            DrawCommand::PipeOpen {
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
            DrawCommand::PipeOpenDirected {
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

            // ── Agent roster query — #286 ─────────────────────────────────
            // Capability gate is unusual: an undeclared `agents.list` does
            // NOT return an error — it returns an empty roster. The check
            // happens here (loud log) but we still defer to the host to
            // emit the response so the wire shape is identical regardless.
            DrawCommand::AgentRosterGet { request_id } => {
                if let PermissionCheck::Denied(_) =
                    check(&self.permissions, Capability::AgentsList)
                {
                    // Per-spec: undeclared → empty roster, not an error.
                    log::debug!(
                        "ProcessApp[{}]: AgentRosterGet without agents.list — empty roster",
                        self.type_id
                    );
                    self.outbound_events
                        .push_back(PlexiEvent::AgentRoster {
                            request_id,
                            agents: Vec::new(),
                        });
                    return;
                }
                self.pending_commands.push(AppCommand::AgentRosterGet {
                    sender_pane_id: self.pane_id,
                    request_id,
                });
            }

            // ── Pipe send ──────────────────────────────────────────────────
            DrawCommand::PipeSend { pipe_id, payload } => {
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
            DrawCommand::StatusSummary { text } => {
                self.status_summary = Some(text);
            }

            // ── Spawn app ──────────────────────────────────────────────────
            DrawCommand::SpawnApp {
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

            // ── HTTP request (broker via HostServices::net) ───────────────
            DrawCommand::HttpRequest {
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
                // Check allowed_hosts if the list is non-empty.
                if !self.permissions.allowed_hosts.is_empty() {
                    let host = extract_host(&url);
                    let allowed = self.permissions.allowed_hosts.iter().any(|pattern| host_matches(host, pattern));
                    if !allowed {
                        log::warn!(
                            "ProcessApp[{}]: HttpRequest {request_id} denied — host '{host}' not in allowed_hosts",
                            self.type_id
                        );
                        self.outbound_events.push_back(PlexiEvent::HttpResponse {
                            request_id,
                            status: 403,
                            body: String::new(),
                            error: Some(format!("host_not_allowed: '{host}' is not in this app's allowed_hosts list")),
                        });
                        return;
                    }
                }
                log::debug!(
                    "ProcessApp[{}]: HttpRequest {request_id} {method} {url}",
                    self.type_id
                );
                // Spawn a background thread so the HTTP call never blocks the UI thread.
                let net = std::sync::Arc::clone(&self.net);
                let tx = self.http_tx.clone();
                let type_id = self.type_id.clone();
                std::thread::spawn(move || {
                    let resp = net.http(&method, &url, &headers, body.as_deref());
                    log::debug!("ProcessApp[{type_id}]: HttpRequest {request_id} → {}", resp.status);
                    let _ = tx.send(PlexiEvent::HttpResponse {
                        request_id,
                        status: resp.status,
                        body: resp.body,
                        error: resp.error,
                    });
                });
            }
            // ── LLM request (broker via Anthropic API) ────────────────────
            DrawCommand::LlmRequest {
                request_id,
                prompt,
                model,
                system,
            } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::Llm)
                {
                    log::warn!(
                        "ProcessApp[{}]: LlmRequest {request_id} denied — {reason}",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::LlmResponse {
                        request_id,
                        content: String::new(),
                        error: Some(format!("capability_denied: {reason}")),
                    });
                    return;
                }

                let api_key = crate::secrets::resolve_secret(
                    "ANTHROPIC_API_KEY",
                    &self.type_id,
                    self.workspace_root.to_str().unwrap_or(""),
                );

                let Some(api_key) = api_key else {
                    log::warn!(
                        "ProcessApp[{}]: LlmRequest {request_id} — ANTHROPIC_API_KEY not in secrets store",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::LlmResponse {
                        request_id,
                        content: String::new(),
                        error: Some("api_key_missing: store ANTHROPIC_API_KEY in Plexi secrets to use llm capability".to_string()),
                    });
                    return;
                };

                log::debug!(
                    "ProcessApp[{}]: LlmRequest {request_id} model={model}",
                    self.type_id
                );
                let tx = self.http_tx.clone();
                let type_id = self.type_id.clone();
                std::thread::spawn(move || {
                    let result = call_anthropic_api(&api_key, &model, &prompt, system.as_deref());
                    let _ = tx.send(match result {
                        Ok(content) => PlexiEvent::LlmResponse {
                            request_id,
                            content,
                            error: None,
                        },
                        Err(e) => PlexiEvent::LlmResponse {
                            request_id,
                            content: String::new(),
                            error: Some(e),
                        },
                    });
                    log::debug!("ProcessApp[{type_id}]: LlmRequest completed");
                });
            }
            // ── iq.query broker (#284) ─────────────────────────────────────
            DrawCommand::IqQuery {
                request_id,
                model_tier,
                system,
                messages,
                tools,
            } => {
                if let PermissionCheck::Denied(_reason) =
                    check(&self.permissions, Capability::IqQuery)
                {
                    log::warn!(
                        "ProcessApp[{}]: IqQuery {request_id} denied — capability not declared",
                        self.type_id
                    );
                    self.outbound_events.push_back(PlexiEvent::IqResponse {
                        request_id,
                        content: None,
                        tokens_in: 0,
                        tokens_out: 0,
                        error: Some(
                            "capability denied: iq.query not declared in manifest".to_string(),
                        ),
                    });
                    return;
                }

                log::debug!(
                    "ProcessApp[{}]: IqQuery {request_id} tier={:?} messages={} tools={}",
                    self.type_id,
                    model_tier,
                    messages.len(),
                    tools.len()
                );

                let broker = self.iq_broker.clone();
                let app_id = self.type_id.clone();
                let tx = self.http_tx.clone();
                std::thread::spawn(move || {
                    let resp = broker.dispatch(IqBrokerRequest {
                        app_id,
                        model_tier,
                        system,
                        messages,
                        tools,
                    });
                    let event = PlexiEvent::IqResponse {
                        request_id,
                        content: resp.content,
                        tokens_in: resp.tokens_in,
                        tokens_out: resp.tokens_out,
                        error: resp.error,
                    };
                    if let Err(e) = tx.send(event) {
                        log::warn!("iq broker: response receiver dropped: {e}");
                    }
                });
            }

            DrawCommand::AudioPlay { .. } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::AudioPlayback)
                {
                    log::warn!(
                        "ProcessApp[{}]: AudioPlay denied — {reason}",
                        self.type_id
                    );
                    return;
                }
                log::warn!(
                    "ProcessApp[{}]: AudioPlay not yet implemented (v3.1)",
                    self.type_id
                );
            }
            DrawCommand::AudioCapture {
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
            DrawCommand::ListAudioDevices { request_id } => {
                // Enumeration is not gated — device names are already visible
                // to any macOS app via System Information. The per-device
                // capture call goes through the `AudioRecord` gate.
                let inputs = self
                    .audio_device
                    .list_input_devices()
                    .into_iter()
                    .map(AudioDeviceWire::from)
                    .collect();
                let outputs = self
                    .audio_device
                    .list_output_devices()
                    .into_iter()
                    .map(AudioDeviceWire::from)
                    .collect();
                self.outbound_events
                    .push_back(PlexiEvent::AudioDevicesListed {
                        request_id,
                        inputs,
                        outputs,
                        error: None,
                    });
            }
            DrawCommand::ListMidiDevices { request_id } => {
                // MIDI enumeration mirrors audio: not gated. Port names are
                // already visible in Audio MIDI Setup.app, no privacy gate.
                let inputs = self
                    .midi_device
                    .list_input_ports()
                    .into_iter()
                    .map(MidiPortWire::from)
                    .collect();
                let outputs = self
                    .midi_device
                    .list_output_ports()
                    .into_iter()
                    .map(MidiPortWire::from)
                    .collect();
                self.outbound_events
                    .push_back(PlexiEvent::MidiDevicesListed {
                        request_id,
                        inputs,
                        outputs,
                        error: None,
                    });
            }
            DrawCommand::OpenMidiInput { port_id, pipe_id } => {
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
            DrawCommand::CloseMidiInput { port_id } => {
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
            DrawCommand::SendMidi { port_id, bytes } => {
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
            DrawCommand::Image { .. } => {
                log::warn!(
                    "ProcessApp[{}]: Image not yet implemented (v3.1)",
                    self.type_id
                );
            }
            DrawCommand::OpenVideo {
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
            DrawCommand::SetVideoState { handle_id, state } => {
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
            DrawCommand::CloseVideo { handle_id } => {
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
            DrawCommand::AudioMeter { .. } => {
                log::warn!(
                    "ProcessApp[{}]: AudioMeter not yet implemented (v3.1)",
                    self.type_id
                );
            }

            // ── Cd request ─────────────────────────────────────────────────
            DrawCommand::CdRequest { cwd } => {
                log::info!("ProcessApp[{}]: CdRequest cwd='{cwd}'", self.type_id);
                self.pending_commands.push(AppCommand::CdRequest { cwd, sender_pane_id: self.pane_id });
            }

            // ── Set timer ──────────────────────────────────────────────────
            DrawCommand::SetTimer { timer_id, after_ms } => {
                if let PermissionCheck::Denied(reason) = check(&self.permissions, Capability::Timer) {
                    log::warn!("ProcessApp[{}]: SetTimer {timer_id} denied — {reason}", self.type_id);
                    return;
                }
                let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.pending_timers.insert(timer_id.clone(), std::sync::Arc::clone(&cancelled));
                let tx = self.http_tx.clone();
                let type_id = self.type_id.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(after_ms));
                    if !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        log::debug!("ProcessApp[{type_id}]: timer {timer_id} fired");
                        let _ = tx.send(PlexiEvent::Timer { timer_id });
                    }
                });
            }

            // ── Cancel timer ───────────────────────────────────────────────
            DrawCommand::CancelTimer { timer_id } => {
                if let Some(flag) = self.pending_timers.remove(&timer_id) {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    log::debug!("ProcessApp[{}]: timer {timer_id} cancelled", self.type_id);
                }
            }

            _ => unreachable!("route_command called with non-control command"),
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
        if self.audio_capture_sessions.contains_key(&pipe_id) {
            log::warn!(
                "ProcessApp[{}]: AudioCapture pipe_id={pipe_id} already capturing",
                self.type_id
            );
            self.outbound_events.push_back(PlexiEvent::AudioCaptureError {
                pipe_id,
                error: "already capturing on this pipe_id".to_owned(),
            });
            return;
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

/// Call the Anthropic Messages API synchronously. Returns the text of the first content block.
fn call_anthropic_api(
    api_key: &zeroize::Zeroizing<String>,
    model: &str,
    prompt: &str,
    system: Option<&str>,
) -> Result<String, String> {
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "messages": [{"role": "user", "content": prompt}]
    });
    if let Some(sys) = system {
        body["system"] = serde_json::Value::String(sys.to_string());
    }

    let body_str = body.to_string();
    let resp = ureq::post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", api_key.as_str())
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .send_string(&body_str);

    match resp {
        Ok(r) => {
            let resp_body = r.into_string().map_err(|e| format!("read_error: {e}"))?;
            let v: serde_json::Value =
                serde_json::from_str(&resp_body).map_err(|e| format!("parse_error: {e}"))?;
            v["content"][0]["text"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("unexpected_response: {resp_body}"))
        }
        Err(ureq::Error::Status(status, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("http_error: status={status} body={body}"))
        }
        Err(e) => Err(format!("http_error: {e}")),
    }
}

/// Extract the hostname from a URL string. Returns empty string on parse failure.
fn extract_host(url: &str) -> &str {
    if let Some(after_scheme) = url.find("://").map(|i| &url[i + 3..]) {
        let end = after_scheme.find('/').unwrap_or(after_scheme.len());
        let host_port = &after_scheme[..end];
        if let Some(colon) = host_port.rfind(':') {
            &host_port[..colon]
        } else {
            host_port
        }
    } else {
        ""
    }
}

/// Check if `host` matches `pattern`. Supports exact match and `*.domain.com` wildcards.
fn host_matches(host: &str, pattern: &str) -> bool {
    if pattern.starts_with("*.") {
        let suffix = &pattern[1..]; // ".domain.com"
        host.ends_with(suffix) || host == &pattern[2..]
    } else {
        host == pattern
    }
}
