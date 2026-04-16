use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::theme::Colors;
use chrono::{DateTime, Local};
use egui::{Color32, CornerRadius, Stroke, StrokeKind};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

const ROW_HEIGHT: f32 = 40.0;

#[derive(Clone, Debug)]
struct DepthNode {
    path: PathBuf,
    parent: Option<PathBuf>,
    child_count: usize,
    last_scanned_at: SystemTime,
}

#[derive(Clone, Debug)]
pub(crate) struct DepthSnapshot {
    root: PathBuf,
    scanned_at: SystemTime,
    nodes: HashMap<PathBuf, DepthNode>,
    children: HashMap<PathBuf, Vec<PathBuf>>,
}

#[derive(Clone, Debug)]
struct VisibleRow {
    path: PathBuf,
    label: String,
    depth: usize,
    child_count: usize,
    last_scanned_at: SystemTime,
}

pub struct DepthTreeApp {
    workspace_root: PathBuf,
    focused_root: PathBuf,
    selected_path: PathBuf,
    snapshot: DepthSnapshot,
    error: Option<String>,
    /// Receives scan results from the background thread.
    scan_rx: Option<mpsc::Receiver<Result<DepthSnapshot, String>>>,
    scanning: bool,
    /// Deferred focus/selection to apply after the next scan completes.
    pending_restore: Option<(PathBuf, PathBuf)>,
    /// Queued commands to emit back to the host.
    pending_commands: Vec<AppCommand>,
}

impl DepthTreeApp {
    pub fn new(workspace_root: PathBuf) -> Self {
        let root = canonical_or(workspace_root);
        let mut app = Self {
            workspace_root: root.clone(),
            focused_root: root.clone(),
            selected_path: root.clone(),
            snapshot: DepthSnapshot {
                root: root.clone(),
                scanned_at: SystemTime::now(),
                nodes: HashMap::new(),
                children: HashMap::new(),
            },
            error: None,
            scan_rx: None,
            scanning: false,
            pending_restore: None,
            pending_commands: Vec::new(),
        };
        app.start_scan();
        app
    }

    /// Kick off a background scan. Non-blocking — results arrive via `scan_rx`.
    fn start_scan(&mut self) {
        let root = self.workspace_root.clone();
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.scanning = true;
        std::thread::Builder::new()
            .name("depth-tree-scan".into())
            .spawn(move || {
                let _ = tx.send(scan_depth_snapshot(&root));
            })
            .ok();
    }

    /// Poll the background scan channel. Called at the top of each `ui()` frame.
    fn check_scan(&mut self) {
        let rx = match &self.scan_rx {
            Some(rx) => rx,
            None => return,
        };
        match rx.try_recv() {
            Ok(Ok(snapshot)) => {
                self.snapshot = snapshot;
                self.error = None;
                self.scanning = false;
                self.scan_rx = None;
                // Apply deferred focus/selection from restore_state.
                if let Some((focused, selected)) = self.pending_restore.take() {
                    if self.is_visible(&focused) {
                        self.focused_root = focused;
                    }
                    if self.is_visible(&selected) {
                        self.selected_path = selected;
                    }
                }
                if !self.is_visible(&self.focused_root) {
                    self.focused_root = self.workspace_root.clone();
                }
                if !self.is_visible(&self.selected_path) {
                    self.selected_path = self.focused_root.clone();
                }
            }
            Ok(Err(e)) => {
                self.error = Some(e);
                self.scanning = false;
                self.scan_rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {} // still scanning
            Err(mpsc::TryRecvError::Disconnected) => {
                self.scanning = false;
                self.scan_rx = None;
            }
        }
    }

    fn is_visible(&self, path: &Path) -> bool {
        path == self.workspace_root || self.snapshot.nodes.contains_key(path)
    }

    fn current_children(&self) -> Vec<PathBuf> {
        if self.focused_root == self.workspace_root {
            self.snapshot
                .children
                .get(&self.workspace_root)
                .cloned()
                .unwrap_or_default()
        } else {
            self.snapshot
                .children
                .get(&self.focused_root)
                .cloned()
                .unwrap_or_default()
        }
    }

    fn parent_for(&self, path: &Path) -> Option<PathBuf> {
        self.snapshot
            .nodes
            .get(path)
            .and_then(|node| node.parent.clone())
    }

    fn focus_selected_node(&mut self) {
        let target = if self.selected_path == self.focused_root {
            self.current_children().into_iter().next()
        } else {
            Some(self.selected_path.clone())
        };

        if let Some(target) = target {
            if self.snapshot.nodes.contains_key(&target) {
                self.focused_root = target.clone();
                self.selected_path = target;
            }
        }
    }

    fn focus_parent(&mut self) {
        if let Some(parent) = self.parent_for(&self.focused_root) {
            self.focused_root = parent.clone();
            self.selected_path = parent;
        }
    }

    fn visible_rows(&self) -> Vec<VisibleRow> {
        let mut rows = Vec::new();
        let mut seen = HashSet::new();
        self.collect_rows(&self.focused_root, 0, &mut rows, &mut seen);
        rows
    }

    fn collect_rows(
        &self,
        path: &Path,
        depth: usize,
        rows: &mut Vec<VisibleRow>,
        seen: &mut HashSet<PathBuf>,
    ) {
        if !seen.insert(path.to_path_buf()) {
            return;
        }

        let label = if path == self.workspace_root {
            current_depth_label(path)
        } else {
            relative_label(&self.workspace_root, path)
        };
        let child_count = self
            .snapshot
            .children
            .get(path)
            .map(|children| children.len())
            .unwrap_or(0);
        rows.push(VisibleRow {
            path: path.to_path_buf(),
            label,
            depth,
            child_count,
            last_scanned_at: self
                .snapshot
                .nodes
                .get(path)
                .map(|node| node.last_scanned_at)
                .unwrap_or(self.snapshot.scanned_at),
        });

        if let Some(children) = self.snapshot.children.get(path) {
            for child in children {
                self.collect_rows(child, depth + 1, rows, seen);
            }
        }
    }

    fn draw_row(&mut self, ui: &mut egui::Ui, colors: &Colors, row: &VisibleRow) {
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click(),
        );
        let is_selected = row.path == self.selected_path;
        let is_current = row.path == self.focused_root;

        let fill = if is_selected {
            colors.bg_active
        } else if resp.hovered() {
            colors.bg_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        if is_current {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(6),
                Stroke::new(1.5, colors.accent),
                StrokeKind::Inside,
            );
        } else {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(6),
                Stroke::new(1.0, colors.border),
                StrokeKind::Inside,
            );
        }

