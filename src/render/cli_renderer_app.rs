//! Built-in CLI renderer: a native host app that turns any CLI into a GUI.
//!
//! Parses a `PlexiDescriptor` JSON file (resolved by the Tier 1/2/3 chain in
//! `cli::descriptor` — native `--plexi`, registry, or recursive `--help`
//! crawl), renders a navigable command list and per-command form, and executes
//! the assembled command in a linked terminal pane.

use crate::app::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};
use crate::app::plexi_descriptor::{ArgSpec, ArgType, Command, PlexiDescriptor};
use crate::ui::{
    button::{self, ButtonKind},
    style,
};
use egui::{self, RichText};
use std::collections::HashMap;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Strip CLI name and "version" prefix from raw `--version` output.
/// "git version 2.54.0" -> "v2.54.0", "cargo 1.84.0" -> "v1.84.0"
fn clean_version(name: &str, raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    // Strip leading CLI name (case-insensitive)
    let s = if s.to_lowercase().starts_with(&name.to_lowercase()) {
        s[name.len()..].trim_start()
    } else {
        s
    };
    // Strip "version" word
    let s = s.strip_prefix("version").unwrap_or(s).trim_start();
    // Strip leading "v" if already there, then re-add
    let s = s.strip_prefix('v').unwrap_or(s);
    if s.is_empty() {
        String::new()
    } else {
        format!("v{s}")
    }
}

// ── View state ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum View {
    Loading,
    Error(String),
    List,
    Form,
}

pub struct CliRendererApp {
    descriptor: Option<PlexiDescriptor>,
    view: View,
    /// Breadcrumb path into nested commands (each entry is a command name).
    cmd_path: Vec<String>,
    /// Selected index in the current command list.
    selected: usize,
    /// Previous frame's selection, so keyboard nav can scroll the newly
    /// selected row into view exactly once when it changes.
    last_selected: usize,
    /// Per-flag form values, keyed by flag name.
    field_values: HashMap<String, String>,
    /// Linked terminal pane id (0 = not yet linked).
    terminal_pane_id: u64,
    /// Pending AppCommands to drain each frame.
    pending_commands: Vec<AppCommand>,
    /// Last command string that was run (shown as a hint).
    last_run: String,
    /// Whether we already requested a linked terminal.
    terminal_requested: bool,
    /// Unique request ID for the terminal link.
    terminal_request_id: String,
    /// Binary name for display.
    binary_name: String,
    /// Which text field has focus (flag name), if any.
    focused_field: Option<String>,
    /// Set when a leaf form opens so the first text field grabs keyboard
    /// focus on the next frame; cleared once focus is requested.
    pending_focus: bool,
}

impl CliRendererApp {
    pub fn new(descriptor_path: String) -> Self {
        let (descriptor, view) = match std::fs::read_to_string(&descriptor_path) {
            Ok(json) => match serde_json::from_str::<PlexiDescriptor>(&json) {
                Ok(d) => {
                    log::info!(
                        "CliRendererApp: loaded descriptor for '{}' v{}",
                        d.name,
                        d.version
                    );
                    (Some(d), View::Loading)
                }
                Err(e) => {
                    log::warn!("CliRendererApp: bad JSON in {descriptor_path}: {e}");
                    (None, View::Error(format!("Invalid descriptor JSON: {e}")))
                }
            },
            Err(e) => {
                log::warn!("CliRendererApp: cannot read {descriptor_path}: {e}");
                (
                    None,
                    View::Error(format!("Cannot read {descriptor_path}: {e}")),
                )
            }
        };

        let binary_name = descriptor
            .as_ref()
            .map(|d| d.name.clone())
            .unwrap_or_default();

        Self {
            descriptor,
            view,
            cmd_path: vec![],
            selected: 0,
            last_selected: 0,
            field_values: HashMap::new(),
            terminal_pane_id: 0,
            pending_commands: vec![],
            last_run: String::new(),
            terminal_requested: false,
            terminal_request_id: format!("cli-term-{}", uuid::Uuid::new_v4()),
            binary_name,
            focused_field: None,
            pending_focus: false,
        }
    }

