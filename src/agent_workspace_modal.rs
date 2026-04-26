//! "New Agent Workspace…" modal picker (#349).
//!
//! A keyboard-first modal with three inputs:
//!
//! 1. CLI dropdown — Claude Code / Codex / Gemini CLI. Unavailable binaries
//!    are greyed out with a "(not installed)" suffix.
//! 2. Repo picker — recent repos from `LastCliMap` + a free-form text field
//!    for an arbitrary path (no native file dialog: keeps the modal
//!    keyboard-driven and avoids a `rfd`-style dependency for one feature).
//! 3. Optional task prompt textarea.
//!
//! Submit (Enter) calls `open_agent_workspace_pane` and updates `LastCliMap`.
//! Escape closes the modal without spawning.
//!
//! The substrate's three flat palette commands ("New Agent Workspace: <CLI>")
//! are unaffected — this modal is the additive richer path.

use crate::agent_workspace::AgentCli;
use crate::overlays::MODAL_WIDTH;
use egui::{Align2, Color32, CornerRadius, RichText, Stroke, Vec2};
use std::path::PathBuf;

/// Active modal state. Lives on `PlexiApp::agent_workspace_modal` while the
/// modal is open; `None` means closed.
pub struct AgentWorkspaceModal {
    pub selected_cli: AgentCli,
    /// Text in the repo path field. Defaults to the active context's CWD if
    /// it's a git repo, else empty (the user fills it).
    pub repo_path: String,
    /// Free-form task label, multi-line.
    pub task: String,
    /// Recent repo paths, populated from `LastCliMap` at open-time. Capped at 5.
    pub recent_repos: Vec<PathBuf>,
    /// Detected install state per CLI — computed at open-time so we don't
    /// hit the filesystem every frame.
    pub install_state: [bool; 3],
}

impl AgentWorkspaceModal {
    /// Construct a fresh modal pre-filled from `last_cli_map` and the active
    /// context's CWD if it's a git repo.
    pub fn open(
        last_cli_map: &crate::agent_workspace::persistence::LastCliMap,
        default_cwd: Option<&std::path::Path>,
    ) -> Self {
        // Default repo: active context's CWD if it's a git repo.
        let repo_path = default_cwd
            .and_then(crate::agent_workspace::find_git_repo_root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Default CLI: most recent for the chosen repo, else the first
        // installed CLI, else Claude Code.
        let install_state = [
            AgentCli::ClaudeCode.is_installed(),
            AgentCli::Codex.is_installed(),
            AgentCli::GeminiCli.is_installed(),
        ];
        let selected_cli = (!repo_path.is_empty())
            .then(|| last_cli_map.lookup(std::path::Path::new(&repo_path)).map(|e| e.last_cli))
            .flatten()
            .or_else(|| {
                AgentCli::all()
                    .into_iter()
                    .find(|c| match c {
                        AgentCli::ClaudeCode => install_state[0],
                        AgentCli::Codex => install_state[1],
                        AgentCli::GeminiCli => install_state[2],
                    })
            })
            .unwrap_or(AgentCli::ClaudeCode);

        let recent_repos = last_cli_map.recent_repos(5);

        Self {
            selected_cli,
            repo_path,
            task: String::new(),
            recent_repos,
            install_state,
        }
    }

    fn cli_installed(&self, cli: AgentCli) -> bool {
        match cli {
            AgentCli::ClaudeCode => self.install_state[0],
            AgentCli::Codex => self.install_state[1],
            AgentCli::GeminiCli => self.install_state[2],
        }
    }
}

/// Outcome of one frame of the modal — the parent (PlexiApp::update) consumes
/// this to either spawn a pane (Submit) or close the modal (Cancel).
pub enum ModalOutcome {
    None,
    Cancel,
    Submit {
        cli: AgentCli,
        repo_path: PathBuf,
        task: String,
    },
}

impl crate::app::PlexiApp {
    /// Open the modal — entry point for the "New Agent Workspace…" palette
    /// command. If the modal was already open, this is a no-op (re-opening it
    /// would clobber in-progress input).
    pub(crate) fn open_agent_workspace_modal(&mut self) {
        if self.agent_workspace_modal.is_some() {
            return;
        }
        let default_cwd: Option<PathBuf> = self.contexts[self.active_context]
            .focused_pane
            .and_then(|fp| {
                self.contexts[self.active_context].get_focused_pane_cwd(fp)
            })
            .or_else(|| Some(self.contexts[self.active_context].path.clone()));

        let modal = AgentWorkspaceModal::open(&self.last_cli_map, default_cwd.as_deref());
        self.agent_workspace_modal = Some(modal);
        self.focus_stack
            .push(crate::app::FocusLayer::AgentWorkspaceModal);
    }

