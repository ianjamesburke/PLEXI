//! Dive, surface, and the human coordinate of a nested space.
//!
//! Team runs, agent calls, and app opens are the same thing: a pane that
//! contains a space. Cmd+Enter dives. Cmd+Esc and Cmd+Backspace surface one
//! level. Attention navigation lands on the pane and rebuilds the depth stack
//! so the coordinate bar shows the path back out.

use super::PlexiApp;
use crate::agent::fleet::{AgentRow, DisplayState};
use crate::host::space_coord::{
    join_coordinate, resolve_space_title, SpaceFace, SpaceTitleInput, UrgentCounts,
};
use crate::protocol::AgentState;
use std::collections::HashMap;

fn urgent_rank(counts: &UrgentCounts) -> u8 {
    if counts.needs_you > 0 {
        0
    } else if counts.blocked > 0 {
        1
    } else if counts.error > 0 {
        2
    } else if counts.working > 0 {
        3
    } else if counts.done > 0 {
        4
    } else {
        5
    }
}

pub(crate) struct AnchoredSpace {
    pub context_id: u64,
    pub window_index: usize,
}

pub(crate) enum PreparedAppLaunch {
    InPlace,
    Anchored {
        space: AnchoredSpace,
        previous_window: usize,
    },
}

impl PreparedAppLaunch {
    pub(crate) fn is_anchored(&self) -> bool {
        matches!(self, Self::Anchored { .. })
    }
}

impl PlexiApp {
    pub(crate) fn space_title(&self, ctx_id: u64) -> String {
        let found = self
            .router
            .iter()
            .find(|c| c.context_id == ctx_id)
            .map(|c| (c.parent_id.is_none(), c.name.displayed().to_string()));
        let Some((is_root, context_name)) = found else {
            return "space".to_string();
        };
        let team_name = self
            .team_runs
            .iter()
            .find(|run| run.context_id() == ctx_id)
            .map(|run| run.title().to_string());
        let mut agent_names = Vec::new();
        let mut task = None;
        if !is_root {
            for win in self.windows.iter().filter(|w| w.context_id == ctx_id) {
                for pane in win.panes.values() {
                    if let Some(terminal) = pane.as_terminal() {
                        if let Some(name) = terminal.name.clone() {
                            agent_names.push(name);
                        }
                        if task.is_none() {
                            task = terminal
                                .agent
                                .as_ref()
                                .and_then(|agent| agent.detail.clone());
                        }
                    } else if let Some(app) = pane.as_app() {
                        if !app.name.is_empty() {
                            agent_names.push(app.name.clone());
                        }
                    }
                }
            }
        }
        resolve_space_title(SpaceTitleInput {
            is_root,
            context_name: &context_name,
            team_name: team_name.as_deref(),
            agent_names: &agent_names,
            task: task.as_deref(),
        })
    }

    /// Root-first human names for the active space, plus the focused pane.
    pub(crate) fn space_coordinate(&self) -> Vec<String> {
        let active = self.router.active().context_id;
        let mut chain = Vec::new();
        let mut current = Some(active);
        let mut guard = 0;
        while let Some(ctx_id) = current {
            chain.push(self.space_title(ctx_id));
            current = self
                .router
                .iter()
                .find(|c| c.context_id == ctx_id)
                .and_then(|c| c.parent_id);
            guard += 1;
            if guard > 32 {
                log::warn!("space: coordinate chain exceeded 32 at ctx={ctx_id}");
                break;
            }
        }
        chain.reverse();
        let focused = if chain.len() > 1 {
            self.focused_pane_human_name()
        } else {
            None
        };
        crate::host::space_coord::coordinate_segments(&chain, focused.as_deref())
    }

