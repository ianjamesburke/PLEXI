use super::*;
use crate::app_protocol::UiNode;

/// Build a `UiNode::Text` for the pane ID label displayed in each inspector row.
///
/// Renders as `#<id>` at caption size with the dim text color.  Extracted so
/// construction is testable without a live egui context.
pub(crate) fn build_pane_id_label_node(pane_id: crate::tiling::PaneId, colors: &Colors) -> UiNode {
    let color_hex = |c: egui::Color32| {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r(), c.g(), c.b(), c.a())
    };
    UiNode::Text {
        text: format!("#{pane_id}"),
        size: style::TEXT_CAPTION,
        color: color_hex(colors.text_dim),
        bold: false,
        monospace: false,
    }
}

struct PaneRow {
    id: PaneId,
    kind: &'static str,
    name: String,
    detail: String,
    status: &'static str,
    /// Supplementary label shown next to status (e.g. agent name from OSC title when busy).
    osc_badge: Option<String>,
}

fn render_inspector_pane_row(
    ui: &mut egui::Ui,
    row: &PaneRow,
    is_selected: bool,
    colors: &Colors,
) -> egui::Response {
    let row_id = row.id;
    let (row_resp, _) = crate::widgets::selectable_row(ui, is_selected, colors, |ui| {
        ui.horizontal_centered(|ui| {
            // Pane ID label via component tree.
            let id_node = build_pane_id_label_node(row_id, colors);
            crate::render_components::render_component_tree(ui, &id_node, colors);
            crate::widgets::pane_type_badge(ui, row.kind, colors);
            let display_name: &str = if row.name.is_empty() { row.kind } else { &row.name };
            ui.label(
                RichText::new(display_name)
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                crate::widgets::status_chip(ui, row.status, colors);
                if let Some(badge) = &row.osc_badge {
                    ui.label(
                        RichText::new(format!("· {badge}"))
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                }
                if !row.detail.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(120.0);
                        crate::widgets::description_label(ui, row.detail.as_str(), colors);
                    });
                }
            });
        });
    });
    row_resp
}

/// Returns `(want_root_overlay, want_desc_overlay, start_rename, want_close, use_pane_dir)`.
/// `use_pane_dir` fires when the user clicks "From pane" to set the context root from the
/// focused pane's cwd without opening the text-input overlay.
fn render_inspector_header(
    ui: &mut egui::Ui,
    ctx_name: &str,
    ctx_root: &Option<std::path::PathBuf>,
    ctx_description: &Option<String>,
    focused_cwd: Option<&std::path::Path>,
    colors: &Colors,
    renaming: bool,
    rename_buffer: &mut String,
) -> (bool, bool, bool, bool, bool) {
    let mut open_root_overlay = false;
    let mut open_description_overlay = false;
    let mut start_rename = false;
    let mut want_close = false;
    let mut use_pane_dir = false;

    // Render the RTL close button BEFORE the greedy title/input widget so egui's
    // left-to-right layout doesn't let the greedy widget consume all available width
    // and push the close button off-screen.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("\u{2715}")
                            .size(style::TEXT_BODY)
                            .color(colors.text_dim),
                    )
                    .frame(false),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Close inspector")
                .clicked()
            {
                want_close = true;
            }
            if renaming {
                let te_id = egui::Id::new("inspector_rename_input");
                // Do NOT call request_focus() here — focus is managed via the one-shot
                // inspector_rename_focus_requested flag in draw_context_inspector, called
                // AFTER all UI renders so it wins egui's last-caller-wins focus contest.
                crate::widgets::styled_text_input(ui, rename_buffer, "Context name...", te_id, colors);
            } else {
                let name_resp = ui.add(
                    egui::Label::new(
                        RichText::new(ctx_name)
                            .size(style::TEXT_TITLE_XL)
                            .color(colors.text_primary)
                            .strong(),
                    )
                    .sense(egui::Sense::click()),
                );
                if name_resp.clicked() {
                    start_rename = true;
                }
                name_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
        });
    });

    if let Some(root) = ctx_root {
        ui.add_space(style::SPACE_SM);
        let root_str = root.display().to_string();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&root_str).size(style::TEXT_CAPTION).color(colors.text_dim));
            crate::widgets::copy_button(ui, egui::Id::new("inspector_copy_root"), &root_str);
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("\u{270e}")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                    )
                    .frame(false),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Edit root path")
                .clicked()
            {
                open_root_overlay = true;
            }
        });
    } else {
        ui.add_space(style::SPACE_SM);
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("Set root\u{2026}")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_primary),
                    )
                    .frame(false),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Set a project root directory for this context")
                .clicked()
            {
                open_root_overlay = true;
            }
            if let Some(cwd) = focused_cwd {
                let cwd_str = cwd.display().to_string();
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("From pane")
                                .size(style::TEXT_CAPTION)
                                .color(colors.accent),
                        )
                        .frame(false),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!("Set root to {cwd_str}"))
                    .clicked()
                {
                    use_pane_dir = true;
                }
            }
        });
    }
    // Description row
    ui.add_space(style::SPACE_SM);
    if let Some(desc) = ctx_description {
        ui.horizontal(|ui| {
            ui.label(RichText::new(desc.as_str()).size(style::TEXT_CAPTION).color(colors.text_dim));
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("\u{270e}")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                    )
                    .frame(false),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Edit description")
                .clicked()
            {
                open_description_overlay = true;
            }
        });
    } else if ui
        .add(
            egui::Button::new(
                RichText::new("Add description\u{2026}")
                    .size(style::TEXT_CAPTION)
                    .color(colors.text_primary),
            )
            .frame(false),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Add a description for this context")
        .clicked()
    {
        open_description_overlay = true;
    }
    ui.add_space(style::SPACE_XL);
    crate::widgets::section_header(ui, "Panes", false, colors);
    ui.add_space(style::SPACE_SM);
    (open_root_overlay, open_description_overlay, start_rename, want_close, use_pane_dir)
}

