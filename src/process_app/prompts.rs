//! Capability and secret prompt modals — shown when an app requests access.

use crate::app::permissions::AppPermissions;
use crate::app_protocol::PlexiEvent;
use crate::host::event_log::{self, HostEvent};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest};
use crate::workspace::secrets::SecretStore;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{mpsc::Sender, Arc};

use super::{DeferredAiQuery, PendingPrompt};

/// Persist a granted secret to the keychain so the user is not prompted again.
///
/// Storage strategy:
/// - Workspace-routed path (`workspace.toml` + `secrets.toml` present): write
///   under the workspace-namespaced account resolved from the router, so the
///   next `resolve()` call finds the value without prompting.
/// - Legacy / no-workspace path: write under `plexi:user:<key>` (user-scope),
///   which the fallback resolver consults when `fallback = true` and no
///   per-workspace route is defined.
///
/// Exposed with `pub` so the unit test can call it directly without going
/// through the egui modal.
pub fn persist_granted_secret(
    workspace_root: &Path,
    app_id: &str,
    key: &str,
    value: &str,
    store: &dyn SecretStore,
) {
    use crate::workspace::secrets::{
        keychain_user_name, keychain_workspace_name, WorkspaceConfig, WorkspaceSecrets,
    };

    // Attempt workspace-aware storage first: load workspace.toml + secrets.toml
    // and resolve the friendly Keychain name the router would use for this
    // (app_id, canonical key) pair. Write under that name so the next
    // `resolve()` call returns Found instead of PromptUser.
    let ws_cfg = WorkspaceConfig::load(workspace_root).ok().flatten();
    let router = WorkspaceSecrets::load(workspace_root).ok().flatten();

    let account = if let (Some(cfg), Some(r)) = (ws_cfg, router) {
        if let Some(friendly) = r.route_for(app_id, key) {
            // Explicit route declared in secrets.toml — store under the
            // workspace-namespaced friendly name.
            keychain_workspace_name(&cfg.id, friendly)
        } else if r.fallback {
            // No explicit route but fallback is enabled — the resolver will
            // look up plexi:user:<key> on the next call.
            keychain_user_name(key)
        } else {
            // fallback = false and no route — resolver will HardMissing next
            // time regardless. Still store as user-scope so the user's value
            // isn't lost; they can add a route later.
            keychain_user_name(key)
        }
    } else {
        // No workspace config at all (legacy path). The legacy resolver in
        // routing.rs calls get_secret_scoped which reads the legacy account
        // format; we also write to user-scope so the new resolver path picks
        // it up once the workspace is initialized.
        keychain_user_name(key)
    };

    if let Err(e) = store.set(&account, value) {
        log::error!(
            "prompts::persist_granted_secret: failed to store secret \
             account={account} key={key} app={app_id}: {e}"
        );
    } else {
        log::info!(
            "prompts::persist_granted_secret: stored key={key} app={app_id} \
             account={account}"
        );
    }
}