    pub(crate) fn space_faces_for(
        &self,
        strips: &HashMap<u64, Vec<AgentRow>>,
    ) -> HashMap<u64, SpaceFace> {
        let mut faces = HashMap::new();
        for (&parent, rows) in strips {
            let ctx_id = self
                .conjoined_contexts
                .iter()
                .find_map(|(&ctx, &id)| (id == parent).then_some(ctx));
            let title = if let Some(ctx_id) = ctx_id {
                self.space_title(ctx_id)
            } else {
                let names: Vec<String> = rows.iter().map(|row| row.role.clone()).collect();
                let raw = rows
                    .first()
                    .map(|row| row.context_name.as_str())
                    .unwrap_or("");
                resolve_space_title(SpaceTitleInput {
                    is_root: false,
                    context_name: raw,
                    team_name: None,
                    agent_names: &names,
                    task: None,
                })
            };
            let mut counts = UrgentCounts {
                panes: rows.len(),
                ..UrgentCounts::default()
            };
            for row in rows {
                if row.needs_you {
                    counts.needs_you += 1;
                    continue;
                }
                match row.display_state {
                    DisplayState::Blocked => counts.blocked += 1,
                    DisplayState::Failed | DisplayState::Stalled => counts.error += 1,
                    DisplayState::Working => counts.working += 1,
                    DisplayState::DoneUnseen => counts.done += 1,
                    DisplayState::Idle => {}
                }
            }
            faces.insert(parent, SpaceFace::from_counts(title, counts));
        }
        let mut grouped: HashMap<u64, Vec<u64>> = HashMap::new();
        for (&ctx_id, &parent) in &self.conjoined_contexts {
            grouped.entry(parent).or_default().push(ctx_id);
        }
        for (parent, ctxs) in grouped {
            let mut counts = UrgentCounts::default();
            let mut best: Option<(u8, u64, String)> = None;
            for ctx_id in ctxs {
                let part = self.counts_for_context(ctx_id);
                counts.panes += part.panes;
                counts.needs_you += part.needs_you;
                counts.blocked += part.blocked;
                counts.error += part.error;
                counts.working += part.working;
                counts.done += part.done;
                let rank = urgent_rank(&part);
                if best
                    .as_ref()
                    .is_none_or(|(prev, id, _)| rank < *prev || (rank == *prev && ctx_id < *id))
                {
                    best = Some((rank, ctx_id, self.space_title(ctx_id)));
                }
            }
            let title = best
                .map(|(_, _, title)| title)
                .unwrap_or_else(|| "space".into());
            faces.insert(parent, SpaceFace::from_counts(title, counts));
        }
        faces
    }

    /// Non-empty spaces anchored to `parent`, most urgent first.
    pub(crate) fn anchored_spaces(&self, parent: u64) -> Vec<u64> {
        let mut spaces: Vec<u64> = self
            .conjoined_contexts
            .iter()
            .filter_map(|(&ctx, &id)| {
                (id == parent
                    && self
                        .windows
                        .iter()
                        .any(|w| w.context_id == ctx && !w.panes.is_empty()))
                .then_some(ctx)
            })
            .collect();
        spaces.sort_by_key(|ctx| (urgent_rank(&self.counts_for_context(*ctx)), *ctx));
        spaces
    }

    /// Most urgent real pane in a space. Equal urgency keeps the lower pane id
    /// so dive lands on a stable pane.
    pub(crate) fn urgent_pane_in(&self, ctx_id: u64) -> Option<u64> {
        let mut best: Option<(u8, u64)> = None;
        for win in self.windows.iter().filter(|w| w.context_id == ctx_id) {
            for (pane_id, pane) in &win.panes {
                if pane.as_portal().is_some() {
                    continue;
                }
                let rank = match pane.effective_activity() {
                    Some(AgentState::NeedsYou) => 0,
                    Some(AgentState::Blocked) => 1,
                    Some(AgentState::Failed) | Some(AgentState::Interrupted) => 2,
                    Some(AgentState::Working) => 4,
                    _ => 5,
                };
                if best
                    .as_ref()
                    .is_none_or(|(prev, id)| rank < *prev || (rank == *prev && pane_id < id))
                {
                    best = Some((rank, *pane_id));
                }
            }
        }
        best.map(|(_, id)| id)
    }

    /// Fill the frame with `ctx_id` and focus its most urgent pane.
    /// Rebuilds the depth stack from the parent chain so Cmd+Esc / Cmd+Backspace
    /// surface one level at a time.
    pub(crate) fn dive_into_space(&mut self, ctx_id: u64) -> bool {
        if self.router.position(|c| c.context_id == ctx_id).is_none() {
            log::warn!("space: dive missing context={ctx_id}");
            return false;
        }
        if let Some(target) = self.urgent_pane_in(ctx_id) {
            let ok = self.pane_navigate(target);
            if ok {
                log::info!(
                    "space: dive context={ctx_id} pane={target} coordinate={}",
                    join_coordinate(&self.space_coordinate())
                );
            }
            return ok;
        }
        // Empty space: still rebuild the whole ancestor chain so Cmd+Esc
        // surfaces one level at a time from any depth.
        self.sync_space_depth(ctx_id);
        if self.router.active().context_id != ctx_id {
            let Some(idx) = self.router.position(|c| c.context_id == ctx_id) else {
                return false;
            };
            self.switch_workspace(idx);
        }
        log::info!(
            "space: dive context={ctx_id} empty coordinate={}",
            join_coordinate(&self.space_coordinate())
        );
        true
    }

