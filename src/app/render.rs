//! Rendering helpers extracted from the main `eframe::App::update()` loop.

use super::{ClickFlash, FocusLayer, PlexiApp};
use crate::spatial::tiling::{PaneId, PlexiBehavior};
use egui::{Color32, CornerRadius, Stroke, StrokeKind, Vec2};
use egui_tiles::Tile;
use std::collections::HashMap;

const FLASH_DUR: f32 = 0.4;
const FLASH_TAU: f32 = 0.10;

impl PlexiApp {
    /// Early per-frame work that runs before overlay dispatch and panel rendering.
    /// Handles adopted context paths, finder service drains, notification polling,
    /// pane command draining, update channel checks, PTY event draining, and
    /// focus-stack reconciliation.
    pub(super) fn update_preamble(&mut self, ctx: &egui::Context) {
        if let Some(ctx_path) = crate::config::take_adopted_context_path() {
            log::info!("adopted context path: {}", ctx_path.display());
            self.new_context_at_path(ctx_path);
            self.save_workspace();
        }
        #[cfg(target_os = "macos")]
        {
            let finder_paths = crate::platform::finder_service::drain();
            if !finder_paths.is_empty() {
                for path in finder_paths {
                    log::info!("finder_service: opening context for {}", path.display());
                    self.new_context_at_path(path);
                }
                self.save_workspace();
            }
        }
        if self.last_notify_poll.elapsed() >= std::time::Duration::from_secs(1) {
            self.last_notify_poll = std::time::Instant::now();
            self.drain_spawn_queue();
            self.tick_scheduler();
            self.tick_notification_timeouts();
        }
        self.drain_pane_cmd_channel();
        if let Some(rx) = &self.update_rx {
            if let Ok(version) = rx.try_recv() {
                log::info!("update check: badge set to v{version}");
                self.update_available = Some(version);
            }
        }
        self.drain_pty_events();

        // Update the global pane context snapshot so that AiQuery dispatches
        // include all open panes in the workspace context (#396).
        self.update_pane_context_snapshot();

        // Auto-dismiss notifications from the focused pane before reconciling
        // the focus stack so modal state is already correct this frame.
        self.auto_dismiss_sender_focused_notifications();

        // Focus stack: reconcile layer state BEFORE any input routing so
        // `input_captured_by_overlay()` answers correctly this frame.
        self.sync_notification_modal_focus();
        self.sync_confirm_close_focus();
        self.sync_context_close_focus();
        self.sync_command_palette_focus();
        self.sync_rename_pane_focus();
        self.sync_context_rename_focus();
        self.sync_cli_setup_prompt_focus();
        self.sync_text_input_focus();
        self.sync_capability_modal_focus();

        let _ = ctx; // ctx is unused in the preamble itself; parameter reserved for future use
    }

