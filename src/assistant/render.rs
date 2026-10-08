//! egui rendering for the host Assistant pane: header bar, transcript,
//! streaming row, slash-command picker, and multiline composer. Pure view
//! over `AssistantModel` — all state transitions go back through the model.
//!
//! Layout: the hint bar and composer are placed from the pane floor at a
//! height measured this frame. A content-sized bottom panel clips to last
//! frame's height, so a Shift+Enter newline hid the hint bar for one frame.
//! The slash-command picker is a floating `Area` popup anchored to the top
//! edge of the composer — it grows and shrinks upward without resizing the
//! composer or the transcript, so filtering never shifts surrounding layout.
//! Enter/Tab/arrow keys are consumed *before* the composer TextEdit renders,
//! so completion and submit never flash an intermediate buffer state.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::RichText;

use crate::ui::button::{chrome_button, ButtonKind};
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::list::ListRow;
use crate::ui::style;
use crate::ui::text_field::TextArea;
use crate::ui::theme::Colors;

use crate::protocol::ModelTier;
use crate::broker::Decision;

use super::commands;

/// Stable accessibility label for a chess-style `move` field in a tool summary.
fn move_label(summary: &str) -> Option<String> {
    let mv = if let Some(start) = summary.find("move: ") {
        let rest = &summary[start + "move: ".len()..];
        let end = rest.find([',', '}', '\n']).unwrap_or(rest.len());
        rest[..end].trim().trim_matches('"').to_string()
    } else {
        let start = summary.find("\"move\"")?;
        let rest = summary[start + "\"move\"".len()..].trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        let rest = rest.strip_prefix('"')?;
        let end = rest.find('"')?;
        rest[..end].to_string()
    };
    if mv.is_empty() {
        None
    } else {
        Some(format!("move {mv}"))
    }
}
use super::model::{
    AssistantModel, AssistantOverlay, CompactionState, PermissionChoice, ToolStatus, TurnRole,
};

/// Braille beat shared by every animated activity row (running tool,
/// tool-call generation). Keyed to wall clock at 100ms per frame.
const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// What the composer asked the pane shell to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerEvent {
    Submit,
    /// The user decided the pending permission sheet.
    Permission(PermissionChoice),
    /// Open the Permissions app from a denied tool row.
    ReviewPermissions,
    /// Enter pressed in an open picker/manager overlay: apply the selection.
    OverlayConfirm,
}

fn footer_hints(model: &AssistantModel) -> Vec<HintGroup<'static>> {
    const TAB: &[&str] = &["\u{21e5}"];
    const ENTER: &[&str] = &["\u{21b5}"];
    const ESC: &[&str] = &["Esc"];
    const ARROWS: &[&str] = &["\u{2191}", "\u{2193}"];
    const SPACE: &[&str] = &["Space"];
    const SHIFT_ENTER: &[&str] = &["\u{21e7}", "\u{21b5}"];
    const SLASH: &[&str] = &["/"];
    if model.pending_permission.is_some() {
        vec![
            HintGroup::new(TAB, "navigate"),
            HintGroup::new(ENTER, "confirm"),
            HintGroup::new(ESC, "deny"),
        ]
    } else if model.overlay_active() {
        let mut hints = vec![
            HintGroup::new(ARROWS, "navigate"),
            HintGroup::new(ENTER, "confirm"),
            HintGroup::new(ESC, "cancel"),
        ];
        if matches!(model.overlay, AssistantOverlay::PermissionsManager { .. }) {
            hints.push(HintGroup::new(SPACE, "cycle"));
        }
        hints
    } else {
        let mut hints = vec![
            HintGroup::new(ENTER, "send"),
            HintGroup::new(SHIFT_ENTER, "newline"),
            HintGroup::new(SLASH, "commands"),
        ];
        if model.streaming.in_flight {
            hints.push(HintGroup::new(ESC, "stop"));
        }
        hints
    }
}

fn composer_text_height(ui: &mut egui::Ui, text: &str, width: f32, cap: f32) -> f32 {
    let font = egui::FontId::proportional(style::TEXT_BODY);
    TextArea::composer(egui::Id::new("assistant_composer_measure"), "hint")
        .font(font)
        .content_height(ui, text, width, cap)
}

fn composer_outer_height(text_h: f32) -> f32 {
    (text_h + style::SPACE_XS * 2.0 + COMPOSER_STROKE * 2.0).ceil()
}

/// `rows` menu entries including the shared list-row gap.
fn menu_block_height(ui: &egui::Ui, rows: usize) -> f32 {
    if rows == 0 {
        return 0.0;
    }
    let row = style::LIST_ROW_H + style::LIST_ROW_GAP_V;
    let gap = ui.spacing().item_spacing.y;
    rows as f32 * row + rows.saturating_sub(1) as f32 * gap
}

/// "Medium — xiaomi/mimo-v2.5-pro", or just the tier when no id is configured.
fn tier_menu_label(tier: ModelTier, model_id: Option<&str>) -> String {
    let title = match tier {
        ModelTier::Low => "Low",
        ModelTier::Medium => "Medium",
        ModelTier::High => "High",
    };
    match model_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => format!("{title} — {id}"),
        None => title.to_string(),
    }
}

fn role_caption(ui: &mut egui::Ui, label: &str, colors: &Colors, align: egui::Align) {
    ui.with_layout(egui::Layout::top_down(align), |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(label)
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            )
            .selectable(false),
        );
    });
}

fn draw_jump_to_latest(ui: &mut egui::Ui, transcript: egui::Rect, colors: &Colors) -> bool {
    let label = "↓  Latest";
    let galley = ui.fonts_mut(|fonts| {
        fonts.layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(style::TEXT_CAPTION),
            colors.text_primary,
        )
    });
    let pad = egui::vec2(style::SPACE_MD, style::SPACE_XS);
    let size = galley.size() + pad * 2.0;
    if size.x > transcript.width() || size.y > transcript.height() {
        return false;
    }
    let origin = egui::pos2(
        transcript.center().x - size.x * 0.5,
        transcript.bottom() - size.y - style::SPACE_SM,
    );
    let mut clicked = false;
    egui::Area::new(ui.id().with("assistant_jump"))
        .order(egui::Order::Foreground)
        .fixed_pos(origin)
        .show(ui.ctx(), |ui| {
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            let fill = if response.hovered() {
                colors.bg_hover
            } else {
                colors.bg_active
            };
            ui.painter().rect_filled(rect, style::RADIUS_MD, fill);
            ui.painter().rect_stroke(
                rect,
                style::RADIUS_MD,
                egui::Stroke::new(1.0_f32, colors.accent.gamma_multiply(0.65)),
                egui::StrokeKind::Inside,
            );
            ui.painter().galley(
                egui::pos2(rect.left() + pad.x, rect.center().y - galley.size().y * 0.5),
                galley,
                colors.text_primary,
            );
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            clicked = response.clicked();
        });
    clicked
}

