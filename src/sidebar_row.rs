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
    /// Geometry is always carved out when Some; glyph + interaction are gated on hover in draw().
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

/// Typed result from rendering a sidebar row. All interaction state flows through here.
pub struct RowResult {
    pub primary_clicked: bool,
    pub primary_double_clicked: bool,
    pub drag_started: bool,
    pub drag_stopped: bool,
    pub action_clicked: bool,
    /// Exposed for context_menu and tooltip attachment only.
    pub drag_response: egui::Response,
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
    /// The action glyph and interaction are gated on hover inside draw() — geometry is stable.
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

    /// Render the row and return typed interaction results.
    ///
    /// `content_fn` receives a `&mut Ui` scoped to the content zone only.
    /// It must not call `interact()` or set cursor icons — the row builder owns those.
    pub fn draw(
        self,
        ui: &mut egui::Ui,
        id: Id,
        colors: &Colors,
        content_fn: impl FnOnce(&mut egui::Ui, bool),
    ) -> RowResult {
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

        // Action zone — glyph and interaction only active when hovered.
        // Geometry is always carved out (content rect is stable), so no hit-rect shifts on hover.
        let action_clicked = if let Some(az) = self.layout.action {
            if hovered && !self.is_this_dragging {
                let in_action = self.layout.in_action(ui);
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
                let resp = ui.interact(az, id.with("action"), Sense::click());
                let clicked = resp.clicked();
                if in_action {
                    resp.on_hover_text("Delete context");
                }
                clicked
            } else {
                false
            }
        } else {
            false
        };

        // Drag / click on content zone — registered after action zone so no overlap
        let drag_response = ui.interact(
            self.layout.content,
            id.with("drag"),
            Sense::click_and_drag(),
        );

        // Full-row double-click — widens the rename hit target to the entire row
        // (content zone excludes the action zone, leaving a 30px gap on the right)
        let full_dblclick = ui
            .interact(self.layout.full, id.with("full_dblclick"), Sense::click())
            .double_clicked();

        // Cursor — single authority, derived from zones only
        if let Some(icon) = self.layout.resolve_cursor(ui, self.is_this_dragging, self.is_any_dragging) {
            ui.ctx().set_cursor_icon(icon);
        }

        RowResult {
            primary_clicked: drag_response.clicked(),
            primary_double_clicked: drag_response.double_clicked() || full_dblclick,
            drag_started: drag_response.drag_started(),
            drag_stopped: drag_response.drag_stopped(),
            action_clicked,
            drag_response,
        }
    }
}