    /// Render the toolbar, toolbar separator, sidebar, central panel, quit overlay,
    /// feature effects, and focus re-request blocks. Called at the end of `update()`
    /// after all overlay dispatch and command draining.
    pub(super) fn render_panels(&mut self, ctx: &egui::Context) {
        // Toolbar
        egui::TopBottomPanel::top("toolbar")
            .exact_height(28.0)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.bg_toolbar)
                    .inner_margin(egui::Margin {
                        left: 80,
                        right: 8,
                        top: 4,
                        bottom: 4,
                    }),
            )
            .show(ctx, |ui| {
                self.draw_toolbar(ui);
            });

        // Separator line under toolbar
        egui::TopBottomPanel::top("toolbar_sep")
            .exact_height(1.0)
            .frame(egui::Frame::new().fill(self.colors.border))
            .show(ctx, |_ui| {});

        // Sidebar
        if self.sidebar_visible {
            ctx.input(|i| {
                for e in &i.events {
                    if let egui::Event::Key { key: egui::Key::A, pressed: true, modifiers: m, .. } = e {
                        log::info!("[diag-pre-sidebar] Key::A alive: cmd={}", m.command);
                    }
                }
            });
            egui::SidePanel::left("sidebar")
                .default_width(220.0)
                .width_range(140.0..=400.0)
                .resizable(true)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(self.colors.bg_sidebar)
                        .inner_margin(egui::Margin::same(0)),
                )
                .show(ctx, |ui| {
                    self.draw_sidebar(ui);
                });
            ctx.input(|i| {
                for e in &i.events {
                    if let egui::Event::Key { key: egui::Key::A, pressed: true, modifiers: m, .. } = e {
                        log::info!("[diag-post-sidebar] Key::A alive: cmd={}", m.command);
                    }
                }
            });
        }

        // Central panel — terminal tiles (or welcome screen when context is empty)
        egui::CentralPanel::default()
            .frame(egui::Frame {
                fill: self.colors.bg_darkest,
                inner_margin: egui::Margin::same(4),
                outer_margin: egui::Margin::ZERO,
                ..Default::default()
            })
            .show(ctx, |ui| {
                let active = self.active_window;
                if self.windows[active].panes.is_empty() || self.windows[active].tree.root.is_none() {
                    if self.router.len() > 1 {
                        let delete_pressed = ctx.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
                                || input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                        });
                        if delete_pressed {
                            let now = std::time::Instant::now();
                            let elapsed = self
                                .welcome_delete_last_press
                                .and_then(|t| now.checked_duration_since(t))
                                .unwrap_or(std::time::Duration::MAX);
                            if elapsed > std::time::Duration::from_millis(1500) {
                                self.welcome_delete_press_count = 0;
                            }
                            self.welcome_delete_press_count += 1;
                            self.welcome_delete_last_press = Some(now);
                            if self.welcome_delete_press_count >= 3 {
                                let ctx_idx = self.router.active_idx();
                                log::info!(
                                    "welcome_delete: triple-tap delete context idx={ctx_idx} name={:?}",
                                    self.router.active().name
                                );
                                self.welcome_delete_press_count = 0;
                                self.welcome_delete_last_press = None;
                                self.delete_context(ctx_idx);
                                self.save_workspace();
                                return;
                            }
                        }
                    }
                    self.draw_welcome_screen(ui);
                    if self.welcome_delete_press_count > 0 {
                        let timed_out = self
                            .welcome_delete_last_press
                            .map(|t| t.elapsed() > std::time::Duration::from_millis(1500))
                            .unwrap_or(false);
                        if timed_out {
                            self.welcome_delete_press_count = 0;
                            self.welcome_delete_last_press = None;
                        } else {
                            self.draw_welcome_delete_overlay(ctx);
                            ctx.request_repaint_after(std::time::Duration::from_millis(100));
                        }
                    }
                    return;
                }

                // Build per-pane notification counts before `ctx` mutably borrows
                // `self.windows`. Inline the notification_is_visible logic to
                // avoid a borrow conflict with the mutable ctx reference below.
                let notify_counts: std::collections::HashMap<u64, usize> = {
                    let active_context_id = self.router.active().context_id;
                    let active_window_id = self.windows[self.active_window].window_id;
                    let mut counts = std::collections::HashMap::new();
                    for n in &self.pending_notifications {
                        let visible = match n.scope {
                            crate::app_protocol::NotifyScope::Global => true,
                            crate::app_protocol::NotifyScope::Window => {
                                n.source_window_id == active_window_id
                            }
                            crate::app_protocol::NotifyScope::Context => {
                                n.source_context_id == active_context_id
                            }
                        };
                        if visible {
                            *counts.entry(n.sender_pane_id).or_insert(0) += 1;
                        }
                    }
                    counts
                };

                // Capture focus before rendering so we can record a history entry
                // if the user clicks a different pane this frame.
                let canvas_old_focus = self.windows[self.active_window].focused_pane;
                let canvas_old_window_id = self.windows[self.active_window].window_id;

                // Build portal preview data before taking the mutable ctx borrow.
                let portal_info: std::collections::HashMap<crate::spatial::tiling::PaneId, crate::spatial::tiling::PortalPreview> = {
                    let active_win = self.active_window;
                    let mut map = std::collections::HashMap::new();
                    let portal_panes: Vec<(crate::spatial::tiling::PaneId, u64)> = self.windows[active_win]
                        .panes
                        .iter()
                        .filter_map(|(pid, p)| p.portal_target().map(|cid| (*pid, cid)))
                        .collect();
                    for (pane_id, child_ctx_id) in portal_panes {
                        let ctx_entry = self.router.iter().find(|c| c.context_id == child_ctx_id);
                        let ctx_name = ctx_entry
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|| "(deleted)".to_string());
                        let ctx_description = ctx_entry
                            .and_then(|c| c.description.clone())
                            .unwrap_or_default();
                        let pane_count = self.windows.iter()
                            .filter(|w| w.context_id == child_ctx_id)
                            .flat_map(|w| w.panes.values())
                            .filter(|p| !matches!(p, crate::host::pane::Pane::Portal(_)))
                            .count();
                        let notif_count = self.context_notification_count_recursive(child_ctx_id);
                        let active_win_for_child = self.context_active_window.get(&child_ctx_id).copied();
                        let first_win_id_for_child = self.windows.iter()
                            .find(|ww| ww.context_id == child_ctx_id)
                            .map(|ww| ww.window_id);
                        let child_windows: Vec<crate::spatial::tiling::MiniWindow> = self.windows.iter()
                            .filter(|w| w.context_id == child_ctx_id)
                            .filter_map(|w| {
                                let is_active_win = active_win_for_child
                                    .map(|win_id| w.window_id == win_id)
                                    .unwrap_or_else(|| {
                                        first_win_id_for_child.map(|fid| fid == w.window_id).unwrap_or(false)
                                    });
                                let root = w.tree.root?;
                                let leaves = crate::spatial::tiling::compute_minimap_rects(&w.tree.tiles, root);
                                let panes = leaves.iter().map(|(norm_rect, tile_id)| {
                                    let pane_ref = match w.tree.tiles.get(*tile_id) {
                                        Some(egui_tiles::Tile::Pane(pid)) => w.panes.get(pid),
                                        _ => None,
                                    };
                                    let kind = match pane_ref {
                                        Some(p) if p.as_app().is_some() => crate::spatial::tiling::PaneKind::App,
                                        Some(p) if p.as_portal().is_some() => crate::spatial::tiling::PaneKind::Portal,
                                        _ => crate::spatial::tiling::PaneKind::Terminal,
                                    };
                                    let title = pane_ref.and_then(|p| {
                                        p.as_terminal().and_then(|t| t.name.clone())
                                            .or_else(|| p.as_app().map(|a| a.name.clone()))
                                    });
                                    let focused = is_active_win && w.focused_pane == Some(*tile_id);
                                    crate::spatial::tiling::MiniPane {
                                        norm_rect: *norm_rect,
                                        kind,
                                        focused,
                                        has_content: true,
                                        title,
                                        active: true,
                                    }
                                }).collect();
                                Some(crate::spatial::tiling::MiniWindow {
                                    grid_x: w.grid_x,
                                    grid_y: w.grid_y,
                                    panes,
                                })
                            })
                            .collect();
                        let window_count = child_windows.len();
                        map.insert(pane_id, crate::spatial::tiling::PortalPreview {
                            context_name: ctx_name,
                            context_description: ctx_description,
                            pane_count,
                            notification_count: notif_count,
                            windows: child_windows,
                            window_count,
                        });
                    }
                    map
                };

                // Update cached ContextState on portal panes (recomputed each frame for now;
                // future: throttle to change events only).
                {
                    let contexts: Vec<crate::host::context::Context> = self.router.iter().cloned().collect();
                    let active_win = self.active_window;
                    let portal_pane_ids: Vec<(crate::spatial::tiling::PaneId, u64)> = self.windows[active_win]
                        .panes
                        .iter()
                        .filter_map(|(pid, p)| p.portal_target().map(|cid| (*pid, cid)))
                        .collect();
                    for (pid, child_ctx_id) in portal_pane_ids {
                        let state = crate::context_state::ContextState::compute(
                            child_ctx_id,
                            &contexts,
                            &self.windows,
                        );
                        if let Some(portal) = self.windows[active_win].panes.get_mut(&pid)
                            .and_then(|p| p.as_portal_mut())
                        {
                            let old_status = portal.context_state.as_ref().map(|s| s.status.clone());
                            let new_status = state.status.clone();
                            if old_status.as_ref() != Some(&new_status) {
                                log::info!(
                                    "portal state change: pane_id={pid} ctx={child_ctx_id} status={:?} panes={} agents={}",
                                    new_status, state.pane_count, state.active_agents,
                                );
                            }
                            portal.context_state = Some(state);
                        }
                    }
                }

                // Computed before the mutable borrow of `ctx` — needed by PlexiBehavior
                // to prevent terminal panes from stealing egui focus while a modal is open.
                let modal_open = self.input_captured_by_overlay();

                let ctx = &mut self.windows[self.active_window];

                // Resolve focused_pane if simplifier moved the tile
                if let Some(fp) = ctx.focused_pane {
                    if !matches!(ctx.tree.tiles.get(fp), Some(Tile::Pane(_))) {
                        ctx.focused_pane = ctx.find_first_pane_in(fp);
                    }
                }

                // Validate zoomed pane still exists
                if let Some(zp) = ctx.zoomed_pane {
                    if !matches!(ctx.tree.tiles.get(zp), Some(Tile::Pane(_))) {
                        ctx.zoomed_pane = None;
                        self.ctx.memory_mut(|m| {
                            if let Some(id) = m.focused() {
                                m.surrender_focus(id);
                            }
                        });
                    }
                }

                let zoomed_pane = ctx.zoomed_pane;
                let tab_info = ctx.compute_tab_info();
                let pane_names: HashMap<PaneId, String> = ctx
                    .panes
                    .iter()
                    .filter_map(|(&id, p)| p.as_terminal()?.name.as_ref().map(|n| (id, n.clone())))
                    .collect();
                let suppress_focus = self.show_command_palette
                    || self.renaming_pane.is_some()
                    || self.renaming_window.is_some();

                // When a pane is zoomed, drag targeting is moot (the whole
                // window targets one pane), so skip the unsafe ObjC cursor
                // probe entirely. Otherwise, throttle the repaint to 100ms
                // — 60fps polling of NSApplication APIs from the render
                // loop is wasteful and a candidate cause of the file-drag
                // spinning ball seen in production.
                #[cfg(target_os = "macos")]
                let drag_cursor_pos: Option<egui::Pos2> = if zoomed_pane.is_some() {
                    None
                } else {
                    let has_drag = ui.input(|i| {
                        !i.raw.hovered_files.is_empty() || !i.raw.dropped_files.is_empty()
                    });
                    if has_drag {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                        use objc2_app_kit::NSApplication;
                        use objc2_foundation::MainThreadMarker;
                        MainThreadMarker::new()
                            .and_then(|mtm| {
                                let app = NSApplication::sharedApplication(mtm);
                                app.keyWindow().or_else(|| app.mainWindow())
                            })
                            .map(|w| {
                                let p = w.mouseLocationOutsideOfEventStream();
                                let content_height = ui.ctx().screen_rect().height();
                                egui::pos2(p.x as f32, content_height - p.y as f32)
                            })
                    } else {
                        None
                    }
                };
                #[cfg(not(target_os = "macos"))]
                let drag_cursor_pos: Option<egui::Pos2> = None;

                // Cache once per frame — used by PlexiBehavior to avoid O(n) ui.input() reads.
                let hovered_files = ui.input(|i| !i.raw.hovered_files.is_empty());

                // Edge-trigger: log the first frame a file drag enters the window (zoomed path).
                // Subsequent frames are silent. Used to pinpoint freeze location in the log.
                if hovered_files && zoomed_pane.is_some() {
                    let hover_id = egui::Id::new("drag_hover_was_active");
                    let was_hovering: bool =
                        ui.ctx().data(|d| d.get_temp(hover_id).unwrap_or(false));
                    if !was_hovering {
                        log::info!("[DRAG] hover: hovered_files became non-empty on zoomed pane");
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(hover_id, true));
                } else {
                    let hover_id = egui::Id::new("drag_hover_was_active");
                    ui.ctx().data_mut(|d| d.insert_temp(hover_id, false));
                }

                // Propagate pre-computed notification counts into each app pane so
                // ProcessApp can render the per-pane chrome badge without
                // holding a reference to PlexiApp.
                for pane in ctx.panes.values_mut() {
                    if let Some(app_pane) = pane.as_app_mut() {
                        let count = notify_counts.get(&app_pane.id).copied().unwrap_or(0);
                        app_pane.runtime.set_pending_notification_count(count);
                    }
                }

                let unfocused_opacity =
                    self.config.beta.as_ref().and_then(|b| b.unfocused_opacity());
                {
                    let ghost_log_id = egui::Id::new("ghost_opacity_logged");
                    let cur = unfocused_opacity.map(|v| (v * 100.0) as u32);
                    let prev: Option<u32> = ui.ctx().data(|d| d.get_temp(ghost_log_id));
                    if prev != cur {
                        if let Some(opacity) = unfocused_opacity {
                            log::info!("[ghost] unfocused pane opacity: {opacity:.2}");
                        } else if prev.is_some() {
                            log::info!("[ghost] disabled");
                        }
                        ui.ctx().data_mut(|d| {
                            if let Some(v) = cur {
                                d.insert_temp(ghost_log_id, v);
                            } else {
                                d.remove_temp::<u32>(ghost_log_id);
                            }
                        });
                    }
                }

                let ctrl_held = ui.input(|i| i.modifiers.ctrl) && ctx.panes.len() > 1;
                {
                    let overlay_log_id = egui::Id::new("pane_id_overlay_on");
                    let was_held: bool =
                        ui.ctx().data(|d| d.get_temp(overlay_log_id).unwrap_or(false));
                    if was_held != ctrl_held {
                        if ctrl_held {
                            log::info!("[pane-id-overlay] on");
                        } else {
                            log::info!("[pane-id-overlay] off");
                        }
                        ui.ctx().data_mut(|d| d.insert_temp(overlay_log_id, ctrl_held));
                    }
                }

                let mut behavior = PlexiBehavior {
                    panes: &mut ctx.panes,
                    focused_tile: if suppress_focus {
                        None
                    } else {
                        ctx.focused_pane
                    },
                    theme: self.theme.clone(),
                    new_focused: None,
                    close_exited: None,
                    tab_info,
                    zoomed_pane,
                    colors: self.colors,
                    pane_names,
                    drag_cursor_pos,
                    hovered_files,
                    workspace_root: self.router.active().root.clone().or_else(crate::config::active_workspace_root),
                    unfocused_opacity,
                    portal_info,
                    modal_open,
                    ctrl_held,
                    pane_gap: self.config.pane_gap.unwrap_or(4.0).clamp(0.0, 20.0),
                    pane_title_font_size: self.config.pane_title_font_size.unwrap_or(11.0).clamp(6.0, 32.0),
                    portal_zoom_request: None,
                };
                log::debug!("[DRAG] tiling: start (zoomed={}, hovered_files={hovered_files})", zoomed_pane.is_some());
                ui.scope(|ui| {
                    // When a pane is zoomed, the resize handles inside egui_tiles' linear
                    // container are still rendered unconditionally. Disabling the UI prevents
                    // their interact() calls from returning hovered/dragged, blocking background
                    // pane resizing while the zoom overlay is active.
                    if zoomed_pane.is_some() {
                        ui.disable();
                    }
                    ctx.tree.ui(&mut behavior, ui);
                });
                log::debug!("[DRAG] tiling: done");

                // Paint the active pane focus outline using the parent painter which
                // has the full window clip rect. paint_on_top_of_tile cannot do this —
                // its painter is clipped to the tile rect, making Outside strokes invisible.
                if zoomed_pane.is_none() && !suppress_focus {
                    if let Some(tile_id) = ctx.focused_pane {
                        if let Some(rect) = ctx.tree.tiles.rect(tile_id) {
                            let gap = behavior.pane_gap;
                            let stroke_color = if let Some(ref flash) = self.click_flash {
                                if flash.window_id == ctx.window_id && flash.tile == tile_id {
                                    let elapsed = flash.started_at.elapsed().as_secs_f32();
                                    if elapsed < FLASH_DUR {
                                        let boost = (-elapsed / FLASH_TAU).exp();
                                        ui.ctx().request_repaint();
                                        self.colors.accent.gamma_multiply((1.0 + boost).clamp(1.0, 2.0))
                                    } else {
                                        self.colors.accent
                                    }
                                } else {
                                    self.colors.accent
                                }
                            } else {
                                self.colors.accent
                            };
                            let stroke = egui::Stroke::new(gap, stroke_color);
                            ui.painter().rect_stroke(
                                rect,
                                0.0,
                                stroke,
                                egui::StrokeKind::Outside,
                            );
                        }
                    }
                }

                // Expire click_flash once FLASH_DUR has elapsed.
                if let Some(ref flash) = self.click_flash {
                    if flash.started_at.elapsed().as_secs_f32() >= FLASH_DUR {
                        self.click_flash = None;
                    }
                }

                let canvas_focus_changed = if let Some(new) = behavior.new_focused {
                    let changed = Some(new) != canvas_old_focus;
                    ctx.focused_pane = Some(new);
                    changed
                } else {
                    false
                };

                if canvas_focus_changed {
                    if let Some(new_tile) = ctx.focused_pane {
                        let win_id = ctx.window_id;
                        log::info!("focus: canvas click flash → win={win_id} tile={new_tile:?}");
                        self.click_flash = Some(ClickFlash { window_id: win_id, tile: new_tile, started_at: std::time::Instant::now() });
                    }
                }

                let should_close_exited = behavior.close_exited.is_some();
                let portal_zoom = behavior.portal_zoom_request.take();

                // Draw zoom overlay if a pane is zoomed
                if let Some(zoomed_tile) = zoomed_pane {
                    if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(zoomed_tile) {
                        let pane_id = *pane_id;
                        let panel_rect = ui.max_rect();
                        let zoomed_tab_info = behavior.tab_info.get(&zoomed_tile).copied();
                        let zoomed_pane_name = behavior.pane_names.get(&pane_id).cloned();

                        // Drop behavior to release the mutable borrow on ctx.panes
                        drop(behavior);

                        // Semi-transparent scrim over the entire central panel
                        ui.painter()
                            .rect_filled(panel_rect, 0.0, Color32::from_black_alpha(75));

                        // Inset rect for the zoomed pane
                        let inset = 10.0;
                        let zoom_rect = panel_rect.shrink(inset);

                        // Thicker accent border (2px)
                        ui.painter().rect_stroke(
                            zoom_rect,
                            CornerRadius::same(4),
                            Stroke::new(2.0, self.colors.accent),
                            StrokeKind::Inside,
                        );

                        // Render zoomed terminal in the inset rect
                        let inner_rect = zoom_rect.shrink(2.0); // inside the border
                        let mut child_ui =
                            ui.new_child(egui::UiBuilder::new().max_rect(inner_rect));
                        // Files dropped onto a zoomed pane must go to the
                        // zoomed terminal, NOT a background tile. The
                        // per-tile drop path in `tiling.rs` is gated off
                        // while zoomed; we handle the drop here instead.
                        let has_drop =
                            child_ui.input(|i| !i.raw.dropped_files.is_empty());
                        // When zoomed there is only one pane covering the entire overlay —
                        // no ambiguity about the target. Skip rect_contains_pointer: egui's
                        // pointer position is stale during macOS OS-level file drags (we skip
                        // the NSApplication cursor query when zoomed), so rect_contains_pointer
                        // would only return true if the last known position happened to fall
                        // inside inner_rect (typically the original split-pane location).
                        let dropped_to_zoom = has_drop;
                        if has_drop {
                            log::info!(
                                "drop: zoomed overlay received drop event — pane_id={pane_id:?}"
                            );
                        }
                        child_ui.set_opacity(0.88);
                        // Render name bar outside the Frame so the terminal's background
                        // fill inside the Frame cannot paint over it. The Frame gets its
                        // own child_ui scoped to the area below the name bar, so there
                        // is no cursor/spacing gap between them.
                        let has_name = zoomed_pane_name.is_some();
                        const NAME_BAR_HEIGHT: f32 = 20.0;
                        if has_name {
                            let bar_rect = egui::Rect::from_min_size(
                                inner_rect.min,
                                egui::vec2(inner_rect.width(), NAME_BAR_HEIGHT),
                            );
                            child_ui.painter().rect_filled(bar_rect, 0.0, self.colors.terminal_bg);
                            if let Some((active_idx, count)) = zoomed_tab_info {
                                crate::spatial::tiling::paint_tab_dots(
                                    child_ui.painter(),
                                    bar_rect.left(),
                                    bar_rect.center().y,
                                    active_idx,
                                    count,
                                    self.colors.accent,
                                    self.colors.bg_active,
                                );
                            }
                            if let Some(ref name) = zoomed_pane_name {
                                child_ui.painter().text(
                                    bar_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    name,
                                    egui::FontId::proportional(11.0),
                                    self.colors.text_dim,
                                );
                            }
                        }
                        // Frame rect starts exactly where the name bar ends (or at the
                        // top of inner_rect when there is no name bar), so no gap appears.
                        let frame_rect = if has_name {
                            inner_rect.with_min_y(inner_rect.min.y + NAME_BAR_HEIGHT)
                        } else {
                            inner_rect
                        };
                        let mut frame_ui = ui.new_child(egui::UiBuilder::new().max_rect(frame_rect));
                        frame_ui.set_opacity(0.88);
                        egui::Frame::new()
                            .fill(self.colors.terminal_bg)
                            .inner_margin(egui::Margin::same(8))
                            .show(&mut frame_ui, |ui| {
                                if let Some(pane) = ctx.panes.get_mut(&pane_id) {
                                    if let Some(t) = pane.as_terminal_mut() {
                                        if dropped_to_zoom {
                                            crate::spatial::tiling::write_dropped_paths_to_terminal(ui, t);
                                        }
                                        if t.exited {
                                            let rect = ui.max_rect();
                                            ui.painter().rect_filled(
                                                rect,
                                                0.0,
                                                self.colors.terminal_bg,
                                            );
                                            ui.allocate_new_ui(
                                                egui::UiBuilder::new().max_rect(rect),
                                                |ui| {
                                                    ui.centered_and_justified(|ui| {
                                                        ui.colored_label(
                                                            self.colors.text_dim,
                                                            "[process exited]",
                                                        );
                                                    });
                                                },
                                            );
                                        } else if hovered_files {
                                            // Skip TerminalView render while a file is being
                                            // dragged over the zoomed pane. TerminalView::show()
                                            // calls backend.sync() which clones the full grid
                                            // under FairMutex contention — on a large terminal
                                            // (e.g. a full-window Claude Code session) this
                                            // blocked the main thread for several seconds.
                                            // The drop itself is handled above (dropped_to_zoom
                                            // guard), so we skip only the hover-frame renders.
                                            log::debug!("[DRAG] zoom overlay: skipping TerminalView render during file hover");
                                            let rect = ui.max_rect();
                                            ui.allocate_new_ui(
                                                egui::UiBuilder::new().max_rect(rect),
                                                |ui| {
                                                    ui.centered_and_justified(|ui| {
                                                        ui.colored_label(
                                                            self.colors.text_dim,
                                                            "Drop to paste path",
                                                        );
                                                    });
                                                },
                                            );
                                        } else {
                                            // Reserve space for tab dots when no name bar
                                            if !has_name && zoomed_tab_info.is_some() {
                                                ui.add_space(
                                                    crate::spatial::tiling::TAB_DOT_RESERVED_HEIGHT,
                                                );
                                            }
                                            let font_size = t.font_size;
                                            log::debug!("[DRAG] zoom overlay: TerminalView render start");
                                            use egui_term::TerminalView;
                                            use crate::ui::theme;
                                            let terminal = TerminalView::new(ui, &mut t.backend)
                                                .set_focus(true)
                                                .set_theme(self.theme.clone())
                                                .set_font(theme::terminal_font(font_size))
                                                .set_size(Vec2::new(
                                                    ui.available_width(),
                                                    ui.available_height(),
                                                ));
                                            ui.add(terminal);
                                            log::debug!("[DRAG] zoom overlay: TerminalView render done");
                                        }
                                    } else if let Some(a) = pane.as_app_mut() {
                                        let app_ctx = crate::app::app_trait::AppRenderContext {
                                            colors: &self.colors,
                                            is_focused: true, // zoomed pane is always focused
                                        };
                                        a.runtime.ui(ui, &app_ctx);
                                    }
                                }

                                // Draw tab indicator dots for unnamed panes in a tab group
                                if !has_name {
                                    if let Some((active_idx, count)) = zoomed_tab_info {
                                        let rect = ui.max_rect();
                                        crate::spatial::tiling::paint_tab_dots(
                                            ui.painter(),
                                            rect.left(),
                                            rect.top() + 2.0 + 4.0, // 4.0 = dot radius
                                            active_idx,
                                            count,
                                            self.colors.accent,
                                            self.colors.bg_active,
                                        );
                                    }
                                }
                            });
                    } else {
                        drop(behavior);
                    }
                } else {
                    drop(behavior);
                }

                // ── Pane swap animation overlays ────────────────────────────────────────
                {
                    let anim_dur = std::time::Duration::from_millis(160);
                    let edge_dur = std::time::Duration::from_millis(120);
                    let now_anim = std::time::Instant::now();

                    self.pane_anims.retain(|a| now_anim.duration_since(a.started_at) < anim_dur);
                    if let Some(ref pulse) = self.edge_pulse {
                        if now_anim.duration_since(pulse.started_at) >= edge_dur {
                            self.edge_pulse = None;
                        }
                    }

                    for anim in &self.pane_anims {
                        let elapsed = now_anim.duration_since(anim.started_at).as_secs_f32();
                        let t = (elapsed / anim_dur.as_secs_f32()).clamp(0.0, 1.0);
                        let t_eased = 1.0 - (1.0 - t).powi(3);
                        let animated_rect = egui::Rect {
                            min: anim.from.min.lerp(anim.to.min, t_eased),
                            max: anim.from.max.lerp(anim.to.max, t_eased),
                        };
                        ui.painter().rect_filled(
                            animated_rect,
                            egui::CornerRadius::same(4),
                            self.colors.accent.gamma_multiply(0.25),
                        );
                        self.ctx.request_repaint();
                    }

                    if let Some(ref pulse) = self.edge_pulse {
                        if let Some(pane_rect) = self.windows[self.active_window].tree.tiles.rect(pulse.tile) {
                            let elapsed = now_anim.duration_since(pulse.started_at).as_secs_f32();
                            let alpha = (1.0 - elapsed / edge_dur.as_secs_f32()).max(0.0);
                            let edge_color = self.colors.accent.gamma_multiply(alpha);
                            let (p1, p2) = match pulse.dir {
                                crate::host::keys::Direction::Left => (
                                    egui::pos2(pane_rect.left(), pane_rect.top()),
                                    egui::pos2(pane_rect.left(), pane_rect.bottom()),
                                ),
                                crate::host::keys::Direction::Right => (
                                    egui::pos2(pane_rect.right(), pane_rect.top()),
                                    egui::pos2(pane_rect.right(), pane_rect.bottom()),
                                ),
                                crate::host::keys::Direction::Up => (
                                    egui::pos2(pane_rect.left(), pane_rect.top()),
                                    egui::pos2(pane_rect.right(), pane_rect.top()),
                                ),
                                crate::host::keys::Direction::Down => (
                                    egui::pos2(pane_rect.left(), pane_rect.bottom()),
                                    egui::pos2(pane_rect.right(), pane_rect.bottom()),
                                ),
                            };
                            ui.painter().line_segment([p1, p2], egui::Stroke::new(3.0, edge_color));
                            self.ctx.request_repaint();
                        }
                    }
                }

                if should_close_exited {
                    self.close_focused();
                }

                // Portal double-click zoom — same logic as ToggleZoom on a Portal pane.
                if let Some(child_ctx_id) = portal_zoom {
                    if let Some(ctx_idx) = self.router.position(|c| c.context_id == child_ctx_id) {
                        log::info!("portal double-click: zooming into context_id={child_ctx_id}");
                        let current_ctx_id = self.router.active().context_id;
                        let current_win_id = self.windows[self.active_window].window_id;
                        let focused_tile = self.windows[self.active_window].focused_pane;
                        self.router.push_depth(current_ctx_id, current_win_id, focused_tile);
                        self.switch_workspace(ctx_idx);
                    }
                }

                // Record canvas click focus change in pane history (ctx borrow released above).
                if canvas_focus_changed {
                    self.push_focus_history(canvas_old_window_id, canvas_old_focus);
                }
            });

        // Shortcuts overlay
        self.draw_shortcuts_overlay(ctx);

        // Changelog overlay
        self.draw_changelog_overlay(ctx);

        // First-launch completions nudge
        self.draw_completions_banner(ctx);

        // Minimap overlay — auto-hidden when current workspace has <2 windows.
        let ws_id = self.router.active().context_id;
        let window_count = self.windows.iter().filter(|c| c.context_id == ws_id).count();
        if window_count >= 2 {
            self.draw_minimap_overlay(ctx);
        } else {
            self.minimap.visible = false;
        }

        // Command palette, run palette, rename-pane overlay, notification
        // modal, and confirm-close are all drawn by the early input-capture
        // path at the top of `update()` — they own a `FocusLayer` and render
        // their own keystrokes before the drain. Drawing again here would
        // double-dispatch Enter/Escape after keys have been drained.
        // Quit confirmation overlay
        if self.confirm_quit() && self.quit_press_count > 0 {
            // Reset on Escape or timeout
            let timed_out = self
                .quit_last_press
                .map(|t| t.elapsed() > std::time::Duration::from_millis(1500))
                .unwrap_or(false);
            if timed_out || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.quit_press_count = 0;
                self.quit_last_press = None;
            } else {
                self.draw_quit_confirm_overlay(ctx);
                // Keep repainting so the timeout dismissal fires promptly
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        self.draw_feature_effects(ctx);

        // Re-request focus for the palette search field after all pane rendering.
        // App panes call request_focus() on their TextInput widgets during
        // CentralPanel rendering, and egui focus is last-write-wins — without
        // this, a keyboard-capture app pane steals focus from the palette every
        // frame, making the search field non-typeable even though it's visible.
        if self.show_command_palette {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("palette_search")));
        }

        // Same pattern: QuickNote compose mode needs re-focus every frame so
        // pane TextInput widgets rendered in CentralPanel can't steal it.
        if matches!(self.focus_stack.last(), Some(FocusLayer::QuickNote)) {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("quick_note_text")));
        }

        // Same pattern for all remaining text-owning overlays. Each overlay's one-shot
        // request fires during early overlay dispatch (BEFORE CentralPanel), so pane
        // TextInput widgets rendered in CentralPanel steal focus back. Re-requesting
        // here wins the last-write-wins contest for the frame.
        if self.renaming_pane.is_some() {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("rename_pane_input")));
        }
        if self.renaming_window.is_some() && !self.sidebar_visible {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("rename_context_input")));
        }
        if self.editing_description.is_some() {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("edit_description_input")));
        }
        if self.text_overlay.is_some() {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("text_input_overlay_field")));
        }
        // Capability/secret modal: only re-request for Secret prompts — Capability prompts
        // have no text field, so requesting a non-existent ID would leave egui holding a
        // stale focus pointer that interferes with button interactions.
        if matches!(self.focus_stack.last(), Some(FocusLayer::CapabilityModal)) {
            let has_secret_prompt = {
                let win = &self.windows[self.active_window];
                win.focused_pane
                    .and_then(|tile_id| Self::find_pane_in_tile(&win.tree, tile_id))
                    .and_then(|pane_id| win.panes.get(&pane_id))
                    .and_then(|pane| pane.as_app())
                    .map(|app| {
                        if let crate::host::pane::AppRuntime::Process(ref proc) = app.runtime {
                            matches!(
                                proc.pending_prompts.front(),
                                Some(crate::process_app::PendingPrompt::Secret { .. })
                            )
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false)
            };
            if has_secret_prompt {
                log::debug!("capability_modal: re-requesting focus for capability_secret_input post-CentralPanel");
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("capability_secret_input")));
            }
        }
    }
}