    // ── Shared widgets ────────────────────────────────────────────────────────

    /// A "← Back" row rendered with the shared host `ListRow` so it is
    /// pixel-identical to the command rows above it. Returns true on click.
    fn back_row(ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) -> bool {
        crate::ui::list::ListRow::new("← Back")
            .dense()
            .show(ui, colors)
            .row_clicked()
    }

    // ── Descriptor navigation ────────────────────────────────────────────────

    fn commands_at_path(&self) -> Vec<&Command> {
        let Some(d) = &self.descriptor else {
            return vec![];
        };
        let mut cmds: &[Command] = &d.commands;
        for segment in &self.cmd_path {
            if let Some(cmd) = cmds.iter().find(|c| &c.name == segment) {
                cmds = &cmd.commands;
            } else {
                return vec![];
            }
        }
        cmds.iter().collect()
    }

    fn current_command(&self) -> Option<&Command> {
        let Some(d) = &self.descriptor else {
            return None;
        };
        let mut cmds: &[Command] = &d.commands;
        for (i, segment) in self.cmd_path.iter().enumerate() {
            if let Some(cmd) = cmds.iter().find(|c| &c.name == segment) {
                if i == self.cmd_path.len() - 1 {
                    return Some(cmd);
                }
                cmds = &cmd.commands;
            } else {
                return None;
            }
        }
        None
    }

    fn navigate_into(&mut self, idx: usize) {
        let commands = self.commands_at_path();
        let Some(cmd) = commands.get(idx) else {
            return;
        };
        let name = cmd.name.clone();
        let has_children = !cmd.commands.is_empty();
        // Collect defaults before mutating self
        let defaults: Vec<(String, String)> = if !has_children {
            cmd.flags
                .iter()
                .filter_map(|f| {
                    f.default
                        .as_ref()
                        .map(|d| (f.name.clone(), d.to_string().trim_matches('"').to_string()))
                })
                .collect()
        } else {
            vec![]
        };
        // Drop borrow before mutating
        drop(commands);

        self.cmd_path.push(name);
        if has_children {
            self.selected = 0;
            self.view = View::List;
        } else {
            self.field_values.clear();
            self.focused_field = None;
            for (k, v) in defaults {
                self.field_values.insert(k, v);
            }
            self.pending_focus = true;
            self.view = View::Form;
        }
    }

    fn navigate_back(&mut self) {
        if self.cmd_path.pop().is_some() {
            self.selected = 0;
            self.field_values.clear();
            self.focused_field = None;
            self.view = View::List;
        }
    }

    // ── Command execution ────────────────────────────────────────────────────

    fn build_command_string(&self) -> String {
        let Some(d) = &self.descriptor else {
            return String::new();
        };
        let mut parts = vec![d.name.clone()];
        parts.extend(self.cmd_path.iter().cloned());

        if let Some(cmd) = self.current_command() {
            for flag in &cmd.flags {
                if let Some(val) = self.field_values.get(&flag.name) {
                    if val.is_empty() {
                        continue;
                    }
                    match flag.ty {
                        ArgType::Bool => {
                            if val == "true" {
                                parts.push(flag.name.clone());
                            }
                        }
                        _ => {
                            parts.push(flag.name.clone());
                            if val.contains(' ') {
                                parts.push(format!("'{val}'"));
                            } else {
                                parts.push(val.clone());
                            }
                        }
                    }
                }
            }
            for arg in &cmd.args {
                if let Some(val) = self.field_values.get(&arg.name) {
                    if !val.is_empty() {
                        if val.contains(' ') {
                            parts.push(format!("'{val}'"));
                        } else {
                            parts.push(val.clone());
                        }
                    }
                }
            }
        }
        parts.join(" ")
    }

