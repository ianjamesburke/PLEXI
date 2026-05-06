use egui::{Align, Color32, CornerRadius, CursorIcon, Id, Layout, Pos2, Rect, Sense, UiBuilder, Vec2};
use crate::theme::Colors;

pub const ROW_HEIGHT: f32 = 26.0;
pub const ACTION_ZONE_WIDTH: f32 = 30.0;

pub(crate) fn with_alpha(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * alpha) as u8)
}

/// Pre-computed, non-overlapping layout zones for a sidebar row.
/// All rects are fixed at construction time from the row origin — never from content layout.
pub struct RowLayout {
    /// The full row rect: background, border, hover detection.
    pub full: Rect,
    /// The drag/click/context-menu zone (left portion). Stable regardless of hover state.
    pub content: Rect,
    /// The action zone (delete button) — right-anchored, None when the action is disabled.
    /// Geometry is always carved out when Some; glyph is gated on hover in draw().
    pub action: Option<Rect>,
}

impl RowLayout {
    fn new(origin: Pos2, width: f32, action_enabled: bool) -> Self {
        let full = Rect::from_min_size(origin, Vec2::new(width, ROW_HEIGHT));
        let action = action_enabled.then(|| {
            Rect::from_min_size(
                egui::pos2(full.max.x - ACTION_ZONE_WIDTH, full.min.y),
                Vec2::new(ACTION_ZONE_WIDTH, ROW_HEIGHT),
            )
        });
        let content = action.map(|a| full.with_max_x(a.min.x)).unwrap_or(full);
        Self { full, content, action }
    }

    fn in_action(&self, ui: &egui::Ui) -> bool {
        self.action.map_or(false, |r| ui.rect_contains_pointer(r))
    }

    fn hovered(&self, ui: &egui::Ui) -> bool {
        ui.rect_contains_pointer(self.full)
    }

    /// Single cursor authority — zone containment only, never widget responses.
    fn resolve_cursor(&self, ui: &egui::Ui, is_this_dragging: bool, is_any_dragging: bool) -> Option<CursorIcon> {
        if self.in_action(ui) {
            Some(CursorIcon::PointingHand)
        } else if ui.rect_contains_pointer(self.content) || is_this_dragging {
            Some(if is_any_dragging { CursorIcon::Grabbing } else { CursorIcon::Grab })
        } else {
            None
        }
    }
}

/// Single action returned from rendering a sidebar row. Priority is encoded inside draw() —
/// at most one action fires per frame. The caller matches exhaustively.
pub enum SidebarAction {
    None,
    Activate,
    Rename,
    Delete,
    DragStart,
    DragEnd,
}

/// Builder that snapshots the cursor origin at construction and computes all zones
/// before any rendering or interaction registration occurs.
pub struct SidebarRow {
    pub layout: RowLayout,
    is_active: bool,
    is_this_dragging: bool,
    is_any_dragging: bool,
}

impl SidebarRow {
    /// Compute layout zones from the current cursor position.
    /// `action_enabled`: whether to carve out the action zone.
    /// Pass `false` when only one context exists or dragging is active.
    /// The action glyph is gated on hover inside draw() — geometry is stable.
    pub fn new(ui: &egui::Ui, width: f32, action_enabled: bool) -> Self {
        let origin = ui.cursor().min;
        Self {
            layout: RowLayout::new(origin, width, action_enabled),
            is_active: false,
            is_this_dragging: false,
            is_any_dragging: false,
        }
    }

    pub fn active(mut self, v: bool) -> Self { self.is_active = v; self }

    pub fn dragging(mut self, this_row: bool, any_row: bool) -> Self {
        self.is_this_dragging = this_row;
        self.is_any_dragging = any_row;
        self
    }

    /// Render the row and return a typed action plus the full-row response.
    ///
    /// `content_fn` receives a `&mut Ui` scoped to the content zone only.
    /// It must not call `interact()` or set cursor icons — the row builder owns those.
    ///
    /// The returned `Response` covers the full row and is intended for context_menu attachment.
    pub fn draw(
        self,
        ui: &mut egui::Ui,
        id: Id,
        colors: &Colors,
        content_fn: impl FnOnce(&mut egui::Ui, bool),
    ) -> (SidebarAction, egui::Response) {
        let row_alpha = if self.is_this_dragging { 0.4_f32 } else { 1.0_f32 };
        let hovered = self.layout.hovered(ui);

        // Advance the layout cursor — must happen before any allocate_new_ui calls
        // that would otherwise try to use the same origin.
        ui.allocate_space(Vec2::new(self.layout.full.width(), ROW_HEIGHT));

        // Background
        let fill = if self.is_active {
            with_alpha(colors.bg_active, row_alpha)
        } else if hovered && !self.is_this_dragging {
            with_alpha(colors.bg_hover, row_alpha)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(self.layout.full, CornerRadius::ZERO, fill);

        // Active accent bar
        if self.is_active {
            ui.painter().rect_filled(
                Rect::from_min_size(self.layout.full.min, Vec2::new(3.0, ROW_HEIGHT)),
                CornerRadius::ZERO,
                with_alpha(colors.accent, row_alpha),
            );
        }

        // Content zone — sub-Ui is restricted to the content rect
        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(self.layout.content)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| content_fn(ui, hovered),
        );

        // Action zone — glyph only (no interact call). Click detection is via pointer-position
        // geometry against the single full-row interact registered below.
        let in_action = self.layout.in_action(ui);
        if let Some(az) = self.layout.action {
            if hovered && !self.is_this_dragging {
                let glyph_color = with_alpha(
                    if in_action { colors.text_primary } else { colors.text_dim },
                    row_alpha,
                );
                ui.painter().text(
                    az.center(),
                    egui::Align2::CENTER_CENTER,
                    "\u{2715}",
                    egui::FontId::proportional(13.0),
                    glyph_color,
                );
            }
        }

        // Single interact on the full row. Priority chain encodes mutual exclusion:
        // 1. double_clicked → Rename (full row is the target)
        // 2. drag_started   → DragStart
        // 3. drag_stopped   → DragEnd
        // 4. clicked + in_action + hovered → Delete
        // 5. clicked        → Activate
        let response = ui.interact(self.layout.full, id, Sense::click_and_drag());

        // Tooltip for the action zone — shown via the full-row response when pointer is in zone.
        if in_action {
            response.clone().on_hover_text("Delete context");
        }

        // Cursor — single authority, derived from zones only
        if let Some(icon) = self.layout.resolve_cursor(ui, self.is_this_dragging, self.is_any_dragging) {
            ui.ctx().set_cursor_icon(icon);
        }

        let action = if response.double_clicked() {
            SidebarAction::Rename
        } else if response.drag_started() {
            SidebarAction::DragStart
        } else if response.drag_stopped() {
            SidebarAction::DragEnd
        } else if response.clicked() && in_action && hovered {
            SidebarAction::Delete
        } else if response.clicked() {
            SidebarAction::Activate
        } else {
            SidebarAction::None
        };

        (action, response)
    }
}