fn render_inspector_hints(
    ui: &mut egui::Ui,
    pane_count: usize,
    num_contexts: usize,
    colors: &Colors,
    renaming: bool,
) {
    ui.horizontal(|ui| {
        if renaming {
            crate::widgets::key_combo_list(ui, &[&["Enter"]], Some("save"), colors);
            ui.add_space(style::SPACE_MD);
            crate::widgets::key_combo_list(ui, &[&["Esc"]], Some("cancel"), colors);
        } else {
            if num_contexts > 1 {
                crate::widgets::key_combo_list(ui, &[&["⌫"]], Some("delete"), colors);
                ui.add_space(style::SPACE_XL);
            }
            crate::widgets::key_combo_list(ui, &[&["⌘", "R"]], Some("rename"), colors);
            ui.add_space(style::SPACE_MD);
            crate::widgets::key_combo_list(ui, &[&["Esc"]], Some("close"), colors);
            ui.add_space(style::SPACE_MD);
            crate::widgets::key_combo_list(ui, &[&["j"], &["k"]], Some("navigate"), colors);
            if pane_count > 0 {
                ui.add_space(style::SPACE_MD);
                crate::widgets::key_combo_list(ui, &[&["Enter"]], Some("focus pane"), colors);
                ui.add_space(style::SPACE_MD);
                crate::widgets::key_combo_list(ui, &[&["⌘", "W"]], Some("close pane"), colors);
            }
        }
    });
}


impl PlexiApp {
    pub(crate) fn inspector_pane_order(&self) -> Vec<PaneId> {
        self.collect_inspector_rows().1
    }