fn scope_top_down<R>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        add,
    )
    .inner
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

/// Composer frame stroke, matched by the footer height reservation.
const COMPOSER_STROKE: f32 = 1.0;

/// How close to the bottom (px) still counts as pinned.
const SCROLL_BOTTOM_SLACK: f32 = 24.0;

/// Which edge of the transcript a chat bubble anchors to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BubbleSide {
    /// Assistant replies and the streaming row — grow from the left edge.
    Left,
    /// User messages — pinned to the right edge, iMessage/Slack-style.
    Right,
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
            TurnRole::Command => 5,
            TurnRole::Local => 6,
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
        host_pane_id: u64,
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
        // The composer is this pane's default text surface: the post-frame
        // reconciler (stint 0429) grants it egui focus while the pane owns
        // input and surrenders it the moment ownership moves elsewhere.
        crate::ui::focus::register_default_text_surface(
            ui.ctx(),
            crate::ui::focus::SurfaceKey::Pane(host_pane_id),
            te_id,
        );

        // Picker shows up to 10 command rows, clamped so it never overruns
        // the transcript area in a short pane (but always fits at least 3).
        let total_h = ui.available_rect_before_wrap().height();
        let picker_max_h =
            (style::LIST_ROW_H * 10.0).min((total_h * 0.8).max(style::LIST_ROW_H * 3.0));
        let mut event = None;

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

        // Footer rects are derived from this frame's measurements, then each
        // region paints top-down inside its own rect. The hint slot's height
        // does not include the composer, so a newline cannot move or clip it.
        let full = ui.available_rect_before_wrap();
        let content = egui::Rect::from_min_max(
            egui::pos2(full.left() + style::SPACE_MD, full.top()),
            egui::pos2(full.right() - style::SPACE_MD, full.bottom() - style::SPACE_SM),
        );
        let hints = footer_hints(model);
        let gap = style::SPACE_XS;
        let hint_h = HintBar::new(&hints).height(ui, content.width());
        let text_w =
            (content.width() - style::SPACE_SM * 2.0 - COMPOSER_STROKE * 2.0).max(1.0);
        let cap = (total_h * Self::COMPOSER_MAX_FRACTION).max(style::TEXT_BODY);
        let text_h = composer_text_height(ui, &model.composer, text_w, cap);
        let composer_h = composer_outer_height(text_h);
        let lines = model.composer.lines().count().max(1);
        let lines_id = pane_id.with("assistant_composer_lines");
        let prev_lines = ui.ctx().data(|data| data.get_temp::<usize>(lines_id));
        if prev_lines != Some(lines) {
            log::info!(
                "assistant: composer {:.0}px ({lines} lines); hint bar held at pane floor",
                composer_h
            );
        }
        ui.ctx()
            .data_mut(|data| data.insert_temp(lines_id, lines));

        let hint_rect = egui::Rect::from_min_max(
            egui::pos2(content.left(), content.bottom() - hint_h),
            content.max,
        );
        let composer_bottom = hint_rect.top() - gap;
        let composer_slot = egui::Rect::from_min_max(
            egui::pos2(content.left(), composer_bottom - composer_h),
            egui::pos2(content.right(), composer_bottom),
        );
        let perm_id = pane_id.with("assistant_perm_h");
        let pending = model.pending_permission.is_some();
        let stored_perm = if pending {
            ui.ctx()
                .data(|data| data.get_temp::<f32>(perm_id).unwrap_or(0.0))
        } else {
            0.0
        };
        let above_bottom = composer_slot.top() - gap;
        let above = egui::Rect::from_min_max(
            content.min,
            egui::pos2(content.right(), above_bottom.max(content.top())),
        );
        let perm_known = pending && stored_perm > 1.0;
        let perm_slot = if perm_known {
            egui::Rect::from_min_max(
                egui::pos2(
                    content.left(),
                    (above.bottom() - stored_perm).max(above.top()),
                ),
                above.max,
            )
        } else if pending {
            above
        } else {
            egui::Rect::NOTHING
        };
        let transcript_bottom = if perm_known {
            (perm_slot.top() - gap).max(content.top())
        } else {
            above.bottom()
        };
        let transcript_rect = egui::Rect::from_min_max(
            content.min,
            egui::pos2(content.right(), transcript_bottom),
        );

        scope_top_down(ui, hint_rect, |ui| {
            HintBar::new(&hints).show(ui, colors);
        });
        let composer_rect = scope_top_down(ui, composer_slot, |ui| {
            Self::draw_composer(ui, model, te_id, colors, cap, text_h)
        });
        if pending {
            let measured = scope_top_down(ui, perm_slot, |ui| {
                let choice = Self::draw_permission_sheet(ui, model, colors);
                let measured = ui.min_rect().height();
                (choice, measured)
            });
            if let Some(choice) = measured.0 {
                event = Some(ComposerEvent::Permission(choice));
            }
            if (measured.1 - stored_perm).abs() > 1.0 && !ui.ctx().will_discard() {
                ui.ctx().data_mut(|data| data.insert_temp(perm_id, measured.1));
                ui.ctx()
                    .request_discard("assistant permission sheet height");
            }
        }
        let transcript_event = scope_top_down(ui, transcript_rect, |ui| {
            Self::draw_transcript(ui, model, md_cache, text_cache, colors)
        });
        if event.is_none() {
            event = transcript_event;
        }

        {
            if model.overlay_active() {
                Self::draw_overlay_popup(ui, model, pane_id, colors, composer_rect, picker_max_h);
            } else if model.picker_active() {
                Self::draw_picker_popup(
                    ui,
                    model,
                    te_id,
                    pane_id,
                    colors,
                    composer_rect,
                    picker_max_h,
                );
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
    ) -> Option<ComposerEvent> {
        // Native cross-widget text selection: a drag started in one bubble must
        // extend through the gaps into the next, so the user can copy a span
        // across several messages. egui's multi-widget selection is already on
        // by default; the only thing that breaks it here is the scroll area
        // stealing the drag in the inter-bubble margins — `drag_to_scroll(false)`
        // hands those drags to the selection instead. Wheel and the scrollbar
        // still scroll.
        ui.style_mut().interaction.selectable_labels = true;
        ui.style_mut().interaction.multi_widget_text_select = true;

        let follow_id = ui.id().with("assistant_follow");
        let user_count_id = ui.id().with("assistant_user_turns");
        let conv_id = ui.id().with("assistant_follow_conv");
        let mut follow = ui
            .ctx()
            .data(|d| d.get_temp::<bool>(follow_id).unwrap_or(true));
        let was_following = follow;
        let prev_conv = ui.ctx().data(|d| d.get_temp::<String>(conv_id));
        if prev_conv.as_deref() != Some(model.conversation_id.as_str()) {
            follow = true;
        }
        let user_turns = model
            .turns
            .iter()
            .filter(|turn| turn.role == TurnRole::User)
            .count();
        let prev_users = ui
            .ctx()
            .data(|d| d.get_temp::<usize>(user_count_id).unwrap_or(0));
        if user_turns > prev_users {
            follow = true;
            log::info!("assistant: transcript pinned to bottom on send");
        }
        // A wheel toward older messages releases the pin before this frame's
        // stick runs, so the same gesture is not snapped back to the end.
        // `animated(false)` consumes the raw delta; smoothing may lag a frame.
        let scroll_up = ui.input(|input| input.smooth_scroll_delta.y > 1.0);
        if scroll_up {
            follow = false;
        }
        let scroll_id = ui.make_persistent_id(egui::Id::new("assistant_transcript"));
        if follow {
            if let Some(mut state) = egui::scroll_area::State::load(ui.ctx(), scroll_id) {
                state.offset.y = f32::MAX;
                state.store(ui.ctx(), scroll_id);
            }
        }

        let output = egui::ScrollArea::vertical()
            .id_salt("assistant_transcript")
            .auto_shrink([false, false])
            .animated(false)
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: false,
                ..Default::default()
            })
            // Follow new content while pinned. Releasing the pin (scroll up)
            // leaves the offset where the reader put it.
            .stick_to_bottom(follow)
            .show(ui, |ui| {
                ui.add_space(style::SPACE_SM);
                // The in-flight turn renders at its anchor — right after the
                // message that started it — so rows appended mid-turn
                // (slash-view output, queued messages) appear below it, in
                // the position the committed reply will land in.
                Self::forward_selection_wheel_scroll(ui);
                let anchor = model
                    .turn_anchor
                    .unwrap_or(model.turns.len())
                    .min(model.turns.len());
                let mut review = Self::draw_turn_range(
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
                    Self::draw_active_tool_row(ui, colors, active);
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
                review = review.or(Self::draw_turn_range(
                    ui,
                    md_cache,
                    text_cache,
                    &model.conversation_id,
                    anchor,
                    colors,
                    &model.turns[anchor..],
                    model.show_thoughts,
                ));
                ui.add_space(style::SPACE_SM);
                let anchor = ui.allocate_rect(
                    egui::Rect::from_min_size(ui.cursor().min, egui::vec2(1.0, 1.0)),
                    egui::Sense::hover(),
                );
                if follow {
                    anchor.scroll_to_me(Some(egui::Align::BOTTOM));
                }
                review
            });

        let max_offset = (output.content_size.y - output.inner_rect.height()).max(0.0);
        let distance = max_offset - output.state.offset.y;
        let at_bottom = distance <= SCROLL_BOTTOM_SLACK;
        if at_bottom && !scroll_up {
            follow = true;
        }
        let bar_drag = ui.input(|input| {
            input.pointer.is_decidedly_dragging()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| output.inner_rect.right() - pos.x < 16.0)
        });
        if bar_drag && !at_bottom {
            follow = false;
        }
        if was_following && !follow {
            log::info!("assistant: transcript follow paused — scrolled up to read");
        }
        if !at_bottom && draw_jump_to_latest(ui, output.inner_rect, colors) {
            follow = true;
            log::info!("assistant: jump to latest");
            if let Some(mut state) = egui::scroll_area::State::load(ui.ctx(), output.id) {
                state.offset.y = f32::MAX;
                state.store(ui.ctx(), output.id);
            }
        }
        if follow && distance > SCROLL_BOTTOM_SLACK && !ui.ctx().will_discard() {
            ui.ctx().request_discard("assistant stick to bottom");
        }
        ui.ctx().data_mut(|data| {
            data.insert_temp(follow_id, follow);
            data.insert_temp(user_count_id, user_turns);
            data.insert_temp(conv_id, model.conversation_id.clone());
            data.insert_temp(
                egui::Id::new("assistant_scroll_debug"),
                (output.state.offset.y, max_offset, follow, at_bottom),
            );
        });
        output.inner
    }

    /// Egui intentionally ignores wheel input while a child widget owns a
    /// drag. That is normally right for a scroll area's own drag gesture, but
    /// transcript labels own the drag for text selection. Forward the wheel
    /// delta to the enclosing scroll area so a selection can continue beyond
    /// the visible page without handing drag ownership back to the area.
    fn forward_selection_wheel_scroll(ui: &egui::Ui) {
        if ui.ctx().dragged_id().is_some() {
            let delta = ui.input(|input| input.smooth_scroll_delta());
            if delta != egui::Vec2::ZERO {
                ui.scroll_with_delta(delta);
            }
        }
    }

    /// Render a contiguous slice of `model.turns` in order — one row per
    /// turn, in strict chronological order (stint 0455). `index_offset` is
    /// the slice's absolute start index into `model.turns`, so cross-frame
    /// widget/cache keys stay stable.
    // Arg-struct refactor is a design change tracked in stint 0661.
    #[allow(clippy::too_many_arguments)]
    fn draw_turn_range(
        ui: &mut egui::Ui,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        text_cache: &mut MarkdownTextCache,
        conversation_id: &str,
        index_offset: usize,
        colors: &Colors,
        turns: &[super::model::Turn],
        show_thoughts: bool,
    ) -> Option<ComposerEvent> {
        let mut event = None;
        for (i, turn) in turns.iter().enumerate() {
            let grouped = turns.get(i.wrapping_sub(1)).is_some_and(|prev| {
                i > 0 && prev.role == turn.role
            });
            let next_grouped = turns
                .get(i + 1)
                .is_some_and(|next| next.role == turn.role);
            ui.push_id(index_offset + i, |ui| {
                let row = Self::draw_turn_row(
                    ui,
                    md_cache,
                    text_cache,
                    conversation_id,
                    index_offset + i,
                    colors,
                    turn,
                    show_thoughts,
                    grouped,
                );
                if event.is_none() {
                    event = row;
                }
            });
            let gap = if next_grouped {
                style::SPACE_XS
            } else {
                style::SPACE_MD
            };
            ui.add_space(gap);
        }
        event
    }

    /// One completed tool call: a caret dropdown headed by status icon +
    /// tool name, closed by default, whose body shows the call's input
    /// summary, output preview, and file-edit diff where present (stint
    /// 0455). Failures open by default and render in the danger hue — a
    /// failure must never hide. Rows persisted before the dropdown payloads
    /// existed render as a plain line (no caret over an empty body).
    fn draw_tool_call_row(
        ui: &mut egui::Ui,
        colors: &Colors,
        turn: &super::model::Turn,
    ) -> Option<ComposerEvent> {
        let failed = turn.status == Some(ToolStatus::Failed);
        let (icon, color) = if failed {
            ("✗", colors.danger)
        } else {
            ("✓", colors.text_dim)
        };
        if turn.text.contains("stale revision") {
            ui.label(
                RichText::new("stale revision")
                    .size(style::TEXT_CAPTION)
                    .color(colors.danger),
            );
        }
        if let Some(preview) = &turn.output_preview {
            if let Some(line) = preview.lines().find(|line| line.starts_with("actor:")) {
                ui.label(
                    RichText::new(line)
                        .size(style::TEXT_CAPTION)
                        .monospace()
                        .color(colors.text_primary),
                );
            }
        }
        let name = turn.text.split(" — ").next().unwrap_or(turn.text.as_str());
        let summary = turn.input_summary.as_deref().and_then(|raw| {
            raw.lines()
                .find(|line| !line.trim().is_empty())
                .map(str::trim)
        });
        let has_body =
            turn.input_summary.is_some() || turn.output_preview.is_some() || turn.detail.is_some();
        if !has_body {
            ui.label(
                RichText::new(format!("{icon} {name}"))
                    .size(style::TEXT_CAPTION)
                    .monospace()
                    .color(color),
            );
        } else {
            let id = ui.make_persistent_id("tool_call");
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                failed,
            );
            let _collapsed = state
                .show_header(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{icon} {name}"))
                                .size(style::TEXT_CAPTION)
                                .monospace()
                                .color(color),
                        );
                        if let Some(summary) = summary {
                            ui.label(
                                RichText::new(summary)
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                        }
                    });
                })
                .body_unindented(|ui| {
                    if let Some(input) = &turn.input_summary {
                        Self::draw_preview_block(ui, colors, "in", input);
                    }
                    if let Some(output) = &turn.output_preview {
                        Self::draw_preview_block(ui, colors, "out", output);
                    }
                    if let Some(diff) = &turn.detail {
                        Self::draw_diff_block(ui, colors, diff);
                    }
                });
        }
        if turn.text.contains("plexi permissions list") {
            let response =
                chrome_button(ui, "Review permissions", ButtonKind::Secondary, colors, 0.0);
            response
                .clicked()
                .then_some(ComposerEvent::ReviewPermissions)
        } else {
            None
        }
    }

    /// A labeled multi-line preview (tool input/output, stint 0460): real
    /// newlines render as separate rows inside a framed block that fills the
    /// available width, and long rows wrap instead of clipping at the pane
    /// edge.
    fn draw_preview_block(ui: &mut egui::Ui, colors: &Colors, label: &str, body: &str) {
        ui.label(
            RichText::new(label)
                .size(style::TEXT_HINT)
                .monospace()
                .color(colors.text_dim),
        );
        egui::Frame::new()
            .fill(colors.bg_active)
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(
                style::SPACE_SM as i8,
                style::SPACE_XS as i8,
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                for line in body.lines() {
                    ui.label(
                        RichText::new(line)
                            .size(style::TEXT_CAPTION)
                            .monospace()
                            .color(colors.text_dim),
                    );
                }
            });
        ui.add_space(style::SPACE_XS);
    }

    /// A unified diff rendered line-by-line: additions in the success hue,
    /// removals in the danger hue, headers and context dimmed.
    fn draw_diff_block(ui: &mut egui::Ui, colors: &Colors, diff: &str) {
        egui::Frame::new()
            .fill(colors.bg_active)
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(
                style::SPACE_SM as i8,
                style::SPACE_XS as i8,
            ))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for line in diff.lines() {
                    let color = match line.as_bytes().first() {
                        Some(b'+') if !line.starts_with("+++") => colors.success,
                        Some(b'-') if !line.starts_with("---") => colors.danger,
                        _ => colors.text_dim,
                    };
                    ui.label(
                        RichText::new(line)
                            .size(style::TEXT_CAPTION)
                            .monospace()
                            .color(color),
                    );
                }
            });
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

    // Arg-struct refactor is a design change tracked in stint 0661.
    #[allow(clippy::too_many_arguments)]
    fn draw_turn_row(
        ui: &mut egui::Ui,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        text_cache: &mut MarkdownTextCache,
        conversation_id: &str,
        turn_index: usize,
        colors: &Colors,
        turn: &super::model::Turn,
        show_thoughts: bool,
        grouped: bool,
    ) -> Option<ComposerEvent> {
        let text = turn.text.as_str();
        match turn.role {
            // Delivered app events are compact single-line rows.
            TurnRole::Event => {
                ui.label(
                    RichText::new(text)
                        .size(style::TEXT_CAPTION)
                        .monospace()
                        .color(colors.accent),
                );
            }
            // Completed tool calls are caret-dropdown rows (stint 0455).
            TurnRole::Tool => {
                return Self::draw_tool_call_row(ui, colors, turn);
            }
            // User turns sit right-aligned in an outlined bubble, like every
            // mainstream chat client. The body is a galley measured up-front at
            // the bubble cap and painted as-is: the frame shrinks to exactly
            // that galley, `Align::Max` pins the shrunk frame to the right edge,
            // and the galley's own `LEFT` halign keeps wrapped lines left-read.
            TurnRole::User => {
                if !grouped {
                    role_caption(ui, "You", colors, egui::Align::Max);
                }
                let cap = Self::bubble_content_cap(ui, 1.0);
                let galley = ui.fonts_mut(|f| {
                    f.layout(
                        text.to_owned(),
                        egui::FontId::proportional(style::TEXT_BODY),
                        colors.text_primary,
                        cap,
                    )
                });
                Self::chat_bubble(ui, colors, BubbleSide::Right, true, |ui| {
                    ui.add(egui::Label::new(galley));
                });
            }
            // Assistant replies sit left-aligned in a soft unstroked bubble —
            // visible against the terminal surface — and render as markdown.
            TurnRole::Assistant => {
                if show_thoughts {
                    if let Some(thoughts) = &turn.thoughts {
                        Self::draw_thoughts_section(ui, colors, thoughts);
                    }
                }
                if !grouped {
                    role_caption(ui, "Assistant", colors, egui::Align::Min);
                }
                let markdown = text_cache.softened_turn_text(conversation_id, turn_index, turn);
                Self::assistant_bubble(ui, colors, md_cache, markdown);
            }
            // Slash-command output reads like an assistant reply either way;
            // the split is who receives it, not how it looks (stint 0380).
            TurnRole::Command | TurnRole::Local => {
                if !grouped {
                    role_caption(ui, "Assistant", colors, egui::Align::Min);
                }
                let markdown = text_cache.softened_turn_text(conversation_id, turn_index, turn);
                Self::assistant_bubble(ui, colors, md_cache, markdown);
            }
            TurnRole::Error => {
                if !grouped {
                    role_caption(ui, "Error", colors, egui::Align::Min);
                }
                ui.label(
                    RichText::new(text)
                        .size(style::TEXT_BODY)
                        .color(colors.danger),
                );
            }
        }
        None
    }

    /// A tool call currently running inside the in-flight turn. The input
    /// summary shows what the call is doing (which file, which command)
    /// while it runs.
    fn draw_active_tool_row(
        ui: &mut egui::Ui,
        colors: &Colors,
        active: &super::model::ActiveToolCall,
    ) {
        // Braille spinner keyed to wall clock — the assistant already
        // repaints continuously while a turn is in flight, so this animates
        // without extra repaint requests. Elapsed seconds make a long tool
        // call (a 60s+ `app check`) read as progress, not a hang.
        let elapsed = active.started.elapsed();
        let frame = SPINNER_FRAMES[(elapsed.as_millis() / 100) as usize % SPINNER_FRAMES.len()];
        let elapsed_label = if elapsed.as_secs() >= 2 {
            format!(" · {}s", elapsed.as_secs())
        } else {
            String::new()
        };
        let line = if active.input_summary.is_empty() {
            format!("{frame} {}{elapsed_label}", active.tool)
        } else {
            format!(
                "{frame} {} {}{elapsed_label}",
                active.tool, active.input_summary
            )
        };
        ui.scope(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.label(
                RichText::new(line)
                    .size(style::TEXT_CAPTION)
                    .monospace()
                    .color(colors.accent),
            );
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
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
            .stroke(egui::Stroke::new(1.0_f32, colors.accent))
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
                    let who = if pending.actor_id.is_empty() {
                        "assistant (medium)".to_string()
                    } else {
                        pending.actor_id.clone()
                    };
                    let resource = if pending.resource_id.is_empty() {
                        String::new()
                    } else {
                        format!(" on {}", pending.resource_id)
                    };
                    ui.label(
                        RichText::new(format!(
                            "{who} wants to run '{tool}'{resource}",
                            tool = pending.tool
                        ))
                        .size(style::TEXT_BODY)
                        .color(colors.text_primary),
                    );
                    if let Some(mv) = move_label(&pending.input_summary) {
                        ui.label(
                            RichText::new(mv)
                                .size(style::TEXT_BODY)
                                .color(colors.text_primary),
                        );
                    }
                });
                if !pending.input_summary.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(ui.available_width());
                        crate::ui::labels::description_label(ui, &pending.input_summary, colors);
                    });
                }
                ui.add_space(style::SPACE_XS);
                let actions: [(&str, ButtonKind, PermissionChoice); 5] = [
                    (
                        "Allow once",
                        ButtonKind::Accent,
                        PermissionChoice::AllowOnce,
                    ),
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
                    (
                        "Always deny",
                        ButtonKind::Danger,
                        PermissionChoice::DenyAlways,
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
                                egui::Stroke::new(2.0_f32, colors.accent),
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

    /// The soft, left-aligned assistant reply bubble, rendered as markdown.
    /// Shared by committed turns (`draw_turn_row`) and the in-flight streaming
    /// row so the background is identical from the first frame of a turn — it
    /// must not pop in only once the turn commits. Shrinks to fit: the bubble
    /// is capped to the measured wrapped width of the source, so a one-line
    /// reply gets a one-line bubble rather than a full-cap slab.
    fn assistant_bubble(
        ui: &mut egui::Ui,
        colors: &Colors,
        md_cache: &mut egui_commonmark::CommonMarkCache,
        markdown: &str,
    ) {
        let inner_w = Self::measure_wrapped_width(ui, markdown, Self::bubble_content_cap(ui, 0.0));
        Self::chat_bubble(ui, colors, BubbleSide::Left, false, |ui| {
            // Left origin, so `set_width` both constrains the fill-width
            // markdown renderer and keeps the frame anchored left.
            ui.set_width(inner_w);
            Self::markdown_body(ui, md_cache, colors, markdown);
        });
    }

    /// The one chat-row primitive both roles use: a padded, rounded bubble
    /// frame anchored to the given side of the transcript. A right bubble is
    /// pinned to the pane edge by the row's `Align::Max` cross-align and must
    /// shrink to a fixed-size child (the caller paints a pre-measured galley);
    /// a left bubble shares the row's left origin, so the caller may
    /// `set_width` to constrain fill-width content. The frame chrome is
    /// identical bar the outline — `stroke` gives the user bubble its border.
    fn chat_bubble(
        ui: &mut egui::Ui,
        colors: &Colors,
        side: BubbleSide,
        stroke: bool,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        let outline = if stroke {
            egui::Stroke::new(1.0_f32, colors.border)
        } else {
            egui::Stroke::NONE
        };
        let frame = egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(outline)
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(
                style::SPACE_SM as i8,
                style::SPACE_XS as i8,
            ));
        let row_align = match side {
            BubbleSide::Left => egui::Align::Min,
            BubbleSide::Right => egui::Align::Max,
        };
        ui.with_layout(egui::Layout::top_down(row_align), |ui| {
            frame.show(ui, add_contents);
        });
    }

    /// The content-area width cap for a chat bubble: the bubble frame caps at
    /// `BUBBLE_MAX_FRACTION` of the row, and the content sits inside the
    /// horizontal padding on each edge.
    fn bubble_content_cap(ui: &egui::Ui, stroke_width: f32) -> f32 {
        (ui.available_width() * Self::BUBBLE_MAX_FRACTION - 2.0 * (style::SPACE_SM + stroke_width))
            .max(0.0)
    }

    /// Natural wrapped width `text` wants at body scale, clamped to `cap`. Used
    /// to size the assistant bubble to its content; the user bubble measures a
    /// galley directly so it can also paint it.
    fn measure_wrapped_width(ui: &egui::Ui, text: &str, cap: f32) -> f32 {
        let galley = ui.fonts_mut(|f| {
            f.layout(
                text.to_owned(),
                egui::FontId::proportional(style::TEXT_BODY),
                egui::Color32::PLACEHOLDER,
                cap,
            )
        });
        galley.size().x.ceil()
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
        let show_dots = model.streaming.tool_progress.is_none() && model.active_tools.is_empty();
        if !model.streaming.partial_answer.is_empty() {
            let markdown = crate::ui::markdown::harden_soft_breaks(&model.streaming.partial_answer);
            // One bubble from the first token through the rest of the turn:
            // the thinking beat sits under the text instead of a second bubble
            // popping in below it.
            let inner_w =
                Self::measure_wrapped_width(ui, &markdown, Self::bubble_content_cap(ui, 0.0));
            Self::chat_bubble(ui, colors, BubbleSide::Left, false, |ui| {
                ui.set_width(inner_w);
                Self::markdown_body(ui, md_cache, colors, &markdown);
                if show_dots {
                    ui.add_space(style::SPACE_XS);
                    Self::draw_thinking_dots(ui, colors);
                }
            });
        } else if show_dots {
            Self::chat_bubble(ui, colors, BubbleSide::Left, false, |ui| {
                Self::draw_thinking_dots(ui, colors);
            });
        }
        // Never-frozen rule (stint 0467): while a turn is in flight,
        // something on screen always animates. Priority: the tool-generation
        // row when the model is writing a call, the running-tool row (drawn
        // by the caller) while one executes, the thinking dots otherwise.
        if let Some(progress) = &model.streaming.tool_progress {
            Self::draw_tool_progress_row(ui, colors, progress);
        }
        ui.add_space(style::SPACE_MD);
    }

    /// The model is writing a tool call — the longest otherwise-silent
    /// stretch of an app-build turn. Spinner + tool name + cumulative
    /// argument size + elapsed, so a 60s code generation reads as progress.
    fn draw_tool_progress_row(
        ui: &mut egui::Ui,
        colors: &Colors,
        progress: &super::model::ToolArgProgress,
    ) {
        let elapsed = progress.started.elapsed();
        let frame = SPINNER_FRAMES[(elapsed.as_millis() / 100) as usize % SPINNER_FRAMES.len()];
        let chars = if progress.arg_chars >= 1000 {
            format!("{:.1}k chars", progress.arg_chars as f64 / 1000.0)
        } else {
            format!("{} chars", progress.arg_chars)
        };
        let elapsed_label = if elapsed.as_secs() >= 2 {
            format!(" · {}s", elapsed.as_secs())
        } else {
            String::new()
        };
        let name = progress.name.as_deref().unwrap_or("tool call");
        let line = format!("{frame} writing {name} · {chars}{elapsed_label}");
        ui.scope(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.label(
                RichText::new(line)
                    .size(style::TEXT_CAPTION)
                    .monospace()
                    .color(colors.accent),
            );
        });
        ui.add_space(style::SPACE_SM);
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
        let content_h = menu_block_height(ui, matches.len());
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
                    .stroke(egui::Stroke::new(1.0_f32, colors.border))
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
        let content_h = menu_block_height(ui, row_count);
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
                    .stroke(egui::Stroke::new(1.0_f32, colors.border))
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
                                    &model.tier_model_ids,
                                ),
                                AssistantOverlay::PermissionsManager { grants, .. } => {
                                    Self::draw_permissions_rows(ui, colors, selected, grants)
                                }
                                AssistantOverlay::None => {}
                            });
                    });
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_model_picker_rows(
        ui: &mut egui::Ui,
        colors: &Colors,
        selected: usize,
        current_tier: ModelTier,
        active_agent_id: &str,
        tiers: &[ModelTier],
        agents: &[super::model::AgentChoice],
        model_ids: &[Option<String>],
    ) {
        Self::overlay_section_label(ui, colors, "Model tier");
        let labels: Vec<String> = tiers
            .iter()
            .enumerate()
            .map(|(i, tier)| {
                tier_menu_label(*tier, model_ids.get(i).and_then(|id| id.as_deref()))
            })
            .collect();
        for (i, label) in labels.iter().enumerate() {
            let mut row = ListRow::new(label).selected(i == selected);
            if tiers[i] == current_tier {
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
        cap: f32,
        text_h: f32,
    ) -> egui::Rect {
        let font_id = egui::FontId::proportional(style::TEXT_BODY);

        // Accent outline while the composer holds keyboard focus — same
        // affordance as the host text fields.
        let has_kb_focus = ui.memory(|m| m.has_focus(te_id));
        let stroke_color = if has_kb_focus {
            colors.accent
        } else {
            colors.border
        };

        let frame_response = egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(egui::Stroke::new(COMPOSER_STROKE, stroke_color))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(
                style::SPACE_SM as i8,
                style::SPACE_XS as i8,
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Pin the viewport to the height measured before this slot was
                // reserved. A content-sized scroll area lags one frame and
                // shoves the hint bar.
                let response = TextArea::composer(
                    te_id,
                    RichText::new("Message the assistant — / for commands")
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
                )
                .max_height(cap)
                .pin_viewport(text_h)
                .font(font_id)
                .hint_color(colors.text_dim)
                .show(ui, &mut model.composer, colors);
                if response.changed() {
                    model.reset_history_recall();
                }
            });
        frame_response.response.rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::model::{AssistantModel, Turn, TurnRole};
    use std::sync::Arc;

    /// Walk a paint shape tree, collecting every text galley emitted.
    fn collect_galleys(shape: &egui::Shape, out: &mut Vec<Arc<egui::Galley>>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.clone()),
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    collect_galleys(s, out);
                }
            }
            _ => {}
        }
    }

    /// Walk a paint shape tree, collecting every filled rectangle as
    /// `(fill, rect)` — the bubble frame is found by its `bg_active` fill.
    fn collect_rects(shape: &egui::Shape, out: &mut Vec<(egui::Color32, egui::Rect)>) {
        match shape {
            egui::Shape::Rect(r) => out.push((r.fill, r.rect)),
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    collect_rects(s, out);
                }
            }
            _ => {}
        }
    }

    struct Rendered {
        /// The transcript row's content rect at the moment the turn was drawn.
        row_rect: egui::Rect,
        galleys: Vec<Arc<egui::Galley>>,
        rects: Vec<(egui::Color32, egui::Rect)>,
        colors: crate::ui::theme::Colors,
    }

    /// Render a single user turn of `msg` into a `width`-wide pane, collecting
    /// the painted galleys, filled rects, and the row's content rect. Bare
    /// `egui::Context` needs `setup_fonts` or `ListRow`/galley layout panics.
    fn render_user_turn(msg: &str, width: f32) -> Rendered {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = crate::ui::theme::Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );
        let mut md_cache = egui_commonmark::CommonMarkCache::default();
        let mut text_cache = MarkdownTextCache::default();

        let turn = Turn {
            role: TurnRole::User,
            text: msg.to_string(),
            created_at: "2026-07-18T00:00:00Z".to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        };

        let mut raw_input = egui::RawInput::default();
        raw_input.screen_rect = Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(width, 600.0),
        ));

        let mut row_rect = egui::Rect::NOTHING;
        let output = ctx.run_ui(raw_input, |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                row_rect = ui.available_rect_before_wrap();
                AssistantRenderer::draw_turn_row(
                    ui,
                    &mut md_cache,
                    &mut text_cache,
                    "test-conversation",
                    0,
                    &colors,
                    &turn,
                    false,
                    false,
                );
            });
        });

        let mut galleys = Vec::new();
        let mut rects = Vec::new();
        for clipped in &output.shapes {
            collect_galleys(&clipped.shape, &mut galleys);
            collect_rects(&clipped.shape, &mut rects);
        }
        Rendered {
            row_rect,
            galleys,
            rects,
            colors,
        }
    }

    /// The single filled bubble frame — identified by its `bg_active` fill,
    /// which nothing else in an isolated user turn paints.
    fn bubble_rect(r: &Rendered) -> egui::Rect {
        r.rects
            .iter()
            .find(|(fill, _)| *fill == r.colors.bg_active)
            .map(|(_, rect)| *rect)
            .expect("user bubble frame must be painted with the bg_active fill")
    }

    fn selection_scroll_frame(ctx: &egui::Context, input: egui::RawInput) -> f32 {
        let mut offset = 0.0;
        let _ = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                let output = egui::ScrollArea::vertical()
                    .id_salt("assistant_transcript")
                    .auto_shrink([false, false])
                    .scroll_source(egui::scroll_area::ScrollSource {
                        drag: false,
                        ..Default::default()
                    })
                    .show(ui, |ui| {
                        ui.style_mut().interaction.selectable_labels = true;
                        AssistantRenderer::forward_selection_wheel_scroll(ui);
                        for row in 0..100 {
                            ui.label(format!("transcript row {row}"));
                        }
                    });
                offset = output.state.offset.y;
            });
        });
        offset
    }

    fn transcript_input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(240.0, 120.0),
            )),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn transcript_wheel_scrolls_while_text_selection_drag_is_active() {
        let ctx = egui::Context::default();
        let pointer = egui::pos2(20.0, 10.0);
        selection_scroll_frame(&ctx, transcript_input(vec![]));
        selection_scroll_frame(
            &ctx,
            transcript_input(vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]),
        );
        selection_scroll_frame(
            &ctx,
            transcript_input(vec![egui::Event::PointerMoved(egui::pos2(20.0, 32.0))]),
        );
        assert!(
            ctx.dragged_id().is_some(),
            "test setup must start a label-owned selection drag"
        );

        selection_scroll_frame(
            &ctx,
            transcript_input(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -80.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            }]),
        );
        let offset = selection_scroll_frame(&ctx, transcript_input(vec![]));

        assert!(
            offset > 0.0,
            "a wheel event must advance the transcript while a label owns the selection drag"
        );
    }

    /// A wrapped user message renders left-justified: even though the bubble
    /// frame is right-anchored (`Align::Max`), the text galley inside lays out
    /// with `halign == LEFT`, or every wrapped line ragged-lefts against the
    /// bubble's right edge (stints 0435, 0442).
    #[test]
    fn user_bubble_wrapped_text_is_left_justified() {
        // Long enough to wrap several times inside a narrow bubble.
        const MSG: &str = "This is a deliberately long user message that absolutely \
             must wrap across several lines inside the narrow chat bubble so the test \
             can confirm each wrapped line stays left-justified.";
        let r = render_user_turn(MSG, 320.0);
        let bubble = r
            .galleys
            .iter()
            .find(|g| g.text().contains("absolutely"))
            .expect("user message galley must be painted");

        assert!(
            bubble.rows.len() > 1,
            "message must wrap so justification is observable (got {} row)",
            bubble.rows.len()
        );
        assert_eq!(
            bubble.job.halign,
            egui::Align::LEFT,
            "wrapped user-bubble text must be left-justified, not right-anchored"
        );
    }

    /// A short user message shrinks to fit: the bubble frame is far narrower
    /// than the row, and its right edge is pinned to the row's right edge —
    /// the standard right-anchored chat bubble (stint 0442).
    #[test]
    fn short_user_bubble_shrinks_and_right_anchors() {
        let r = render_user_turn("Hi", 320.0);
        let bubble = bubble_rect(&r);

        assert!(
            bubble.width() < r.row_rect.width() * 0.4,
            "short bubble ({:.1}) must be far narrower than the row ({:.1})",
            bubble.width(),
            r.row_rect.width()
        );
        assert!(
            (r.row_rect.right() - bubble.right()).abs() < 2.0,
            "bubble right edge ({:.1}) must sit at the row's right edge ({:.1})",
            bubble.right(),
            r.row_rect.right()
        );
    }

    /// A long user message wraps at the cap: the bubble fills the bubble-max
    /// fraction of the row (never wider), while its text stays left-justified
    /// across the wrapped rows (stint 0442).
    #[test]
    fn long_user_bubble_wraps_at_cap_left_justified() {
        const MSG: &str = "Another long user message engineered to wrap onto several \
             rows so we can confirm the bubble grows to the cap and no further, with \
             every wrapped line still reading from the left edge of the bubble.";
        let r = render_user_turn(MSG, 320.0);
        let bubble = bubble_rect(&r);
        let cap = r.row_rect.width() * AssistantRenderer::BUBBLE_MAX_FRACTION;

        assert!(
            bubble.width() <= cap + 1.0,
            "bubble ({:.1}) must not exceed the cap ({:.1})",
            bubble.width(),
            cap
        );
        assert!(
            bubble.width() > cap * 0.6,
            "a wrapping bubble ({:.1}) should fill most of the cap ({:.1})",
            bubble.width(),
            cap
        );

        let galley = r
            .galleys
            .iter()
            .find(|g| g.text().contains("engineered"))
            .expect("user message galley must be painted");
        assert!(
            galley.rows.len() > 1,
            "message must wrap (got {} row)",
            galley.rows.len()
        );
        assert_eq!(
            galley.job.halign,
            egui::Align::LEFT,
            "wrapped user-bubble text must be left-justified"
        );
    }

    fn paint_assistant(
        ctx: &egui::Context,
        model: &mut AssistantModel,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        let colors = crate::ui::theme::Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );
        let mut md_cache = egui_commonmark::CommonMarkCache::default();
        let mut text_cache = MarkdownTextCache::default();
        let mut raw = egui::RawInput::default();
        raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
        ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                let _ = AssistantRenderer::draw(
                    ui,
                    model,
                    &mut md_cache,
                    &mut text_cache,
                    &colors,
                    7,
                );
            });
        })
    }

    fn text_clips(output: &egui::FullOutput, needle: &str) -> Vec<(egui::Rect, egui::Rect)> {
        fn walk(
            clip: egui::Rect,
            shape: &egui::Shape,
            needle: &str,
            out: &mut Vec<(egui::Rect, egui::Rect)>,
        ) {
            match shape {
                egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                    out.push((clip, rect));
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(clip, shape, needle, out);
                    }
                }
                _ => {}
            }
        }
        let mut found = Vec::new();
        for clipped in &output.shapes {
            walk(clipped.clip_rect, &clipped.shape, needle, &mut found);
        }
        found
    }

    /// Shift+Enter grows the composer upward. The hint label stays on the
    /// same y on that frame and the next, and stays inside its clip.
    #[test]
    fn newline_keeps_the_hint_bar_on_the_pane_floor() {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let mut model = AssistantModel::fresh();
        model.composer = "hello".to_string();
        let size = egui::vec2(480.0, 640.0);
        let settled = paint_assistant(&ctx, &mut model, size);
        model.composer = "hello\nworld".to_string();
        let grown = paint_assistant(&ctx, &mut model, size);
        let again = paint_assistant(&ctx, &mut model, size);

        let hint = |output: &egui::FullOutput| {
            let hits = text_clips(output, "newline");
            assert!(!hits.is_empty(), "hint bar must paint its newline label");
            let (clip, rect) = hits[0];
            assert!(
                clip.contains_rect(rect.shrink(0.5)),
                "newline hint {rect:?} must sit inside clip {clip:?}"
            );
            (clip, rect.bottom())
        };
        let (settled_clip, settled_bottom) = hint(&settled);
        let (grown_clip, grown_bottom) = hint(&grown);
        let (again_clip, again_bottom) = hint(&again);
        assert!(
            (settled_bottom - grown_bottom).abs() < 1.0,
            "hint bar moved when the composer grew ({settled_bottom} -> {grown_bottom})"
        );
        assert!(
            (grown_bottom - again_bottom).abs() < 1.0,
            "hint bar moved between the newline frame ({grown_bottom}) and the next ({again_bottom})"
        );
        assert_eq!(
            settled_clip, grown_clip,
            "hint clip changed on the newline frame"
        );
        assert_eq!(grown_clip, again_clip, "hint clip changed after settling");
        assert!(
            size.y - grown_bottom < 64.0,
            "hint bar bottom {grown_bottom} should stay in the footer, pane is {}",
            size.y
        );
    }

    #[test]
    fn send_and_stream_stick_to_bottom_until_the_reader_scrolls_up() {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let mut model = AssistantModel::fresh();
        model.turns = (0..40)
            .map(|i| {
                let role = if i % 2 == 0 {
                    TurnRole::User
                } else {
                    TurnRole::Assistant
                };
                Turn {
                    role,
                    text: format!("message {i} {}", "line ".repeat(8)),
                    created_at: format!("2026-07-18T00:00:{i:02}Z"),
                    status: None,
                    thoughts: None,
                    detail: None,
                    input_summary: None,
                    output_preview: None,
                }
            })
            .collect();
        let size = egui::vec2(420.0, 360.0);
        // First frame creates the scroll area; the next presents the pinned offset.
        let _ = paint_assistant(&ctx, &mut model, size);
        let _ = paint_assistant(&ctx, &mut model, size);
        let pinned = paint_assistant(&ctx, &mut model, size);
        let (offset, max_offset, follow, at_bottom) = ctx.data(|data| {
            data.get_temp::<(f32, f32, bool, bool)>(egui::Id::new("assistant_scroll_debug"))
                .expect("scroll debug")
        });
        let _ = pinned;
        assert!(follow && at_bottom, "a fresh transcript starts pinned");
        assert!(
            max_offset - offset <= super::SCROLL_BOTTOM_SLACK,
            "offset {offset} should be at the bottom {max_offset}"
        );

        model.streaming.in_flight = true;
        model.streaming.partial_answer = "streaming reply that keeps growing".to_string();
        let _ = paint_assistant(&ctx, &mut model, size);
        let _ = paint_assistant(&ctx, &mut model, size);
        let (_offset, _max, follow, at_bottom) = ctx.data(|data| {
            data.get_temp::<(f32, f32, bool, bool)>(egui::Id::new("assistant_scroll_debug"))
                .unwrap()
        });
        assert!(follow && at_bottom, "streaming stays pinned");

        let mut raw = egui::RawInput::default();
        raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
        raw.events.push(egui::Event::PointerMoved(egui::pos2(200.0, 80.0)));
        // Point deltas under 8px are applied in full this frame. A single
        // notch is smoothed across frames and would not leave the bottom yet.
        for _ in 0..8 {
            raw.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 6.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let colors = crate::ui::theme::Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );
        let mut md_cache = egui_commonmark::CommonMarkCache::default();
        let mut text_cache = MarkdownTextCache::default();
        let _ = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                let _ = AssistantRenderer::draw(
                    ui,
                    &mut model,
                    &mut md_cache,
                    &mut text_cache,
                    &colors,
                    7,
                );
            });
        });
        let (_offset, _max, follow, at_bottom) = ctx.data(|data| {
            data.get_temp::<(f32, f32, bool, bool)>(egui::Id::new("assistant_scroll_debug"))
                .unwrap()
        });
        assert!(!follow, "scrolling up releases the pin");
        assert!(!at_bottom, "the reader is above the latest line");
        let jumped = text_clips(
            &paint_assistant(&ctx, &mut model, size),
            "Latest",
        );
        assert!(!jumped.is_empty(), "a jump-to-latest affordance is shown");
    }

    #[test]
    fn model_picker_row_includes_the_configured_model_id() {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let mut model = AssistantModel::fresh();
        model.set_tier_model_ids(vec![
            Some("qwen/qwen3.6-flash".to_string()),
            Some("xiaomi/mimo-v2.5-pro".to_string()),
            None,
        ]);
        model.open_model_picker(
            crate::protocol::ModelTier::Medium,
            vec![
                crate::protocol::ModelTier::Low,
                crate::protocol::ModelTier::Medium,
                crate::protocol::ModelTier::High,
            ],
            vec![],
        );
        // An `Area` spends its first frame on a sizing pass and paints on the next.
        let _ = paint_assistant(&ctx, &mut model, egui::vec2(480.0, 640.0));
        let output = paint_assistant(&ctx, &mut model, egui::vec2(480.0, 640.0));
        let hits = text_clips(&output, "xiaomi/mimo-v2.5-pro");
        assert!(
            hits.iter().any(|(_, rect)| rect.width() > 0.0),
            "medium tier must show its configured model id"
        );
        assert_eq!(
            super::tier_menu_label(
                crate::protocol::ModelTier::Medium,
                Some("xiaomi/mimo-v2.5-pro")
            ),
            "Medium — xiaomi/mimo-v2.5-pro"
        );
        assert_eq!(
            super::tier_menu_label(crate::protocol::ModelTier::High, None),
            "High"
        );
    }
}
