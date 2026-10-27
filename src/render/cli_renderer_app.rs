//! Built-in CLI renderer: a native host app that turns any CLI into a GUI.
//!
//! Parses a `PlexiDescriptor` JSON file (resolved by the Tier 1/2/3 chain in
//! `cli::descriptor` — native `--plexi`, registry, or recursive `--help`
//! crawl), renders a navigable command list and per-command form, and executes
//! the assembled command in a linked terminal pane.

use crate::app::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};
use crate::app::canvas_bindings::quote_for_shell;
use crate::app::plexi_descriptor::{ArgSpec, ArgType, Command, PlexiDescriptor, ROOT_COMMAND_NAME};
use crate::ui::{
    app_chrome::AppChrome,
    button::{self, ButtonKind},
    hints::{HintBar, HintGroup},
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
    /// Path the descriptor was read from. Owned so the per-pane temp descriptor
    /// (`$TMPDIR/plexi-descriptor-<uuid>.json`) is deleted on pane close via
    /// `Drop`, instead of leaking one file per CLI open (#2246).
    descriptor_path: String,
    /// Linked terminal pane id (0 = not yet linked).
    terminal_pane_id: u64,
    /// Set when the terminal-link handshake returns pane id 0 (host could not
    /// spawn the linked terminal). Distinguishes a hard failure from the normal
    /// "still connecting" state so the renderer surfaces a visible error instead
    /// of a stuck "(connecting terminal…)" label (#2246).
    terminal_link_failed: bool,
    /// Pending AppCommands to drain each frame.
    pending_commands: Vec<AppCommand>,
    /// Last command string that was run (shown as a hint).
    last_run: String,
    /// Fully assembled command selected before its linked terminal was ready.
    /// Keeping the command rather than the leaf path lets the list remain at
    /// its parent level while the terminal-link handshake completes.
    pending_auto_run: Option<String>,
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
    #[cfg(test)]
    pub(crate) last_header_rect: Option<egui::Rect>,
    #[cfg(test)]
    pub(crate) last_footer_chip_rects: Vec<egui::Rect>,
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

        Self::with_descriptor(descriptor, view, descriptor_path)
    }

    /// Open an already resolved descriptor without a temporary file.
    pub fn from_descriptor(descriptor: PlexiDescriptor) -> Self {
        log::info!("CliRendererApp: UI upcycle opened '{}'", descriptor.name);
        Self::with_descriptor(Some(descriptor), View::Loading, String::new())
    }

    fn with_descriptor(
        descriptor: Option<PlexiDescriptor>,
        view: View,
        descriptor_path: String,
    ) -> Self {
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
            descriptor_path,
            terminal_pane_id: 0,
            terminal_link_failed: false,
            pending_commands: vec![],
            last_run: String::new(),
            pending_auto_run: None,
            terminal_requested: false,
            terminal_request_id: format!("cli-term-{}", uuid::Uuid::new_v4()),
            binary_name,
            focused_field: None,
            pending_focus: false,
            #[cfg(test)]
            last_header_rect: None,
            #[cfg(test)]
            last_footer_chip_rects: Vec::new(),
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
            {
                let cmd = cmds.iter().find(|c| &c.name == segment)?;
                if i == self.cmd_path.len() - 1 {
                    return Some(cmd);
                }
                cmds = &cmd.commands;
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
        let has_fields = !cmd.flags.is_empty() || !cmd.args.is_empty();
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
        } else if !has_fields {
            // Argless leaves have nothing to configure. Run them directly, but
            // return to this list so the user can run a neighboring command.
            let command = self.build_command_string();
            if !self.execute_command(command.clone()) {
                self.pending_auto_run = Some(command);
                self.request_terminal();
            }
            self.cmd_path.pop();
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
        let mut parts = vec![quote_for_shell(&d.name)];
        parts.extend(
            self.cmd_path
                .iter()
                .filter(|name| name.as_str() != ROOT_COMMAND_NAME)
                .map(|name| quote_for_shell(name)),
        );

        if let Some(cmd) = self.current_command() {
            for flag in &cmd.flags {
                if let Some(val) = self.field_values.get(&flag.name) {
                    if val.is_empty() {
                        continue;
                    }
                    match flag.ty {
                        ArgType::Bool => {
                            if val == "true" {
                                parts.push(quote_for_shell(&flag.name));
                            }
                        }
                        _ => {
                            parts.push(quote_for_shell(&flag.name));
                            parts.push(quote_for_shell(val));
                        }
                    }
                }
            }
            for arg in &cmd.args {
                if let Some(val) = self.field_values.get(&arg.name) {
                    if !val.is_empty() {
                        parts.push(quote_for_shell(val));
                    }
                }
            }
        }
        parts.join(" ")
    }

    fn execute(&mut self) {
        let _ = self.execute_command(self.build_command_string());
    }

    /// Queue an already assembled command in the linked terminal. Returns
    /// whether it was queued, so an argless leaf can retry after its terminal
    /// link becomes ready.
    fn execute_command(&mut self, cmd: String) -> bool {
        if self.terminal_pane_id == 0 {
            log::warn!("CliRendererApp: no linked terminal, cannot run");
            return false;
        }
        if cmd.is_empty() {
            return false;
        }
        self.last_run = cmd.clone();
        self.pending_commands.push(AppCommand::RunInLinkedTerminal {
            sender_pane_id: 0, // host fills this
            terminal_pane_id: self.terminal_pane_id,
            command: cmd.clone(),
            echo: true,
        });
        log::info!("CliRendererApp: queued run command_chars={}", cmd.chars().count());
        true
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
        if self.commands_at_path().is_empty() {
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
            let path = self
                .cmd_path
                .iter()
                .filter(|name| name.as_str() != ROOT_COMMAND_NAME)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            if path.is_empty() {
                d.name.clone()
            } else {
                format!("{} {path}", d.name)
            }
        };
        let top_desc = if self.cmd_path.is_empty() {
            d.description.clone()
        } else {
            None
        };
        let has_path = !self.cmd_path.is_empty();

        // Shared AppBar header
        let header_rect =
            AppChrome::new(colors).paint_app_bar(ui, &title, top_desc.as_deref().unwrap_or(""));
        #[cfg(test)]
        {
            self.last_header_rect = Some(header_rect);
        }
        #[cfg(not(test))]
        let _ = header_rect;

        // Back button if navigated deep
        if has_path && Self::back_row(ui, colors) {
            self.navigate_back();
            return;
        }

        let hint_groups: &[HintGroup<'_>] = if has_path {
            &[
                HintGroup::new(&["↑", "↓"], "navigate"),
                HintGroup::new(&["⏎"], "open"),
                HintGroup::new(&["Esc"], "back"),
            ]
        } else {
            &[
                HintGroup::new(&["↑", "↓"], "navigate"),
                HintGroup::new(&["⏎"], "open"),
            ]
        };
        let footer_h = HintBar::new(hint_groups).band_height(ui, true);
        let list_max_h = (ui.available_height() - footer_h).max(0.0);

        // Command list — rendered with the shared host ListRow so it matches
        // the command palette, sidebar, and PGAP list views exactly.
        let mut clicked_idx: Option<usize> = None;
        let selection_changed = self.selected != self.last_selected;
        let rows = self.commands_at_path();
        egui::ScrollArea::vertical()
            .animated(false)
            .max_height(list_max_h)
            .show(ui, |ui| {
                clicked_idx = super::descriptor_widgets::command_list(
                    ui,
                    colors,
                    rows.iter().copied(),
                    &format!("Run {}", self.binary_name),
                    self.selected,
                    selection_changed,
                );
            });
        self.last_selected = self.selected;

        if let Some(i) = clicked_idx {
            self.selected = i;
            self.navigate_into(i);
            return;
        }

        // Footer hints band
        let remaining = (ui.available_height() - footer_h).max(0.0);
        if remaining > 0.0 {
            ui.add_space(remaining);
        }
        let hint_resp = HintBar::new(hint_groups)
            .pane_footer_band(true)
            .show_measured(ui, colors);
        #[cfg(test)]
        {
            self.last_footer_chip_rects = hint_resp.chip_rects;
        }
        #[cfg(not(test))]
        let _ = hint_resp;
    }

    fn render_form(&mut self, ui: &mut egui::Ui, colors: &crate::ui::theme::Colors, pane_key: u64) {
        let cmd = match self.current_command() {
            Some(c) => c,
            None => {
                self.navigate_back();
                return;
            }
        };

        let d = self.descriptor.as_ref().unwrap();
        let path = self
            .cmd_path
            .iter()
            .filter(|name| name.as_str() != ROOT_COMMAND_NAME)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        let title = if path.is_empty() {
            d.name.clone()
        } else {
            format!("{} {path}", d.name)
        };
        let cmd_desc = cmd.description.clone();
        let args: Vec<ArgSpec> = cmd.args.clone();
        let flags: Vec<ArgSpec> = cmd.flags.clone();
        let has_fields = !args.is_empty() || !flags.is_empty();

        // Shared AppBar header
        let header_rect =
            AppChrome::new(colors).paint_app_bar(ui, &title, cmd_desc.as_deref().unwrap_or(""));
        #[cfg(test)]
        {
            self.last_header_rect = Some(header_rect);
        }
        #[cfg(not(test))]
        let _ = header_rect;

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

        let form_hints = [
            HintGroup::new(&["⏎"], "run"),
            HintGroup::new(&["Esc"], "back"),
        ];
        let footer_h = HintBar::new(&form_hints).band_height(ui, true);

        let mut should_execute = false;
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - footer_h).max(40.0))
            .show(ui, |ui| {
                // Arg fields. On a freshly opened form, the first text field grabs
                // keyboard focus so the user can type immediately.
                if has_fields {
                    super::descriptor_widgets::form_fields(
                        ui,
                        colors,
                        args.iter().chain(flags.iter()),
                        &mut self.field_values,
                        self.pending_focus,
                        pane_key,
                    );
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
                    let run_text = if self.terminal_link_failed {
                        "▶  Run  (terminal unavailable)"
                    } else if self.terminal_pane_id == 0 {
                        "▶  Run  (connecting terminal…)"
                    } else {
                        "▶  Run"
                    };
                    if button::chrome_button(ui, run_text, ButtonKind::Accent, colors, 120.0)
                        .clicked()
                    {
                        should_execute = true;
                    }
                });

                // Surface a visible error when the terminal link hard-failed, so the Run
                // affordance is not silently inert (#2246).
                if self.terminal_link_failed {
                    ui.add_space(style::SPACE_SM);
                    ui.horizontal(|ui| {
                        ui.add_space(style::SPACE_MD);
                        ui.label(
                            RichText::new(
                                "Could not open a linked terminal — reopen this tool to retry.",
                            )
                            .size(style::TEXT_CAPTION)
                            .color(colors.danger),
                        );
                    });
                }

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
            });
        self.pending_focus = false;
        if should_execute {
            self.execute();
        }

        // Shared footer hints band
        let remaining = (ui.available_height() - footer_h).max(0.0);
        if remaining > 0.0 {
            ui.add_space(remaining);
        }
        let hint_resp = HintBar::new(&form_hints)
            .pane_footer_band(true)
            .show_measured(ui, colors);
        #[cfg(test)]
        {
            self.last_footer_chip_rects = hint_resp.chip_rects;
        }
        #[cfg(not(test))]
        let _ = hint_resp;
    }
}

