//! egui rendering for the host Assistant pane: header bar, transcript,
//! streaming row, slash-command picker, and multiline composer. Pure view
//! over `AssistantModel` — all state transitions go back through the model.
//!
//! Layout: the composer (plus permission sheet and hint bar) lives in a
//! content-sized bottom panel so it stays glued to the pane's bottom edge.
//! The slash-command picker is a floating `Area` popup anchored to the top
//! edge of the composer — it grows and shrinks upward without resizing the
//! panel or the transcript, so filtering never shifts surrounding layout.
//! Enter/Tab/arrow keys are consumed *before* the composer TextEdit renders,
//! so completion and submit never flash an intermediate buffer state.

use egui::RichText;

use crate::ui::button::{chrome_button, ButtonKind};
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::list::ListRow;
use crate::ui::style;
use crate::ui::theme::Colors;

use super::commands;
use super::model::{AssistantModel, PermissionChoice, ToolStatus, TurnRole};

/// What the composer asked the pane shell to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerEvent {
    Submit,
    /// The user decided the pending permission sheet.
    Permission(PermissionChoice),
}

/// Stateless renderer for the Assistant pane.
pub struct AssistantRenderer;

impl AssistantRenderer {
    /// Header bar height — mirrors the terminal pane name bar and the
    /// text-editor note header so the assistant reads as the same chrome.
    const HEADER_BAR_H: f32 = 20.0;
    const HEADER_FONT_SIZE: f32 = 11.0;
    /// Composer stops growing past this many text rows and scrolls instead.
    const COMPOSER_MAX_ROWS: f32 = 8.0;
    /// Chat bubbles cap at this fraction of the transcript width.
    const BUBBLE_MAX_FRACTION: f32 = 0.72;

    pub fn draw(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        colors: &Colors,
        is_focused: bool,
    ) -> Option<ComposerEvent> {
        // Match the terminal/editor surface, not the darker app-pane base
        // fill — the assistant is host chrome, same as the scratchpad.
        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0.0, colors.terminal_bg);
        ui.visuals_mut().extreme_bg_color = colors.terminal_bg;

        Self::draw_header(ui, model, colors);

        // Egui ids are salted by the pane's own ui id — NOT the conversation
        // id. A conversation-salted id would reset the bottom panel's stored
        // height on /new and /clear, making the composer visibly re-converge
        // (flash) the moment the conversation switches.
        let pane_id = ui.id();
        let te_id = pane_id.with("assistant_composer");

        let total_h = ui.available_rect_before_wrap().height();
        let picker_max_h = (total_h * 0.4).max(0.0);
        let mut event = None;
        let mut composer_rect = None;