    fn execute(&mut self) {
        if self.terminal_pane_id == 0 {
            log::warn!("CliRendererApp: no linked terminal, cannot run");
            return;
        }
        let cmd = self.build_command_string();
        if cmd.is_empty() {
            return;
        }
        self.last_run = cmd.clone();
        self.pending_commands.push(AppCommand::RunInLinkedTerminal {
            sender_pane_id: 0, // host fills this
            terminal_pane_id: self.terminal_pane_id,
            command: cmd.clone(),
            echo: true,
        });
        log::info!("CliRendererApp: queued run '{cmd}'");
    }

    fn request_terminal(&mut self) {
        if self.terminal_requested {
            return;
        }
        self.terminal_requested = true;
        self.pending_commands
            .push(AppCommand::RequestLinkedTerminal {
                sender_pane_id: 0, // host fills
                request_id: self.terminal_request_id.clone(),
                cwd: None,
                label: Some(format!("{} terminal", self.binary_name)),
                // Stack the output terminal under the form, not beside it.
                place_below: true,
            });
        log::info!("CliRendererApp: requested linked terminal");
    }

    // ── Rendering helpers ────────────────────────────────────────────────────

    fn render_list(&mut self, ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) {
        // Snapshot command data as owned values to avoid borrowing self in closures.
        struct CmdRow {
            name: String,
            description: Option<String>,
            has_children: bool,
        }
        let rows: Vec<CmdRow> = self
            .commands_at_path()
            .iter()
            .map(|c| CmdRow {
                name: c.name.clone(),
                description: c.description.clone(),
                has_children: !c.commands.is_empty(),
            })
            .collect();

        if rows.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("No commands found")
                        .size(style::TEXT_BODY)
                        .color(colors.text_dim),
                );
            });
            return;
        }

        let d = self.descriptor.as_ref().unwrap();
        let title = if self.cmd_path.is_empty() {
            let ver = clean_version(&d.name, &d.version);
            if ver.is_empty() {
                d.name.clone()
            } else {
                format!("{}  {}", d.name, ver)
            }
        } else {
            format!("{} {}", d.name, self.cmd_path.join(" "))
        };
        let top_desc = if self.cmd_path.is_empty() {
            d.description.clone()
        } else {
            None
        };
        let has_path = !self.cmd_path.is_empty();

        // Title bar
        ui.add_space(style::SPACE_SM);
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_MD);
            ui.label(
                RichText::new(&title)
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary)
                    .strong(),
            );
        });

        if let Some(desc) = &top_desc {
            if !desc.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(style::SPACE_MD);
                    ui.label(
                        RichText::new(desc)
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                    );
                });
            }
        }

        ui.add_space(style::SPACE_SM);
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_MD);
            ui.separator();
        });

        // Back button if navigated deep
        if has_path && Self::back_row(ui, colors) {
            self.navigate_back();
            return;
        }

        // Command list — rendered with the shared host ListRow so it matches
        // the command palette, sidebar, and PGAP list views exactly.
        let mut clicked_idx: Option<usize> = None;
        let selection_changed = self.selected != self.last_selected;
        egui::ScrollArea::vertical()
            .animated(false)
            .show(ui, |ui| {
                for (i, row) in rows.iter().enumerate() {
                    let is_selected = i == self.selected;
                    // A trailing "›" marks command groups (rows that drill into
                    // a sub-list rather than open a form).
                    let mut list_row = crate::ui::list::ListRow::new(&row.name)
                        .selected(is_selected);
                    if let Some(desc) = &row.description {
                        list_row = list_row.secondary(desc);
                    }
                    if row.has_children {
                        list_row = list_row.chip("›");
                    }
                    let resp = list_row.show(ui, colors);
                    if is_selected {
                        resp.scroll_into_view(ui, selection_changed);
                    }
                    if resp.row_clicked() {
                        clicked_idx = Some(i);
                    }
                }
            });
        self.last_selected = self.selected;

        if let Some(i) = clicked_idx {
            self.selected = i;
            self.navigate_into(i);
            return;
        }

        // Footer hints
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(style::SPACE_SM);
            ui.horizontal(|ui| {
                ui.add_space(style::SPACE_MD);
                let hints = if has_path {
                    "↑↓ navigate  ⏎ open  esc back"
                } else {
                    "↑↓ navigate  ⏎ open"
                };
                ui.label(
                    RichText::new(hints)
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                );
            });
        });
    }

    fn render_form(&mut self, ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) {
        let cmd = match self.current_command() {
            Some(c) => c,
            None => {
                self.navigate_back();
                return;
            }
        };

        let d = self.descriptor.as_ref().unwrap();
        let title = format!("{} {}", d.name, self.cmd_path.join(" "));
        let cmd_desc = cmd.description.clone();
        let args: Vec<ArgSpec> = cmd.args.clone();
        let flags: Vec<ArgSpec> = cmd.flags.clone();
        let has_fields = !args.is_empty() || !flags.is_empty();

        // Title
        ui.add_space(style::SPACE_SM);
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_MD);
            ui.label(
                RichText::new(&title)
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary)
                    .strong(),
            );
        });

        if let Some(desc) = &cmd_desc {
            if !desc.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(style::SPACE_MD);
                    ui.label(
                        RichText::new(desc)
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                    );
                });
            }
        }

        ui.add_space(style::SPACE_SM);

        // Back button (same ListRow affordance as the list view)
        if Self::back_row(ui, colors) {
            self.navigate_back();
            return;
        }

        ui.add_space(style::SPACE_SM);
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_MD);
            ui.separator();
        });
        ui.add_space(style::SPACE_SM);

        // Arg fields. On a freshly opened form, the first text field grabs
        // keyboard focus so the user can type immediately.
        if has_fields {
            let mut want_focus = self.pending_focus;
            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 80.0)
                .show(ui, |ui| {
                    for arg in args.iter().chain(flags.iter()) {
                        if self.render_field(ui, colors, arg, want_focus) {
                            want_focus = false;
                        }
                    }
                });
            self.pending_focus = false;

            ui.add_space(style::SPACE_SM);
        }

        // Command preview
        let cmd_str = self.build_command_string();
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_MD);
            ui.label(
                RichText::new(format!("$ {cmd_str}"))
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim)
                    .monospace(),
            );
        });

        ui.add_space(style::SPACE_SM);

        // Run button
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_MD);
            let run_text = if self.terminal_pane_id == 0 {
                "▶  Run  (connecting terminal…)"
            } else {
                "▶  Run"
            };
            if button::chrome_button(ui, run_text, ButtonKind::Accent, colors, 120.0).clicked() {
                self.execute();
            }
        });

        // Last run hint
        if !self.last_run.is_empty() {
            ui.add_space(style::SPACE_SM);
            ui.horizontal(|ui| {
                ui.add_space(style::SPACE_MD);
                ui.label(
                    RichText::new(format!("last: $ {}", self.last_run))
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim)
                        .monospace(),
                );
            });
        }

        // Footer
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(style::SPACE_SM);
            ui.horizontal(|ui| {
                ui.add_space(style::SPACE_MD);
                ui.label(
                    RichText::new("⏎ run  esc back")
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                );
            });
        });
    }

    /// Renders one arg/flag field. When `want_focus` is true and this field is
    /// a text input, it requests keyboard focus and returns true so the caller
    /// stops offering focus to later fields.
    fn render_field(
        &mut self,
        ui: &mut egui::Ui,
        colors: &crate::ui::theme::Colors,
        arg: &ArgSpec,
        want_focus: bool,
    ) -> bool {
        let mut took_focus = false;
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_MD);
            ui.vertical(|ui| {
                // Label
                let required = arg.required.unwrap_or(false);
                let label_text = if required {
                    format!("{}*", arg.name)
                } else {
                    arg.name.clone()
                };
                ui.label(
                    RichText::new(&label_text)
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_primary),
                );

                if let Some(desc) = &arg.description {
                    if !desc.is_empty() {
                        ui.label(
                            RichText::new(desc)
                                .size(style::TEXT_HINT)
                                .color(colors.text_dim),
                        );
                    }
                }

                // Input
                let val = self.field_values.entry(arg.name.clone()).or_default();

                match arg.ty {
                    ArgType::Bool => {
                        let mut checked = val == "true";
                        if ui.checkbox(&mut checked, "").changed() {
                            *val = if checked {
                                "true".into()
                            } else {
                                String::new()
                            };
                        }
                    }
                    ArgType::Enum => {
                        if let Some(options) = &arg.enum_values {
                            egui::ComboBox::from_id_salt(&arg.name)
                                .selected_text(if val.is_empty() {
                                    "(select)"
                                } else {
                                    val.as_str()
                                })
                                .show_ui(ui, |ui| {
                                    for opt in options {
                                        ui.selectable_value(val, opt.clone(), opt);
                                    }
                                });
                        }
                    }
                    _ => {
                        let placeholder = arg.placeholder.as_deref().unwrap_or("");
                        ui.set_max_width(ui.available_width() - style::SPACE_MD);
                        let resp = crate::ui::text_field::styled_text_input(
                            ui,
                            val,
                            placeholder,
                            egui::Id::new(("cli_renderer_field", arg.name.as_str())),
                            colors,
                        );
                        if want_focus && !resp.has_focus() {
                            resp.request_focus();
                            took_focus = true;
                            log::info!(
                                "CliRendererApp: auto-focused field '{}'",
                                arg.name
                            );
                        }
                    }
                }

                ui.add_space(style::SPACE_SM);
            });
        });
        took_focus
    }
}

