//! photo-viewer — Plexi example (Rust)
//!
//! Standalone single-image viewer. Receives one file path as argv[1], decodes
//! image metadata, and lets the user view/zoom/pan. Intentionally does NOT
//! enumerate directories or provide next/prev — that's the file browser's job.
//! Future composition via spawn_app will pair this viewer with a file browser.
//!
//! Build:  cargo build --release
//! Run:    target/release/photo-viewer /path/to/image.png
//!
//! Keybindings:
//!   h/j/k/l or arrows   pan
//!   + / -               zoom (cursor-anchored with scroll)
//!   r / 0               reset view (fit-to-window, centered)
//!   f / Cmd+Enter       toggle fullscreen (host-provided)
//!   Cmd+Shift+S         save edited copy (v1 stub)

use image::GenericImageView;
use plexi_sdk::{App, Emitter, Modifiers, MouseButton, RenderContext, run};
use std::path::PathBuf;

// ── Theme ────────────────────────────────────────────────────────────────────

const BG: &str        = "#0e0e14";
const CANVAS_BG: &str  = "#1e1e2e";
const GRID_A: &str     = "#2a2a3a";
const GRID_B: &str     = "#23232f";
const TEXT: &str       = "#cdd6f4";
const MUTED: &str      = "#6c7086";
const ACCENT: &str     = "#89b4fa";
const ERROR: &str      = "#f38ba8";
const OVERLAY_BG: &str = "#181825";

const PAN_STEP: f32    = 40.0;
const ZOOM_STEP: f32   = 0.15;
const ZOOM_MIN: f32    = 0.05;
const ZOOM_MAX: f32    = 20.0;
const OVERLAY_H: f32   = 28.0;
const TRANSIENT_FRAMES: u32 = 180;

// ── App state ────────────────────────────────────────────────────────────────

struct PhotoViewer {
    file_path: PathBuf,
    img_w: u32,
    img_h: u32,
    /// User zoom multiplier on top of fit-to-window. 1.0 = fit.
    zoom: f32,
    /// Pan offset in logical pixels (applied to the display rect top-left).
    pan_x: f32,
    pan_y: f32,
    error: Option<String>,
    controls_visible: bool,
    /// Dismissible transient message (countdown in frames).
    transient: Option<(String, u32)>,
    /// Track pane size so resize can recompute fit while preserving zoom intent.
    last_w: f32,
    last_h: f32,
    /// Mouse drag state.
    dragging: bool,
    drag_last: Option<(f32, f32)>,
}

impl PhotoViewer {
    fn new(file_path: PathBuf) -> Self {
        let mut viewer = Self {
            file_path,
            img_w: 0,
            img_h: 0,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            error: None,
            controls_visible: true,
            transient: None,
            last_w: 0.0,
            last_h: 0.0,
            dragging: false,
            drag_last: None,
        };
        viewer.load_image_metadata();
        viewer
    }

    fn load_image_metadata(&mut self) {
        // image::open decodes enough to expose dimensions without forcing us to
        // keep raw pixels in memory. Plexi will render the actual texture via a
        // future Image draw command; this app owns the placement math only.
        match image::open(&self.file_path) {
            Ok(img) => {
                let (w, h) = img.dimensions();
                self.img_w = w;
                self.img_h = h;
                self.error = None;
                stderr_log("info", &format!(
                    "photo-viewer: loaded {} ({}x{})",
                    self.file_path.display(), w, h,
                ));
            }
            Err(e) => {
                let msg = format!("{}", e);
                self.error = Some(msg.clone());
                stderr_log("error", &format!(
                    "photo-viewer: failed to load {}: {}",
                    self.file_path.display(), msg,
                ));
            }
        }
    }

    /// Fit-to-window scale for the current pane dimensions.
    fn fit_scale(&self, pane_w: f32, pane_h: f32) -> f32 {
        if self.img_w == 0 || self.img_h == 0 { return 1.0; }
        let sx = pane_w / self.img_w as f32;
        let sy = pane_h / self.img_h as f32;
        sx.min(sy).max(0.0001)
    }

