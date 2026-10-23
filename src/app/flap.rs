//! One sub-context presentation: a flap that unfolds over the larger side of
//! the frame, away from the pane that opened it.
//!
//! Team runs, agent spawns, and pane-created sub-contexts all open this flap.
//! The agent's own pane is the front face. Helpers and teammates share the
//! remainder. The header carries a human name, state counts, and a breadcrumb
//! back to the parent.

use super::PlexiApp;
use crate::host::context_state::{ContextState, PaneSummaryStatus};
use crate::host::keys::Direction;
use crate::ui::style;
use std::collections::HashMap;

/// Which edge the flap is hinged to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlapSide {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FlapPlacement {
    pub side: FlapSide,
    pub rect: egui::Rect,
}

/// The open flap. `focused_pane` is set once the user dives in or attention
/// lands inside; until then the spawning pane keeps the keyboard.
#[derive(Clone, Debug)]
pub(crate) struct OpenFlap {
    pub context_id: u64,
    pub source_pane: Option<u64>,
    pub front_pane: u64,
    pub focused_pane: Option<u64>,
    pub display_name: Option<String>,
}

pub(crate) struct SubcontextTitle<'a> {
    pub context_name: &'a str,
    pub display_name: Option<&'a str>,
    pub front_name: Option<&'a str>,
    pub task: Option<&'a str>,
}

/// A generated context key (`team-<hex>`, a uuid, `agent-tools-<n>`) is not a
/// name a person chose.
pub(crate) fn is_generated_context_name(name: &str) -> bool {
    if name.starts_with("agent-tools-") {
        return true;
    }
    if let Some(rest) = name.strip_prefix("team-") {
        return rest.len() >= 8 && rest.chars().all(|c| c.is_ascii_hexdigit());
    }
    let hex_or_dash = !name.is_empty() && name.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    let hex_count = name.chars().filter(|c| c.is_ascii_hexdigit()).count();
    hex_or_dash && hex_count >= 16
}

pub(crate) fn task_derived_name(task: &str) -> Option<String> {
    let line = task.lines().find(|line| !line.trim().is_empty())?;
    let sentence = line.split(['.', '!', '?']).next().unwrap_or(line).trim();
    let words: Vec<&str> = sentence.split_whitespace().take(6).collect();
    if words.is_empty() {
        return None;
    }
    let mut name = words.join(" ");
    if name.chars().count() > 48 {
        name = name.chars().take(48).collect();
    }
    Some(name)
}

