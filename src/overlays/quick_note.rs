use super::*;

impl PlexiApp {
    /// Quick note compose phase: full-screen scrim + centered text input.
    pub(crate) fn draw_quick_note_modal(&mut self, ctx: &egui::Context) {
        use crate::ui::style;
        use egui::{Align2, RichText, Vec2};

        // Consume Esc to close.
        let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if esc {
            self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
            self.quick_note_text.clear();
            log::info!("QuickNote: modal dismissed");
            return;
        }

        // Explicitly consume paste events before the TextEdit renders.
        // egui::TextEdit processes Paste via filtered_events(), which requires
        // settled egui focus. When the zoom overlay is active it calls
        // set_focus(true) unconditionally, stealing focus from the TextEdit and
        // leaving paste unhandled — the event then falls through to the terminal
        // backend. Consuming here is consistent with how Esc and Enter are
        // handled above and guarantees paste lands in QuickNote every frame.
        let pasted: String = ctx.input_mut(|i| {
            let mut text = String::new();
            i.events.retain(|e| {
                if let egui::Event::Paste(t) = e {
                    text.push_str(t);
                    false
                } else {
                    true
                }
            });
            text
        });
        if !pasted.is_empty() {
            let te_id = egui::Id::new("quick_note_text");
            let inserted = if let Some(mut state) = egui::TextEdit::load_state(ctx, te_id) {
                if let Some(range) = state.cursor.char_range() {
                    let lo = range.primary.index.min(range.secondary.index);
                    let hi = range.primary.index.max(range.secondary.index);
                    let chars: Vec<char> = self.quick_note_text.chars().collect();
                    let mut new_chars: Vec<char> = chars[..lo].to_vec();
                    new_chars.extend(pasted.chars());
                    new_chars.extend(chars[hi..].iter().copied());
                    self.quick_note_text = new_chars.into_iter().collect();
                    let new_pos = lo + pasted.chars().count();
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                        egui::text::CCursor::new(new_pos),
                    )));
                    state.store(ctx, te_id);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !inserted {
                self.quick_note_text.push_str(&pasted);
            }
            log::info!("QuickNote: pasted {} chars", pasted.len());
        }

        // Plain Enter (no shift) → advance to destination picker.
        let advance = ctx.input_mut(|i| {
            if !i.modifiers.shift && !i.modifiers.command {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            } else {
                false
            }
        });
        if advance && !self.quick_note_text.trim().is_empty() {
            self.quick_note_dest_cursor = 0;
            self.push_focus_layer(crate::app::FocusLayer::QuickNoteDestination);
            log::info!("QuickNote: advancing to destination picker");
            // Draw destination in the same frame to avoid a one-frame scrim flash.
            self.draw_quick_note_destination(ctx);
            return;
        }

        let screen_rect = ctx.screen_rect();