        let indent = 14.0 * row.depth as f32;
        let indicator_x = rect.left() + 10.0 + indent;
        let text_x = indicator_x + 14.0;

        if row.child_count > 0 {
            ui.painter().text(
                egui::pos2(indicator_x, rect.top() + 11.0),
                egui::Align2::LEFT_TOP,
                "▸",
                egui::FontId::proportional(12.0),
                if is_current {
                    colors.accent
                } else {
                    colors.text_dim
                },
            );
        } else {
            ui.painter().text(
                egui::pos2(indicator_x + 2.0, rect.top() + 12.0),
                egui::Align2::LEFT_TOP,
                "•",
                egui::FontId::proportional(10.0),
                colors.text_dim,
            );
        }

        ui.painter().text(
            egui::pos2(text_x, rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            row.label.clone(),
            egui::FontId::proportional(11.5),
            if is_current {
                colors.accent
            } else {
                colors.text_primary
            },
        );

        let subtitle = format!(
            "{} child{} · scanned {}",
            row.child_count,
            if row.child_count == 1 { "" } else { "ren" },
            format_scan_time(row.last_scanned_at)
        );
        ui.painter().text(
            egui::pos2(text_x, rect.top() + 24.0),
            egui::Align2::LEFT_TOP,
            subtitle,
            egui::FontId::proportional(9.3),
            colors.text_dim,
        );

        if is_current {
            ui.painter().text(
                egui::pos2(rect.right() - 12.0, rect.top() + 11.0),
                egui::Align2::RIGHT_TOP,
                "current",
                egui::FontId::proportional(9.0),
                colors.accent,
            );
        }

        if resp.clicked() {
            self.selected_path = row.path.clone();
        }
        if resp.double_clicked() {
            self.selected_path = row.path.clone();
            if row.path.join(".plexi").is_dir() {
                self.pending_commands
                    .push(AppCommand::DescendDepth(row.path.clone()));
            }
        }
    }
}