    /// Reconcile focus stack state for the agent workspace modal — invoked
    /// from the same `update()` block that runs the other `sync_*_focus`
    /// helpers.
    pub(crate) fn sync_agent_workspace_modal_focus(&mut self) {
        let open = self.agent_workspace_modal.is_some();
        let on_top = matches!(
            self.focus_stack.last(),
            Some(crate::app::FocusLayer::AgentWorkspaceModal)
        );
        if open && !on_top
            && !self
                .focus_stack
                .contains(&crate::app::FocusLayer::AgentWorkspaceModal)
        {
            self.focus_stack
                .push(crate::app::FocusLayer::AgentWorkspaceModal);
        }
        if !open && on_top {
            self.focus_stack.pop();
        }
    }

    /// Render the modal. Returns true if input was captured this frame.
    pub(crate) fn draw_agent_workspace_modal(&mut self, ctx: &egui::Context) {
        // We hold the modal state out of `self` while rendering so we can
        // mutate it freely without re-borrowing. The parent thread re-installs
        // it (if still open) at the end of the frame.
        let Some(mut modal) = self.agent_workspace_modal.take() else {
            return;
        };

        let outcome = render_modal(ctx, &mut modal, &self.colors);

        match outcome {
            ModalOutcome::None => {
                self.agent_workspace_modal = Some(modal);
            }
            ModalOutcome::Cancel => {
                // Modal stays gone. Focus pop happens in the next sync call.
            }
            ModalOutcome::Submit { cli, repo_path, task } => {
                self.spawn_agent_workspace_from_modal(cli, repo_path, task);
            }
        }
    }

