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

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::RichText;

use crate::ui::button::{chrome_button, ButtonKind};
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::list::ListRow;
use crate::ui::style;
use crate::ui::theme::Colors;

use crate::app_protocol::ModelTier;
use crate::broker::Decision;

use super::commands;
use super::model::{
    AssistantModel, AssistantOverlay, CompactionState, PermissionChoice, ToolStatus, TurnRole,
};

/// What the composer asked the pane shell to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerEvent {
    Submit,
    /// The user decided the pending permission sheet.
    Permission(PermissionChoice),
    /// Enter pressed in an open picker/manager overlay: apply the selection.
    OverlayConfirm,
}

/// Row label for a model tier.
fn model_tier_label(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Low => "low",
        ModelTier::Medium => "medium",
        ModelTier::High => "high",
    }
}

/// Row label for a permission decision — "block" reads clearer than "deny" in
/// the manager (matches the Space-cycle affordance).
fn decision_label(decision: Decision) -> &'static str {
    match decision {
        Decision::Allow => "allow",
        Decision::Ask => "ask",
        Decision::Deny => "block",
    }
}

/// Stateless renderer for the Assistant pane.
pub struct AssistantRenderer;

#[derive(Default)]
pub(crate) struct MarkdownTextCache {
    softened_by_turn: HashMap<u64, CachedMarkdownText>,
}

struct CachedMarkdownText {
    source_len: usize,
    softened: String,
}

impl MarkdownTextCache {
    fn softened_turn_text<'a>(
        &'a mut self,
        conversation_id: &str,
        turn_index: usize,
        turn: &super::model::Turn,
    ) -> &'a str {
        let key = Self::turn_key(conversation_id, turn_index, turn);
        let entry = self
            .softened_by_turn
            .entry(key)
            .or_insert_with(|| CachedMarkdownText {
                source_len: 0,
                softened: String::new(),
            });
        if entry.source_len != turn.text.len() {
            entry.source_len = turn.text.len();
            entry.softened = crate::ui::markdown::harden_soft_breaks(&turn.text);
        }
        &entry.softened
    }

    fn turn_key(conversation_id: &str, turn_index: usize, turn: &super::model::Turn) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        conversation_id.hash(&mut hasher);
        turn_index.hash(&mut hasher);
        turn.created_at.hash(&mut hasher);
        match turn.role {
            TurnRole::User => 0_u8,
            TurnRole::Assistant => 1,
            TurnRole::Tool => 2,
            TurnRole::Error => 3,
            TurnRole::Event => 4,
        }
        .hash(&mut hasher);
        hasher.finish()
    }
}

impl AssistantRenderer {
    /// Composer grows until it reaches this fraction of the pane height, then
    /// scrolls — generous on purpose so long drafts get room to breathe.
    const COMPOSER_MAX_FRACTION: f32 = 0.75;
    /// Chat bubbles cap at this fraction of the transcript width.
    const BUBBLE_MAX_FRACTION: f32 = 0.72;

    pub fn draw(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        text_cache: &mut MarkdownTextCache,
        colors: &Colors,
        is_focused: bool,
    ) -> Option<ComposerEvent> {
        // Match the terminal/editor surface, not the darker app-pane base
        // fill — the assistant is host chrome, same as the scratchpad.
        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0.0, colors.terminal_bg);
        ui.visuals_mut().extreme_bg_color = colors.terminal_bg;

        // Egui ids are salted by the pane's own ui id — NOT the conversation
        // id. A conversation-salted id would reset the bottom panel's stored
        // height on /new and /clear, making the composer visibly re-converge
        // (flash) the moment the conversation switches.
        let pane_id = ui.id();
        let te_id = pane_id.with("assistant_composer");