        // Bottom-anchored, content-sized: composer pinned to the pane bottom.
        // Stable height (composer + hints only) — the picker floats above it.
        egui::TopBottomPanel::bottom(pane_id.with("assistant_bottom"))
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: style::SPACE_MD as i8,
                right: style::SPACE_MD as i8,
                top: style::SPACE_XS as i8,
                bottom: style::SPACE_SM as i8,
            }))
            .show_inside(ui, |ui| {
                if let Some(choice) = Self::draw_permission_sheet(ui, model, colors) {
                    event = Some(ComposerEvent::Permission(choice));
                }
                // Keys first: Enter/Tab/arrows must be consumed before the
                // TextEdit processes input, or completion renders a one-frame
                // stale buffer (the autocomplete "glitch").
                if let Some(key_event) = Self::handle_composer_keys(ui, model, te_id) {
                    event = Some(key_event);
                }
                composer_rect = Some(Self::draw_composer(ui, model, te_id, colors, is_focused));
                let hints = [
                    HintGroup::new(&["\u{21b5}"], "send"),
                    HintGroup::new(&["\u{21e7}", "\u{21b5}"], "newline"),
                    HintGroup::new(&["/"], "commands"),
                ];
                HintBar::new(&hints).show(ui, colors);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(
                style::SPACE_MD as i8,
                0,
            )))
            .show_inside(ui, |ui| {
                Self::draw_transcript(ui, model, md_cache, colors);
            });

        if model.picker_active() {
            if let Some(rect) = composer_rect {
                Self::draw_picker_popup(ui, model, te_id, pane_id, colors, rect, picker_max_h);
            }
        }

        event
    }

    /// Title bar styled like the terminal pane name bar: session name (or
    /// "Assistant"), with a status pip — pulsing accent while a turn streams,
    /// danger when the last turn errored, dim when idle.
    fn draw_header(ui: &mut egui::Ui, model: &AssistantModel, colors: &Colors) {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), Self::HEADER_BAR_H),
        );
        ui.advance_cursor_after_rect(bar_rect);
        ui.painter()
            .rect_filled(bar_rect, 0.0, colors.pane_header_bg());

        let pip_color = if model.streaming.in_flight {
            let t = ui.input(|i| i.time);
            let phase = ((t * 2.5).sin() * 0.5 + 0.5) as f32;
            colors.accent.gamma_multiply(0.45 + 0.55 * phase)
        } else if matches!(
            model.turns.last().map(|t| t.role),
            Some(TurnRole::Error)
        ) {
            colors.danger
        } else {
            colors.text_dim
        };
        ui.painter().circle_filled(
            egui::pos2(bar_rect.left() + style::SPACE_SM + 3.0, bar_rect.center().y),
            3.0,
            pip_color,
        );

        let title = model
            .session_name
            .clone()
            .unwrap_or_else(|| "Assistant".to_string());
        // A real label (not painter text) so the title stays in the
        // accessibility tree for UI-harness queries.
        let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect));
        bar_ui.centered_and_justified(|ui| {
            ui.label(
                RichText::new(title)
                    .size(Self::HEADER_FONT_SIZE)
                    .color(colors.text_dim),
            );
        });
    }

    fn draw_transcript(
        ui: &mut egui::Ui,
        model: &AssistantModel,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        colors: &Colors,
    ) {
        egui::ScrollArea::vertical()
            .id_salt("assistant_transcript")
            .auto_shrink([false, false])
            // Follow new content (streamed chunks, command output) whenever
            // the view is already at the bottom; egui releases the stick as
            // soon as the user scrolls up to read history.
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add_space(style::SPACE_SM);
                for (i, turn) in model.turns.iter().enumerate() {
                    ui.push_id(i, |ui| {
                        Self::draw_turn_row(ui, md_cache, colors, turn, model.show_thoughts);
                    });
                }
                for active in &model.active_tools {
                    Self::draw_active_tool_row(ui, colors, &active.tool);
                }
                if model.streaming.in_flight {
                    ui.push_id("streaming", |ui| {
                        Self::draw_streaming_row(ui, model, md_cache, colors);
                    });
                }
                ui.add_space(style::SPACE_SM);
            });
    }

    /// Convert markdown soft breaks to hard breaks: every newline outside a
    /// fenced code block gets a trailing two-space hard break, so single
    /// newlines stay visible line breaks (the chat-client convention) instead
    /// of collapsing into one paragraph. Block structure — lists, headings,
    /// fences — is decided per line before inline rendering, so the trailing
    /// spaces never change it.
    fn soften_newlines(text: &str) -> String {
        let mut out = String::with_capacity(text.len() + text.lines().count() * 2);
        let mut in_fence = false;
        for line in text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
            }
            out.push_str(line);
            if !in_fence && !line.trim().is_empty() {
                out.push_str("  ");
            }
            out.push('\n');
        }
        out
    }

    /// Render `text` as markdown (links, emphasis, inline code, fenced
    /// blocks) sized to the chat body scale. Hyperlinks open through egui's
    /// native `open_url`. Raw inline HTML is not interpreted — it renders as
    /// literal text, which is the honest behavior for an immediate-mode UI.
    fn markdown_body(
        ui: &mut egui::Ui,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        colors: &Colors,
        text: &str,
    ) {
        let s = ui.style_mut();
        s.visuals.override_text_color = Some(colors.text_primary);
        s.visuals.hyperlink_color = colors.accent;
        s.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(style::TEXT_BODY),
        );
        s.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::proportional(style::TEXT_BODY * 1.3),
        );
        s.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::monospace(style::TEXT_BODY * 0.9),
        );
        s.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional(style::TEXT_HINT),
        );
        egui_commonmark::CommonMarkViewer::new().show(ui, md_cache, &Self::soften_newlines(text));
    }

    fn draw_turn_row(
        ui: &mut egui::Ui,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        colors: &Colors,
        turn: &super::model::Turn,
        show_thoughts: bool,
    ) {
        let text = turn.text.as_str();
        let status = turn.status;
        match turn.role {
            // Delivered app events are compact single-line rows.
            TurnRole::Event => {
                ui.label(
                    RichText::new(text)
                        .size(style::TEXT_CAPTION)
                        .monospace()
                        .color(colors.accent),
                );
                ui.add_space(style::SPACE_SM);
            }
            // Completed tool calls are compact single-line rows.
            TurnRole::Tool => {
                let (icon, color) = match status {
                    Some(ToolStatus::Succeeded) | None => ("✓", colors.text_dim),
                    Some(ToolStatus::Failed) => ("✗", colors.danger),
                };
                ui.label(
                    RichText::new(format!("{icon} {text}"))
                        .size(style::TEXT_CAPTION)
                        .monospace()
                        .color(color),
                );
                ui.add_space(style::SPACE_SM);
            }
            // User turns sit right-aligned in an outlined bubble, like every
            // mainstream chat client.
            TurnRole::User => {
                let bubble_max = ui.available_width() * Self::BUBBLE_MAX_FRACTION;
                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                    egui::Frame::new()
                        .fill(colors.bg_active)
                        .stroke(egui::Stroke::new(1.0, colors.border))
                        .corner_radius(style::RADIUS_MD)
                        .inner_margin(egui::Margin::symmetric(
                            style::SPACE_SM as i8,
                            style::SPACE_XS as i8,
                        ))
                        .show(ui, |ui| {
                            ui.set_max_width(bubble_max);
                            ui.label(
                                RichText::new(text)
                                    .size(style::TEXT_BODY)
                                    .color(colors.text_primary),
                            );
                        });
                });
                ui.add_space(style::SPACE_MD);
            }
            // Assistant replies sit left-aligned in a soft unstroked bubble —
            // visible against the terminal surface — and render as markdown.
            TurnRole::Assistant => {
                if show_thoughts {
                    if let Some(thoughts) = &turn.thoughts {
                        Self::draw_thoughts_section(ui, colors, thoughts);
                    }
                }
                let bubble_max = ui.available_width() * 0.85;
                egui::Frame::new()
                    .fill(colors.bg_active)
                    .corner_radius(style::RADIUS_MD)
                    .inner_margin(egui::Margin::symmetric(
                        style::SPACE_SM as i8,
                        style::SPACE_XS as i8,
                    ))
                    .show(ui, |ui| {
                        ui.set_max_width(bubble_max);
                        Self::markdown_body(ui, md_cache, colors, text);
                    });
                ui.add_space(style::SPACE_MD);
            }
            TurnRole::Error => {
                ui.label(
                    RichText::new("error")
                        .size(style::TEXT_HINT)
                        .color(colors.danger),
                );
                ui.label(
                    RichText::new(text)
                        .size(style::TEXT_BODY)
                        .color(colors.danger),
                );
                ui.add_space(style::SPACE_MD);
            }
        }
    }

    /// A tool call currently running inside the in-flight turn.
    fn draw_active_tool_row(ui: &mut egui::Ui, colors: &Colors, tool: &str) {
        ui.label(
            RichText::new(format!("⟳ {tool} — running…"))
                .size(style::TEXT_CAPTION)
                .monospace()
                .color(colors.accent),
        );
        ui.add_space(style::SPACE_SM);
    }

    /// Permission sheet for the pending ask-gated tool call, rendered above
    /// the composer. Returns the user's decision, if any, this frame.
    fn draw_permission_sheet(
        ui: &mut egui::Ui,
        model: &AssistantModel,
        colors: &Colors,
    ) -> Option<PermissionChoice> {
        let pending = model.pending_permission.as_ref()?;
        let mut choice = None;
        egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(egui::Stroke::new(1.0, colors.accent))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::same(style::SPACE_SM as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Permission required")
                        .size(style::TEXT_CAPTION)
                        .color(colors.accent),
                );
                ui.label(
                    RichText::new(format!(
                        "assistant (medium) wants to run the app tool '{}'",
                        pending.tool
                    ))
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary),
                );
                if !pending.input_summary.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(ui.available_width());
                        crate::ui::labels::description_label(ui, &pending.input_summary, colors);
                    });
                }
                ui.add_space(style::SPACE_XS);
                ui.horizontal(|ui| {
                    if chrome_button(ui, "Allow once", ButtonKind::Accent, colors, 0.0).clicked() {
                        choice = Some(PermissionChoice::AllowOnce);
                    }
                    if chrome_button(ui, "Allow this session", ButtonKind::Primary, colors, 0.0)
                        .clicked()
                    {
                        choice = Some(PermissionChoice::AllowSession);
                    }
                    if chrome_button(ui, "Always allow", ButtonKind::Primary, colors, 0.0).clicked()
                    {
                        choice = Some(PermissionChoice::AllowAlways);
                    }
                    if chrome_button(ui, "Deny", ButtonKind::Danger, colors, 0.0).clicked() {
                        choice = Some(PermissionChoice::Deny);
                    }
                });
            });
        ui.add_space(style::SPACE_XS);
        choice
    }

    /// Collapsed "thoughts" section: the model's reasoning tokens, visible
    /// only while the `/thoughts` toggle is on. Used both for the in-flight
    /// turn and for persisted assistant turns.
    fn draw_thoughts_section(ui: &mut egui::Ui, colors: &Colors, thoughts: &str) {
        egui::CollapsingHeader::new(
            RichText::new("thoughts")
                .size(style::TEXT_CAPTION)
                .color(colors.text_dim),
        )
        .id_salt("assistant_thoughts")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                RichText::new(thoughts)
                    .size(style::TEXT_CAPTION)
                    .color(colors.text_dim),
            );
        });
    }

    fn draw_streaming_row(
        ui: &mut egui::Ui,
        model: &AssistantModel,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        colors: &Colors,
    ) {
        if model.show_thoughts && !model.streaming.partial_reasoning.is_empty() {
            Self::draw_thoughts_section(ui, colors, &model.streaming.partial_reasoning);
        }
        if model.streaming.partial_answer.is_empty() {
            Self::draw_thinking_dots(ui, colors);
        } else {
            Self::markdown_body(ui, md_cache, colors, &model.streaming.partial_answer);
        }
        ui.add_space(style::SPACE_MD);
    }

    /// Three dots pulsing in sequence — the "assistant is thinking" beat
    /// shown before the first answer token arrives. The pane already
    /// requests repaints every 50ms while a turn is in flight.
    fn draw_thinking_dots(ui: &mut egui::Ui, colors: &Colors) {
        let t = ui.input(|i| i.time);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 18.0), egui::Sense::hover());
        for k in 0..3 {
            let phase = (((t * 2.2 - k as f64 * 0.45).sin() * 0.5 + 0.5)) as f32;
            let color = colors.text_dim.gamma_multiply(0.35 + 0.65 * phase);
            ui.painter().circle_filled(
                egui::pos2(rect.left() + 6.0 + k as f32 * 11.0, rect.center().y),
                2.5,
                color,
            );
        }
    }

    /// Move the composer caret to the end of `text` — used after picker
    /// completion replaces the buffer, so typing continues after the
    /// inserted trailing space instead of mid-command.
    fn set_caret_to_end(ctx: &egui::Context, te_id: egui::Id, text: &str) {
        let mut state = egui::TextEdit::load_state(ctx, te_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(text.chars().count()),
            )));
        state.store(ctx, te_id);
    }

    /// Replace the composer with the completed slash command plus a trailing
    /// space, caret at the end ready for arguments.
    fn complete_command(
        ctx: &egui::Context,
        model: &mut AssistantModel,
        te_id: egui::Id,
        name: &str,
        via: &str,
    ) {
        model.composer = format!("/{name} ");
        Self::set_caret_to_end(ctx, te_id, &model.composer);
        model.picker_selected = 0;
        log::info!("assistant: picker completed '/{name}' via {via}");
    }

    /// Consume composer keyboard input ahead of the TextEdit: picker
    /// navigation (arrows), completion (Tab/Enter) while picking, and plain
    /// Enter submit otherwise. Shift+Enter is left for the TextEdit to
    /// insert a newline natively.
    fn handle_composer_keys(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        te_id: egui::Id,
    ) -> Option<ComposerEvent> {
        if !ui.memory(|m| m.has_focus(te_id)) {
            return None;
        }
        if model.picker_active() {
            let matches = commands::filter_commands(&model.picker_query());
            if !matches.is_empty() {
                if model.picker_selected >= matches.len() {
                    model.picker_selected = matches.len() - 1;
                }
                let mut complete = false;
                ui.input_mut(|input| {
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                        && model.picker_selected + 1 < matches.len()
                    {
                        model.picker_selected += 1;
                    }
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                        && model.picker_selected > 0
                    {
                        model.picker_selected -= 1;
                    }
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                    {
                        complete = true;
                    }
                });
                if complete {
                    let (name, _) = matches[model.picker_selected];
                    Self::complete_command(ui.ctx(), model, te_id, name, "key");
                }
                return None;
            }
            // No matches: fall through so Enter submits the raw text.
        }
        let mut submit = false;
        ui.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                submit = true;
            }
        });
        submit.then_some(ComposerEvent::Submit)
    }

    /// Slash-command picker as a floating popup anchored to the composer's
    /// top edge, growing upward over the transcript. Floating keeps the
    /// bottom panel's height fixed, so opening/filtering the picker never
    /// resizes the transcript or moves the composer.
    fn draw_picker_popup(
        ui: &egui::Ui,
        model: &mut AssistantModel,
        te_id: egui::Id,
        pane_id: egui::Id,
        colors: &Colors,
        composer_rect: egui::Rect,
        max_h: f32,
    ) {
        let matches = commands::filter_commands(&model.picker_query());
        if matches.is_empty() {
            return;
        }
        if model.picker_selected >= matches.len() {
            model.picker_selected = matches.len() - 1;
        }
        let selected_idx = model.picker_selected;
        let mut clicked: Option<&'static str> = None;
        let mut hover_select: Option<usize> = None;

        // Command-palette scroll/hover discipline: scroll-to-selected only
        // when the selection actually changed (a per-frame scroll_to_me
        // fights the user's wheel and snaps the list back), and hover moves
        // the selection only when the mouse itself moved (so rows sliding
        // under a stationary cursor during scroll don't steal selection).
        let prev_selected_id = pane_id.with("assistant_picker_prev_selected");
        let prev_selected = ui
            .ctx()
            .data(|d| d.get_temp::<usize>(prev_selected_id))
            .unwrap_or(selected_idx);
        let should_scroll = selected_idx != prev_selected;
        let mouse_moved = ui.ctx().input(|i| i.pointer.delta().length_sq() > 0.5);

        egui::Area::new(pane_id.with("assistant_picker"))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(composer_rect.left_top() - egui::vec2(0.0, style::SPACE_XS))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.set_width(composer_rect.width());
                egui::Frame::new()
                    .fill(colors.bg_active)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(style::RADIUS_MD)
                    .inner_margin(egui::Margin::same(style::SPACE_XS as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt("assistant_picker_scroll")
                            .max_height(max_h.max(0.0))
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (i, (name, purpose)) in matches.iter().enumerate() {
                                    let row = ListRow::new(&format!("/{name}"))
                                        .secondary(purpose)
                                        .selected(i == selected_idx)
                                        .show(ui, colors);
                                    if i == selected_idx && should_scroll {
                                        row.scroll_to_me(None);
                                    }
                                    if row.row_clicked() {
                                        clicked = Some(*name);
                                    }
                                    if row.row_hovered() {
                                        hover_select = Some(i);
                                    }
                                }
                            });
                    });
            });

        if let Some(i) = hover_select {
            if mouse_moved {
                model.picker_selected = i;
            }
        }
        ui.ctx()
            .data_mut(|d| d.insert_temp(prev_selected_id, model.picker_selected));
        if let Some(name) = clicked {
            Self::complete_command(ui.ctx(), model, te_id, name, "click");
        }
    }

    /// The growable composer; returns its outer rect so the picker popup can
    /// anchor to it. Key handling happens in `handle_composer_keys` before
    /// this runs.
    fn draw_composer(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        te_id: egui::Id,
        colors: &Colors,
        is_focused: bool,
    ) -> egui::Rect {
        let font_id = egui::FontId::proportional(style::TEXT_BODY);
        let row_height = ui.fonts(|f| f.row_height(&font_id));
        let max_text_h = row_height * Self::COMPOSER_MAX_ROWS;

        // Accent outline while the composer holds keyboard focus — same
        // affordance as the host text fields.
        let has_kb_focus = ui.memory(|m| m.has_focus(te_id));
        let stroke_color = if has_kb_focus { colors.accent } else { colors.border };

        let mut response = None;
        let frame_response = egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(egui::Stroke::new(1.0, stroke_color))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(
                style::SPACE_SM as i8,
                style::SPACE_XS as i8,
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Caret styling from the host TextField primitive (the
                // singleline widget itself can't host a growable multiline
                // composer, but its visual conventions carry over).
                ui.visuals_mut().text_cursor.stroke.width = 1.5;
                ui.visuals_mut().text_cursor.stroke.color = colors.accent;
                // Grows with content up to COMPOSER_MAX_ROWS, then scrolls —
                // same shape as the quick-note entry.
                egui::ScrollArea::vertical()
                    .id_salt("assistant_composer_scroll")
                    .max_height(max_text_h)
                    .show(ui, |ui| {
                        response = Some(ui.add(
                            egui::TextEdit::multiline(&mut model.composer)
                                .id(te_id)
                                .desired_rows(1)
                                .desired_width(f32::INFINITY)
                                .frame(false)
                                .hint_text(
                                    RichText::new("Message the assistant — / for commands")
                                        .size(style::TEXT_CAPTION)
                                        .color(colors.text_dim),
                                )
                                .font(font_id.clone()),
                        ));
                    });
            });
        if let Some(response) = response {
            if is_focused && !response.has_focus() && model.composer.is_empty() {
                response.request_focus();
            }
        }
        frame_response.response.rect
    }
}

#[cfg(test)]
mod tests {
    use super::AssistantRenderer;

    #[test]
    fn soften_newlines_makes_single_breaks_hard_outside_fences() {
        let out = AssistantRenderer::soften_newlines("line one\nline two\n\nline three");
        assert_eq!(out, "line one  \nline two  \n\nline three  \n");
    }

    #[test]
    fn soften_newlines_leaves_fenced_code_untouched() {
        let out = AssistantRenderer::soften_newlines("before\n```\nlet x = 1;\nlet y = 2;\n```\nafter");
        assert!(out.contains("before  \n"));
        assert!(out.contains("let x = 1;\nlet y = 2;\n"), "code lines must stay byte-identical");
        assert!(out.contains("after  \n"));
    }
}