impl App for DepthTreeApp {
    fn type_id(&self) -> &'static str {
        "depth_tree"
    }

    fn display_name(&self) -> String {
        "Depth Tree".to_string()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;
        self.check_scan();

        egui::Frame::new()
            .fill(colors.terminal_bg)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Depth tree")
                            .size(12.0)
                            .color(colors.text_primary)
                            .strong(),
                    );
                    ui.add_space(8.0);
                    if self.scanning {
                        ui.label(
                            egui::RichText::new("scanning...")
                                .size(10.5)
                                .color(colors.text_dim)
                                .italics(),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(abbreviate(
                                &self.focused_root,
                                &self.workspace_root,
                            ))
                            .size(10.5)
                            .color(colors.text_dim)
                            .monospace(),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let refresh_enabled = !self.scanning;
                        if ui
                            .add_enabled(refresh_enabled, egui::Button::new("Refresh").small())
                            .on_hover_text("Rescan the workspace tree")
                            .clicked()
                        {
                            self.start_scan();
                        }
                        if ui
                            .small_button("Parent")
                            .on_hover_text("Return to the parent depth")
                            .clicked()
                        {
                            self.focus_parent();
                        }
                    });
                });

                if let Some(err) = &self.error {
                    ui.add_space(6.0);
                    ui.colored_label(colors.text_dim, err);
                    return;
                }

                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Current depth: {}",
                        abbreviate(&self.focused_root, &self.workspace_root)
                    ))
                    .size(10.0)
                    .color(colors.text_section),
                );

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                let rows = self.visible_rows();
                if rows.is_empty() {
                    ui.colored_label(colors.text_dim, "No .plexi depths found");
                    return;
                }

                if !rows.iter().any(|row| row.path == self.selected_path) {
                    self.selected_path = self.focused_root.clone();
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for row in rows {
                            self.draw_row(ui, colors, &row);
                            ui.add_space(3.0);
                        }
                    });
            });
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        let rows = self.visible_rows();
        if rows.is_empty() {
            if input.key_pressed(egui::Key::R) {
                self.start_scan();
                return true;
            }
            return false;
        }
        let mut selected_idx = rows
            .iter()
            .position(|row| row.path == self.selected_path)
            .unwrap_or(0);
        let last = rows.len().saturating_sub(1);
        let mut consumed = false;

        if input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::J) {
            selected_idx = (selected_idx + 1).min(last);
            self.selected_path = rows[selected_idx].path.clone();
            consumed = true;
        }
        if input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::K) {
            selected_idx = selected_idx.saturating_sub(1);
            self.selected_path = rows[selected_idx].path.clone();
            consumed = true;
        }
        if input.key_pressed(egui::Key::Home) {
            self.selected_path = rows
                .first()
                .map(|row| row.path.clone())
                .unwrap_or_else(|| self.focused_root.clone());
            consumed = true;
        }
        if input.key_pressed(egui::Key::End) {
            self.selected_path = rows
                .last()
                .map(|row| row.path.clone())
                .unwrap_or_else(|| self.focused_root.clone());
            consumed = true;
        }
        if input.key_pressed(egui::Key::PageDown) {
            selected_idx = (selected_idx + 8).min(last);
            self.selected_path = rows[selected_idx].path.clone();
            consumed = true;
        }
        if input.key_pressed(egui::Key::PageUp) {
            selected_idx = selected_idx.saturating_sub(8);
            self.selected_path = rows[selected_idx].path.clone();
            consumed = true;
        }

        if input.key_pressed(egui::Key::ArrowRight) {
            self.focus_selected_node();
            consumed = true;
        }
        if input.key_pressed(egui::Key::Backspace) || input.key_pressed(egui::Key::ArrowLeft) {
            self.focus_parent();
            consumed = true;
        }
        // Enter = descend into the selected depth boundary.
        if input.key_pressed(egui::Key::Enter) {
            let target = self.selected_path.clone();
            if target.join(".plexi").is_dir() {
                self.pending_commands
                    .push(AppCommand::DescendDepth(target));
            }
            consumed = true;
        }
        if input.key_pressed(egui::Key::R) {
            self.start_scan();
            consumed = true;
        }

        consumed
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_commands)
    }

    fn current_dir(&self) -> Option<&Path> {
        Some(&self.workspace_root)
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "workspace_root": self.workspace_root.display().to_string(),
            "focused_root": self.focused_root.display().to_string(),
            "selected_path": self.selected_path.display().to_string(),
        }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(root) = state.get("workspace_root").and_then(|v| v.as_str()) {
            let root = canonical_or(PathBuf::from(root));
            if root.is_dir() {
                self.workspace_root = root.clone();
                self.focused_root = root.clone();
                self.selected_path = root;
                self.start_scan();
            }
        }
        // Defer focus/selection — the scan hasn't completed yet.
        let focused = state
            .get("focused_root")
            .and_then(|v| v.as_str())
            .map(|s| canonical_or(PathBuf::from(s)));
        let selected = state
            .get("selected_path")
            .and_then(|v| v.as_str())
            .map(|s| canonical_or(PathBuf::from(s)));
        if focused.is_some() || selected.is_some() {
            self.pending_restore = Some((
                focused.unwrap_or_else(|| self.focused_root.clone()),
                selected.unwrap_or_else(|| self.selected_path.clone()),
            ));
        }
    }
}