        // Picker shows up to 10 command rows, clamped so it never overruns
        // the transcript area in a short pane (but always fits at least 3).
        let total_h = ui.available_rect_before_wrap().height();
        let picker_max_h =
            (style::LIST_ROW_H * 10.0).min((total_h * 0.8).max(style::LIST_ROW_H * 3.0));
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
                // Keys first: Enter/Tab/arrows must be consumed before the
                // TextEdit processes input, or completion renders a one-frame
                // stale buffer (the autocomplete "glitch"). While the
                // permission sheet or an overlay is open, it owns navigation
                // keys and the composer stays inert. The sheet takes
                // priority over the overlay — a permission ask can interrupt
                // an open overlay's underlying turn.
                let permission_pending = model.pending_permission.is_some();
                if permission_pending {
                    if let Some(perm_event) = Self::handle_permission_keys(ui, model) {
                        event = Some(perm_event);
                    }
                } else if model.overlay_active() {
                    if let Some(overlay_event) = Self::handle_overlay_keys(ui, model) {
                        event = Some(overlay_event);
                    }
                } else if let Some(key_event) = Self::handle_composer_keys(ui, model, te_id) {
                    event = Some(key_event);
                }
                if let Some(choice) = Self::draw_permission_sheet(ui, model, colors) {
                    event = Some(ComposerEvent::Permission(choice));
                }
                composer_rect = Some(Self::draw_composer(
                    ui,
                    model,
                    te_id,
                    colors,
                    is_focused,
                    total_h * Self::COMPOSER_MAX_FRACTION,
                ));
                if permission_pending {
                    HintBar::new(&[
                        HintGroup::new(&["\u{21e5}"], "navigate"),
                        HintGroup::new(&["\u{21b5}"], "confirm"),
                        HintGroup::new(&["Esc"], "deny"),
                    ])
                    .show(ui, colors);
                } else if model.overlay_active() {
                    let mut hints = vec![
                        HintGroup::new(&["\u{2191}", "\u{2193}"], "navigate"),
                        HintGroup::new(&["\u{21b5}"], "confirm"),
                        HintGroup::new(&["Esc"], "cancel"),
                    ];
                    if matches!(model.overlay, AssistantOverlay::PermissionsManager { .. }) {
                        hints.push(HintGroup::new(&["Space"], "cycle"));
                    }
                    HintBar::new(&hints).show(ui, colors);
                } else {
                    let mut hints = vec![
                        HintGroup::new(&["\u{21b5}"], "send"),
                        HintGroup::new(&["\u{21e7}", "\u{21b5}"], "newline"),
                        HintGroup::new(&["/"], "commands"),
                    ];
                    if model.streaming.in_flight {
                        hints.push(HintGroup::new(&["Esc"], "stop"));
                    }
                    HintBar::new(&hints).show(ui, colors);
                }
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new().inner_margin(egui::Margin::symmetric(style::SPACE_MD as i8, 0)),
            )
            .show_inside(ui, |ui| {
                Self::draw_transcript(ui, model, md_cache, text_cache, colors);
            });

        if let Some(rect) = composer_rect {
            if model.overlay_active() {
                Self::draw_overlay_popup(ui, model, pane_id, colors, rect, picker_max_h);
            } else if model.picker_active() {
                Self::draw_picker_popup(ui, model, te_id, pane_id, colors, rect, picker_max_h);
            }
        }

        event
    }

    fn draw_transcript(
        ui: &mut egui::Ui,
        model: &AssistantModel,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        text_cache: &mut MarkdownTextCache,
        colors: &Colors,
    ) {
        // Native cross-widget text selection: a drag started in one bubble must
        // extend through the gaps into the next, so the user can copy a span
        // across several messages. egui's multi-widget selection is already on
        // by default; the only thing that breaks it here is the scroll area
        // stealing the drag in the inter-bubble margins — `drag_to_scroll(false)`
        // hands those drags to the selection instead. Wheel and the scrollbar
        // still scroll.
        ui.style_mut().interaction.selectable_labels = true;
        ui.style_mut().interaction.multi_widget_text_select = true;
        egui::ScrollArea::vertical()
            .id_salt("assistant_transcript")
            .auto_shrink([false, false])
            .drag_to_scroll(false)
            // Follow new content (streamed chunks, command output) whenever
            // the view is already at the bottom; egui releases the stick as
            // soon as the user scrolls up to read history.
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add_space(style::SPACE_SM);
                // The in-flight turn renders at its anchor — right after the
                // message that started it — so rows appended mid-turn
                // (slash-view output, queued messages) appear below it, in
                // the position the committed reply will land in.
                let anchor = model
                    .turn_anchor
                    .unwrap_or(model.turns.len())
                    .min(model.turns.len());
                Self::draw_turn_range(
                    ui,
                    md_cache,
                    text_cache,
                    &model.conversation_id,
                    0,
                    colors,
                    &model.turns[..anchor],
                    model.show_thoughts,
                );
                for active in &model.active_tools {
                    Self::draw_active_tool_row(ui, colors, &active.tool);
                }
                if matches!(model.compaction, CompactionState::Compacting) {
                    ui.label(
                        RichText::new("⟳ compacting…")
                            .size(style::TEXT_CAPTION)
                            .monospace()
                            .color(colors.accent),
                    );
                    ui.add_space(style::SPACE_SM);
                }
                if model.streaming.in_flight {
                    ui.push_id("streaming", |ui| {
                        Self::draw_streaming_row(ui, model, md_cache, colors);
                    });
                }
                Self::draw_turn_range(
                    ui,
                    md_cache,
                    text_cache,
                    &model.conversation_id,
                    anchor,
                    colors,
                    &model.turns[anchor..],
                    model.show_thoughts,
                );
                ui.add_space(style::SPACE_SM);
            });
    }

    /// Render a contiguous slice of `model.turns` in order, collapsing runs of
    /// consecutive successful tool calls into a single compact trail so they
    /// don't bury the message they precede. Failures and permission denials
    /// (`ToolStatus::Failed`) break a run and always render as their own row —
    /// only routine successful calls collapse. `index_offset` is the slice's
    /// absolute start index into `model.turns`, so cross-frame widget/cache
    /// keys stay stable.
    fn draw_turn_range(
        ui: &mut egui::Ui,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        text_cache: &mut MarkdownTextCache,
        conversation_id: &str,
        index_offset: usize,
        colors: &Colors,
        turns: &[super::model::Turn],
        show_thoughts: bool,
    ) {
        for group in super::model::group_turns_for_render(turns) {
            match group {
                super::model::TurnGroup::Row(i, turn) => {
                    ui.push_id(index_offset + i, |ui| {
                        Self::draw_turn_row(
                            ui,
                            md_cache,
                            text_cache,
                            conversation_id,
                            index_offset + i,
                            colors,
                            turn,
                            show_thoughts,
                        );
                    });
                }
                super::model::TurnGroup::ToolRun(i, run) => {
                    ui.push_id(index_offset + i, |ui| {
                        Self::draw_tool_trail(ui, colors, run);
                    });
                }
            }
        }
    }

    /// A compact, scannable trail for a run of consecutive successful tool
    /// calls. A single call renders exactly like before (one line); more than
    /// one collapses into a summary line the user can expand.
    fn draw_tool_trail(ui: &mut egui::Ui, colors: &Colors, run: &[super::model::Turn]) {
        if let [only] = run {
            Self::draw_succeeded_tool_line(ui, colors, &only.text);
            return;
        }
        egui::CollapsingHeader::new(
            RichText::new(format!("✓ {} tool calls", run.len()))
                .size(style::TEXT_CAPTION)
                .monospace()
                .color(colors.text_dim),
        )
        .id_salt("tool_trail")
        .default_open(false)
        .show(ui, |ui| {
            for turn in run {
                Self::draw_succeeded_tool_line(ui, colors, &turn.text);
            }
        });
        ui.add_space(style::SPACE_SM);
    }

    /// A single completed, successful tool-call row — the pre-collapse look,
    /// reused both for lone calls and inside an expanded trail.
    fn draw_succeeded_tool_line(ui: &mut egui::Ui, colors: &Colors, text: &str) {
        ui.label(
            RichText::new(format!("✓ {text}"))
                .size(style::TEXT_CAPTION)
                .monospace()
                .color(colors.text_dim),
        );
        ui.add_space(style::SPACE_SM);
    }

    /// Render `text` as markdown (links, emphasis, inline code, fenced
    /// blocks) sized to the chat body scale. Hyperlinks open through egui's
    /// native `open_url`. Raw inline HTML is not interpreted — it renders as
    /// literal text, which is the honest behavior for an immediate-mode UI.
    fn markdown_body(
        ui: &mut egui::Ui,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        colors: &Colors,
        markdown: &str,
    ) {
        crate::ui::markdown::show(
            ui,
            md_cache,
            colors,
            markdown,
            colors.text_primary,
            style::TEXT_BODY,
        );
    }

    fn draw_turn_row(
        ui: &mut egui::Ui,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        text_cache: &mut MarkdownTextCache,
        conversation_id: &str,
        turn_index: usize,
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
                Self::assistant_bubble(ui, colors, |ui| {
                    let markdown = text_cache.softened_turn_text(conversation_id, turn_index, turn);
                    Self::markdown_body(ui, md_cache, colors, markdown);
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
    ///
    /// Sizing is responsive: the action row wraps onto a second line
    /// instead of overflowing once the pane is too narrow to fit all four
    /// buttons, and the summary text wraps within the available width
    /// rather than clipping — so the sheet scales down to a small pane and
    /// up to an ultra-wide one without layout breakage.
    fn draw_permission_sheet(
        ui: &mut egui::Ui,
        model: &AssistantModel,
        colors: &Colors,
    ) -> Option<PermissionChoice> {
        let pending = model.pending_permission.as_ref()?;
        let selected = pending.selected;
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
                ui.scope(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    ui.set_max_width(ui.available_width());
                    ui.label(
                        RichText::new(format!(
                            "assistant (medium) wants to run the app tool '{}'",
                            pending.tool
                        ))
                        .size(style::TEXT_BODY)
                        .color(colors.text_primary),
                    );
                });
                if !pending.input_summary.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(ui.available_width());
                        crate::ui::labels::description_label(ui, &pending.input_summary, colors);
                    });
                }
                ui.add_space(style::SPACE_XS);
                let actions: [(&str, ButtonKind, PermissionChoice); 4] = [
                    ("Allow once", ButtonKind::Accent, PermissionChoice::AllowOnce),
                    (
                        "Allow this session",
                        ButtonKind::Primary,
                        PermissionChoice::AllowSession,
                    ),
                    (
                        "Always allow",
                        ButtonKind::Primary,
                        PermissionChoice::AllowAlways,
                    ),
                    ("Deny", ButtonKind::Danger, PermissionChoice::Deny),
                ];
                // `Ui::horizontal_wrapped` reflows buttons onto additional
                // rows instead of clipping or forcing the frame wider than
                // the pane, so the sheet stays usable at small window sizes.
                ui.horizontal_wrapped(|ui| {
                    for (i, (label, kind, value)) in actions.into_iter().enumerate() {
                        let resp = chrome_button(ui, label, kind, colors, 0.0);
                        if i == selected {
                            ui.painter().rect_stroke(
                                resp.rect.expand(2.0),
                                style::RADIUS_MD,
                                egui::Stroke::new(2.0, colors.accent),
                                egui::StrokeKind::Outside,
                            );
                        }
                        if resp.clicked() {
                            choice = Some(value);
                        }
                    }
                });
            });
        ui.add_space(style::SPACE_XS);
        choice
    }

    /// Consume the permission sheet's keyboard-nav keys before the composer
    /// TextEdit renders, mirroring `handle_overlay_keys`: Tab/Shift-Tab and
    /// the arrow keys move the focused action, Enter activates it, and Esc
    /// denies outright (the safe default) rather than merely dismissing the
    /// sheet, since a pending ask-gated tool call has nothing safe to fall
    /// back to.
    fn handle_permission_keys(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
    ) -> Option<ComposerEvent> {
        let mut next = false;
        let mut prev = false;
        let mut confirm = false;
        let mut deny = false;
        ui.input_mut(|input| {
            next = input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
            prev = input.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
            confirm = input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            deny = input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        });
        if next {
            model.permission_move_next();
        }
        if prev {
            model.permission_move_prev();
        }
        if deny {
            return Some(ComposerEvent::Permission(PermissionChoice::Deny));
        }
        if confirm {
            return model
                .permission_selected_choice()
                .map(ComposerEvent::Permission);
        }
        None
    }

    /// "thoughts" section: the model's reasoning tokens, rendered open and
    /// dim above the answer. `/thoughts` is the visibility switch — when the
    /// user opted in, the thoughts just show; no per-turn disclosure
    /// triangle to click. Used both for the in-flight turn and for persisted
    /// assistant turns.
    fn draw_thoughts_section(ui: &mut egui::Ui, colors: &Colors, thoughts: &str) {
        ui.label(
            RichText::new("thoughts")
                .size(style::TEXT_HINT)
                .color(colors.text_dim),
        );
        ui.label(
            RichText::new(thoughts)
                .size(style::TEXT_CAPTION)
                .italics()
                .color(colors.text_dim),
        );
        ui.add_space(style::SPACE_XS);
    }

    /// The soft, left-aligned assistant reply bubble. Shared by committed turns
    /// (`draw_turn_row`) and the in-flight streaming row so the background is
    /// identical from the first frame of a turn — it must not pop in only once
    /// the turn commits.
    fn assistant_bubble(
        ui: &mut egui::Ui,
        colors: &Colors,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        let bubble_max = ui.available_width() * Self::BUBBLE_MAX_FRACTION;
        egui::Frame::new()
            .fill(colors.bg_active)
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(
                style::SPACE_SM as i8,
                style::SPACE_XS as i8,
            ))
            .show(ui, |ui| {
                ui.set_max_width(bubble_max);
                add_contents(ui);
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
        // Same bubble as a committed reply, present from the first frame —
        // the thinking-dots beat and every streamed token sit on the
        // background, so it never appears only after streaming ends.
        Self::assistant_bubble(ui, colors, |ui| {
            if model.streaming.partial_answer.is_empty() {
                Self::draw_thinking_dots(ui, colors);
            } else {
                let markdown =
                    crate::ui::markdown::harden_soft_breaks(&model.streaming.partial_answer);
                Self::markdown_body(ui, md_cache, colors, &markdown);
            }
        });
        ui.add_space(style::SPACE_MD);
    }

    /// Three dots pulsing in sequence — the "assistant is thinking" beat
    /// shown before the first answer token arrives. The pane already
    /// requests repaints every 50ms while a turn is in flight.
    fn draw_thinking_dots(ui: &mut egui::Ui, colors: &Colors) {
        const DOT_R: f32 = 2.5;
        const DOT_GAP: f32 = 11.0;
        const EDGE_PAD: f32 = 3.5;
        const DOTS_W: f32 = EDGE_PAD * 2.0 + DOT_R * 2.0 + DOT_GAP * 2.0;

        let t = ui.input(|i| i.time);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(DOTS_W, 18.0), egui::Sense::hover());
        for k in 0..3 {
            let phase = ((t * 2.2 - k as f64 * 0.45).sin() * 0.5 + 0.5) as f32;
            let color = colors.text_dim.gamma_multiply(0.35 + 0.65 * phase);
            ui.painter().circle_filled(
                egui::pos2(
                    rect.left() + EDGE_PAD + DOT_R + k as f32 * DOT_GAP,
                    rect.center().y,
                ),
                DOT_R,
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
    /// navigation (arrows), Tab-complete and Enter-send while picking, and
    /// plain Enter submit otherwise. Shift+Enter is left for the TextEdit to
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
                // Tab completes the selection into the composer for further
                // editing; Enter completes it AND sends it in the same frame.
                let mut complete = false;
                let mut send = false;
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
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                        complete = true;
                    }
                    // Plain Enter completes + sends; Shift+Enter is left for the
                    // TextEdit to insert a newline. `consume_key` matches
                    // logically (ignores extra Shift), so guard on `!shift`
                    // first — otherwise Shift+Enter would be eaten here.
                    if !input.modifiers.shift
                        && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                    {
                        complete = true;
                        send = true;
                    }
                });
                if complete {
                    let (name, _) = matches[model.picker_selected];
                    Self::complete_command(
                        ui.ctx(),
                        model,
                        te_id,
                        name,
                        if send { "enter" } else { "tab" },
                    );
                    if send {
                        return Some(ComposerEvent::Submit);
                    }
                }
                return None;
            }
            // No matches: fall through so Enter submits the raw text.
        }
        let mut submit = false;
        ui.input_mut(|input| {
            // Shift+Enter inserts a newline (handled natively by the TextEdit's
            // `return_key`); only plain Enter submits. `consume_key` ignores
            // extra Shift, so guard on `!shift` before consuming.
            if !input.modifiers.shift && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            {
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

        // The popup rect is computed here, not by egui: a bottom-pivoted Area
        // positions itself from its own last-frame size, and the ScrollArea
        // clamps to the space below that position — a feedback loop with a
        // stable collapsed state (the "one visible row" bug). Deriving the
        // height from the row count breaks the loop.
        let row_gap = ui.spacing().item_spacing.y;
        let content_h = matches.len() as f32 * style::LIST_ROW_H
            + matches.len().saturating_sub(1) as f32 * row_gap;
        let list_h = content_h.min(max_h.max(0.0));
        let margin = style::SPACE_XS;
        let popup_h = list_h + 2.0 * margin + 2.0;
        let pos = composer_rect.left_top() - egui::vec2(0.0, style::SPACE_XS + popup_h);

        egui::Area::new(pane_id.with("assistant_picker"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.set_width(composer_rect.width());
                egui::Frame::new()
                    .fill(colors.bg_active)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(style::RADIUS_MD)
                    .inner_margin(egui::Margin::same(margin as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt("assistant_picker_scroll")
                            .max_height(list_h)
                            .min_scrolled_height(list_h)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (i, (name, purpose)) in matches.iter().enumerate() {
                                    let row = ListRow::new(&format!("/{name}"))
                                        .secondary(purpose)
                                        .selected(i == selected_idx)
                                        .show(ui, colors);
                                    if i == selected_idx {
                                        row.scroll_into_view(ui, should_scroll);
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

    /// Consume overlay navigation keys before the composer TextEdit renders.
    /// Arrows move the cursor, Enter confirms (returned to the shell), Esc
    /// cancels, and Space cycles a decision — but only in the permissions
    /// manager, so Space keeps inserting literal spaces everywhere else.
    fn handle_overlay_keys(ui: &mut egui::Ui, model: &mut AssistantModel) -> Option<ComposerEvent> {
        let is_perms = matches!(model.overlay, AssistantOverlay::PermissionsManager { .. });
        let mut up = false;
        let mut down = false;
        let mut confirm = false;
        let mut cancel = false;
        let mut cycle = false;
        ui.input_mut(|input| {
            up = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            down = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
            confirm = input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            cancel = input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            if is_perms {
                cycle = input.consume_key(egui::Modifiers::NONE, egui::Key::Space);
            }
        });
        if up {
            model.overlay_move_up();
        }
        if down {
            model.overlay_move_down();
        }
        if cycle {
            model.overlay_cycle_decision();
        }
        if cancel {
            model.cancel_overlay();
            return None;
        }
        confirm.then_some(ComposerEvent::OverlayConfirm)
    }

    /// The model/agent picker and permissions manager share the slash-command
    /// picker's floating geometry: an `Area` anchored above the composer,
    /// growing upward, with a scrollable `ListRow` body clamped to `max_h`.
    fn draw_overlay_popup(
        ui: &egui::Ui,
        model: &AssistantModel,
        pane_id: egui::Id,
        colors: &Colors,
        composer_rect: egui::Rect,
        max_h: f32,
    ) {
        let row_count = model.overlay_len().max(1);
        let selected = model.overlay_selected();
        let row_gap = ui.spacing().item_spacing.y;
        let content_h =
            row_count as f32 * style::LIST_ROW_H + row_count.saturating_sub(1) as f32 * row_gap;
        let list_h = content_h.min(max_h.max(0.0));
        let margin = style::SPACE_XS;
        let popup_h = list_h + 2.0 * margin + 2.0;
        let pos = composer_rect.left_top() - egui::vec2(0.0, style::SPACE_XS + popup_h);

        egui::Area::new(pane_id.with("assistant_overlay"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.set_width(composer_rect.width());
                egui::Frame::new()
                    .fill(colors.bg_active)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(style::RADIUS_MD)
                    .inner_margin(egui::Margin::same(margin as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt("assistant_overlay_scroll")
                            .max_height(list_h)
                            .min_scrolled_height(list_h)
                            .auto_shrink([false, true])
                            .show(ui, |ui| match &model.overlay {
                                AssistantOverlay::ModelPicker {
                                    current_tier,
                                    active_agent_id,
                                    tiers,
                                    agents,
                                    ..
                                } => Self::draw_model_picker_rows(
                                    ui,
                                    colors,
                                    selected,
                                    *current_tier,
                                    active_agent_id,
                                    tiers,
                                    agents,
                                ),
                                AssistantOverlay::PermissionsManager { grants, .. } => {
                                    Self::draw_permissions_rows(ui, colors, selected, grants)
                                }
                                AssistantOverlay::None => {}
                            });
                    });
            });
    }

    fn draw_model_picker_rows(
        ui: &mut egui::Ui,
        colors: &Colors,
        selected: usize,
        current_tier: ModelTier,
        active_agent_id: &str,
        tiers: &[ModelTier],
        agents: &[super::model::AgentChoice],
    ) {
        Self::overlay_section_label(ui, colors, "Model tier");
        for (i, tier) in tiers.iter().enumerate() {
            let mut row = ListRow::new(model_tier_label(*tier)).selected(i == selected);
            if *tier == current_tier {
                row = row.chip("current");
            }
            let resp = row.show(ui, colors);
            if i == selected {
                resp.scroll_into_view(ui, true);
            }
        }
        Self::overlay_section_label(ui, colors, "Agent");
        for (j, agent) in agents.iter().enumerate() {
            let idx = tiers.len() + j;
            let mut row = ListRow::new(&agent.display_name)
                .secondary(&agent.id)
                .selected(idx == selected);
            if agent.id == active_agent_id {
                row = row.chip("active");
            }
            let resp = row.show(ui, colors);
            if idx == selected {
                resp.scroll_into_view(ui, true);
            }
        }
    }

    fn draw_permissions_rows(
        ui: &mut egui::Ui,
        colors: &Colors,
        selected: usize,
        grants: &[super::model::GrantRow],
    ) {
        if grants.is_empty() {
            ui.label(
                RichText::new("No grants yet — tool calls will ask.")
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
            return;
        }
        for (i, grant) in grants.iter().enumerate() {
            let resp = ListRow::new(&grant.target_id)
                .secondary(decision_label(grant.decision))
                .selected(i == selected)
                .show(ui, colors);
            if i == selected {
                resp.scroll_into_view(ui, true);
            }
        }
    }

    fn overlay_section_label(ui: &mut egui::Ui, colors: &Colors, label: &str) {
        ui.add_space(style::SPACE_XS);
        ui.label(
            RichText::new(label)
                .size(style::TEXT_HINT)
                .color(colors.text_dim),
        );
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
        max_h: f32,
    ) -> egui::Rect {
        let font_id = egui::FontId::proportional(style::TEXT_BODY);
        let row_height = ui.fonts(|f| f.row_height(&font_id));
        // Grow up to `max_h` (75% of the pane) before scrolling, never below a
        // single row even in a very short pane.
        let max_text_h = max_h.max(row_height);

        // Accent outline while the composer holds keyboard focus — same
        // affordance as the host text fields.
        let has_kb_focus = ui.memory(|m| m.has_focus(te_id));
        let stroke_color = if has_kb_focus {
            colors.accent
        } else {
            colors.border
        };

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
                // Glyph-height caret, unified with every other host text input
                // (see `text_field::draw_text_caret`): hide egui's full-row
                // caret (transparent + no blink) and paint a replacement that
                // matches the glyph height on top.
                ui.visuals_mut().text_cursor.blink = false;
                ui.visuals_mut().text_cursor.stroke.color = egui::Color32::TRANSPARENT;
                // The TextEdit reports its own height; the ScrollArea caps it at
                // `max_text_h` and scrolls past that. No hand-rolled galley
                // measurement — same shape as the quick-note composer. Measuring
                // the height ourselves fought the TextEdit's real layout (it
                // wraps against a width already reduced by the floating
                // scrollbar's reserved gutter), which flashed a stray scrollbar
                // mid-growth.
                egui::ScrollArea::vertical()
                    .id_salt("assistant_composer_scroll")
                    .max_height(max_text_h)
                    .show(ui, |ui| {
                        let output = egui::TextEdit::multiline(&mut model.composer)
                            .id(te_id)
                            .desired_rows(1)
                            .desired_width(f32::INFINITY)
                            .frame(false)
                            // Left padding beyond egui's default `symmetric(4,
                            // 2)` margin, so the caret and hint text never
                            // sit flush against the composer's edge.
                            .margin(egui::Margin {
                                left: 6,
                                right: 4,
                                top: 2,
                                bottom: 2,
                            })
                            // The hint galley (rendered at its own, smaller
                            // size below) and the real-text/caret galley
                            // otherwise both default to top-aligned, which
                            // reads as off-center whenever their line
                            // heights differ. Center both so the composer's
                            // single row always looks vertically balanced.
                            .vertical_align(egui::Align::Center)
                            // Keep Tab in the composer: without the focus
                            // lock, egui's frame-start focus traversal
                            // moves focus away before the picker's Tab
                            // handler ever sees the key.
                            .lock_focus(true)
                            // Plain Enter is consumed for submit before
                            // the TextEdit runs; Shift+Enter is the
                            // newline key (the default return_key is
                            // unmodified Enter, so Shift+Enter would
                            // otherwise insert nothing).
                            .return_key(Some(egui::KeyboardShortcut::new(
                                egui::Modifiers::SHIFT,
                                egui::Key::Enter,
                            )))
                            .hint_text(
                                RichText::new("Message the assistant — / for commands")
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim),
                            )
                            .font(font_id.clone())
                            .show(ui);
                        crate::ui::text_field::draw_text_caret(
                            ui,
                            &output,
                            style::TEXT_BODY,
                            row_height,
                            egui::Stroke::new(1.0, colors.accent),
                        );
                        if output.response.changed() {
                            model.reset_history_recall();
                        }
                        response = Some(output.response);
                    });
            });
        // Keep the composer focused whenever the pane is active but the
        // TextEdit has lost focus — covers initial open, pane switches, and
        // window zoom/fullscreen toggles (which silently drop egui keyboard
        // focus, even with a draft in the box). This is the same unconditional
        // rule the text-editor pane uses (`text_editor_app.rs`): a one-shot
        // edge guard breaks after the very focus changes it needs to react to.
        // It does not fight transcript selection — selecting a label takes
        // pointer interaction, not keyboard focus, and an empty composer never
        // consumes the copy event, so cross-bubble copy still works.
        if let Some(response) = response {
            if is_focused && !response.has_focus() {
                response.request_focus();
            }
        }
        frame_response.response.rect
    }
}