// ── App trait ─────────────────────────────────────────────────────────────────

impl App for CliRendererApp {
    fn type_id(&self) -> &'static str {
        "cli-renderer"
    }

    fn display_name(&self) -> String {
        if self.binary_name.is_empty() {
            "CLI Renderer".to_string()
        } else {
            self.binary_name.clone()
        }
    }

    fn keyboard_capture(&self) -> bool {
        matches!(self.view, View::Form)
    }

    fn handle_key(&mut self, input: &egui::InputState) -> KeyDisposition {
        match &self.view {
            View::List => {
                let commands = self.commands_at_path();
                let count = commands.len();
                if count == 0 {
                    return KeyDisposition::Passthrough;
                }

                if input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::J) {
                    self.selected = (self.selected + 1).min(count - 1);
                    return KeyDisposition::Consumed;
                }
                if input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::K) {
                    self.selected = self.selected.saturating_sub(1);
                    return KeyDisposition::Consumed;
                }
                // Cmd+Enter is the host pane-zoom toggle (src/host/keys.rs) — let
                // it pass through; plain Enter opens the selected command.
                if input.key_pressed(egui::Key::Enter) && !input.modifiers.command {
                    self.navigate_into(self.selected);
                    return KeyDisposition::Consumed;
                }
                if input.key_pressed(egui::Key::Escape) && !self.cmd_path.is_empty() {
                    self.navigate_back();
                    return KeyDisposition::Consumed;
                }
                KeyDisposition::Passthrough
            }
            View::Form => {
                if input.key_pressed(egui::Key::Escape) {
                    self.navigate_back();
                    return KeyDisposition::Consumed;
                }
                // Plain Enter runs the assembled command (single-line fields, so
                // Enter has no in-field meaning to compete with). Cmd+Enter is
                // reserved for the host pane-zoom toggle (src/host/keys.rs), so
                // let it pass through instead of running.
                if input.key_pressed(egui::Key::Enter) && !input.modifiers.command {
                    self.execute();
                    return KeyDisposition::Consumed;
                }
                KeyDisposition::Passthrough
            }
            _ => KeyDisposition::Passthrough,
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        // Request terminal on first render
        if !self.terminal_requested && self.descriptor.is_some() {
            self.request_terminal();
            if self.view == View::Loading {
                self.view = View::List;
            }
        }

        let colors = ctx.colors;

        match &self.view {
            View::Loading => {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new("Loading…")
                            .size(style::TEXT_BODY)
                            .color(colors.text_dim),
                    );
                });
            }
            View::Error(msg) => {
                let msg = msg.clone();
                ui.vertical(|ui| {
                    ui.add_space(style::SPACE_XL);
                    ui.horizontal(|ui| {
                        ui.add_space(style::SPACE_MD);
                        ui.label(
                            RichText::new("Error")
                                .size(style::TEXT_BODY)
                                .color(colors.danger)
                                .strong(),
                        );
                    });
                    ui.add_space(style::SPACE_SM);
                    ui.horizontal(|ui| {
                        ui.add_space(style::SPACE_MD);
                        ui.label(
                            RichText::new(&msg)
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim),
                        );
                    });
                });
            }
            View::List => {
                self.render_list(ui, colors);
            }
            View::Form => {
                self.render_form(ui, colors);
            }
        }
    }

    fn wants_close(&self) -> bool {
        false
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_commands)
    }

    fn queue_outbound_event(&mut self, event: crate::app_protocol::PlexiEvent) {
        // Handle LinkedTerminalReady response
        if let crate::app_protocol::PlexiEvent::LinkedTerminalReady {
            request_id,
            terminal_pane_id,
        } = &event
        {
            if request_id == &self.terminal_request_id {
                self.terminal_pane_id = *terminal_pane_id;
                log::info!(
                    "CliRendererApp: linked terminal ready, pane_id={}",
                    terminal_pane_id
                );
            }
        }
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        None
    }

    fn restore_state(&mut self, _state: &serde_json::Value) {}
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Colors;

    const FIXTURE: &str = r#"{
        "plexi_version": "0.1",
        "name": "demo",
        "version": "1.2.3",
        "description": "A demo CLI",
        "default_view": "list",
        "commands": [
            {
                "name": "greet",
                "description": "Greet someone",
                "args": [
                    {"name": "name", "type": "string", "required": true, "placeholder": "world"}
                ],
                "flags": [
                    {"name": "--loud", "type": "bool"},
                    {"name": "--lang", "type": "enum", "enum_values": ["en", "fr"]},
                    {"name": "--output", "type": "path", "placeholder": "FILE"}
                ]
            },
            {
                "name": "remote",
                "description": "Manage remotes",
                "commands": [
                    {"name": "add", "args": [{"name": "url", "type": "string", "required": true}]}
                ]
            }
        ]
    }"#;

    fn app_from_fixture() -> (CliRendererApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("descriptor.json");
        std::fs::write(&path, FIXTURE).expect("write fixture");
        let app = CliRendererApp::new(path.to_string_lossy().into_owned());
        (app, dir)
    }

    #[test]
    fn loads_descriptor_and_lists_top_level_commands() {
        let (app, _dir) = app_from_fixture();
        let d = app.descriptor.as_ref().expect("descriptor loaded");
        assert_eq!(d.name, "demo");
        let names: Vec<&str> = app.commands_at_path().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["greet", "remote"]);
    }

    #[test]
    fn navigate_into_leaf_opens_form_with_defaults() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(0); // greet — a leaf with fields
        assert_eq!(app.view, View::Form);
        assert_eq!(app.cmd_path, vec!["greet".to_string()]);
        assert!(app.current_command().is_some());
    }

    #[test]
    fn navigate_into_parent_opens_list() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(1); // remote — has children
        assert_eq!(app.view, View::List);
        let names: Vec<&str> = app.commands_at_path().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["add"]);
    }

    #[test]
    fn build_command_string_assembles_flags_and_args() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(0); // greet
        app.field_values.insert("name".into(), "Ada Lovelace".into());
        app.field_values.insert("--loud".into(), "true".into());
        app.field_values.insert("--lang".into(), "fr".into());
        let cmd = app.build_command_string();
        // Binary + subcommand present.
        assert!(cmd.starts_with("demo greet"), "got: {cmd}");
        // Bool flag rendered as bare flag.
        assert!(cmd.contains("--loud"), "got: {cmd}");
        // Value flag rendered as flag + value.
        assert!(cmd.contains("--lang fr"), "got: {cmd}");
        // Positional value with spaces is quoted.
        assert!(cmd.contains("'Ada Lovelace'"), "got: {cmd}");
        // Unset path flag is omitted.
        assert!(!cmd.contains("--output"), "got: {cmd}");
    }

    #[test]
    fn renders_without_panicking() {
        // Smoke test: drive a real egui frame through the renderer in List and
        // Form views to catch layout panics on the host UI thread.
        let (mut app, _dir) = app_from_fixture();
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let ctx = egui::Context::default();
        // ListRow renders with the custom "ui-medium" font family, which only
        // exists once the app's fonts are installed.
        crate::ui::theme::setup_fonts(&ctx);

        let render_once = |app: &mut CliRendererApp| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rctx = AppRenderContext {
                        colors: &colors,
                        is_focused: true,
                    };
                    app.ui(ui, &rctx);
                });
            });
        };

        // First frame transitions Loading → List and requests a terminal.
        render_once(&mut app);
        assert_eq!(app.view, View::List);
        // Drill into the form view and render again.
        app.navigate_into(0);
        render_once(&mut app);
        assert_eq!(app.view, View::Form);
    }

    /// Drive `handle_key` with a single key event through a real egui frame.
    fn press_key(app: &mut CliRendererApp, key: egui::Key, modifiers: egui::Modifiers) -> KeyDisposition {
        let ctx = egui::Context::default();
        let mut disposition = KeyDisposition::Passthrough;
        let _ = ctx.run(
            egui::RawInput {
                // `input.modifiers` reads from the top-level snapshot, not the
                // per-event field — set both so the guard sees the modifier.
                modifiers,
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ctx| {
                ctx.input(|i| disposition = app.handle_key(i));
            },
        );
        disposition
    }

    #[test]
    fn plain_enter_runs_the_form() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(0); // greet — a leaf form
        app.terminal_pane_id = 5; // pretend the linked terminal is ready
        app.field_values.insert("name".into(), "Ada".into());

        // Plain Enter (no Cmd) must execute — the prior contract required Cmd+Enter.
        let disp = press_key(&mut app, egui::Key::Enter, egui::Modifiers::default());
        assert_eq!(disp, KeyDisposition::Consumed);

        let cmds = app.take_pending_commands();
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                AppCommand::RunInLinkedTerminal { command, .. } if command.contains("greet")
            )),
            "plain Enter should queue a RunInLinkedTerminal for the greet command"
        );
    }

    #[test]
    fn cmd_enter_passes_through_for_pane_zoom() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(0); // greet — a leaf form
        app.terminal_pane_id = 5;
        app.field_values.insert("name".into(), "Ada".into());

        // Cmd+Enter is the host pane-zoom toggle — the renderer must not eat it.
        let disp = press_key(
            &mut app,
            egui::Key::Enter,
            egui::Modifiers {
                command: true,
                ..Default::default()
            },
        );
        assert_eq!(disp, KeyDisposition::Passthrough);
        assert!(
            app.take_pending_commands().is_empty(),
            "Cmd+Enter must not run the command — it belongs to the host zoom toggle"
        );
    }

    #[test]
    fn opening_a_leaf_form_arms_field_focus() {
        let (mut app, _dir) = app_from_fixture();
        assert!(!app.pending_focus);
        app.navigate_into(0); // greet — a leaf form with a text field
        assert!(
            app.pending_focus,
            "opening a leaf form must arm auto-focus for the first text field"
        );
    }

    #[test]
    fn requests_linked_terminal_below() {
        let (mut app, _dir) = app_from_fixture();
        app.request_terminal();
        let cmds = app.take_pending_commands();
        let placed_below = cmds.iter().any(|c| {
            matches!(
                c,
                AppCommand::RequestLinkedTerminal { place_below, .. } if *place_below
            )
        });
        assert!(
            placed_below,
            "CLI renderer must request its terminal stacked below the form"
        );
    }

    #[test]
    fn bad_descriptor_path_yields_error_view() {
        let app = CliRendererApp::new("/nonexistent/descriptor/path.json".into());
        assert!(matches!(app.view, View::Error(_)));
        assert!(app.descriptor.is_none());
    }
}