        // Scrim — click-away dismisses the modal (click on modal area is consumed by
        // Order::Foreground before reaching this allocate_rect, so clicked() only fires
        // when the pointer lands outside the modal).
        let scrim_clicked = egui::Area::new(egui::Id::new("quick_note_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_black_alpha(style::SCRIM_ALPHA),
                );
                ui.allocate_rect(screen_rect, egui::Sense::click()).clicked()
            })
            .inner;
        if scrim_clicked {
            self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
            self.quick_note_text.clear();
            log::info!("QuickNote: modal dismissed via click-away");
            return;
        }

        // Modal — grows from ~25% to ~80% of screen height as the user types.
        let modal_w = (screen_rect.width() * 0.72).min(864.0).max(480.0);
        let max_text_h = (screen_rect.height() * 0.8).max(80.0);
        let line_h = style::TEXT_BODY + 4.0;
        let initial_rows = ((screen_rect.height() * 0.25) / line_h).round() as usize;
        let initial_rows = initial_rows.max(3);
        egui::Area::new(egui::Id::new("quick_note_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_width(modal_w);

                        // Hint
                        ui.label(
                            RichText::new("Enter to pick destination  ·  Shift+Enter for new line  ·  Esc to discard  ·  ⌘, to configure")
                                .color(self.colors.text_dim.linear_multiply(0.5))
                                .size(style::TEXT_HINT)
                                .family(egui::FontFamily::Monospace),
                        );
                        ui.add_space(style::SPACE_SM);

                        // Text input — starts at ~25% screen height, grows with content, caps at ~80%.
                        egui::ScrollArea::vertical()
                            .max_height(max_text_h)
                            .show(ui, |ui| {
                                ui.scope(|ui| {
                                    ui.visuals_mut().text_cursor.stroke.width = 1.5;
                                    ui.visuals_mut().text_cursor.stroke.color = self.colors.accent;
                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.quick_note_text)
                                            .id(egui::Id::new("quick_note_text"))
                                            .font(egui::FontId::monospace(style::TEXT_BODY))
                                            .text_color(self.colors.text_primary)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(initial_rows)
                                            .frame(false)
                                            .hint_text(
                                                RichText::new("What's on your mind?")
                                                    .color(self.colors.text_dim.linear_multiply(0.3))
                                                    .size(style::TEXT_BODY),
                                            ),
                                    );
                                });
                            });
                    });
            });
    }

    /// Quick note destination picker: digit keys route instantly; arrows/jk navigate.
    pub(crate) fn draw_quick_note_destination(&mut self, ctx: &egui::Context) {
        use crate::ui::style;
        use crate::ui::widgets;
        use egui::{Align2, RichText, Vec2};

        let dest_count = self.config.quick_note.as_ref()
            .map(|qn| qn.destinations.len())
            .unwrap_or(0);
        let total = 1 + dest_count;

        // H or Esc → back to compose.
        let back = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::H)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
        });
        if back {
            self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
            log::info!("QuickNote: destination picker dismissed, back to composing");
            self.draw_quick_note_modal(ctx); // same-frame draw to avoid flash
            return;
        }

        // Arrow / vim navigation.
        let up = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::K)
        });
        let down = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::J)
        });
        if up {
            self.quick_note_dest_cursor = if self.quick_note_dest_cursor == 0 {
                total.saturating_sub(1)
            } else {
                self.quick_note_dest_cursor - 1
            };
            log::info!("QuickNote: dest cursor → {}", self.quick_note_dest_cursor);
        }
        if down {
            self.quick_note_dest_cursor = (self.quick_note_dest_cursor + 1) % total.max(1);
            log::info!("QuickNote: dest cursor → {}", self.quick_note_dest_cursor);
        }

        // Enter → dispatch selected item.
        let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if enter {
            let cursor = self.quick_note_dest_cursor;
            if cursor == 0 {
                let text = self.quick_note_text.clone();
                log::info!("QuickNote: destination selected via Enter: cursor=0 (global backlog)");
                if self.commit_quick_note_global_backlog(&text) {
                    self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                    self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                    self.quick_note_text.clear();
                }
                return;
            }
            let dest = self.config.quick_note.as_ref()
                .and_then(|qn| qn.destinations.get(cursor - 1).cloned());
            if let Some(dest) = dest {
                log::info!("QuickNote: destination selected via Enter: cursor={cursor} key={}", dest.key);
                if dest.options.is_some() || dest.children_cmd.is_some() {
                    let key = dest.key;
                    self.quick_note_sub_cursor = 0;
                    self.quick_note_children_cache.clear();
                    self.quick_note_children_rx = None;
                    self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(vec![key]));
                    log::info!("QuickNote: submenu opened: path=[{key}]");
                    self.draw_quick_note_menu(ctx, &[key]);
                } else {
                    let text = self.quick_note_text.clone();
                    if self.commit_quick_note(&text, &dest) {
                        self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                        self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                        self.quick_note_text.clear();
                        self.quick_note_children_cache.clear();
                        self.quick_note_children_rx = None;
                    }
                }
            }
            return;
        }

        // L → enter submenu for selected item (zero-flash).
        let enter_sub = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::L)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
        });
        if enter_sub {
            let cursor = self.quick_note_dest_cursor;
            if cursor > 0 {
                let dest = self.config.quick_note.as_ref()
                    .and_then(|qn| qn.destinations.get(cursor - 1).cloned());
                if let Some(dest) = dest {
                    if dest.options.is_some() || dest.children_cmd.is_some() {
                        let key = dest.key;
                        self.quick_note_sub_cursor = 0;
                        self.quick_note_children_cache.clear();
                        self.quick_note_children_rx = None;
                        self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(vec![key]));
                        log::info!("QuickNote: submenu opened via L: path=[{key}]");
                        self.draw_quick_note_menu(ctx, &[key]);
                        return;
                    }
                }
            }
        }

        // N → open config file to add a new destination.
        let add_dest = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::N)
        });
        if add_dest {
            log::info!("QuickNote: opening config to add destination");
            crate::config::open_config_file();
            return;
        }

        // Digit keys → fast-dispatch by number.
        let pressed = consume_digit_key(ctx);

        if let Some(key) = pressed {
            if key == 0 {
                let text = self.quick_note_text.clone();
                log::info!("QuickNote: destination selected: key=0 (global backlog)");
                if self.commit_quick_note_global_backlog(&text) {
                    self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                    self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                    self.quick_note_text.clear();
                }
                return;
            }
            let dest = self.config.quick_note.as_ref()
                .and_then(|qn| qn.destinations.iter().find(|d| d.key == key).cloned());
            if let Some(dest) = dest {
                log::info!("QuickNote: destination selected: key={key}");
                if dest.options.is_some() || dest.children_cmd.is_some() {
                    self.quick_note_sub_cursor = 0;
                    self.quick_note_children_cache.clear();
                    self.quick_note_children_rx = None;
                    self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(vec![key]));
                    log::info!("QuickNote: submenu opened: path=[{key}]");
                    self.draw_quick_note_menu(ctx, &[key]);
                } else {
                    let text = self.quick_note_text.clone();
                    if self.commit_quick_note(&text, &dest) {
                        self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                        self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                        self.quick_note_text.clear();
                        self.quick_note_children_cache.clear();
                        self.quick_note_children_rx = None;
                    }
                }
            }
            return;
        }

        // Render
        let screen_rect = ctx.screen_rect();

        egui::Area::new(egui::Id::new("quick_note_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_black_alpha(style::SCRIM_ALPHA),
                );
            });

        let modal_w = (screen_rect.width() * 0.6).min(672.0).max(408.0);
        let cursor = self.quick_note_dest_cursor;
        let destinations = self.config.quick_note.as_ref()
            .map(|qn| qn.destinations.as_slice())
            .unwrap_or(&[]);
        egui::Area::new(egui::Id::new("quick_note_dest_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_width(modal_w);

                        // Note preview — up to 4 lines, each truncated at 80 chars.
                        let preview = {
                            let t = self.quick_note_text.trim();
                            let mut lines: Vec<String> = t.lines()
                                .take(4)
                                .map(|line| {
                                    if let Some((idx, _)) = line.char_indices().nth(80) {
                                        format!("{}…", &line[..idx])
                                    } else {
                                        line.to_string()
                                    }
                                })
                                .collect();
                            if t.lines().count() > 4 {
                                lines.push("…".to_string());
                            }
                            lines.join("\n")
                        };
                        ui.label(
                            RichText::new(&preview)
                                .color(self.colors.text_dim.linear_multiply(0.5))
                                .size(style::TEXT_BODY)
                                .family(egui::FontFamily::Monospace),
                        );
                        ui.add_space(style::SPACE_XL);

                        ui.label(
                            RichText::new("Send to…")
                                .color(self.colors.text_primary)
                                .size(style::TEXT_BODY),
                        );
                        ui.add_space(style::SPACE_MD);

                        // Row 0: global backlog
                        let row0_frame = if cursor == 0 {
                            egui::Frame::new()
                                .fill(self.colors.bg_active)
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(4, 2))
                        } else {
                            egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(4, 2))
                        };
                        row0_frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = style::SPACE_SM;
                                widgets::key_chip(ui, "0", &self.colors);
                                ui.label(
                                    RichText::new("Backlog (global)")
                                        .color(self.colors.text_dim)
                                        .size(style::TEXT_BODY),
                                );
                            });
                        });
                        ui.add_space(style::SPACE_SM);

                        // Config destinations
                        for (idx, dest) in destinations.iter().enumerate() {
                            let row_idx = idx + 1;
                            let label = if dest.options.is_some() || dest.children_cmd.is_some() {
                                format!("{} ›", dest.label)
                            } else {
                                dest.label.clone()
                            };
                            let row_frame = if cursor == row_idx {
                                egui::Frame::new()
                                    .fill(self.colors.bg_active)
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(4, 2))
                            } else {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(4, 2))
                            };
                            row_frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = style::SPACE_SM;
                                    widgets::key_chip(ui, &dest.key.to_string(), &self.colors);
                                    ui.label(
                                        RichText::new(&label)
                                            .color(self.colors.text_dim)
                                            .size(style::TEXT_BODY),
                                    );
                                });
                            });
                            ui.add_space(style::SPACE_SM);
                        }

                        ui.add_space(style::SPACE_XL);
                        ui.label(
                            RichText::new("↑↓/jk navigate  ·  Enter select  ·  n add dest  ·  H/Esc back")
                                .color(self.colors.text_dim.linear_multiply(0.4))
                                .size(style::TEXT_HINT)
                                .family(egui::FontFamily::Monospace),
                        );
                    });
            });
    }

    /// Recursive quick-note menu renderer. `key_path` is the sequence of keys from the
    /// root destinations list to the current node. E.g. `&[3]` = children of destination 3.
    pub(crate) fn draw_quick_note_menu(&mut self, ctx: &egui::Context, key_path: &[u8]) {
        use crate::ui::style;
        use crate::ui::widgets;
        use egui::{Align2, RichText, Vec2};

        // Resolve the current node by walking key_path through the config tree.
        let node: Option<crate::config::QuickNoteNode> = {
            let mut current: Option<&Vec<crate::config::QuickNoteNode>> =
                self.config.quick_note.as_ref().map(|qn| &qn.destinations);
            let mut found = None;
            for &key in key_path {
                if let Some(nodes) = current {
                    if let Some(n) = nodes.iter().find(|n| n.key == key) {
                        found = Some(n.clone());
                        current = n.options.as_ref();
                    } else {
                        found = None;
                        break;
                    }
                } else {
                    found = None;
                    break;
                }
            }
            found
        };

        let current_layer = crate::app::FocusLayer::QuickNoteSubDestination(key_path.to_vec());

        // ── Drain children_cmd receiver if pending ────────────────────────────────
        // Cache key is the full key_path to avoid collisions when different submenus
        // share child keys (e.g. both have a child with key=1).
        let node_path = key_path.to_vec();
        {
            let mut received = None;
            if let Some((pending_path, rx)) = &self.quick_note_children_rx {
                if pending_path == &node_path {
                    match rx.try_recv() {
                        Ok(Ok(children)) => {
                            log::info!(
                                "QuickNote: children_cmd result received for path={:?} count={}",
                                node_path, children.len()
                            );
                            received = Some((node_path.clone(), Ok(children)));
                        }
                        Ok(Err(e)) => {
                            log::error!("QuickNote: children_cmd failed for path={:?}: {e}", node_path);
                            received = Some((node_path.clone(), Err(e)));
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            log::error!("QuickNote: children_cmd sender dropped for path={:?}", node_path);
                            received = Some((node_path.clone(), Err("sender dropped".to_string())));
                        }
                    }
                }
            }
            if let Some((path, result)) = received {
                self.quick_note_children_rx = None;
                match result {
                    Ok(children) => { self.quick_note_children_cache.insert(path, children); }
                    Err(_) => { self.quick_note_children_cache.insert(path, vec![]); }
                }
                self.quick_note_sub_cursor = 0;
            }
        }

        // Determine the list of children to display.
        // Priority: static options > dynamic children_cmd results.
        let static_options = node.as_ref().and_then(|n| n.options.as_ref());
        let dynamic_children = if static_options.is_none() {
            self.quick_note_children_cache.get(&node_path).cloned()
        } else {
            None
        };
        let loading = static_options.is_none()
            && dynamic_children.is_none()
            && self.quick_note_children_rx.as_ref().map(|(p, _)| p == &node_path).unwrap_or(false);

        // Kick off children_cmd loading if needed and not already in flight.
        if static_options.is_none() && dynamic_children.is_none() && !loading {
            if let Some(n) = &node {
                if let Some(cmd_template) = &n.children_cmd {
                    let cmd = Self::substitute_note_tokens_static(
                        cmd_template,
                        &self.quick_note_text,
                        &self.quick_note_ctx,
                    );
                    let (tx, rx) = std::sync::mpsc::channel();
                    let path_for_log = node_path.clone();
                    std::thread::spawn(move || {
                        let result = std::process::Command::new("sh")
                            .args(["-c", &cmd])
                            .output()
                            .map_err(|e| e.to_string())
                            .and_then(|out| {
                                if out.status.success() {
                                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                                    let children = parse_children_cmd_output(&stdout);
                                    Ok(children)
                                } else {
                                    Err(String::from_utf8_lossy(&out.stderr).to_string())
                                }
                            });
                        let _ = tx.send(result);
                    });
                    log::info!("QuickNote: children_cmd spawned for path={:?}", path_for_log);
                    self.quick_note_children_rx = Some((node_path, rx));
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }

        // Resolve effective children list (empty if loading or no children).
        let children: Vec<crate::config::QuickNoteNode> = if let Some(opts) = static_options {
            opts.clone()
        } else if let Some(dyn_children) = dynamic_children {
            dyn_children
        } else {
            vec![]
        };

        let total = children.len();

        // ── Input handling ────────────────────────────────────────────────────────

        // H or Esc → navigate up.
        let back = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::H)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
        });
        if back {
            self.pop_focus_layer(&current_layer);
            log::info!("QuickNote: menu back: path={key_path:?}");
            if key_path.len() > 1 {
                let parent_path = key_path[..key_path.len() - 1].to_vec();
                self.draw_quick_note_menu(ctx, &parent_path);
            } else {
                self.draw_quick_note_destination(ctx);
            }
            return;
        }

        // Arrow / vim navigation (only when children are available).
        if !children.is_empty() {
            let up = ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::K)
            });
            let down = ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::J)
            });
            if up {
                self.quick_note_sub_cursor = if self.quick_note_sub_cursor == 0 {
                    total.saturating_sub(1)
                } else {
                    self.quick_note_sub_cursor - 1
                };
                log::info!("QuickNote: menu cursor → {}", self.quick_note_sub_cursor);
            }
            if down {
                self.quick_note_sub_cursor = (self.quick_note_sub_cursor + 1) % total.max(1);
                log::info!("QuickNote: menu cursor → {}", self.quick_note_sub_cursor);
            }
        }

        // Enter → dispatch selected child.
        let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if enter && !children.is_empty() {
            if let Some(child) = children.get(self.quick_note_sub_cursor).cloned() {
                let key_path_owned = key_path.to_vec();
                self.dispatch_menu_child(ctx, &key_path_owned, &child);
            }
            return;
        }

        // L → enter submenu for selected child.
        let enter_sub = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::L)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
        });
        if enter_sub && !children.is_empty() {
            if let Some(child) = children.get(self.quick_note_sub_cursor).cloned() {
                if child.options.is_some() || child.children_cmd.is_some() {
                    let mut new_path = key_path.to_vec();
                    new_path.push(child.key);
                    self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(new_path.clone()));
                    self.quick_note_sub_cursor = 0;
                    log::info!("QuickNote: menu enter via L: path={new_path:?}");
                    self.draw_quick_note_menu(ctx, &new_path);
                    return;
                }
            }
        }

        // N → open config file to add a new destination.
        let add_dest = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::N)
        });
        if add_dest {
            log::info!("QuickNote: opening config to add destination (from submenu)");
            crate::config::open_config_file();
            return;
        }

        // Digit keys → fast-dispatch by number.
        let pressed = consume_digit_key(ctx);
        if let Some(key) = pressed {
            if let Some(child) = children.iter().find(|c| c.key == key).cloned() {
                let key_path_owned = key_path.to_vec();
                self.dispatch_menu_child(ctx, &key_path_owned, &child);
            }
            return;
        }

        // ── Render ────────────────────────────────────────────────────────────────
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("quick_note_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(style::SCRIM_ALPHA));
            });

        let modal_w = (screen_rect.width() * 0.6).min(672.0).max(408.0);
        let sub_cursor = self.quick_note_sub_cursor;
        let node_label = node.as_ref().map(|n| n.label.as_str()).unwrap_or("Menu");

        egui::Area::new(egui::Id::new("quick_note_sub_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_width(modal_w);

                        ui.label(
                            RichText::new(node_label)
                                .color(self.colors.text_primary)
                                .size(style::TEXT_BODY)
                                .strong(),
                        );
                        ui.add_space(style::SPACE_MD);

                        if loading {
                            ui.label(
                                RichText::new("Loading…")
                                    .color(self.colors.text_dim)
                                    .size(style::TEXT_BODY),
                            );
                        } else if children.is_empty() {
                            ui.label(
                                RichText::new("No items")
                                    .color(self.colors.text_dim.linear_multiply(0.5))
                                    .size(style::TEXT_BODY),
                            );
                        } else {
                            for (idx, child) in children.iter().enumerate() {
                                let child_label = if child.options.is_some() || child.children_cmd.is_some() {
                                    format!("{} ›", child.label)
                                } else {
                                    child.label.clone()
                                };
                                let row_frame = if sub_cursor == idx {
                                    egui::Frame::new()
                                        .fill(self.colors.bg_active)
                                        .corner_radius(egui::CornerRadius::same(4))
                                        .inner_margin(egui::Margin::symmetric(4, 2))
                                } else {
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::symmetric(4, 2))
                                };
                                row_frame.show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = style::SPACE_SM;
                                        widgets::key_chip(ui, &child.key.to_string(), &self.colors);
                                        ui.label(
                                            RichText::new(&child_label)
                                                .color(self.colors.text_dim)
                                                .size(style::TEXT_BODY),
                                        );
                                    });
                                });
                                ui.add_space(style::SPACE_SM);
                            }
                        }

                        ui.add_space(style::SPACE_XL);
                        ui.label(
                            RichText::new("↑↓/jk navigate  ·  Enter select  ·  L enter  ·  n add dest  ·  H/Esc back")
                                .color(self.colors.text_dim.linear_multiply(0.4))
                                .size(style::TEXT_HINT)
                                .family(egui::FontFamily::Monospace),
                        );
                    });
            });
    }

    /// Dispatch a child node selected from within a menu at `parent_path`.
    /// Handles: entering deeper submenus, committing leaf commands.
    fn dispatch_menu_child(
        &mut self,
        ctx: &egui::Context,
        parent_path: &[u8],
        child: &crate::config::QuickNoteNode,
    ) {
        if child.options.is_some() || child.children_cmd.is_some() {
            // Submenu — navigate deeper.
            let mut new_path = parent_path.to_vec();
            new_path.push(child.key);
            self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(new_path.clone()));
            self.quick_note_sub_cursor = 0;
            log::info!("QuickNote: menu enter: path={new_path:?}");
            self.draw_quick_note_menu(ctx, &new_path);
        } else {
            // Leaf — commit.
            let text = self.quick_note_text.clone();
            log::info!(
                "QuickNote: menu commit: path={:?} key={} label='{}'",
                parent_path, child.key, child.label
            );
            if self.commit_quick_note(&text, child) {
                // Pop all submenu layers + destination + compose layers.
                while matches!(
                    self.focus_stack.last(),
                    Some(crate::app::FocusLayer::QuickNoteSubDestination(_))
                ) {
                    self.focus_stack.pop();
                }
                self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                self.quick_note_text.clear();
                self.quick_note_children_cache.clear();
                self.quick_note_children_rx = None;
            }
        }
    }

    pub(crate) fn quick_note_handle_key(
        &mut self,
        ctx: &egui::Context,
    ) -> crate::app::app_trait::KeyDisposition {
        // Consume Cmd+0 so poll_actions doesn't fire OpenQuickNote while the modal
        // is already open — that would reset mid-session note state.
        ctx.input_mut(|i| { i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0); });
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn quick_note_destination_handle_key(
        &mut self,
        ctx: &egui::Context,
    ) -> crate::app::app_trait::KeyDisposition {
        ctx.input_mut(|i| { i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0); });
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn quick_note_sub_destination_handle_key(
        &mut self,
        ctx: &egui::Context,
    ) -> crate::app::app_trait::KeyDisposition {
        ctx.input_mut(|i| { i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0); });
        crate::app::app_trait::KeyDisposition::Consumed
    }
}

fn parse_children_cmd_output(stdout: &str) -> Vec<crate::config::QuickNoteNode> {
    stdout.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() { return None; }
            let (key_str, rest) = line.split_once('|')?;
            let (label, command) = rest.split_once('|')?;
            let key: u8 = key_str.trim().parse().ok()?;
            Some(crate::config::QuickNoteNode {
                key,
                label: label.trim().to_string(),
                command: Some(command.trim().to_string()),
                hidden: false,
                stay_alive: None,
                options: None,
                children_cmd: None,
                dest_type: None,
                position: None,
                path: None,
            })
        })
        .collect()
}