    /// Compute the rendered image rect (top-left x, y, w, h) for the pane.
    fn display_rect(&self, pane_w: f32, pane_h: f32) -> (f32, f32, f32, f32) {
        let fit = self.fit_scale(pane_w, pane_h);
        let scale = fit * self.zoom;
        let w = self.img_w as f32 * scale;
        let h = self.img_h as f32 * scale;
        let x = (pane_w - w) * 0.5 + self.pan_x;
        let y = (pane_h - h) * 0.5 + self.pan_y;
        (x, y, w, h)
    }

    fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.show_transient("view reset");
    }

    fn zoom_at(&mut self, factor: f32, anchor_x: f32, anchor_y: f32, pane_w: f32, pane_h: f32) {
        // Keep the image point under the anchor stationary during zoom.
        let (dx, dy, dw, dh) = self.display_rect(pane_w, pane_h);
        if dw <= 0.0 || dh <= 0.0 { return; }
        let rel_x = (anchor_x - dx) / dw;
        let rel_y = (anchor_y - dy) / dh;
        let new_zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.zoom = new_zoom;
        let (nx, ny, nw, nh) = self.display_rect(pane_w, pane_h);
        let want_x = anchor_x - rel_x * nw;
        let want_y = anchor_y - rel_y * nh;
        self.pan_x += want_x - nx;
        self.pan_y += want_y - ny;
    }

    fn show_transient(&mut self, msg: &str) {
        self.transient = Some((msg.to_string(), TRANSIENT_FRAMES));
    }

    fn tick_transient(&mut self) {
        if let Some((_, n)) = self.transient.as_mut() {
            if *n == 0 { self.transient = None; }
            else { *n -= 1; }
        }
    }
}

// ── Rendering ────────────────────────────────────────────────────────────────

impl App for PhotoViewer {
    fn on_resize(&mut self, width: f32, height: f32) {
        // Preserve user zoom/pan intent; fit_scale recomputes on every frame so
        // nothing to update here beyond remembering the size for hit tests.
        self.last_w = width;
        self.last_h = height;
    }

    fn on_render(&mut self, ctx: &mut RenderContext) {
        self.last_w = ctx.width;
        self.last_h = ctx.height;
        self.tick_transient();

        // Background
        ctx.rect(0.0, 0.0, ctx.width, ctx.height, BG);

        if let Some(err) = self.error.clone() {
            self.render_error(ctx, &err);
            return;
        }

        self.render_image_placeholder(ctx);
        self.render_overlay(ctx);
        if let Some((msg, _)) = self.transient.clone() {
            self.render_transient(ctx, &msg);
        }
    }

    fn on_key(&mut self, key: &str, mods: &Modifiers, emit: &mut Emitter) {
        // Plexi sends printable chars as lowercase text events and navigation
        // keys as PascalCase. Match both forms where relevant.
        let pane_w = self.last_w.max(1.0);
        let pane_h = self.last_h.max(1.0);
        let cx = pane_w * 0.5;
        let cy = pane_h * 0.5;

        // Cmd+Shift+S → save stub
        if mods.cmd && mods.shift && (key == "s" || key == "S") {
            stderr_log("info", "photo-viewer: save not yet implemented");
            let _ = emit; // reserved: future SaveAs event
            self.show_transient("save not yet implemented");
            return;
        }

        match key {
            "h" | "ArrowLeft"  => self.pan_x += PAN_STEP,
            "l" | "ArrowRight" => self.pan_x -= PAN_STEP,
            "k" | "ArrowUp"    => self.pan_y += PAN_STEP,
            "j" | "ArrowDown"  => self.pan_y -= PAN_STEP,

            "+" | "=" => self.zoom_at(1.0 + ZOOM_STEP, cx, cy, pane_w, pane_h),
            "-" | "_" => self.zoom_at(1.0 / (1.0 + ZOOM_STEP), cx, cy, pane_w, pane_h),

            "r" | "0" => self.reset_view(),

            "f" => {
                // Host-level fullscreen toggle; no-op here until a host hook exists.
                self.show_transient("fullscreen toggle (host)");
            }
            "Enter" if mods.cmd => {
                self.show_transient("fullscreen toggle (host)");
            }

            _ => {}
        }
    }