impl Drop for CliRendererApp {
    /// Delete the per-pane temp descriptor on close (#2246). Guarded to the
    /// host-minted `$TMPDIR/plexi-descriptor-*.json` pattern so a caller-supplied
    /// or test-fixture path is never touched.
    fn drop(&mut self) {
        let path = std::path::Path::new(&self.descriptor_path);
        let in_temp_dir = path.parent() == Some(std::env::temp_dir().as_path());
        let is_descriptor = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("plexi-descriptor-") && n.ends_with(".json"));
        if in_temp_dir && is_descriptor {
            match std::fs::remove_file(path) {
                Ok(()) => log::info!(
                    "CliRendererApp: cleaned up temp descriptor {}",
                    self.descriptor_path
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => log::warn!(
                    "CliRendererApp: failed to remove temp descriptor {}: {e}",
                    self.descriptor_path
                ),
            }
        }
    }
}

// ── App trait ─────────────────────────────────────────────────────────────────

impl App for CliRendererApp {
    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

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

    fn submit_on_focused_text_input_enter(&self) -> bool {
        // CLI form fields are all single-line controls; bare Enter is the
        // documented Run action, not text entry. Keep this scoped to Form so
        // list navigation and loading/error views retain their normal routing.
        matches!(self.view, View::Form)
    }