    /// Make the depth stack the ancestor chain of `target_ctx` (root first,
    /// not including the target). Returns true when the stack changed.
    pub(crate) fn sync_space_depth(&mut self, target_ctx: u64) -> bool {
        let mut ancestors = Vec::new();
        let mut current = target_ctx;
        let mut guard = 0;
        while let Some(parent) = self
            .router
            .iter()
            .find(|c| c.context_id == current)
            .and_then(|c| c.parent_id)
        {
            let (win_id, tile) = self
                .windows
                .iter()
                .find(|w| w.context_id == parent)
                .map(|w| (w.window_id, w.focused_pane))
                .unwrap_or((0, None));
            ancestors.push((parent, win_id, tile));
            current = parent;
            guard += 1;
            if guard > 32 {
                log::warn!("space: depth chain exceeded 32 at ctx={current}");
                break;
            }
        }
        ancestors.reverse();
        let same = self.router.depth_stack.len() == ancestors.len()
            && self
                .router
                .depth_stack
                .iter()
                .zip(&ancestors)
                .all(|(have, want)| have.0 == want.0);
        if same {
            return false;
        }
        self.router.depth_stack = ancestors;
        true
    }

    /// Human title for a space opened by `app open`. `name` is the CLI's label
    /// (`git`, `mcp: github`); `fallback` is the app id or path stem.
    pub(crate) fn anchored_launch_title(name: Option<&str>, fallback: &str) -> String {
        let pick = |raw: &str| {
            let raw = raw.trim();
            (!raw.is_empty() && !crate::host::space_coord::is_machine_id(raw))
                .then(|| raw.to_string())
        };
        name.and_then(pick)
            .or_else(|| pick(fallback))
            .unwrap_or_else(|| "space".to_string())
    }