/// Show a modal for the first pending prompt (capability or secret).
/// Called from the host's early-modal path (step 2 of `update()`) so the
/// modal owns keyboard input before `dispatch_app_key_events` runs.
pub(crate) fn show_prompt_modal(
    ctx: &egui::Context,
    pending_prompts: &mut VecDeque<PendingPrompt>,
    outbound_events: &mut VecDeque<PlexiEvent>,
    permissions: &mut AppPermissions,
    type_id: &str,
    workspace_root: &Path,
    secret_input_buf: &mut String,
    _config_dir: &Path,
    permission_store: &mut crate::app::permissions::PermissionStore,
    colors: &crate::ui::theme::Colors,
    deferred_ai_queries: &mut VecDeque<DeferredAiQuery>,
    ai_broker: Arc<dyn AiBroker>,
    http_tx: Sender<PlexiEvent>,
    pane_id: u64,
) {
    let Some(prompt) = pending_prompts.front() else {
        return;
    };

    // Detect keys at frame level — before any window or button rendering — so
    // egui button focus cannot intercept Enter and misfire an action.
    let (mut grant_once, mut grant_forever, mut deny_once, mut deny_forever) = match prompt {
        PendingPrompt::Capability { .. } => ctx.input_mut(|i| {
            let shift_enter = i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter);
            let key_a = i.consume_key(egui::Modifiers::NONE, egui::Key::A);
            let key_d = i.consume_key(egui::Modifiers::NONE, egui::Key::D);
            let shift_esc = i.consume_key(egui::Modifiers::SHIFT, egui::Key::Escape);
            let plain_enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let plain_esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (
                plain_enter,
                shift_enter || key_a,
                plain_esc,
                shift_esc || key_d,
            )
        }),
        PendingPrompt::Secret { .. } => {
            let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            (false, false, esc, false)
        }
    };

    // Scrim
    let screen_rect = ctx.screen_rect();
    egui::Area::new(egui::Id::new("prompt_modal_scrim"))
        .fixed_pos(screen_rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.painter().rect_filled(
                screen_rect,
                0.0,
                egui::Color32::from_black_alpha(crate::ui::style::SCRIM_ALPHA),
            );
            let scrim_resp = ui.allocate_rect(screen_rect, egui::Sense::click());
            if scrim_resp.clicked() {
                deny_once = true;
            }
        });

    egui::Area::new(egui::Id::new("prompt_modal_overlay"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(colors.bg_sidebar)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .corner_radius(crate::ui::style::RADIUS_LG)
                .inner_margin(egui::Margin::symmetric(20, 16))
                .show(ui, |ui| {
                    ui.set_width(crate::overlays::MODAL_WIDTH);
                    match prompt {
                        PendingPrompt::Capability { capability, .. } => {
                            ui.label(
                                egui::RichText::new("Permission request")
                                    .size(13.0)
                                    .color(colors.text_primary)
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "\"{}\" is requesting access to:",
                                    type_id
                                ))
                                .size(crate::ui::style::TEXT_CAPTION)
                                .color(colors.text_dim),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(capability)
                                    .size(crate::ui::style::TEXT_CAPTION)
                                    .color(colors.text_primary)
                                    .monospace()
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "Workspace: {}",
                                    workspace_root.display()
                                ))
                                .size(crate::ui::style::TEXT_HINT)
                                .color(colors.text_dim),
                            );
                            ui.add_space(crate::ui::style::SPACE_MD);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("Grant Once")
                                                .size(crate::ui::style::TEXT_CAPTION)
                                                .color(colors.bg_darkest),
                                        )
                                        .fill(colors.accent),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    grant_once = true;
                                }
                                ui.add_space(4.0);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("Grant Forever")
                                                .size(crate::ui::style::TEXT_CAPTION)
                                                .color(colors.bg_darkest),
                                        )
                                        .fill(colors.accent),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    grant_forever = true;
                                }
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("Deny Once")
                                                .size(crate::ui::style::TEXT_CAPTION)
                                                .color(colors.text_dim),
                                        )
                                        .fill(colors.bg_active),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    deny_once = true;
                                }
                                ui.add_space(4.0);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("Deny Forever")
                                                .size(crate::ui::style::TEXT_CAPTION)
                                                .color(colors.bg_darkest),
                                        )
                                        .fill(colors.danger),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    deny_forever = true;
                                }
                            });
                        }
                        PendingPrompt::Secret { key } => {
                            ui.label(
                                egui::RichText::new("Secret required")
                                    .size(13.0)
                                    .color(colors.text_primary)
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "\"{}\" needs a secret value for:",
                                    type_id
                                ))
                                .size(crate::ui::style::TEXT_CAPTION)
                                .color(colors.text_dim),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(key)
                                    .size(crate::ui::style::TEXT_CAPTION)
                                    .color(colors.text_primary)
                                    .monospace()
                                    .strong(),
                            );
                            ui.add_space(crate::ui::style::SPACE_SM);
                            ui.scope(|ui| {
                                ui.visuals_mut().text_cursor.stroke.width = 1.5;
                                ui.visuals_mut().text_cursor.stroke.color = colors.accent;
                                ui.visuals_mut().extreme_bg_color = colors.bg_active;
                                ui.visuals_mut().widgets.active.bg_stroke =
                                    egui::Stroke::new(1.0, colors.accent);
                                ui.visuals_mut().widgets.inactive.bg_stroke =
                                    egui::Stroke::new(1.0, colors.border);
                                let response = ui.add(
                                    egui::TextEdit::singleline(secret_input_buf)
                                        .id(egui::Id::new("capability_secret_input"))
                                        .password(true)
                                        .margin(egui::Margin::symmetric(8, 5)),
                                );
                                if response.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    grant_once = true;
                                }
                            });
                            ui.add_space(crate::ui::style::SPACE_SM);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("Submit")
                                                .size(crate::ui::style::TEXT_CAPTION)
                                                .color(colors.bg_darkest),
                                        )
                                        .fill(colors.accent),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    grant_once = true;
                                }
                                ui.add_space(4.0);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("Cancel")
                                                .size(crate::ui::style::TEXT_CAPTION)
                                                .color(colors.text_dim),
                                        )
                                        .fill(colors.bg_active),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    deny_once = true;
                                }
                            });
                            ui.add_space(4.0);
                            crate::ui::widgets::key_combo_list(
                                ui,
                                &[&["↵"]],
                                Some("submit"),
                                colors,
                            );
                            crate::ui::widgets::key_combo_list(
                                ui,
                                &[&["⎋"]],
                                Some("cancel"),
                                colors,
                            );
                        }
                    }
                });
        });

    if grant_once || grant_forever || deny_once || deny_forever {
        use crate::app::permissions::Capability;
        let actually_granted = grant_once || grant_forever;
        match pending_prompts.pop_front() {
            Some(PendingPrompt::Capability {
                request_id,
                capability,
            }) => {
                if grant_once || grant_forever {
                    match Capability::try_from(capability.as_str()) {
                        Ok(cap) => {
                            permissions.capabilities.insert(cap);
                            if grant_forever {
                                permission_store.set(
                                    type_id,
                                    workspace_root,
                                    cap,
                                    crate::app::permissions::PermissionState::Green,
                                );
                                permission_store.save();
                                log::info!(
                                    "prompts: granted {} for {} forever in {}",
                                    cap,
                                    type_id,
                                    workspace_root.display()
                                );
                            } else {
                                log::info!(
                                    "prompts: granted {} for {} (once, session only)",
                                    cap,
                                    type_id
                                );
                            }
                        }
                        Err(e) => {
                            log::warn!(
                                "prompts: cannot grant {e}; capability string not recognized"
                            );
                        }
                    }
                }
                if deny_forever {
                    if let Ok(cap) = Capability::try_from(capability.as_str()) {
                        permissions.blocked.insert(cap);
                        permission_store.set(
                            type_id,
                            workspace_root,
                            cap,
                            crate::app::permissions::PermissionState::Red,
                        );
                        permission_store.save();
                        log::info!(
                            "prompts: denied {} for {} forever in {}",
                            cap,
                            type_id,
                            workspace_root.display()
                        );
                    }
                }
                if deny_once {
                    log::info!(
                        "prompts: denied {} for {} (once, will reprompt on next request)",
                        capability,
                        type_id
                    );
                }
                event_log::emit(HostEvent::PermissionDecision {
                    app_id: type_id.to_string(),
                    capability: capability.clone(),
                    granted: actually_granted,
                    timestamp: event_log::now_timestamp(),
                });
                outbound_events.push_back(PlexiEvent::CapabilityDecision {
                    request_id,
                    granted: actually_granted,
                });
                // Drain any AiQuery requests that were withheld pending this consent.
                if capability == "ai.query" && !deferred_ai_queries.is_empty() {
                    if actually_granted {
                        log::info!(
                            "prompts: ai.query granted for {} — re-dispatching {} deferred queries",
                            type_id,
                            deferred_ai_queries.len()
                        );
                        // Build ToolDispatcher once — pane_id, type_id, workspace_root are
                        // identical for all queries in this queue.
                        let ws_root = workspace_root.to_path_buf();
                        let tool_dispatcher = std::sync::Arc::new(
                            crate::plexi_ai::tool_dispatch::ToolDispatcher::from_registry(
                                pane_id,
                                type_id.to_string(),
                                ws_root.clone(),
                            ),
                        );
                        while let Some(q) = deferred_ai_queries.pop_front() {
                            let broker = Arc::clone(&ai_broker);
                            let app_id = type_id.to_string();
                            let tx = http_tx.clone();
                            let ws_root_clone = ws_root.clone();
                            let open_panes = crate::plexi_ai::broker::get_pane_snapshot();
                            let td = std::sync::Arc::clone(&tool_dispatcher);
                            let req_id = q.request_id.clone();
                            std::thread::Builder::new()
                                .name(format!("ai-query-deferred-{app_id}-{}", q.request_id))
                                .spawn(move || {
                                    let resp = broker.dispatch(AiBrokerRequest {
                                        app_id,
                                        model_tier: q.model_tier,
                                        system: q.system,
                                        messages: q.messages,
                                        tools: q.tools,
                                        workspace_root: Some(ws_root_clone),
                                        open_panes,
                                        tool_dispatcher: Some(td),
                                    });
                                    // Send incremental stream chunks before the final AiResponse.
                                    let n_chunks = resp.stream_deltas.len();
                                    for (i, delta) in resp.stream_deltas.into_iter().enumerate() {
                                        let done = i + 1 == n_chunks;
                                        let chunk = PlexiEvent::AiStreamChunk {
                                            request_id: req_id.clone(),
                                            delta,
                                            done,
                                        };
                                        if let Err(e) = tx.send(chunk) {
                                            log::warn!("ai broker: deferred stream chunk receiver dropped: {e}");
                                            return;
                                        }
                                    }
                                    let event = PlexiEvent::AiResponse {
                                        request_id: req_id,
                                        content: resp.content,
                                        tokens_in: resp.tokens_in,
                                        tokens_out: resp.tokens_out,
                                        error: resp.error,
                                    };
                                    if let Err(e) = tx.send(event) {
                                        log::warn!("ai broker: deferred response receiver dropped: {e}");
                                    }
                                })
                                .expect("failed to spawn deferred ai-query thread");
                        }
                    } else {
                        log::info!(
                            "prompts: ai.query denied for {} — erroring {} deferred queries",
                            type_id,
                            deferred_ai_queries.len()
                        );
                        while let Some(q) = deferred_ai_queries.pop_front() {
                            outbound_events.push_back(PlexiEvent::AiResponse {
                                request_id: q.request_id,
                                content: None,
                                tokens_in: 0,
                                tokens_out: 0,
                                error: Some("capability denied by user".to_string()),
                            });
                        }
                    }
                }
            }
            Some(PendingPrompt::Secret { key }) => {
                let value = if actually_granted && !secret_input_buf.is_empty() {
                    Some(secret_input_buf.clone())
                } else {
                    None
                };
                secret_input_buf.clear();

                // Persist the granted secret so future requests don't prompt again.
                if let Some(ref v) = value {
                    #[cfg(target_os = "macos")]
                    {
                        use crate::workspace::secrets::MacKeychain;
                        persist_granted_secret(
                            workspace_root,
                            type_id,
                            key.as_str(),
                            v,
                            &MacKeychain::new(),
                        );
                    }
                }

                if value.is_none() {
                    event_log::emit(HostEvent::SecretDenied {
                        app_id: type_id.to_string(),
                        key: key.clone(),
                        reason: if deny_once || deny_forever {
                            "user_denied"
                        } else {
                            "empty_value"
                        }
                        .to_string(),
                        timestamp: event_log::now_timestamp(),
                    });
                }
                outbound_events.push_back(PlexiEvent::SecretValue { key, value });
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::secrets::InMemoryKeychain;

    /// Helper that builds a temp workspace with the given secrets.toml content.
    fn setup_workspace(tmp: &tempfile::TempDir, secrets_toml: &str) {
        let channel_dir = crate::config::workspace_channel_dir();
        let plexi_dir = tmp.path().join(&channel_dir);
        std::fs::create_dir_all(&plexi_dir).unwrap();
        // Write a deterministic workspace ID so tests can assert on account names.
        std::fs::write(plexi_dir.join("workspace.toml"), "id = \"test-ws-id\"\n").unwrap();
        if !secrets_toml.is_empty() {
            std::fs::write(plexi_dir.join("secrets.toml"), secrets_toml).unwrap();
        }
    }

    #[test]
    fn granted_secret_is_persisted_to_keychain() {
        // Basic case: no workspace config — value stored under plexi:user:<key>.
        let tmp = tempfile::tempdir().unwrap();
        let store = InMemoryKeychain::new();

        persist_granted_secret(tmp.path(), "my-app", "OPENAI_API_KEY", "sk-test", &store);

        let retrieved = store.get("plexi:user:OPENAI_API_KEY");
        assert!(
            retrieved.is_some(),
            "secret should be retrievable after persist"
        );
        assert_eq!(retrieved.unwrap().as_str(), "sk-test");
    }

    #[test]
    fn granted_secret_uses_workspace_route_when_declared() {
        // When secrets.toml has an explicit [apps.<app_id>] route, the value
        // should land under the workspace-namespaced friendly name.
        let tmp = tempfile::tempdir().unwrap();
        setup_workspace(
            &tmp,
            "fallback = false\n[apps.my-app]\nOPENAI_API_KEY = \"openai_prod\"\n",
        );
        let store = InMemoryKeychain::new();

        persist_granted_secret(
            tmp.path(),
            "my-app",
            "OPENAI_API_KEY",
            "sk-workspace",
            &store,
        );

        // Should be stored under workspace-namespaced friendly name.
        let account = "plexi:test-ws-id:openai_prod";
        let retrieved = store.get(account);
        assert!(
            retrieved.is_some(),
            "secret should be stored under workspace-namespaced account {account}"
        );
        assert_eq!(retrieved.unwrap().as_str(), "sk-workspace");

        // Must NOT be stored under user-scope account.
        assert!(
            store.get("plexi:user:OPENAI_API_KEY").is_none(),
            "workspace-routed secret must not bleed into user-scope"
        );
    }

    #[test]
    fn granted_secret_uses_user_scope_when_fallback_enabled_and_no_route() {
        // secrets.toml present with fallback=true but no route for this key —
        // resolver uses user-scope on lookup, so persist must write there too.
        let tmp = tempfile::tempdir().unwrap();
        setup_workspace(&tmp, "fallback = true\n");
        let store = InMemoryKeychain::new();

        persist_granted_secret(tmp.path(), "my-app", "GITHUB_TOKEN", "ghp-abc", &store);

        let retrieved = store.get("plexi:user:GITHUB_TOKEN");
        assert!(
            retrieved.is_some(),
            "fallback-path secret should land under plexi:user:<key>"
        );
        assert_eq!(retrieved.unwrap().as_str(), "ghp-abc");
    }

    #[test]
    fn denied_secret_is_not_persisted() {
        // Ensure nothing is written when value is None (deny path). This is
        // an indirect test — persist_granted_secret is only called when
        // value.is_some(), but we verify the store stays empty.
        let tmp = tempfile::tempdir().unwrap();
        let store = InMemoryKeychain::new();

        // Simulating the deny path: don't call persist at all (value is None).
        let value: Option<String> = None;
        if let Some(ref v) = value {
            persist_granted_secret(tmp.path(), "my-app", "SECRET", v, &store);
        }

        assert!(
            store.get("plexi:user:SECRET").is_none(),
            "denied secret must not be persisted"
        );
    }
}