    fn handle_key(&mut self, input: &crate::app::input_router::PlexiInput) -> KeyDisposition {
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
                if input.key_pressed(egui::Key::Enter) && !input.modifiers().command {
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
                if input.key_pressed(egui::Key::Enter) && !input.modifiers().command {
                    self.execute();
                    return KeyDisposition::Consumed;
                }
                KeyDisposition::Passthrough
            }
            _ => KeyDisposition::Passthrough,
        }
    }

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &AppRenderContext<'_>,
        _pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
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
                self.render_form(ui, colors, ctx.pane_id);
            }
        }
    }

    fn wants_close(&self) -> bool {
        false
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_commands)
    }

    fn queue_outbound_event(&mut self, event: crate::protocol::PlexiEvent) {
        // Handle LinkedTerminalReady response
        if let crate::protocol::PlexiEvent::LinkedTerminalReady {
            request_id,
            terminal_pane_id,
        } = &event
        {
            if request_id == &self.terminal_request_id {
                self.terminal_pane_id = *terminal_pane_id;
                if *terminal_pane_id == 0 {
                    // Handshake completed but the host could not spawn a linked
                    // terminal — surface it instead of a stuck "connecting" state.
                    self.terminal_link_failed = true;
                    log::warn!(
                        "CliRendererApp: linked terminal request {request_id} returned pane id 0 \
                         — terminal link failed"
                    );
                } else {
                    self.terminal_link_failed = false;
                    log::info!(
                        "CliRendererApp: linked terminal ready, pane_id={}",
                        terminal_pane_id
                    );
                    if let Some(command) = self.pending_auto_run.take() {
                        if !self.execute_command(command.clone()) {
                            self.pending_auto_run = Some(command);
                        }
                    }
                }
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
                    {"name": "add", "args": [{"name": "url", "type": "string", "required": true}]},
                    {"name": "status", "description": "Show remote status"}
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
        let names: Vec<&str> = app
            .commands_at_path()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
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
        let names: Vec<&str> = app
            .commands_at_path()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["add", "status"]);
    }

    #[test]
    fn navigate_into_argless_leaf_runs_in_linked_terminal_and_stays_in_list() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(1); // remote — parent with children
        app.terminal_pane_id = 5; // pretend the linked terminal is ready

        app.navigate_into(1); // status — leaf with no flags or args

        assert_eq!(app.view, View::List);
        assert_eq!(app.cmd_path, vec!["remote".to_string()]);
        assert_eq!(app.last_run, "demo remote status");
        assert!(
            app.take_pending_commands().iter().any(|command| matches!(
                command,
                AppCommand::RunInLinkedTerminal {
                    terminal_pane_id: 5,
                    command,
                    ..
                } if command == "demo remote status"
            )),
            "argless leaf must queue directly in its linked terminal"
        );
    }

    #[test]
    fn argless_leaf_waits_for_linked_terminal_then_runs() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(1); // remote — parent with children

        app.navigate_into(1); // status — leaf with no flags or args

        assert_eq!(app.view, View::List);
        assert_eq!(app.cmd_path, vec!["remote".to_string()]);
        assert_eq!(
            app.pending_auto_run.as_deref(),
            Some("demo remote status"),
            "argless command must wait for the terminal handshake"
        );

        let request_id = app.terminal_request_id.clone();
        app.queue_outbound_event(crate::protocol::PlexiEvent::LinkedTerminalReady {
            request_id,
            terminal_pane_id: 5,
        });

        assert!(app.pending_auto_run.is_none());
        assert_eq!(app.last_run, "demo remote status");
        assert!(
            app.take_pending_commands().iter().any(|command| matches!(
                command,
                AppCommand::RunInLinkedTerminal {
                    terminal_pane_id: 5,
                    command,
                    ..
                } if command == "demo remote status"
            )),
            "terminal readiness must flush the deferred argless command"
        );
    }

    #[test]
    fn build_command_string_assembles_flags_and_args() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(0); // greet
        app.field_values
            .insert("name".into(), "Ada Lovelace".into());
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
    fn ui_upcycle_registry_keyboard_walks_emit_expected_commands() {
        let cases: &[(&str, &[&str], &str, &str, &str)] = &[
            (
                "git",
                &["log"],
                "--author",
                "Ada Lovelace",
                "git log --author 'Ada Lovelace'",
            ),
            (
                "cargo",
                &["build"],
                "--target",
                "aarch64-apple-darwin",
                "cargo build --target aarch64-apple-darwin",
            ),
            ("gh", &["pr", "view"], "number", "123", "gh pr view 123"),
        ];
        for (binary, path, field, value, expected) in cases {
            let descriptor = crate::cli::registry::lookup_with_backends(
                &[&crate::cli::registry::EmbeddedBackend],
                binary,
                None,
            )
            .expect("shipped descriptor");
            let mut app = CliRendererApp::from_descriptor(descriptor);
            app.view = View::List;
            app.terminal_pane_id = 42;
            for segment in *path {
                let index = app
                    .commands_at_path()
                    .iter()
                    .position(|c| c.name == *segment)
                    .unwrap();
                for _ in 0..index {
                    assert_eq!(
                        press_key(&mut app, egui::Key::ArrowDown, egui::Modifiers::NONE),
                        KeyDisposition::Consumed
                    );
                }
                assert_eq!(
                    press_key(&mut app, egui::Key::Enter, egui::Modifiers::NONE),
                    KeyDisposition::Consumed
                );
            }
            assert_eq!(app.view, View::Form, "{binary}");
            app.field_values.insert((*field).into(), (*value).into());
            press_key(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
            assert!(app.take_pending_commands().iter().any(|command| matches!(command,
                AppCommand::RunInLinkedTerminal { terminal_pane_id: 42, command, .. } if command == expected
            )), "{binary}");
            press_key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
            assert_eq!(app.view, View::List);
        }
    }

    #[test]
    #[ignore = "requires installed git, cargo and gh; local CLI launch acceptance"]
    fn ui_upcycle_installed_cli_commands_produce_output() {
        let tmp = tempfile::tempdir().unwrap();
        for (binary, path, field, value, output_hint) in [
            (
                "git",
                vec!["init"],
                "directory",
                "git workspace",
                "Initialized empty Git repository",
            ),
            (
                "cargo",
                vec!["new"],
                "path",
                "sample_package",
                "Creating binary",
            ),
            ("gh", vec!["pr", "view"], "number", "", "Display the title"),
        ] {
            let descriptor = crate::cli::registry::lookup_with_backends(
                &[&crate::cli::registry::EmbeddedBackend],
                binary,
                None,
            )
            .unwrap();
            let mut app = CliRendererApp::from_descriptor(descriptor);
            app.view = View::List;
            app.terminal_pane_id = 42;
            for segment in path {
                let index = app
                    .commands_at_path()
                    .iter()
                    .position(|c| c.name == segment)
                    .unwrap();
                for _ in 0..index {
                    press_key(&mut app, egui::Key::ArrowDown, egui::Modifiers::NONE);
                }
                press_key(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
            }
            app.field_values.insert(field.into(), value.into());
            press_key(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
            let command = app
                .take_pending_commands()
                .into_iter()
                .find_map(|c| match c {
                    AppCommand::RunInLinkedTerminal { command, .. } => Some(command),
                    _ => None,
                })
                .unwrap();
            // gh help exercises the emitted nested command without account/network access.
            let command = if binary == "gh" {
                format!("{command} --help")
            } else {
                command
            };
            let output = std::process::Command::new("sh")
                .args(["-c", &command])
                .current_dir(tmp.path())
                .env("LC_ALL", "C")
                .output()
                .unwrap();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.status.success(), "{command}: {text}");
            assert!(text.contains(output_hint), "{command}: {text}");
        }
    }

    #[test]
    fn form_values_round_trip_through_shell_as_literal_arguments() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(0);
        for value in [
            "it's ready",
            "$(printf changed)",
            "a;b",
            "line\nbreak",
            "*.rs",
        ] {
            app.field_values.clear();
            app.field_values.insert("name".into(), value.into());
            let command = app.build_command_string();
            let output = std::process::Command::new("sh")
                .args([
                    "-c",
                    &format!("demo() {{ printf '%s' \"$2\"; }}; {command}"),
                ])
                .output()
                .expect("shell round trip");
            assert!(output.status.success(), "{command}");
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                value,
                "{command}"
            );
        }
    }

    #[test]
    fn generated_root_form_omits_synthetic_command_name() {
        let (mut app, _dir) = app_from_fixture();
        let root = Command {
            name: ROOT_COMMAND_NAME.to_string(),
            description: Some("Run demo with options".to_string()),
            icon: None,
            ui_hint: None,
            args: vec![],
            flags: vec![ArgSpec {
                name: "--verbose".to_string(),
                ty: ArgType::Bool,
                required: Some(false),
                default: None,
                description: None,
                placeholder: None,
                enum_values: None,
                min: None,
                max: None,
            }],
            writes: vec![],
            reads: vec![],
            streaming: None,
            output_format: None,
            commands: vec![],
        };
        app.descriptor.as_mut().unwrap().commands = vec![root];

        app.navigate_into(0);
        app.field_values.insert("--verbose".into(), "true".into());

        assert_eq!(app.view, View::Form);
        assert_eq!(app.build_command_string(), "demo --verbose");
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
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    let rctx = AppRenderContext {
                        colors: &colors,
                        pane_id: 1,
                    };
                    app.ui(ui, &rctx, None);
                });
            });
        };

        // First frame transitions Loading → List and requests a terminal.
        render_once(&mut app);
        assert_eq!(app.view, View::List);
        assert!(
            app.last_header_rect.is_some(),
            "list view must render shared AppChrome header"
        );
        assert!(
            app.last_footer_chip_rects.len() >= 3,
            "list view must render key chips (↑, ↓, ⏎) via HintBar"
        );
        // Drill into the form view and render again.
        app.navigate_into(0);
        render_once(&mut app);
        assert_eq!(app.view, View::Form);
        assert!(
            app.last_header_rect.is_some(),
            "form view must render shared AppChrome header"
        );
        assert_eq!(
            app.last_footer_chip_rects.len(),
            2,
            "form view must render ⏎ and Esc key chips via HintBar"
        );
    }

    /// Drive `handle_key` with a single key event through a real egui frame.
    fn press_key(
        app: &mut CliRendererApp,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) -> KeyDisposition {
        let ctx = egui::Context::default();
        let mut disposition = KeyDisposition::Passthrough;
        let _ = ctx.run_ui(
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
            |ui| {
                let input = crate::app::input_router::PlexiInput::take_from(ui.ctx());
                disposition = app.handle_key(&input);
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
    fn focused_text_field_enter_runs_the_form() {
        let (mut app, _dir) = app_from_fixture();
        app.navigate_into(0); // greet — a leaf form with a text field
        app.terminal_pane_id = 5;
        app.field_values.insert("name".into(), "Ada".into());

        // Mount the form once so its first field takes real egui focus, as it
        // does when `app open --cli` first shows a command form.
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                let rctx = AppRenderContext {
                    colors: &colors,
                    pane_id: 1,
                };
                app.ui(ui, &rctx, None);
            });
        });
        // Production's post-frame focus reconciler applies the form's claim
        // only when this pane owns input. Reproduce that hand-off here before
        // feeding the next frame's focused-field key event.
        let field_id = egui::Id::new(("descriptor_field", 1_u64, "name"));
        let surfaces = crate::ui::focus::drain_text_surfaces(&ctx);
        assert_eq!(
            surfaces.claim_for(crate::ui::focus::SurfaceKey::Pane(1)),
            Some(field_id)
        );
        ctx.memory_mut(|memory| memory.request_focus(field_id));
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(field_id),
            "the regression must exercise Enter while the auto-focused field owns egui focus"
        );

        // This mirrors the focused-TextEdit dispatcher path: CLI forms opt in
        // to receiving bare Enter before the event is returned to TextEdit.
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                let mut input = crate::app::input_router::PlexiInput::take_from(ui.ctx());
                assert!(app.submit_on_focused_text_input_enter());
                assert_eq!(app.handle_key(&input), KeyDisposition::Consumed);
                assert!(input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                input.give_back(ui.ctx());
            },
        );

        assert!(
            app.take_pending_commands().iter().any(|command| matches!(
                command,
                AppCommand::RunInLinkedTerminal { command, .. } if command.contains("greet")
            )),
            "focused bare Enter should queue the CLI form command, not reach TextEdit"
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

    /// Dropping the renderer deletes its host-minted temp descriptor (#2246), so
    /// `$TMPDIR/plexi-descriptor-*.json` files do not accumulate per CLI open.
    #[test]
    fn drop_removes_temp_descriptor_file() {
        let path =
            std::env::temp_dir().join(format!("plexi-descriptor-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, FIXTURE).expect("write temp descriptor");
        assert!(path.exists());
        {
            let _app = CliRendererApp::new(path.to_string_lossy().into_owned());
        }
        assert!(
            !path.exists(),
            "temp descriptor must be removed when the renderer pane closes"
        );
    }

    /// A non-temp descriptor path (e.g. a test fixture) is never deleted on drop.
    #[test]
    fn drop_leaves_non_temp_descriptor_file() {
        let (app, dir) = app_from_fixture();
        let path = dir.path().join("descriptor.json");
        assert!(path.exists());
        drop(app);
        assert!(
            path.exists(),
            "a fixture descriptor outside the temp pattern must survive drop"
        );
    }

    /// A `LinkedTerminalReady` handshake that returns pane id 0 is a hard failure,
    /// not the normal "still connecting" state — the renderer flags it so a
    /// visible error replaces the stuck "connecting terminal…" label (#2246).
    #[test]
    fn zero_terminal_pane_id_flags_link_failure() {
        let (mut app, _dir) = app_from_fixture();
        assert!(!app.terminal_link_failed);
        let request_id = app.terminal_request_id.clone();
        app.queue_outbound_event(crate::protocol::PlexiEvent::LinkedTerminalReady {
            request_id,
            terminal_pane_id: 0,
        });
        assert!(
            app.terminal_link_failed,
            "pane id 0 must mark the terminal link as failed"
        );
        assert_eq!(app.terminal_pane_id, 0);
    }
}