    /// One spawn model. The launching pane contains a new space: a child
    /// context anchored to that pane. The window is empty so the caller can
    /// place the app or agent into it.
    pub(crate) fn open_anchored_space(
        &mut self,
        parent_pane: u64,
        title: &str,
    ) -> Result<AnchoredSpace, String> {
        let (parent_win, _) = self
            .find_pane_in_any_window(parent_pane)
            .ok_or_else(|| format!("launching pane {parent_pane} not found"))?;
        let parent_ctx = self.windows[parent_win].context_id;
        let parent_root = self
            .router
            .position(|c| c.context_id == parent_ctx)
            .map(|idx| self.router.get(idx).root.clone())
            .unwrap_or_else(|| self.windows[parent_win].path.clone());
        let parent_depth = self
            .router
            .position(|c| c.context_id == parent_ctx)
            .map(|idx| self.router.get(idx).depth)
            .unwrap_or(0);
        let title = Self::anchored_launch_title(Some(title), title);

        if let Some(existing) = self.router.position(|c| {
            c.parent_id == Some(parent_ctx) && c.name.displayed().eq_ignore_ascii_case(&title)
        }) {
            let ctx_id = self.router.get(existing).context_id;
            if self.conjoined_contexts.get(&ctx_id) == Some(&parent_pane) {
                if let Some(window_index) = self.windows.iter().position(|w| w.context_id == ctx_id)
                {
                    log::info!(
                        "space: reuse anchor parent={parent_pane} context={ctx_id} title={title}"
                    );
                    return Ok(AnchoredSpace {
                        context_id: ctx_id,
                        window_index,
                    });
                }
            }
        }

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;
        self.conjoined_contexts.insert(ctx_id, parent_pane);
        self.router.push(crate::host::context::Context {
            name: crate::host::context::ContextName::custom(title.clone()),
            root: parent_root.clone(),
            description: None,
            context_id: ctx_id,
            parent_id: Some(parent_ctx),
            depth: parent_depth + 1,
            parked: false,
        });
        self.windows.push(crate::host::context::Window {
            name: title.clone(),
            path: parent_root,
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
        log::info!("space: anchor parent={parent_pane} context={ctx_id} title={title}");
        Ok(AnchoredSpace {
            context_id: ctx_id,
            window_index: self.windows.len() - 1,
        })
    }

    /// Show the anchored space: launcher stays on the left, execution fills
    /// the right, and Cmd+Esc / Cmd+Backspace surface one level.
    pub(crate) fn present_anchored_space(&mut self, ctx_id: u64) {
        self.sync_space_depth(ctx_id);
        if self.router.active().context_id != ctx_id {
            if let Some(idx) = self.router.position(|c| c.context_id == ctx_id) {
                self.switch_workspace(idx);
            }
        }
        self.conjoined_parent_focus = None;
        log::info!(
            "space: present context={ctx_id} coordinate={}",
            join_coordinate(&self.space_coordinate())
        );
    }

    /// Point the next app launch at an anchored space when it came from a pane.
    /// Returns where to restore focus if the launch does not present the space.
    pub(crate) fn prepare_app_launch_window(
        &mut self,
        from_pane: Option<u64>,
        title: &str,
    ) -> PreparedAppLaunch {
        let Some(parent) = from_pane else {
            return PreparedAppLaunch::InPlace;
        };
        if self.find_pane_in_any_window(parent).is_none() {
            log::warn!("space: app open from missing pane={parent}");
            return PreparedAppLaunch::InPlace;
        }
        match self.open_anchored_space(parent, title) {
            Ok(space) => {
                let previous_window = self.active_window;
                self.active_window = space.window_index;
                PreparedAppLaunch::Anchored {
                    space,
                    previous_window,
                }
            }
            Err(error) => {
                log::warn!("space: app open anchor failed parent={parent}: {error}");
                PreparedAppLaunch::InPlace
            }
        }
    }

    pub(crate) fn finish_app_launch(
        &mut self,
        prepared: PreparedAppLaunch,
        launch_ok: bool,
        no_focus: bool,
        pane_name: Option<&str>,
    ) {
        let PreparedAppLaunch::Anchored {
            space,
            previous_window,
        } = prepared
        else {
            return;
        };
        if !launch_ok {
            self.active_window = previous_window;
            self.collapse_subcontext_if_empty(space.context_id);
            log::info!(
                "space: app open failed, collapsed context={}",
                space.context_id
            );
            return;
        }
        if let Some(name) = pane_name.map(str::trim).filter(|name| !name.is_empty()) {
            self.rename_focused_app_in(space.context_id, name);
        }
        if no_focus {
            self.active_window = previous_window;
            log::info!(
                "space: app open anchored context={} without taking focus",
                space.context_id
            );
            return;
        }
        self.present_anchored_space(space.context_id);
    }

    fn rename_focused_app_in(&mut self, ctx_id: u64, name: &str) {
        let Some(win_idx) = self.windows.iter().position(|w| w.context_id == ctx_id) else {
            return;
        };
        let Some(pane_id) = self.windows[win_idx]
            .focused_pane
            .and_then(|tile| Self::find_pane_in_tile(&self.windows[win_idx].tree, tile))
        else {
            return;
        };
        if let Some(app) = self.windows[win_idx]
            .panes
            .get_mut(&pane_id)
            .and_then(|pane| pane.as_app_mut())
        {
            app.name = name.to_string();
            log::info!("space: pane={pane_id} title={name}");
        }
    }

    fn counts_for_context(&self, ctx_id: u64) -> UrgentCounts {
        let mut counts = UrgentCounts::default();
        for win in self.windows.iter().filter(|w| w.context_id == ctx_id) {
            for pane in win.panes.values() {
                if pane.as_portal().is_some() {
                    continue;
                }
                counts.panes += 1;
                match pane.effective_activity() {
                    Some(AgentState::NeedsYou) => counts.needs_you += 1,
                    Some(AgentState::Blocked) => counts.blocked += 1,
                    Some(AgentState::Failed) | Some(AgentState::Interrupted) => counts.error += 1,
                    Some(AgentState::Working) => counts.working += 1,
                    Some(AgentState::Completed) => counts.done += 1,
                    _ => {}
                }
            }
        }
        counts
    }

    fn focused_pane_human_name(&self) -> Option<String> {
        let win = self.windows.get(self.active_window)?;
        let pane_id = Self::find_pane_in_tile(&win.tree, win.focused_pane?)?;
        let pane = win.panes.get(&pane_id)?;
        let name = if let Some(terminal) = pane.as_terminal() {
            terminal.name.clone()
        } else {
            pane.as_app().map(|app| app.name.clone())
        }?;
        let name = name.trim();
        if name.is_empty() || crate::host::space_coord::is_machine_id(name) {
            None
        } else {
            Some(name.to_string())
        }
    }
}
