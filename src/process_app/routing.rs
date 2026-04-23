//! DrawCommand routing — dispatches out-of-frame commands to subsystems.
//!
//! All visual draw commands stay in the frame pipeline; only control commands
//! (media, pipes, capabilities, secrets, runs, notifications) are routed here.

use crate::app_permissions::{check, Capability, PermissionCheck};
use crate::app_protocol::{DrawCommand, PlexiEvent};
use crate::app_trait::AppCommand;
use crate::event_log::{self, HostEvent};
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
                    match crate::secrets::get_secret_scoped(
                        &key,
                        &self.type_id,
                        &self.workspace_root,
                    ) {
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
                            self.pending_prompts
                                .push_back(super::PendingPrompt::Secret { key });
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
                    level,
                    title,
                    body,
                    kind,
                    options,
                    input_prompt,
                    required,
                    priority,
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
            DrawCommand::AudioCapture { .. } => {
                if let PermissionCheck::Denied(reason) =
                    check(&self.permissions, Capability::AudioRecord)
                {
                    log::warn!(
                        "ProcessApp[{}]: AudioCapture denied — {reason}",
                        self.type_id
                    );
                    return;
                }
                log::warn!(
                    "ProcessApp[{}]: AudioCapture not yet implemented (v3.1)",
                    self.type_id
                );
            }
            DrawCommand::Image { .. } => {
                log::warn!(
                    "ProcessApp[{}]: Image not yet implemented (v3.1)",
                    self.type_id
                );
            }
            DrawCommand::VideoPlayer { .. } => {
                log::warn!(
                    "ProcessApp[{}]: VideoPlayer not yet implemented (v3.1)",
                    self.type_id
                );
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