    /// Spawn the pane + persist the CLI choice. Errors are logged + surfaced
    /// as a host notification — the modal is already gone by this point.
    fn spawn_agent_workspace_from_modal(
        &mut self,
        cli: AgentCli,
        repo_path: PathBuf,
        task: String,
    ) {
        // Resolve to git repo root (the modal accepts arbitrary subdirs).
        let resolved =
            crate::agent_workspace::find_git_repo_root(&repo_path).unwrap_or(repo_path.clone());

        // Pre-flight installer check for clearer error than a PTY ENOENT.
        if !cli.is_installed() {
            self.push_host_notification(
                "warn".to_string(),
                format!("{} is not installed", cli.display_name()),
                format!(
                    "The `{}` binary was not found on PATH or in common installer dirs. Install it before spawning a workspace.",
                    cli.binary_name()
                ),
            );
            return;
        }

        match self.spawn_agent_workspace_with_repo(cli, resolved.clone(), task) {
            Ok(()) => {
                self.last_cli_map.record(&resolved, cli);
                crate::agent_workspace::persistence::save(&self.last_cli_map);
                log::info!(
                    "agent_workspace: spawned {} at {}",
                    cli.display_name(),
                    resolved.display()
                );
            }
            Err(e) => {
                self.push_host_notification(
                    "error".to_string(),
                    format!("Failed to spawn {}", cli.display_name()),
                    e.to_string(),
                );
            }
        }
    }
}

fn render_modal(
    ctx: &egui::Context,
    modal: &mut AgentWorkspaceModal,
    colors: &crate::theme::Colors,
) -> ModalOutcome {
    let mut outcome = ModalOutcome::None;

    // Keyboard handling — Enter submits, Esc cancels.
    let mut submit_pressed = false;
    let mut escape_pressed = false;
    ctx.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            escape_pressed = true;
        }
        // Cmd+Enter submits even from inside the multi-line task field.
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter) {
            submit_pressed = true;
        }
    });
    if escape_pressed {
        return ModalOutcome::Cancel;
    }

    // Scrim.
    let screen_rect = ctx.screen_rect();
    egui::Area::new(egui::Id::new("agent_workspace_modal_scrim"))
        .fixed_pos(screen_rect.min)
        .show(ctx, |ui| {
            ui.painter()
                .rect_filled(screen_rect, 0.0, Color32::from_black_alpha(120));
            let r = ui.allocate_rect(screen_rect, egui::Sense::click());
            if r.clicked() {
                // Click outside cancels.
                ctx.memory_mut(|m| {
                    m.data
                        .insert_temp::<bool>(egui::Id::new("agent_workspace_modal_cancel"), true);
                });
            }
        });
    if ctx.memory_mut(|m| {
        m.data
            .remove_temp::<bool>(egui::Id::new("agent_workspace_modal_cancel"))
            .unwrap_or(false)
    }) {
        return ModalOutcome::Cancel;
    }

    egui::Area::new(egui::Id::new("agent_workspace_modal"))
        .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(colors.bg_sidebar)
                .stroke(Stroke::new(1.0, colors.border))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(MODAL_WIDTH);
                    ui.label(
                        RichText::new("New Agent Workspace")
                            .size(14.0)
                            .color(colors.text_primary),
                    );
                    ui.add_space(8.0);

                    // ── CLI dropdown ───────────────────────────────────────
                    ui.label(RichText::new("CLI").size(10.0).color(colors.text_dim));
                    ui.horizontal_wrapped(|ui| {
                        for cli in AgentCli::all() {
                            let installed = modal.cli_installed(cli);
                            let label_text = if installed {
                                cli.display_name().to_string()
                            } else {
                                format!("{} (not installed)", cli.display_name())
                            };
                            let selected = modal.selected_cli == cli;
                            let button = egui::Button::new(
                                RichText::new(label_text).size(11.0).color(
                                    if !installed {
                                        colors.text_dim
                                    } else if selected {
                                        colors.accent
                                    } else {
                                        colors.text_primary
                                    },
                                ),
                            )
                            .stroke(Stroke::new(
                                1.0,
                                if selected { colors.accent } else { colors.border },
                            ))
                            .corner_radius(4.0);
                            let r = ui.add_enabled(installed, button);
                            if r.clicked() && installed {
                                modal.selected_cli = cli;
                            }
                        }
                    });
                    ui.add_space(10.0);

                    // ── Repo picker ─────────────────────────────────────────
                    ui.label(RichText::new("Repo path").size(10.0).color(colors.text_dim));
                    let te_id = egui::Id::new("agent_workspace_modal_repo");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut modal.repo_path)
                            .id(te_id)
                            .desired_width(MODAL_WIDTH)
                            .font(egui::TextStyle::Body),
                    );
                    if !r.has_focus() && modal.task.is_empty() {
                        // Auto-focus the repo field when the modal first opens
                        // and the task is still empty (mid-flow re-renders
                        // shouldn't steal focus).
                        r.request_focus();
                    }

                    if !modal.recent_repos.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("RECENT").size(9.0).color(colors.text_dim),
                        );
                        let recent = modal.recent_repos.clone();
                        for path in recent {
                            let display = path.to_string_lossy().into_owned();
                            let r = ui.add(
                                egui::Button::new(
                                    RichText::new(display.clone())
                                        .size(10.0)
                                        .color(colors.text_dim),
                                )
                                .frame(false),
                            );
                            if r.clicked() {
                                modal.repo_path = display;
                            }
                        }
                    }
                    ui.add_space(10.0);

                    // ── Task textarea ───────────────────────────────────────
                    ui.label(
                        RichText::new("Task (optional)")
                            .size(10.0)
                            .color(colors.text_dim),
                    );
                    ui.add(
                        egui::TextEdit::multiline(&mut modal.task)
                            .id(egui::Id::new("agent_workspace_modal_task"))
                            .desired_width(MODAL_WIDTH)
                            .desired_rows(3)
                            .font(egui::TextStyle::Body),
                    );
                    ui.add_space(10.0);

                    // ── Footer buttons ──────────────────────────────────────
                    ui.horizontal(|ui| {
                        let cancel = ui.add(
                            egui::Button::new(
                                RichText::new("Cancel")
                                    .size(11.0)
                                    .color(colors.text_dim),
                            )
                            .corner_radius(4.0),
                        );
                        if cancel.clicked() {
                            outcome = ModalOutcome::Cancel;
                        }
                        let can_submit = modal.cli_installed(modal.selected_cli)
                            && !modal.repo_path.trim().is_empty();
                        let create_btn = egui::Button::new(
                            RichText::new("Create")
                                .size(11.0)
                                .color(if can_submit {
                                    colors.accent
                                } else {
                                    colors.text_dim
                                }),
                        )
                        .stroke(Stroke::new(1.0, colors.accent))
                        .corner_radius(4.0);
                        let create_r = ui.add_enabled(can_submit, create_btn);
                        if (create_r.clicked() || submit_pressed) && can_submit {
                            outcome = ModalOutcome::Submit {
                                cli: modal.selected_cli,
                                repo_path: PathBuf::from(modal.repo_path.trim()),
                                task: modal.task.clone(),
                            };
                        }
                        ui.label(
                            RichText::new("Esc cancels · ⌘↩ creates")
                                .size(9.0)
                                .color(colors.text_dim),
                        );
                    });
                });
        });

    outcome
}