/// Human header for a sub-context. Never a raw id.
pub(crate) fn subcontext_title(input: SubcontextTitle<'_>) -> String {
    if let Some(name) = input.display_name.map(str::trim).filter(|n| !n.is_empty()) {
        return name.to_string();
    }
    let generated = is_generated_context_name(input.context_name);
    if !generated {
        let name = input.context_name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    if let Some(name) = input.front_name.map(str::trim).filter(|n| !n.is_empty()) {
        if !is_generated_context_name(name) {
            return name.to_string();
        }
    }
    if let Some(name) = input.task.and_then(task_derived_name) {
        return name;
    }
    "Sub-context".to_string()
}

/// "2 working · 1 needs you". Empty when nothing is in flight.
pub(crate) fn flap_state_counts(working: u32, blocked: u32, needs_you: u32, errors: u32) -> String {
    let mut parts = Vec::new();
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if blocked > 0 {
        parts.push(format!("{blocked} blocked"));
    }
    if needs_you > 0 {
        parts.push(format!("{needs_you} needs you"));
    }
    if errors > 0 {
        parts.push(crate::ui::text::format_count_plural(
            errors as usize,
            "error",
            "errors",
        ));
    }
    parts.join(" \u{b7} ")
}

fn tally(state: &ContextState, status: PaneSummaryStatus) -> u32 {
    let own = state
        .pane_summaries
        .iter()
        .filter(|pane| pane.status == status)
        .count() as u32;
    own + state
        .children
        .iter()
        .map(|child| tally(child, status.clone()))
        .sum::<u32>()
}

/// The flap covers the side of `frame` away from `source`. A pane on the right
/// unfolds to the left. That away-side is the larger free region; when the
/// source sits in the middle the flap still takes at least half the frame
/// without covering the source center.
pub(crate) fn flap_placement(frame: egui::Rect, source: Option<egui::Rect>) -> FlapPlacement {
    let gap = style::SPACE_SM;
    if let Some(source) = source.filter(|rect| {
        rect.width() > 1.0 && rect.height() > 1.0 && frame.intersects(*rect)
    }) {
        let left_free = (source.left() - frame.left()).max(0.0);
        let right_free = (frame.right() - source.right()).max(0.0);
        let source_on_right = source.center().x >= frame.center().x;
        let side = if source_on_right {
            FlapSide::Left
        } else {
            FlapSide::Right
        };
        let free = if source_on_right { left_free } else { right_free };
        let min_span = frame.width() * 0.5;
        let rect = if source_on_right {
            let edge = if free >= min_span {
                source.left() - gap
            } else {
                (frame.left() + min_span).min(source.center().x - gap)
            };
            egui::Rect::from_min_max(frame.min, egui::pos2(edge.max(frame.left()), frame.bottom()))
        } else {
            let edge = if free >= min_span {
                source.right() + gap
            } else {
                (frame.right() - min_span).max(source.center().x + gap)
            };
            egui::Rect::from_min_max(egui::pos2(edge.min(frame.right()), frame.top()), frame.max)
        };
        if rect.width() >= style::SPACE_XL {
            return FlapPlacement { side, rect };
        }
    }
    let width = (frame.width() * style::FLAP_DEFAULT_SPAN).clamp(0.0, frame.width());
    FlapPlacement {
        side: FlapSide::Left,
        rect: egui::Rect::from_min_size(frame.min, egui::vec2(width, frame.height())),
    }
}

/// Front face on the leading side, helpers stacked in the remainder.
pub(crate) fn layout_flap_body(body: egui::Rect, front: u64, panes: &[u64]) -> Vec<(u64, egui::Rect)> {
    let mut ordered = Vec::new();
    if panes.contains(&front) {
        ordered.push(front);
    }
    let mut rest: Vec<u64> = panes.iter().copied().filter(|id| *id != front).collect();
    rest.sort_unstable();
    ordered.extend(rest);
    if ordered.is_empty() || body.width() <= 0.0 || body.height() <= 0.0 {
        return Vec::new();
    }
    if ordered.len() == 1 {
        return vec![(ordered[0], body)];
    }
    let gap = style::SPACE_SM;
    let front_w = ((body.width() - gap) * style::FLAP_FRONT_FRACTION)
        .clamp(style::SPACE_XL, (body.width() - gap - style::SPACE_XL).max(style::SPACE_XL));
    let front_rect = egui::Rect::from_min_size(body.min, egui::vec2(front_w, body.height()));
    let rest_rect = egui::Rect::from_min_max(
        egui::pos2(front_rect.right() + gap, body.top()),
        body.max,
    );
    let mut out = vec![(ordered[0], front_rect)];
    let n = (ordered.len() - 1) as f32;
    let h = ((rest_rect.height() - gap * (n - 1.0)) / n).max(1.0);
    for (i, id) in ordered.iter().copied().skip(1).enumerate() {
        let y = rest_rect.top() + i as f32 * (h + gap);
        out.push((
            id,
            egui::Rect::from_min_size(egui::pos2(rest_rect.left(), y), egui::vec2(rest_rect.width(), h)),
        ));
    }
    out
}

fn detach_tile(window: &mut crate::host::context::Window, tile_id: egui_tiles::TileId) {
    if let Some(parent_id) = window.tree.tiles.parent_of(tile_id) {
        if let Some(egui_tiles::Tile::Container(parent)) = window.tree.tiles.get_mut(parent_id) {
            parent.remove_child(tile_id);
        }
    }
    window.tree.tiles.remove(tile_id);
    if window.tree.root == Some(tile_id) {
        window.tree.root = None;
    }
    window.tree.simplify(&egui_tiles::SimplificationOptions {
        all_panes_must_have_tabs: true,
        ..egui_tiles::SimplificationOptions::default()
    });
    if window.focused_pane == Some(tile_id) {
        window.focused_pane = window
            .tree
            .root
            .and_then(|root| window.find_first_pane_in(root));
    }
}

fn animated_rect(full: egui::Rect, side: FlapSide, progress: f32) -> egui::Rect {
    let progress = progress.clamp(0.0, 1.0);
    match side {
        FlapSide::Left => egui::Rect::from_min_max(
            full.min,
            egui::pos2(full.left() + full.width() * progress, full.bottom()),
        ),
        FlapSide::Right => egui::Rect::from_min_max(
            egui::pos2(full.right() - full.width() * progress, full.top()),
            full.max,
        ),
    }
}

impl PlexiApp {
    pub(crate) fn flap_keyboard_pane(&self) -> Option<u64> {
        let flap = self.open_flap.as_ref()?;
        let pane = flap.focused_pane?;
        let active_is_flap = self
            .windows
            .get(self.active_window)
            .is_some_and(|window| window.context_id == flap.context_id);
        if active_is_flap {
            return None;
        }
        self.find_pane_in_any_window(pane).map(|_| pane)
    }

    pub(crate) fn front_pane_of(&self, context_id: u64) -> Option<u64> {
        if let Some(flap) = &self.open_flap {
            if flap.context_id == context_id {
                return Some(flap.front_pane);
            }
        }
        self.windows
            .iter()
            .filter(|window| window.context_id == context_id)
            .flat_map(|window| window.panes.iter())
            .filter(|(_, pane)| pane.portal_target().is_none())
            .map(|(id, _)| *id)
            .min()
    }

    fn pane_human_name(&self, pane_id: u64) -> Option<String> {
        self.windows.iter().find_map(|window| {
            let pane = window.panes.get(&pane_id)?;
            match pane {
                crate::host::pane::Pane::Terminal(terminal) => terminal
                    .name
                    .clone()
                    .or_else(|| terminal.agent_role.clone()),
                crate::host::pane::Pane::App(app) => Some(app.name.clone()),
                crate::host::pane::Pane::Portal(_) => None,
            }
        })
    }

    fn pane_task(&self, pane_id: u64) -> Option<String> {
        self.windows.iter().find_map(|window| {
            window
                .panes
                .get(&pane_id)?
                .as_terminal()?
                .agent
                .as_ref()?
                .detail
                .clone()
        })
    }

    fn lookup_agent_display_name(&self, pane_id: u64) -> Option<String> {
        let (role, root) = self.windows.iter().find_map(|window| {
            let terminal = window.panes.get(&pane_id)?.as_terminal()?;
            let role = terminal.agent_role.clone()?;
            let root = self
                .router
                .iter()
                .find(|context| context.context_id == window.context_id)
                .map(|context| context.root.clone())?;
            Some((role, root))
        })?;
        let registry = crate::agent::AgentRegistry::load(&crate::config::config_dir(), &root);
        let name = registry.active(&role)?.display_name.trim().to_string();
        if name.is_empty() { None } else { Some(name) }
    }

    pub(crate) fn subcontext_title(&self, context_id: u64) -> String {
        let context_name = self
            .router
            .iter()
            .find(|context| context.context_id == context_id)
            .map(|context| context.name.to_string())
            .unwrap_or_default();
        let front = self.front_pane_of(context_id);
        // Registry lookup happens once, in `unfold_flap`, and is cached here.
        // The portal card repaints every frame and must not rescan agent files.
        let display_name = self
            .open_flap
            .as_ref()
            .filter(|flap| flap.context_id == context_id)
            .and_then(|flap| flap.display_name.clone());
        let front_name = front.and_then(|pane| self.pane_human_name(pane));
        let task = front.and_then(|pane| self.pane_task(pane));
        subcontext_title(SubcontextTitle {
            context_name: &context_name,
            display_name: display_name.as_deref(),
            front_name: front_name.as_deref(),
            task: task.as_deref(),
        })
    }

    pub(crate) fn flap_counts(&self, context_id: u64) -> String {
        let state = ContextState::compute(context_id, self.router.as_slice(), &self.windows);
        flap_state_counts(
            tally(&state, PaneSummaryStatus::Active),
            tally(&state, PaneSummaryStatus::Blocked),
            tally(&state, PaneSummaryStatus::NeedsYou),
            tally(&state, PaneSummaryStatus::Error),
        )
    }

    pub(crate) fn breadcrumb_label(&self, context_id: u64) -> String {
        if let Some(source) = self
            .open_flap
            .as_ref()
            .filter(|flap| flap.context_id == context_id)
            .and_then(|flap| flap.source_pane)
            .and_then(|pane| self.pane_human_name(pane))
        {
            if !is_generated_context_name(&source) {
                return source;
            }
        }
        self.router
            .iter()
            .find(|context| context.context_id == context_id)
            .and_then(|context| context.parent_id)
            .and_then(|parent| {
                self.router
                    .iter()
                    .find(|context| context.context_id == parent)
                    .map(|context| context.name.to_string())
            })
            .filter(|name| !name.is_empty() && !is_generated_context_name(name))
            .unwrap_or_else(|| "Parent".to_string())
    }

    /// Open (or refresh) the flap for `context_id`. The first front face sticks;
    /// later panes in the same sub-context are helpers.
    pub(crate) fn unfold_flap(
        &mut self,
        context_id: u64,
        source: Option<u64>,
        front: u64,
        focus: bool,
    ) {
        if self.router.iter().all(|context| context.context_id != context_id) {
            log::warn!("flap: unfold ignored, context {context_id} is gone");
            return;
        }
        let display_name = self.lookup_agent_display_name(front);
        if let Some(flap) = &mut self.open_flap {
            if flap.context_id == context_id {
                if flap.source_pane.is_none() {
                    flap.source_pane = source;
                }
                if flap.display_name.is_none() {
                    flap.display_name = display_name;
                }
                if focus {
                    flap.focused_pane = Some(front);
                }
                log::info!(
                    "flap: refresh context={context_id} front={} focus={:?}",
                    flap.front_pane,
                    flap.focused_pane
                );
                self.ctx.request_repaint();
                return;
            }
        }
        self.open_flap = Some(OpenFlap {
            context_id,
            source_pane: source,
            front_pane: front,
            focused_pane: focus.then_some(front),
            display_name,
        });
        log::info!(
            "flap: unfold context={context_id} front={front} source={source:?} focus={focus} title={}",
            self.subcontext_title(context_id)
        );
        self.ctx.request_repaint();
    }

    /// `app open` from a pane joins the same sub-context flap as an agent or
    /// team spawn. The launching pane stays where it is. The new pane moves
    /// into a child context named for the app, and the flap unfolds away from
    /// the launcher — a left-hand launcher keeps the commands on the left and
    /// the execution on the right.
    pub(crate) fn adopt_opened_pane_into_flap(
        &mut self,
        source: u64,
        pane_id: u64,
        explicit_name: Option<&str>,
    ) {
        if pane_id == source {
            return;
        }
        let Some((src_win, src_tile)) = self.find_pane_in_any_window(source) else {
            log::warn!("flap: app-open source pane {source} is gone");
            return;
        };
        let source_ctx = self.windows[src_win].context_id;
        let Some((pane_win, pane_tile)) = self.find_pane_in_any_window(pane_id) else {
            log::warn!("flap: app-open pane {pane_id} is gone");
            return;
        };
        if self.windows[pane_win]
            .panes
            .get(&pane_id)
            .and_then(|pane| pane.as_app())
            .is_some_and(|app| app.overlay_replaced.is_some())
        {
            log::info!("flap: app-open pane {pane_id} is an overlay; leaving it on its tile");
            return;
        }
        if let Some(name) = explicit_name.map(str::trim).filter(|name| !name.is_empty()) {
            if let Some(pane) = self.windows[pane_win].panes.get_mut(&pane_id) {
                if let Some(app) = pane.as_app_mut() {
                    app.name = name.to_string();
                } else if let Some(terminal) = pane.as_terminal_mut() {
                    terminal.name = Some(name.to_string());
                    terminal.name_locked = true;
                }
            }
        }
        let title = self
            .pane_human_name(pane_id)
            .filter(|name| !is_generated_context_name(name))
            .filter(|name| !matches!(name.as_str(), "cli-renderer" | "mcp-renderer"))
            .unwrap_or_else(|| "App".to_string());
        let pane_ctx = self.windows[pane_win].context_id;
        if self.router.iter().any(|context| {
            context.context_id == pane_ctx && context.parent_id == Some(source_ctx)
        }) {
            self.unfold_flap(pane_ctx, Some(source), pane_id, false);
            self.windows[src_win].focused_pane = Some(src_tile);
            log::info!("flap: app-open source={source} pane={pane_id} context={pane_ctx} title={title}");
            return;
        }
        let existing = self.router.iter().find_map(|context| {
            (context.parent_id == Some(source_ctx)
                && context.name.displayed().eq_ignore_ascii_case(&title))
            .then_some(context.context_id)
        });
        let ctx_id = existing.unwrap_or_else(|| self.create_flap_context(source_ctx, &title));
        let pane = match self.windows[pane_win].panes.remove(&pane_id) {
            Some(pane) => pane,
            None => return,
        };
        if existing.is_none() && pane_win == src_win {
            let portal_id = self.host.alloc_pane_id();
            if let Some(egui_tiles::Tile::Pane(slot)) =
                self.windows[pane_win].tree.tiles.get_mut(pane_tile)
            {
                *slot = portal_id;
            }
            self.windows[pane_win].panes.insert(
                portal_id,
                crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
                    pane_id: portal_id,
                    target_context_id: ctx_id,
                    context_state: None,
                    hidden: false,
                })),
            );
        } else {
            if existing.is_none() {
                let portal_id = self.host.alloc_pane_id();
                crate::pane_ops::insert_split_tile(
                    &mut self.windows[src_win].tree,
                    Some(src_tile),
                    portal_id,
                    true,
                    crate::host::command::ShareRatio::new(1.0, 1.0).expect("1:1"),
                    false,
                );
                self.windows[src_win].panes.insert(
                    portal_id,
                    crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
                        pane_id: portal_id,
                        target_context_id: ctx_id,
                        context_state: None,
                        hidden: false,
                    })),
                );
            }
            detach_tile(&mut self.windows[pane_win], pane_tile);
        }
        let Some(child_win) = self.windows.iter().position(|window| window.context_id == ctx_id)
        else {
            log::warn!("flap: app-open context {ctx_id} has no window");
            return;
        };
        self.windows[child_win].panes.insert(pane_id, pane);
        let split_target = self.windows[child_win].focused_pane;
        let tile = crate::pane_ops::insert_split_tile(
            &mut self.windows[child_win].tree,
            split_target,
            pane_id,
            true,
            crate::host::command::ShareRatio::new(1.0, 1.0).expect("1:1"),
            false,
        );
        if self.windows[child_win].focused_pane.is_none() {
            self.windows[child_win].focused_pane = Some(tile);
        }
        if let Some((win, tile)) = self.find_pane_in_any_window(source) {
            self.windows[win].focused_pane = Some(tile);
        }
        self.unfold_flap(ctx_id, Some(source), pane_id, false);
        self.refresh_portal_context_states();
        self.mark_workspace_dirty();
        log::info!("flap: app-open source={source} pane={pane_id} context={ctx_id} title={title}");
    }

    fn create_flap_context(&mut self, parent_ctx: u64, title: &str) -> u64 {
        let parent_depth = self
            .router
            .iter()
            .find(|context| context.context_id == parent_ctx)
            .map(|context| context.depth)
            .unwrap_or(0);
        let root = self
            .router
            .iter()
            .find(|context| context.context_id == parent_ctx)
            .map(|context| context.root.clone())
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;
        self.router.push(crate::host::context::Context {
            name: crate::host::context::ContextName::custom(title.to_string()),
            root: root.clone(),
            description: None,
            context_id: ctx_id,
            parent_id: Some(parent_ctx),
            depth: parent_depth + 1,
            parked: false,
        });
        self.windows.push(crate::host::context::Window {
            name: String::new(),
            path: root,
            tree: egui_tiles::Tree::empty("plexi"),
            panes: std::collections::HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        self.context_active_window.insert(ctx_id, win_id);
        ctx_id
    }

    pub(crate) fn fold_flap(&mut self) {
        if let Some(flap) = self.open_flap.take() {
            log::info!("flap: fold context={}", flap.context_id);
        }
        self.flap_pane_rects.clear();
        self.flap_painted_rect = None;
        self.ctx.request_repaint();
    }

    /// Esc / Cmd+Escape. An overlay folds in place. A zoomed-in sub-context
    /// also returns to its parent.
    pub(crate) fn fold_flap_from_key(&mut self) {
        let inside = self.open_flap.as_ref().is_some_and(|flap| {
            self.windows
                .get(self.active_window)
                .is_some_and(|window| window.context_id == flap.context_id)
        });
        let had = self.open_flap.is_some();
        if had {
            self.fold_flap();
        }
        if !had || inside {
            self.zoom_out_of_context();
        }
    }

    pub(crate) fn navigate_flap(&mut self, dir: Direction) -> bool {
        let Some(flap) = self.open_flap.clone() else {
            return false;
        };
        if self
            .windows
            .get(self.active_window)
            .is_some_and(|window| window.context_id == flap.context_id)
        {
            return false;
        }
        let Some(current) = flap.focused_pane else {
            return false;
        };
        let panes = self.ordered_flap_panes(flap.context_id, flap.front_pane);
        let Some(index) = panes.iter().position(|id| *id == current) else {
            return false;
        };
        let next = match dir {
            Direction::Left | Direction::Up => index.checked_sub(1),
            Direction::Right | Direction::Down => (index + 1 < panes.len()).then_some(index + 1),
        };
        let Some(next) = next else {
            return true;
        };
        let pane = panes[next];
        if let Some(flap) = &mut self.open_flap {
            flap.focused_pane = Some(pane);
        }
        log::info!("flap: focus pane={pane}");
        self.ctx.request_repaint();
        true
    }

    fn ordered_flap_panes(&self, context_id: u64, front: u64) -> Vec<u64> {
        let mut panes: Vec<u64> = self
            .windows
            .iter()
            .filter(|window| window.context_id == context_id)
            .flat_map(|window| window.panes.iter())
            .filter(|(_, pane)| pane.portal_target().is_none())
            .map(|(id, _)| *id)
            .collect();
        panes.sort_unstable();
        panes.retain(|id| *id != front);
        let mut ordered = Vec::new();
        if self
            .windows
            .iter()
            .any(|window| window.panes.contains_key(&front))
        {
            ordered.push(front);
        }
        ordered.extend(panes);
        ordered
    }

    fn source_rect(&self, source: Option<u64>) -> Option<egui::Rect> {
        let pane = source?;
        let (window, tile) = self.find_pane_in_any_window(pane)?;
        self.windows[window].tree.tiles.rect(tile)
    }

    pub(crate) fn paint_flap(
        &mut self,
        ui: &mut egui::Ui,
        input: Option<crate::render::terminal_pane::TerminalInput>,
    ) {
        let Some(flap) = self.open_flap.clone() else {
            self.flap_pane_rects.clear();
            self.flap_painted_rect = None;
            return;
        };
        if self
            .router
            .iter()
            .all(|context| context.context_id != flap.context_id)
        {
            self.fold_flap();
            return;
        }
        let inside = self
            .windows
            .get(self.active_window)
            .is_some_and(|window| window.context_id == flap.context_id);
        let frame = ui.max_rect();
        let placement = flap_placement(frame, self.source_rect(flap.source_pane));
        let progress = style::micro_motion(
            ui.ctx(),
            egui::Id::new(("subcontext_flap", flap.context_id)),
            true,
            style::DURATION_MICRO_MS,
        );
        let rect = if inside {
            egui::Rect::from_min_size(
                frame.min,
                egui::vec2(frame.width(), style::TEXT_BODY + style::SPACE_MD * 2.0),
            )
        } else {
            animated_rect(placement.rect, placement.side, progress)
        };
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }
        self.flap_painted_rect = Some(if inside { placement.rect } else { rect });
        let title = self.subcontext_title(flap.context_id);
        let counts = self.flap_counts(flap.context_id);
        let breadcrumb = self.breadcrumb_label(flap.context_id);
        let logged_id = egui::Id::new(("subcontext_flap_logged", flap.context_id));
        if !ui.ctx().data(|data| data.get_temp::<bool>(logged_id).unwrap_or(false)) {
            log::info!(
                "flap: show context={} title={title} side={:?} panes={} opacity={}",
                flap.context_id,
                placement.side,
                self.ordered_flap_panes(flap.context_id, flap.front_pane).len(),
                style::FLAP_OPACITY
            );
            ui.ctx().data_mut(|data| data.insert_temp(logged_id, true));
        }
        let colors = self.colors;
        let mut fold = false;
        let mut input = input;
        if inside {
            self.flap_pane_rects.clear();
        }
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("subcontext-flap", flap.context_id))
                .max_rect(rect),
            |ui| {
                // Focus rings use this painter: the flap surface is 98% opaque,
                // and that multiplier would tint the accent off the ordinary outline.
                // The parent clip also keeps an Outside stroke from being cropped.
                let focus_painter = ui.painter().clone();
                ui.set_clip_rect(rect);
                if !inside {
                    ui.set_opacity(style::FLAP_OPACITY);
                }
                ui.painter().rect_filled(rect, style::RADIUS_LG, colors.bg_elevated);
                ui.painter().rect_stroke(
                    rect,
                    style::RADIUS_LG,
                    style::hairline_border(colors.text_dim),
                    egui::StrokeKind::Inside,
                );
                let header_h = style::TEXT_BODY + style::SPACE_MD * 2.0;
                let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), header_h.min(rect.height())));
                ui.scope_builder(egui::UiBuilder::new().max_rect(header.shrink(style::SPACE_SM)), |ui| {
                    ui.horizontal(|ui| {
                        let back = format!("\u{2039} {breadcrumb}");
                        let response = ui.add(
                            egui::Label::new(
                                egui::RichText::new(back)
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim),
                            )
                            .sense(egui::Sense::click()),
                        );
                        response.clone().on_hover_text("Fold the flap");
                        let parent = breadcrumb.clone();
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!("Back to {parent}"),
                            )
                        });
                        if response.clicked() {
                            fold = true;
                        }
                        ui.add_space(style::SPACE_SM);
                        ui.label(
                            egui::RichText::new(title.clone())
                                .size(style::TEXT_BODY)
                                .strong()
                                .color(colors.text_primary),
                        );
                        if !counts.is_empty() {
                            ui.add_space(style::SPACE_SM);
                            ui.label(
                                egui::RichText::new(counts.clone())
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new("fold")
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                            crate::ui::shortcuts::key_chip(
                                ui,
                                "esc",
                                &colors,
                                egui::FontId::monospace(style::TEXT_CAPTION),
                            );
                        });
                    });
                });
                if inside {
                    return;
                }
                let body = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + style::SPACE_SM, header.bottom()),
                    egui::pos2(rect.right() - style::SPACE_SM, rect.bottom() - style::SPACE_SM),
                );
                if !body.is_positive() {
                    return;
                }
                let panes = self.ordered_flap_panes(flap.context_id, flap.front_pane);
                let layout = layout_flap_body(body, flap.front_pane, &panes);
                self.flap_pane_rects.clear();
                for (pane_id, pane_rect) in layout {
                    self.flap_pane_rects.insert(pane_id, pane_rect);
                }
                let focused = flap.focused_pane;
                let gap = self.config.pane_gap.unwrap_or(4.0).clamp(0.0, 20.0);
                let theme = self.theme.clone();
                let font = self.config.pane_title_font_size.unwrap_or(12.0);
                let root = self
                    .router
                    .iter()
                    .find(|context| context.context_id == flap.context_id)
                    .map(|context| context.root.clone())
                    .unwrap_or_default();
                let rects = self.flap_pane_rects.clone();
                let mut focus_click = None;
                for (pane_id, pane_rect) in rects {
                    let hit = ui.interact(
                        pane_rect,
                        ui.id().with(("flap-pane", pane_id)),
                        egui::Sense::click(),
                    );
                    if hit.clicked() {
                        focus_click = Some(pane_id);
                    }
                    let is_focused = focused == Some(pane_id);
                    let name = self.pane_human_name(pane_id).unwrap_or_default();
                    let mut names = HashMap::new();
                    if !name.is_empty() {
                        names.insert(pane_id, name);
                    }
                    let tile = self
                        .windows
                        .iter()
                        .find_map(|window| window.tree.tiles.find_pane(&pane_id));
                    let Some(tile) = tile else { continue };
                    let window = self
                        .windows
                        .iter_mut()
                        .find(|window| window.panes.contains_key(&pane_id));
                    let Some(window) = window else { continue };
                    let pane = window.panes.get_mut(&pane_id);
                    let Some(pane) = pane else { continue };
                    let pane_input = if is_focused {
                        input.take().unwrap_or_default()
                    } else {
                        Default::default()
                    };
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .id_salt(("flap-pane-body", pane_id))
                            .max_rect(pane_rect),
                        |ui| {
                            ui.set_clip_rect(pane_rect);
                            if let Some(terminal) = pane.as_terminal_mut() {
                                crate::render::terminal_pane::render(
                                    ui,
                                    terminal,
                                    tile,
                                    &pane_id,
                                    is_focused,
                                    &theme,
                                    &colors,
                                    &names,
                                    &HashMap::new(),
                                    &HashMap::new(),
                                    &HashMap::new(),
                                    Some(&root),
                                    font,
                                    pane_input,
                                );
                            } else if let Some(app) = pane.as_app_mut() {
                                crate::render::app_pane::render(
                                    ui,
                                    app,
                                    &colors,
                                    false,
                                    None,
                                    &root,
                                );
                            }
                        },
                    );
                    if is_focused {
                        focus_painter.rect_stroke(
                            pane_rect,
                            0.0,
                            egui::Stroke::new(gap, colors.accent),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
                if let Some(pane) = focus_click {
                    if let Some(flap) = &mut self.open_flap {
                        flap.focused_pane = Some(pane);
                    }
                    log::info!("flap: focus pane={pane}");
                }
            },
        );
        if fold {
            self.fold_flap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 600.0))
    }

    #[test]
    fn right_pane_unfolds_to_the_left_larger_side() {
        let source = egui::Rect::from_min_max(egui::pos2(700.0, 0.0), egui::pos2(1000.0, 600.0));
        let placement = flap_placement(frame(), Some(source));
        assert_eq!(placement.side, FlapSide::Left);
        assert!(placement.rect.right() <= source.left());
        assert!(placement.rect.width() > frame().width() * 0.5);
    }

    #[test]
    fn left_pane_unfolds_to_the_right() {
        let source = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(300.0, 600.0));
        let placement = flap_placement(frame(), Some(source));
        assert_eq!(placement.side, FlapSide::Right);
        assert!(placement.rect.left() >= source.right());
        assert!(placement.rect.width() > frame().width() * 0.5);
    }

    #[test]
    fn missing_source_covers_the_larger_default_span() {
        let placement = flap_placement(frame(), None);
        assert_eq!(placement.side, FlapSide::Left);
        assert!((placement.rect.width() - frame().width() * style::FLAP_DEFAULT_SPAN).abs() < 1.0);
        assert!(placement.rect.width() > frame().width() * 0.5);
    }

    #[test]
    fn titles_use_a_human_name_never_a_raw_id() {
        assert!(is_generated_context_name(
            "team-017f341796ab4976bee42030d6f5a6c0"
        ));
        assert!(is_generated_context_name("agent-tools-42"));
        assert!(!is_generated_context_name("research"));
        let raw = subcontext_title(SubcontextTitle {
            context_name: "team-017f341796ab4976bee42030d6f5a6c0",
            display_name: None,
            front_name: Some("Design Review"),
            task: Some("Pick a direction"),
        });
        assert_eq!(raw, "Design Review");
        assert!(!raw.contains("team-"));
        let named = subcontext_title(SubcontextTitle {
            context_name: "research",
            display_name: None,
            front_name: Some("researcher"),
            task: None,
        });
        assert_eq!(named, "research");
        let display = subcontext_title(SubcontextTitle {
            context_name: "research",
            display_name: Some("Researcher"),
            front_name: Some("researcher"),
            task: None,
        });
        assert_eq!(display, "Researcher");
        let task = subcontext_title(SubcontextTitle {
            context_name: "team-aaaaaaaaaaaaaaaa",
            display_name: None,
            front_name: None,
            task: Some("Review the auth boundary. Then stop."),
        });
        assert_eq!(task, "Review the auth boundary");
    }

    #[test]
    fn state_counts_read_as_a_sentence() {
        assert_eq!(
            flap_state_counts(2, 0, 1, 0),
            "2 working \u{b7} 1 needs you"
        );
        assert_eq!(flap_state_counts(0, 1, 0, 2), "1 blocked \u{b7} 2 errors");
        assert_eq!(flap_state_counts(0, 0, 0, 0), "");
    }

    #[test]
    fn front_face_is_larger_than_each_helper() {
        let body = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0));
        let layout = layout_flap_body(body, 7, &[9, 7, 3]);
        assert_eq!(layout[0].0, 7);
        assert!(layout[0].1.width() > layout[1].1.width());
        assert!(layout[0].1.height() > layout[1].1.height());
        assert_eq!(layout.len(), 3);
    }

    #[test]
    fn attention_opens_the_flap_on_the_blocked_pane() {
        let mut h = crate::testing::HostHarness::new();
        let parent = h.add_focused_terminal();
        let team = "team-017f341796ab4976bee42030d6f5a6c1";
        let (front, ctx) = h
            .app
            .spawn_agent_worker_pane(
                "coordinator",
                "pi",
                "sleep 60",
                "Design the review",
                Some(team),
                false,
                Some(parent),
                None,
                None,
                None,
                false,
                None,
            )
            .expect("spawn coordinator");
        let (helper, helper_ctx) = h
            .app
            .spawn_agent_worker_pane(
                "researcher",
                "pi",
                "sleep 60",
                "Check the sources",
                Some(team),
                false,
                Some(parent),
                None,
                None,
                None,
                false,
                None,
            )
            .expect("spawn researcher");
        assert_eq!(helper_ctx, ctx);
        let parent_ctx = h.app.router.active().context_id;
        assert_ne!(ctx, parent_ctx);
        let flap = h.app.open_flap.as_ref().expect("spawn unfolds");
        assert_eq!(flap.context_id, ctx);
        assert_eq!(flap.front_pane, front);
        assert!(flap.focused_pane.is_none());
        let title = h.app.subcontext_title(ctx);
        assert_eq!(title, "coordinator");
        assert!(!title.contains("team-"));
        let (win, _) = h.app.find_pane_in_any_window(helper).unwrap();
        h.app.windows[win]
            .panes
            .get_mut(&helper)
            .unwrap()
            .set_agent(Some(crate::protocol::PaneAgentState {
                pane_id: helper,
                state: crate::protocol::AgentState::NeedsYou,
                agent: "pi".into(),
                detail: Some("Need a decision".into()),
                session_id: None,
            }));
        assert!(
            h.app.flap_counts(ctx).contains("needs you"),
            "{}",
            h.app.flap_counts(ctx)
        );
        assert_eq!(h.app.next_blocked_agent(), Some(helper));
        assert_eq!(h.app.router.active().context_id, parent_ctx);
        assert_eq!(
            h.app.open_flap.as_ref().and_then(|flap| flap.focused_pane),
            Some(helper)
        );
        h.app.fold_flap();
        assert!(h.app.open_flap.is_none());
    }

    #[test]
    fn app_open_from_a_pane_unfolds_the_same_flap() {
        let mut h = crate::testing::HostHarness::new();
        let source = h.add_focused_terminal();
        let parent_ctx = h.app.router.active().context_id;
        let tmp = tempfile::tempdir().expect("cwd");
        h.app.handle_pane_ipc_request(crate::protocol::AppRequest::SpawnPane {
            type_id: "text-editor".into(),
            layout: Some("split_h".into()),
            args: Vec::new(),
            from_pane_id: Some(source),
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: Some(tmp.path().to_string_lossy().into_owned()),
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            context_name: None,
            name: Some("git".into()),
            agent_cmd: None,
            boot_timeout_secs: None,
            worktree: None,
            result_schema: None,
            peer_ancestry: None,
        });
        let flap = h.app.open_flap.clone().expect("app open unfolds a flap");
        assert_eq!(h.app.subcontext_title(flap.context_id), "git");
        assert!(!h.app.subcontext_title(flap.context_id).contains("cli-renderer"));
        assert_eq!(h.app.router.active().context_id, parent_ctx);
        assert_eq!(flap.source_pane, Some(source));
        assert!(flap.focused_pane.is_none());
        let (app_win, _) = h.app.find_pane_in_any_window(flap.front_pane).unwrap();
        assert_eq!(h.app.windows[app_win].context_id, flap.context_id);
        assert_ne!(h.app.windows[app_win].context_id, parent_ctx);
        assert!(h.app.windows.iter().flat_map(|window| window.panes.values()).any(|pane| {
            pane.portal_target() == Some(flap.context_id)
        }));
        let (src_win, _) = h.app.find_pane_in_any_window(source).unwrap();
        assert_eq!(h.app.windows[src_win].context_id, parent_ctx);
        assert!(h.app.pane_navigate(flap.front_pane));
        assert_eq!(h.app.router.active().context_id, parent_ctx);
        assert_eq!(
            h.app.open_flap.as_ref().and_then(|flap| flap.focused_pane),
            Some(flap.front_pane)
        );
        h.run_frames(2);
        if let (Some(source_rect), Some(painted)) = (
            h.app.visible_pane_rect(source),
            h.app.flap_painted_rect,
        ) {
            assert!(painted.width() > 100.0, "{painted:?}");
            if source_rect.center().x <= painted.center().x {
                assert!(
                    painted.left() + 1.0 >= source_rect.center().x
                        || painted.center().x > source_rect.center().x,
                    "launcher stays left of the execution flap: source={source_rect:?} flap={painted:?}"
                );
            }
        }
    }
}