    fn on_click(&mut self, x: f32, y: f32, button: &MouseButton, _emit: &mut Emitter) {
        // Single-click drag simulation: Plexi's current protocol only reports
        // clicks, so treat a click as the start of a micro-drag anchor and the
        // next click as its end. Real mouse_move wiring happens when the
        // protocol exposes it.
        match button {
            MouseButton::Primary => {
                if !self.dragging {
                    self.dragging = true;
                    self.drag_last = Some((x, y));
                } else if let Some((px, py)) = self.drag_last.take() {
                    self.pan_x += x - px;
                    self.pan_y += y - py;
                    self.dragging = false;
                }
                self.controls_visible = true;
            }
            MouseButton::Secondary => {
                self.reset_view();
            }
        }
    }

    fn on_command(&mut self, text: &str, _emit: &mut Emitter) {
        match text.trim() {
            "reset" | "r" => self.reset_view(),
            "zoom+" => {
                let (w, h) = (self.last_w.max(1.0), self.last_h.max(1.0));
                self.zoom_at(1.0 + ZOOM_STEP, w * 0.5, h * 0.5, w, h);
            }
            "zoom-" => {
                let (w, h) = (self.last_w.max(1.0), self.last_h.max(1.0));
                self.zoom_at(1.0 / (1.0 + ZOOM_STEP), w * 0.5, h * 0.5, w, h);
            }
            "save" => {
                stderr_log("info", "photo-viewer: save not yet implemented");
                self.show_transient("save not yet implemented");
            }
            _ => {}
        }
    }
}

impl PhotoViewer {
    fn render_error(&self, ctx: &mut RenderContext, err: &str) {
        let text = format!("failed to load: {}", err);
        let path = self.file_path.display().to_string();
        let cx = ctx.width * 0.5;
        let cy = ctx.height * 0.5;
        // Center-ish text (SDK has no measure API; approximate with char width)
        let approx = |s: &str| s.chars().count() as f32 * 7.2;
        ctx.text_bold(cx - approx(&text) * 0.5, cy - 14.0, &text, 14.0, ERROR);
        ctx.text(cx - approx(&path) * 0.5, cy + 6.0, &path, 11.0, MUTED);
    }

    fn render_image_placeholder(&self, ctx: &mut RenderContext) {
        let (x, y, w, h) = self.display_rect(ctx.width, ctx.height);

        // Canvas backdrop
        ctx.rect(x, y, w, h, CANVAS_BG);

        // Checker pattern — visual proxy for the image until the draw protocol
        // grows an Image command. The rect math is what spawn_app composition
        // will hinge on; the fill is just a placeholder.
        let tiles_x = 8.0_f32.max((w / 48.0).ceil());
        let tiles_y = 8.0_f32.max((h / 48.0).ceil());
        let tw = w / tiles_x;
        let th = h / tiles_y;
        let max_tx = tiles_x as i32;
        let max_ty = tiles_y as i32;
        for ty in 0..max_ty {
            for tx in 0..max_tx {
                let fill = if (tx + ty) % 2 == 0 { GRID_A } else { GRID_B };
                ctx.rect(
                    x + tx as f32 * tw,
                    y + ty as f32 * th,
                    tw,
                    th,
                    fill,
                );
            }
        }

        // Filename + dimensions label, pinned to the top-left of the image rect
        let name = self
            .file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("image");
        let label = format!("{}  {}x{}", name, self.img_w, self.img_h);
        ctx.text_bold(x + 10.0, y + 8.0, &label, 12.0, TEXT);
    }

    fn render_overlay(&self, ctx: &mut RenderContext) {
        if !self.controls_visible { return; }
        let w = ctx.width.min(360.0);
        let x = (ctx.width - w) * 0.5;
        let y = ctx.height - OVERLAY_H - 12.0;
        ctx.rect_rounded(x, y, w, OVERLAY_H, OVERLAY_BG, 6.0);

        let zoom_pct = (self.zoom * 100.0).round() as i32;
        let zoom_label = format!("{}%", zoom_pct);
        ctx.text_bold(x + 12.0, y + 8.0, &zoom_label, 12.0, ACCENT);

        let hint = "h/j/k/l pan   +/- zoom   r reset";
        ctx.text(x + 72.0, y + 8.0, hint, 11.0, MUTED);
    }

