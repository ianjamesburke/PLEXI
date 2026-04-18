/// Capability and secret prompt modals — shown when an app requests access.

use crate::app_permissions::AppPermissions;
use crate::app_protocol::PlexiEvent;
use crate::event_log::{self, HostEvent};
use std::collections::VecDeque;
use std::path::PathBuf;

use super::PendingPrompt;

/// Show a modal for the first pending prompt (capability or secret).
/// Blocks user interaction with the pane until the user grants or denies.
pub(super) fn show_prompt_modal(
    ui: &mut egui::Ui,
    pending_prompts: &mut VecDeque<PendingPrompt>,
    outbound_events: &mut VecDeque<PlexiEvent>,
    permissions: &mut AppPermissions,
    type_id: &str,
    workspace_root: &PathBuf,
    secret_input_buf: &mut String,
) {
    let Some(prompt) = pending_prompts.front() else { return };

    let mut granted = false;
    let mut denied = false;

    egui::Window::new("Plexi needs permission")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ui.ctx(), |ui| {
            match prompt {
                PendingPrompt::Capability { capability, .. } => {
                    ui.label(format!("App \"{}\" is requesting access to:", type_id));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(capability).monospace().strong());
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(format!(
                        "Workspace: {}",
                        workspace_root.display()
                    )).small());
                }
                PendingPrompt::Secret { key } => {
                    ui.label(format!("App \"{}\" needs a secret value for:", type_id));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(key).monospace().strong());
                    ui.add_space(8.0);
                    ui.label("Enter value:");
                    ui.text_edit_singleline(secret_input_buf);
                }
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Grant").clicked() {
                    granted = true;
                }
                if ui.button("Deny").clicked() {
                    denied = true;
                }
            });
        });

    if granted || denied {
        use crate::app_permissions::Capability;
        match pending_prompts.pop_front() {
            Some(PendingPrompt::Capability { request_id, capability }) => {
                if granted {
                    let cap = Capability::from(capability.as_str());
                    permissions.capabilities.insert(cap);
                }
                event_log::emit(HostEvent::PermissionDecision {
                    app_id: type_id.to_string(),
                    capability: capability.clone(),
                    granted,
                    timestamp: event_log::now_timestamp(),
                });
                outbound_events.push_back(PlexiEvent::CapabilityDecision {
                    request_id,
                    granted,
                });
            }
            Some(PendingPrompt::Secret { key }) => {
                let value = if granted && !secret_input_buf.is_empty() {
                    Some(secret_input_buf.clone())
                } else {
                    None
                };
                secret_input_buf.clear();
                if value.is_none() {
                    event_log::emit(HostEvent::SecretDenied {
                        app_id: type_id.to_string(),
                        key: key.clone(),
                        reason: if denied { "user_denied" } else { "empty_value" }.to_string(),
                        timestamp: event_log::now_timestamp(),
                    });
                }
                outbound_events.push_back(PlexiEvent::SecretValue { key, value });
            }
            None => {}
        }
    }
}
