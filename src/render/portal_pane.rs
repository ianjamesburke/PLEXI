//! Portal (subcontext) tile render path — extracted from
//! `tiling::PlexiBehavior::pane_ui`.
//!
//! The header is the shared pane header (`terminal_pane::render_name_bar_and_tabs`):
//! status pip, centred elided title, status badge, tab bar and escalation wash
//! match terminal and plugin panes. The body is the minimap alone; lineage,
//! description, status summary and counts live in the body's hover tooltip,
//! so no free-running text competes for the tile's width.

use crate::host::context_state::ContextStatus;
use crate::protocol::AgentState;
use crate::render::terminal_pane::{render_name_bar_and_tabs, NameBarParams};
use crate::spatial::tiling::{
    paint_portal_minimap, PaneId, PortalPreview, TabBarAction, TabGroupInfo,
};
use crate::ui::style;
use crate::ui::theme::Colors;
use egui_tiles::TileId;
use std::collections::HashMap;
use std::sync::Once;

static LOG_PORTAL_HEADER: Once = Once::new();

pub(crate) struct PortalRenderParams<'a> {
    pub(crate) tile_id: TileId,
    pub(crate) pane_id: &'a PaneId,
    pub(crate) preview: &'a PortalPreview,
    /// Display name of the pane this portal's context was spawned from.
    pub(crate) lineage_parent: Option<&'a str>,
    pub(crate) tab_info: &'a HashMap<TileId, TabGroupInfo>,
    pub(crate) tab_labels: &'a HashMap<PaneId, String>,
    pub(crate) tab_activities: &'a HashMap<PaneId, (AgentState, bool)>,
    pub(crate) pane_names: &'a HashMap<PaneId, String>,
    pub(crate) colors: &'a Colors,
    pub(crate) name_font_size: f32,
}

/// The header state a context status shows: the same pip, badge and wash a
/// terminal shows for that agent state. Idle and Done carry none.
fn header_state(status: &ContextStatus) -> Option<AgentState> {
    match status {
        ContextStatus::NeedsYou => Some(AgentState::NeedsYou),
        ContextStatus::Blocked => Some(AgentState::Blocked),
        ContextStatus::Error => Some(AgentState::Failed),
        ContextStatus::Working => Some(AgentState::Working),
        ContextStatus::Idle | ContextStatus::Done => None,
    }
}

/// Hover text for the tile body: everything the minimap does not show.
fn body_tooltip(preview: &PortalPreview, lineage_parent: Option<&str>) -> String {
    use crate::ui::text::format_count_plural;
    let mut lines = Vec::new();
    if let Some(parent) = lineage_parent {
        lines.push(format!("Child of {parent}"));
    }
    if !preview.context_description.is_empty() {
        lines.push(preview.context_description.clone());
    }
    if let Some(summary) = &preview.status_summary {
        lines.push(summary.clone());
    }
    let mut counts = vec![format_count_plural(preview.pane_count, "pane", "panes")];
    if preview.window_count > 1 {
        counts.push(format_count_plural(
            preview.window_count,
            "window",
            "windows",
        ));
    }
    if preview.notification_count > 0 {
        counts.push(format_count_plural(
            preview.notification_count,
            "notification",
            "notifications",
        ));
    }
    lines.push(counts.join(" \u{b7} "));
    lines.join("\n")
}

/// Paint one Portal tile into `ui` (its max rect is the pane rect; the caller
/// painted the background and set the clip). Returns a tab-bar action when
/// the Portal shares a tab group.
pub(crate) fn render(
    ui: &mut egui::Ui,
    params: PortalRenderParams<'_>,
) -> Option<(TileId, TabBarAction)> {
    let PortalRenderParams {
        tile_id,
        pane_id,
        preview,
        lineage_parent,
        tab_info,
        tab_labels,
        tab_activities,
        pane_names,
        colors,
        name_font_size,
    } = params;
    let pane_rect = ui.max_rect();
    LOG_PORTAL_HEADER.call_once(|| {
        log::info!(
            "portal_pane: subcontext tiles paint the shared pane header over a minimap-only body"
        );
    });
    let capture = crate::ui::layout_report::Capture::begin(ui.painter(), "portal_tile");
    let state = header_state(&preview.status);
    let tab_action = render_name_bar_and_tabs(
        ui,
        NameBarParams {
            tile_id,
            pane_id,
            tab_info,
            tab_labels,
            tab_activities,
            pane_names,
            pane_agent_state: state.as_ref(),
            is_error: false,
            colors,
            outside_workspace: false,
            name_font_size,
            spawned_by_badge: None,
            waiting_results_badge: None,
            overtake_hint: false,
        },
    );

    let body = ui.available_rect_before_wrap().intersect(pane_rect);
    let map_area = body.shrink(style::SPACE_MD);
    // A tile too short for a legible minimap shows the header alone; the
    // minimap never paints outside its tile.
    if map_area.is_positive() && map_area.height() >= style::PORTAL_MINIMAP_MIN_H {
        let time = ui.input(|i| i.time);
        paint_portal_minimap(ui, map_area, &preview.windows, colors, time);
    }
    if body.is_positive() {
        ui.allocate_rect(body, egui::Sense::hover())
            .on_hover_text(body_tooltip(preview, lineage_parent));
    }

    crate::ui::activity::paint_attention_effect(
        ui.painter(),
        pane_rect,
        colors.state_needs_you,
        egui::Id::new(("portal_attention", *pane_id)),
        matches!(preview.status, ContextStatus::NeedsYou),
        false,
    );
    capture.finish(ui.painter(), pane_rect, colors.bg_darkest, vec![], vec![]);
    tab_action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_tooltip_carries_lineage_description_summary_and_counts() {
        let preview = PortalPreview {
            context_name: "research".to_string(),
            context_description: "~/src/plexi".to_string(),
            pane_count: 3,
            notification_count: 1,
            windows: Vec::new(),
            window_count: 2,
            status: ContextStatus::Working,
            status_summary: Some("2 agents working".to_string()),
        };
        assert_eq!(
            body_tooltip(&preview, Some("lead")),
            "Child of lead\n~/src/plexi\n2 agents working\n3 panes \u{b7} 2 windows \u{b7} 1 notification"
        );
        let bare = PortalPreview::default();
        assert_eq!(body_tooltip(&bare, None), "0 panes");
    }

    #[test]
    fn portal_header_state_matches_terminal_escalation() {
        assert_eq!(
            header_state(&ContextStatus::NeedsYou),
            Some(AgentState::NeedsYou)
        );
        assert_eq!(
            header_state(&ContextStatus::Error),
            Some(AgentState::Failed)
        );
        assert_eq!(header_state(&ContextStatus::Idle), None);
        assert_eq!(header_state(&ContextStatus::Done), None);
    }
}