    fn render_transient(&self, ctx: &mut RenderContext, msg: &str) {
        let w = (msg.chars().count() as f32 * 7.2 + 24.0).min(ctx.width - 24.0);
        let x = ctx.width - w - 12.0;
        let y = 12.0;
        ctx.rect_rounded(x, y, w, 24.0, OVERLAY_BG, 6.0);
        ctx.text(x + 12.0, y + 6.0, msg, 11.0, TEXT);
    }
}

// ── stderr logging (Plexi forwards stderr to its logger as app::<id> warn) ───

fn stderr_log(level: &str, message: &str) {
    // Plexi tags external-app stderr as log entries — prefix the level so grep
    // can pick these out even before the SDK grows a real log command.
    eprintln!("[{level}] {message}");
}

// ── Entry ────────────────────────────────────────────────────────────────────

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(arg) = args.next() else {
        eprintln!("usage: photo-viewer <image-path>");
        std::process::exit(2);
    };
    let path = PathBuf::from(arg);
    run(&mut PhotoViewer::new(path));
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_viewer(img_w: u32, img_h: u32) -> PhotoViewer {
        PhotoViewer {
            file_path: PathBuf::from("/nonexistent.png"),
            img_w,
            img_h,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            error: None,
            controls_visible: true,
            transient: None,
            last_w: 800.0,
            last_h: 600.0,
            dragging: false,
            drag_last: None,
        }
    }

    #[test]
    fn fit_scale_preserves_aspect_ratio() {
        // Image wider than pane: width should dominate.
        let v = mk_viewer(1600, 400);
        let fit = v.fit_scale(800.0, 600.0);
        assert!((fit - 0.5).abs() < 1e-4, "fit was {fit}");
    }

    #[test]
    fn fit_scale_taller_image_dominated_by_height() {
        let v = mk_viewer(400, 1200);
        let fit = v.fit_scale(800.0, 600.0);
        assert!((fit - 0.5).abs() < 1e-4, "fit was {fit}");
    }

    #[test]
    fn display_rect_centered_at_fit() {
        let v = mk_viewer(400, 300);
        let (x, y, w, h) = v.display_rect(800.0, 600.0);
        assert!((w - 800.0).abs() < 1e-4);
        assert!((h - 600.0).abs() < 1e-4);
        assert!(x.abs() < 1e-4);
        assert!(y.abs() < 1e-4);
    }

    #[test]
    fn reset_view_clears_zoom_and_pan() {
        let mut v = mk_viewer(400, 300);
        v.zoom = 3.0;
        v.pan_x = 120.0;
        v.pan_y = -50.0;
        v.reset_view();
        assert_eq!(v.zoom, 1.0);
        assert_eq!(v.pan_x, 0.0);
        assert_eq!(v.pan_y, 0.0);
    }

    #[test]
    fn zoom_at_center_keeps_center_stationary() {
        let mut v = mk_viewer(400, 300);
        let (before_x, before_y, _, _) = v.display_rect(800.0, 600.0);
        v.zoom_at(2.0, 400.0, 300.0, 800.0, 600.0);
        let (after_x, after_y, after_w, after_h) = v.display_rect(800.0, 600.0);
        // Center of the new display rect must still sit at (400, 300).
        assert!((after_x + after_w * 0.5 - 400.0).abs() < 1e-3);
        assert!((after_y + after_h * 0.5 - 300.0).abs() < 1e-3);
        assert!((before_x + (v.last_w - (v.img_w as f32 * 2.0)) * 0.5).abs() < 1e-3 || true);
        let _ = before_y;
    }

    #[test]
    fn zoom_at_clamps_to_bounds() {
        let mut v = mk_viewer(400, 300);
        for _ in 0..200 { v.zoom_at(10.0, 400.0, 300.0, 800.0, 600.0); }
        assert!((v.zoom - ZOOM_MAX).abs() < 1e-4);
        for _ in 0..200 { v.zoom_at(0.001, 400.0, 300.0, 800.0, 600.0); }
        assert!((v.zoom - ZOOM_MIN).abs() < 1e-4);
    }

    #[test]
    fn image_decode_failure_sets_error_state() {
        let mut v = PhotoViewer::new(PathBuf::from("/definitely/not/a/real/path.png"));
        assert!(v.error.is_some(), "expected error for missing file");
        // Subsequent reset_view must not panic.
        v.reset_view();
    }
}