fn scan_depth_snapshot(root: &Path) -> Result<DepthSnapshot, String> {
    let root = canonical_or(root.to_path_buf());
    if !root.is_dir() {
        return Err(format!(
            "Depth tree root is not a directory: {}",
            root.display()
        ));
    }

    let tree = crate::fractal_depth::DepthTree::scan(&root)
        .map_err(|e| format!("Scan failed: {e}"))?;

    let scanned_at = SystemTime::now();
    let mut nodes = HashMap::new();
    let mut children: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

    // Build node_id → directory_path lookup for parent resolution.
    let id_to_path: HashMap<&str, &PathBuf> = tree
        .nodes
        .iter()
        .map(|n| (n.node_id.as_str(), &n.directory_path))
        .collect();

    for node in &tree.nodes {
        let parent_path = node
            .parent_id
            .as_deref()
            .and_then(|pid| id_to_path.get(pid))
            .map(|p| (*p).clone());

        nodes.insert(
            node.directory_path.clone(),
            DepthNode {
                path: node.directory_path.clone(),
                parent: parent_path.clone(),
                child_count: node.child_count,
                last_scanned_at: scanned_at,
            },
        );

        if let Some(ref parent_path) = parent_path {
            let list = children.entry(parent_path.clone()).or_default();
            if !list.contains(&node.directory_path) {
                list.push(node.directory_path.clone());
            }
        }
    }

    for list in children.values_mut() {
        list.sort();
    }

    Ok(DepthSnapshot {
        root,
        scanned_at,
        nodes,
        children,
    })
}

fn canonical_or(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn relative_label(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .map(|rel| rel.display().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        })
}

fn current_depth_label(path: &Path) -> String {
    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    if label.is_empty() {
        ".".to_string()
    } else {
        label
    }
}

fn abbreviate(path: &Path, root: &Path) -> String {
    if path == root {
        return current_depth_label(path);
    }
    relative_label(root, path)
}

fn format_scan_time(time: SystemTime) -> String {
    let dt: DateTime<Local> = DateTime::from(time);
    dt.format("%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!("plexi-depth-tree-{nonce}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discovers_nested_plexi_depths() {
        let root = unique_temp_dir();
        std::fs::create_dir_all(root.join(".plexi")).unwrap();
        std::fs::create_dir_all(root.join("agents/scraper/.plexi")).unwrap();
        std::fs::create_dir_all(root.join("agents/reviewer/.plexi")).unwrap();
        std::fs::create_dir_all(root.join("services/api/.plexi")).unwrap();
        std::fs::create_dir_all(root.join("notes")).unwrap();

        let snapshot = scan_depth_snapshot(&root).unwrap();
        let canon_root = canonical_or(root.clone());
        let root_children = snapshot
            .children
            .get(&canon_root)
            .cloned()
            .unwrap_or_default();
        // Only boundary nodes — scraper, reviewer, api link directly to root.
        assert_eq!(root_children.len(), 3);

        let scraper = canonical_or(root.join("agents/scraper"));
        let reviewer = canonical_or(root.join("agents/reviewer"));
        let api = canonical_or(root.join("services/api"));

        assert!(snapshot.nodes.contains_key(&scraper));
        assert!(snapshot.nodes.contains_key(&reviewer));
        assert!(snapshot.nodes.contains_key(&api));

        // Non-boundary intermediate dirs are not in the tree.
        let agents = canonical_or(root.join("agents"));
        let services = canonical_or(root.join("services"));
        assert!(!snapshot.nodes.contains_key(&agents));
        assert!(!snapshot.nodes.contains_key(&services));

        // All boundary children point to root as nearest ancestor.
        assert_eq!(
            snapshot.nodes.get(&scraper).and_then(|n| n.parent.as_ref()),
            Some(&canon_root)
        );
        assert_eq!(
            snapshot.nodes.get(&reviewer).and_then(|n| n.parent.as_ref()),
            Some(&canon_root)
        );
        assert_eq!(
            snapshot.nodes.get(&api).and_then(|n| n.parent.as_ref()),
            Some(&canon_root)
        );
    }

    /// Spin until the background scan finishes (test helper).
    fn wait_for_scan(app: &mut DepthTreeApp) {
        for _ in 0..500 {
            app.check_scan();
            if !app.scanning {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("scan did not complete within timeout");
    }

    #[test]
    fn serializes_and_restores_depth_focus() {
        let root = unique_temp_dir();
        std::fs::create_dir_all(root.join(".plexi")).unwrap();
        std::fs::create_dir_all(root.join("agents/scraper/.plexi")).unwrap();

        let mut app = DepthTreeApp::new(root.clone());
        wait_for_scan(&mut app);
        app.focused_root = canonical_or(root.join("agents/scraper"));
        app.selected_path = app.focused_root.clone();

        let state = app.serialize_state().unwrap();
        let mut restored = DepthTreeApp::new(root.clone());
        restored.restore_state(&state);
        wait_for_scan(&mut restored);

        assert_eq!(
            restored.focused_root,
            canonical_or(root.join("agents/scraper"))
        );
        assert_eq!(
            restored.selected_path,
            canonical_or(root.join("agents/scraper"))
        );
    }
}