    fn collect_inspector_rows(&self) -> (Vec<(String, u64, Vec<PaneRow>)>, Vec<PaneId>, Vec<u64>) {
        let mut groups: Vec<(String, u64, Vec<PaneRow>)> = Vec::new();
        let mut all_pane_ids: Vec<PaneId> = Vec::new();
        let mut all_context_ids: Vec<u64> = Vec::new();
        for ctx_entry in self.router.iter() {
            let cname = ctx_entry.name.clone();
            let cid = ctx_entry.context_id;
            let mut rows: Vec<PaneRow> = Vec::new();
            for win in self.windows.iter() {
                if win.context_id != cid {
                    continue;
                }
                for (_, pane) in &win.panes {
                    match pane {
                        crate::pane::Pane::Terminal(t) => {
                            let pane_name = t.name.clone().or_else(|| t.pty_title.clone()).unwrap_or_default();
                            // Show pty_title in detail only when it adds context beyond the displayed name.
                            let pane_detail = t.pty_title.as_deref()
                                .filter(|pt| !pt.is_empty() && *pt != pane_name.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            let (status, osc_badge) = if t.exited {
                                ("exited", None)
                            } else {
                                let busy = crate::shell::has_foreground_child(t.backend.child_pid());
                                // Show the OSC title as the badge so the user can see what's running.
                                let badge = if busy {
                                    t.pty_title.as_deref()
                                        .filter(|t| !t.is_empty())
                                        .map(|t| t.to_string())
                                } else {
                                    None
                                };
                                let status = if busy { "busy" } else { "idle" };
                                (status, badge)
                            };
                            rows.push(PaneRow {
                                id: t.id,
                                kind: "Terminal",
                                name: pane_name,
                                detail: pane_detail,
                                status,
                                osc_badge,
                            });
                        }
                        crate::pane::Pane::App(a) => {
                            let status = if let crate::pane::AppRuntime::Process(ref proc) = a.runtime {
                                match proc.lifecycle.state() {
                                    crate::process_app::LifecycleState::Running => "running",
                                    crate::process_app::LifecycleState::Booting => "booting",
                                    crate::process_app::LifecycleState::Crashed => "crashed",
                                    crate::process_app::LifecycleState::Hung => "hung",
                                    crate::process_app::LifecycleState::ProtocolError => "error",
                                }
                            } else {
                                "active"
                            };
                            rows.push(PaneRow {
                                id: a.id,
                                kind: "App",
                                name: a.name.clone(),
                                detail: a.manifest_id.clone(),
                                status,
                                osc_badge: None,
                            });
                        }
                        crate::pane::Pane::Portal(p) => {
                            rows.push(PaneRow {
                                id: p.pane_id,
                                kind: "Portal",
                                name: format!("portal:{}", p.target_context_id),
                                detail: String::new(),
                                status: "active",
                                osc_badge: None,
                            });
                        }
                    }
                }
            }
            if !rows.is_empty() {
                rows.sort_by_key(|r| r.id);
                for r in &rows {
                    all_pane_ids.push(r.id);
                    all_context_ids.push(cid);
                }
                groups.push((cname, cid, rows));
            }
        }
        (groups, all_pane_ids, all_context_ids)
    }

    pub(crate) fn draw_context_inspector(&mut self, ctx: &egui::Context) {
        let mut dismissed = false;
        let mut close_pane: Option<PaneId> = None;
        let mut focus_pane: Option<PaneId> = None;
        let mut delete_context = false;
        let mut open_root_overlay = false;
        let mut open_description_overlay = false;
        let mut start_rename = false;
        let mut commit_rename = false;
        let mut cancel_rename = false;
        let mut set_root_from_pane = false;

        let renaming = self.inspector_renaming;
        let num_contexts = self.router.len();
        let (nav_down, nav_up, enter_pressed, backspace_pressed, cmd_w_pressed) = ctx.input_mut(|i| {
            if renaming {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    commit_rename = true;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    cancel_rename = true;
                }
                (false, false, false, false, false)
            } else {
                let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                let down = i.consume_key(egui::Modifiers::NONE, egui::Key::J)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
                let up = i.consume_key(egui::Modifiers::NONE, egui::Key::K)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
                let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                let r = i.consume_key(egui::Modifiers::COMMAND, egui::Key::R);
                let backspace = num_contexts > 1 && (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
                        || i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                );
                let cmd_w = i.consume_key(egui::Modifiers::COMMAND, egui::Key::W);
                if esc { dismissed = true; }
                if r { start_rename = true; }
                (down, up, enter, backspace, cmd_w)
            }
        });

        let focused_cwd = self.windows.get(self.active_window)
            .and_then(|w| w.focused_pane.map(|tile| (w, tile)))
            .and_then(|(w, tile)| w.get_focused_pane_cwd(tile));

        let (groups, all_pane_ids, all_context_ids) = self.collect_inspector_rows();
        let pane_count = all_pane_ids.len();
        let selected_ctx_idx = all_context_ids
            .get(self.inspector_selected_pane)
            .and_then(|&cid| self.router.position(|c| c.context_id == cid))
            .unwrap_or_else(|| self.router.active_idx());
        let ctx_name = self.router.get(selected_ctx_idx).name.clone();
        let ctx_root = self.router.get(selected_ctx_idx).root.clone();
        let ctx_description = self.router.get(selected_ctx_idx).description.clone();
        let active_cid = self.router.active().context_id;

        if nav_down && pane_count > 0 {
            self.inspector_selected_pane = (self.inspector_selected_pane + 1) % pane_count;
        }
        if nav_up && pane_count > 0 {
            self.inspector_selected_pane =
                (self.inspector_selected_pane + pane_count - 1) % pane_count;
        }
        if self.inspector_selected_pane >= pane_count && pane_count > 0 {
            self.inspector_selected_pane = pane_count - 1;
        }
        if enter_pressed && pane_count > 0 {
            focus_pane = Some(all_pane_ids[self.inspector_selected_pane]);
        }
        if cmd_w_pressed && pane_count > 0 {
            log::info!("ContextInspector: ⌘W close pane {}", all_pane_ids[self.inspector_selected_pane]);
            close_pane = Some(all_pane_ids[self.inspector_selected_pane]);
        }
        if backspace_pressed {
            let now = std::time::Instant::now();
            let elapsed = self
                .inspector_delete_last_press
                .map(|t| now.duration_since(t))
                .unwrap_or(std::time::Duration::MAX);
            if elapsed > std::time::Duration::from_millis(CONFIRM_TIMEOUT_MS) {
                self.inspector_delete_press_count = 0;
            }
            self.inspector_delete_press_count += 1;
            self.inspector_delete_last_press = Some(now);
            log::info!(
                "ContextInspector: backspace press {} of 3 for context {:?}",
                self.inspector_delete_press_count,
                self.router.active().name
            );
            if self.inspector_delete_press_count >= 3 {
                self.inspector_delete_press_count = 0;
                self.inspector_delete_last_press = None;
                delete_context = true;
            }
        }

        let colors = self.colors;
        let selected = self.inspector_selected_pane;
        let screen_rect = ctx.screen_rect();

        egui::Area::new(egui::Id::new("context_inspector_scrim"))
            .fixed_pos(screen_rect.min).order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen_rect, 0.0, Color32::from_black_alpha(style::SCRIM_ALPHA));
                if ui.allocate_rect(screen_rect, egui::Sense::click()).clicked() { dismissed = true; }
            });

        egui::Area::new(egui::Id::new("context_inspector_overlay"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, colors.border))
                    .corner_radius(style::RADIUS_MD)
                    .inner_margin(egui::Margin::symmetric(
                        style::MODAL_PADDING_H,
                        style::MODAL_PADDING_V,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(style::MODAL_WIDTH_MD);
                        let (want_root, want_desc, rename_clicked, close_clicked, pane_dir_clicked) =
                            render_inspector_header(
                                ui,
                                ctx_name.as_str(),
                                &ctx_root,
                                &ctx_description,
                                focused_cwd.as_deref(),
                                &colors,
                                renaming,
                                &mut self.rename_buffer,
                            );
                        if want_root { open_root_overlay = true; }
                        if want_desc { open_description_overlay = true; }
                        if rename_clicked { start_rename = true; }
                        if close_clicked { dismissed = true; }
                        if pane_dir_clicked { set_root_from_pane = true; }
                        // While renaming, hide the pane list — interactive widgets
                        // rendered after the TextEdit can steal egui focus (last-caller-wins).
                        // See GOTCHAS.md "egui TextEdit focus in complex frames".
                        if !renaming {
                            egui::ScrollArea::vertical()
                                .max_height(ctx.available_rect().height() * 0.6)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    if pane_count == 0 {
                                        ui.label(RichText::new("No panes").size(style::TEXT_BODY).color(colors.text_dim));
                                    } else {
                                        let mut global_idx: usize = 0;
                                        for (group_name, group_cid, rows) in &groups {
                                            let is_active = *group_cid == active_cid;
                                            ui.add_space(style::SPACE_SM);
                                            let group_label = RichText::new(group_name.as_str()).size(style::TEXT_CAPTION);
                                            ui.label(if is_active {
                                                group_label.color(colors.text_primary).strong()
                                            } else {
                                                group_label.color(colors.text_dim)
                                            });
                                            ui.add_space(style::SPACE_SM);
                                            for row in rows {
                                                let resp = render_inspector_pane_row(
                                                    ui, row, global_idx == selected, &colors,
                                                );
                                                global_idx += 1;
                                                if resp.clicked() { focus_pane = Some(row.id); }
                                            }
                                        }
                                    }
                                });
                            ui.add_space(style::SPACE_XL);
                            ui.separator();
                            ui.add_space(style::SPACE_MD);
                        }
                        render_inspector_hints(ui, pane_count, num_contexts, &colors, renaming);
                    });
            });

        // One-shot focus: request AFTER all UI renders so we win egui's
        // last-caller-wins focus contest. See GOTCHAS.md for the pattern.
        if renaming && !self.inspector_rename_focus_requested {
            let te_id = egui::Id::new("inspector_rename_input");
            ctx.memory_mut(|m| m.request_focus(te_id));
            if let Some(mut state) = egui::TextEdit::load_state(ctx, te_id) {
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(self.rename_buffer.chars().count()),
                )));
                state.store(ctx, te_id);
            }
            self.inspector_rename_focus_requested = true;
            log::info!("ContextInspector: rename TextEdit focus requested (one-shot)");
        }

        if start_rename && !renaming {
            self.rename_buffer = ctx_name.to_string();
            self.inspector_renaming = true;
            self.inspector_rename_focus_requested = false;
            log::info!("ContextInspector: rename mode entered for context {:?}", ctx_name);
        }
        if commit_rename {
            let new_name = self.rename_buffer.trim().to_string();
            if !new_name.is_empty() {
                log::info!(
                    "ContextInspector: renamed context {:?} → {:?}",
                    self.router.get(selected_ctx_idx).name,
                    new_name
                );
                self.router.get_mut(selected_ctx_idx).name = new_name;
                self.save_workspace();
            }
            self.inspector_renaming = false;
            self.inspector_rename_focus_requested = false;
        }
        if cancel_rename {
            self.inspector_renaming = false;
            self.inspector_rename_focus_requested = false;
            log::info!("ContextInspector: rename cancelled");
        }
        if dismissed {
            self.show_context_inspector = false;
            self.inspector_renaming = false;
            self.inspector_rename_focus_requested = false;
            self.inspector_delete_press_count = 0;
            self.inspector_delete_last_press = None;
            log::info!("ContextInspector: closed");
        }
        if let Some(pid) = focus_pane {
            log::info!("ContextInspector: focusing pane {pid}");
            self.pane_navigate(pid);
            self.show_context_inspector = false;
            self.inspector_renaming = false;
            self.inspector_rename_focus_requested = false;
            self.inspector_delete_press_count = 0;
            self.inspector_delete_last_press = None;
        }
        if let Some(pid) = close_pane {
            log::info!("ContextInspector: closing pane {pid}");
            self.close_pane_by_id(pid);
        }
        if delete_context {
            let ctx_idx = if pane_count > 0 {
                let target_cid = all_context_ids[self.inspector_selected_pane];
                self.router
                    .position(|c| c.context_id == target_cid)
                    .unwrap_or_else(|| self.router.active_idx())
            } else {
                self.router.active_idx()
            };
            log::info!(
                "ContextInspector: deleting context idx={ctx_idx} name={:?} (via backspace or button)",
                self.router.get(ctx_idx).name
            );
            self.inspector_delete_press_count = 0;
            self.inspector_delete_last_press = None;
            self.delete_context(ctx_idx);
            self.save_workspace();
        }
        if open_root_overlay {
            let idx = self.router.active_idx();
            let existing = self.router.get(idx).root.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            log::info!("TextInputOverlay: opened target=ContextRoot({idx})");
            self.text_overlay_browse_rx = None;
            self.text_overlay = Some((
                crate::app::TextInputOverlay {
                    label: "Set context root".to_string(),
                    hint: "/path/to/project or ~/...".to_string(),
                    buffer: existing,
                    focus_requested: false,
                },
                crate::app::OverlayTarget::ContextRoot(idx),
            ));
            // Close the inspector so the text overlay takes focus.
            self.show_context_inspector = false;
        }
        if open_description_overlay {
            let idx = selected_ctx_idx;
            let existing = self.router.get(idx).description.clone().unwrap_or_default();
            log::info!("ContextInspector: opening description overlay for ctx_idx={idx}");
            self.editing_description = Some(idx);
            self.description_buffer = existing;
            self.description_focus_requested = false;
            self.push_focus_layer(crate::app::FocusLayer::ContextDescription);
            self.show_context_inspector = false;
        }
        if set_root_from_pane {
            if let Some(cwd) = focused_cwd {
                log::info!(
                    "ContextInspector: set root from pane cwd={} ctx_idx={selected_ctx_idx}",
                    cwd.display()
                );
                self.router.get_mut(selected_ctx_idx).root = Some(cwd);
                self.save_workspace();
            }
        }

        if self.inspector_delete_press_count > 0 {
            let timed_out = self
                .inspector_delete_last_press
                .map(|t| t.elapsed() > std::time::Duration::from_millis(CONFIRM_TIMEOUT_MS))
                .unwrap_or(false);
            if timed_out {
                self.inspector_delete_press_count = 0;
                self.inspector_delete_last_press = None;
            } else {
                self.draw_inspector_delete_overlay(ctx);
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }

    pub(crate) fn draw_triple_tap_overlay(&self, ctx: &egui::Context, id: &str, count: u8, label: &str) {
        egui::Area::new(egui::Id::new(id))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{label} {} of 3 — press again to delete context",
                                    count
                                ))
                                .size(12.0)
                                .color(self.colors.text_dim),
                            );
                            ui.add_space(8.0);
                            for i in 1u8..=3 {
                                let color = if i <= count {
                                    self.colors.accent
                                } else {
                                    self.colors.bg_active
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(rect.center(), 4.0, color);
                            }
                        });
                    });
            });
    }


    pub(crate) fn context_inspector_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod inspector_component_tree_tests {
    use super::*;
    use crate::app_protocol::UiNode;
    use crate::config::ThemeConfig;

    fn test_colors() -> Colors {
        Colors::from_config(&ThemeConfig::default())
    }

    /// Pane ID label node must be `UiNode::Text` with `#<id>` format at caption size.
    #[test]
    fn pane_id_label_node_format() {
        let colors = test_colors();
        let node = build_pane_id_label_node(42, &colors);
        if let UiNode::Text { text, size, bold, monospace, color } = node {
            assert_eq!(text, "#42");
            assert_eq!(size, style::TEXT_CAPTION);
            assert!(!bold);
            assert!(!monospace);
            assert!(!color.is_empty(), "color must be set to text_dim hex");
        } else {
            panic!("expected UiNode::Text");
        }
    }

    /// Pane ID 0 should still render as `#0` without panicking.
    #[test]
    fn pane_id_label_node_zero() {
        let colors = test_colors();
        let node = build_pane_id_label_node(0, &colors);
        if let UiNode::Text { text, .. } = node {
            assert_eq!(text, "#0");
        } else {
            panic!("expected UiNode::Text");
        }
    }
}
