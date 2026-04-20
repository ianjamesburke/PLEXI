use fontdue::{Font, FontSettings};
use serde_json::Value;
use tiny_skia::{Paint, PathBuilder, Pixmap, PremultipliedColorU8, Stroke, Transform};

use crate::protocol::view::{CanvasPrimitive, Color, Point, TextStyle, View};

const FONT_DATA: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf");

pub struct HeadlessRenderer {
    font: Font,
}

impl HeadlessRenderer {
    pub fn new() -> Self {
        let font = Font::from_bytes(FONT_DATA, FontSettings::default())
            .expect("bundled DejaVuSans.ttf is valid");
        Self { font }
    }

    /// Render a `View::Canvas` to a `Pixmap`. Document and Media views produce
    /// a blank black frame — add cases here as the renderer grows.
    #[allow(dead_code)] // STEP-11: reference-PNG snapshot test path
    pub fn render_to_pixmap(&self, view: &View, width: u32, height: u32) -> Pixmap {
        let mut pixmap = Pixmap::new(width.max(1), height.max(1)).expect("positive dimensions");
        if let View::Canvas { primitives } = view {
            for prim in primitives {
                self.draw(&mut pixmap, prim);
            }
        }
        pixmap
    }

    /// Convenience wrapper: render → PNG bytes.
    #[allow(dead_code)] // STEP-11: reference-PNG snapshot test path
    pub fn render_to_png(&self, view: &View, width: u32, height: u32) -> Vec<u8> {
        self.render_to_pixmap(view, width, height)
            .encode_png()
            .expect("PNG encoding never fails on a valid Pixmap")
    }

    #[allow(dead_code)] // STEP-11: reference-PNG snapshot test path
    fn draw(&self, pixmap: &mut Pixmap, prim: &CanvasPrimitive) {
        match prim {
            CanvasPrimitive::Rect { rect, fill, radius } => {
                let Some(path) =
                    rounded_rect_path(rect.x, rect.y, rect.w, rect.h, *radius)
                else {
                    return;
                };
                let paint = solid_paint(fill);
                pixmap.fill_path(
                    &path,
                    &paint,
                    tiny_skia::FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }

            CanvasPrimitive::Text { pos, text, style } => {
                self.blit_text(pixmap, pos, text, style);
            }

            CanvasPrimitive::Line { from, to, stroke } => {
                let mut pb = PathBuilder::new();
                pb.move_to(from.x, from.y);
                pb.line_to(to.x, to.y);
                let Some(path) = pb.finish() else { return };
                let mut stroke_style = Stroke::default();
                stroke_style.width = stroke.width;
                pixmap.stroke_path(
                    &path,
                    &solid_paint(&stroke.color),
                    &stroke_style,
                    Transform::identity(),
                    None,
                );
            }

            CanvasPrimitive::Image { .. } => {
                // Image loading not implemented in headless renderer.
            }
        }
    }

    fn blit_text(&self, pixmap: &mut Pixmap, origin: &Point, text: &str, style: &TextStyle) {
        let mut cursor_x = origin.x;
        let w = pixmap.width() as i32;
        let h = pixmap.height() as i32;

        // `origin.y` is the TOP of the text block (LEFT_TOP, matching egui).
        // Compute the baseline by adding the font's ascender height.
        let ascent = self
            .font
            .horizontal_line_metrics(style.size)
            .map(|m| m.ascent.ceil() as i32)
            .unwrap_or((style.size * 0.8) as i32);
        let baseline_y = origin.y as i32 + ascent;

        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, style.size);
            let gx = cursor_x as i32 + metrics.xmin;
            // ymin: pixels from baseline to bottom of glyph (positive = above baseline).
            let gy = baseline_y - metrics.height as i32 - metrics.ymin;

            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let coverage = bitmap[row * metrics.width + col];
                    if coverage == 0 {
                        continue;
                    }
                    let px = gx + col as i32;
                    let py = gy + row as i32;
                    if px < 0 || py < 0 || px >= w || py >= h {
                        continue;
                    }
                    let alpha =
                        (coverage as u16 * style.color.a as u16 / 255) as u8;
                    let pr = (style.color.r as u16 * alpha as u16 / 255) as u8;
                    let pg = (style.color.g as u16 * alpha as u16 / 255) as u8;
                    let pb = (style.color.b as u16 * alpha as u16 / 255) as u8;
                    let idx = py as usize * w as usize + px as usize;
                    // Alpha-over composite onto existing pixel.
                    let dst = pixmap.pixels_mut()[idx];
                    let inv = 255 - alpha as u16;
                    let out_r = (pr as u16 + dst.red() as u16 * inv / 255) as u8;
                    let out_g = (pg as u16 + dst.green() as u16 * inv / 255) as u8;
                    let out_b = (pb as u16 + dst.blue() as u16 * inv / 255) as u8;
                    let out_a = (alpha as u16 + dst.alpha() as u16 * inv / 255) as u8;
                    pixmap.pixels_mut()[idx] =
                        PremultipliedColorU8::from_rgba(out_r, out_g, out_b, out_a)
                            .unwrap_or(pixmap.pixels()[idx]);
                }
            }

            cursor_x += metrics.advance_width;
        }
    }
}

fn solid_paint(c: &Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(c.r, c.g, c.b, c.a);
    paint
}

/// Build a rounded-rect path. Falls back to a sharp-corner rect when radius ≤ 0.
/// Uses the standard cubic-bezier circle approximation (κ ≈ 0.5523).
fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return tiny_skia::Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect);
    }
    const K: f32 = 0.5522847; // kappa
    let kr = K * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + kr, y,        x + w,        y + r - kr,  x + w,     y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w,        y + h - r + kr, x + w - r + kr, y + h,       x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - kr,   y + h,       x,            y + h - r + kr, x,         y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x,             y + r - kr,  x + r - kr,   y,             x + r,     y);
    pb.close();
    pb.finish()
}

/// Parse a CSS hex color string (`"#rrggbb"` or `"#rgb"`) into a `Color`.
/// Returns opaque black on any parse failure.
fn parse_hex_color(s: &str) -> Color {
    let s = s.trim_start_matches('#');
    let (r, g, b) = match s.len() {
        6 => (
            u8::from_str_radix(&s[0..2], 16).unwrap_or(0),
            u8::from_str_radix(&s[2..4], 16).unwrap_or(0),
            u8::from_str_radix(&s[4..6], 16).unwrap_or(0),
        ),
        3 => (
            u8::from_str_radix(&s[0..1], 16).unwrap_or(0) * 17,
            u8::from_str_radix(&s[1..2], 16).unwrap_or(0) * 17,
            u8::from_str_radix(&s[2..3], 16).unwrap_or(0) * 17,
        ),
        _ => (0, 0, 0),
    };
    Color { r, g, b, a: 255 }
}

impl HeadlessRenderer {
    /// Render a PGAP `Vec<Value>` draw-command stream to PNG bytes.
    ///
    /// This is the entry point for the agent dev loop:
    /// ```
    /// let cmds = harness.render_frame(1, 640.0, 480.0);
    /// let png  = HeadlessRenderer::new().render_pgap_frame(&cmds, 640, 480);
    /// ```
    ///
    /// Recognised commands: `rect`, `text`, `line`. Everything else (including
    /// `frame_done`, `image`, `video_player`) is silently skipped.
    pub fn render_pgap_frame(&self, commands: &[Value], width: u32, height: u32) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width.max(1), height.max(1)).expect("positive dimensions");
        for cmd in commands {
            let Some(kind) = cmd.get("type").and_then(Value::as_str) else {
                continue;
            };
            match kind {
                "rect" => self.pgap_rect(&mut pixmap, cmd),
                "text" => self.pgap_text(&mut pixmap, cmd),
                "line" => self.pgap_line(&mut pixmap, cmd),
                "list" => self.pgap_list(&mut pixmap, cmd, width),
                _ => {}
            }
        }
        pixmap.encode_png().expect("PNG encoding never fails on a valid Pixmap")
    }

    fn pgap_rect(&self, pixmap: &mut Pixmap, cmd: &Value) {
        let x = cmd["x"].as_f64().unwrap_or(0.0) as f32;
        let y = cmd["y"].as_f64().unwrap_or(0.0) as f32;
        let w = cmd["w"].as_f64().unwrap_or(0.0) as f32;
        let h = cmd["h"].as_f64().unwrap_or(0.0) as f32;
        let radius = cmd["radius"].as_f64().unwrap_or(0.0) as f32;
        let color_str = cmd["fill"].as_str().unwrap_or("#000000");
        let color = parse_hex_color(color_str);
        let Some(path) = rounded_rect_path(x, y, w, h, radius) else { return };
        pixmap.fill_path(
            &path,
            &solid_paint(&color),
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    fn pgap_text(&self, pixmap: &mut Pixmap, cmd: &Value) {
        let x = cmd["x"].as_f64().unwrap_or(0.0) as f32;
        let y = cmd["y"].as_f64().unwrap_or(0.0) as f32;
        let text = cmd["text"].as_str().unwrap_or("");
        let size = cmd["size"].as_f64().unwrap_or(14.0) as f32;
        let color_str = cmd["color"].as_str().unwrap_or("#ffffff");
        let color = parse_hex_color(color_str);
        let bold = cmd["bold"].as_bool().unwrap_or(false);
        let style = TextStyle { size, color, bold, italic: false };
        let pos = Point { x, y };
        self.blit_text(pixmap, &pos, text, &style);
    }

    fn pgap_line(&self, pixmap: &mut Pixmap, cmd: &Value) {
        let x1 = cmd["x1"].as_f64().unwrap_or(0.0) as f32;
        let y1 = cmd["y1"].as_f64().unwrap_or(0.0) as f32;
        let x2 = cmd["x2"].as_f64().unwrap_or(0.0) as f32;
        let y2 = cmd["y2"].as_f64().unwrap_or(0.0) as f32;
        let color_str = cmd["color"].as_str().unwrap_or("#ffffff");
        let color = parse_hex_color(color_str);
        let width = cmd["width"].as_f64().unwrap_or(1.0) as f32;

        let mut pb = PathBuilder::new();
        pb.move_to(x1, y1);
        pb.line_to(x2, y2);
        let Some(path) = pb.finish() else { return };
        let mut stroke = Stroke::default();
        stroke.width = width;
        pixmap.stroke_path(&path, &solid_paint(&color), &stroke, Transform::identity(), None);
    }

    fn pgap_list(&self, pixmap: &mut Pixmap, cmd: &Value, frame_width: u32) {
        let items = match cmd["items"].as_array() {
            Some(a) => a,
            None => return,
        };
        let selected = cmd["selected"].as_u64().unwrap_or(0) as usize;
        let item_h = cmd["item_height"].as_f64().unwrap_or(40.0) as f32;
        let x = cmd["x"].as_f64().unwrap_or(0.0) as f32;
        let mut y = cmd["y"].as_f64().unwrap_or(88.0) as f32;
        let w = cmd["w"].as_f64().unwrap_or(frame_width as f64) as f32;
        let h = cmd["h"].as_f64().unwrap_or(f64::INFINITY) as f32;
        let bottom = y + h;

        let surface_color = parse_hex_color("#313244");
        let fg = parse_hex_color("#cdd6f4");
        let muted = parse_hex_color("#6c7086");

        for (i, item) in items.iter().enumerate() {
            if y >= bottom {
                break;
            }
            let label = item["label"].as_str().unwrap_or("");
            let is_sel = i == selected;
            if is_sel {
                if let Some(path) = rounded_rect_path(x, y, w, item_h, 0.0) {
                    pixmap.fill_path(
                        &path,
                        &solid_paint(&surface_color),
                        tiny_skia::FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
            }
            let color = if is_sel { fg } else { muted };
            let style = TextStyle { size: 15.0, color, bold: false, italic: false };
            self.blit_text(pixmap, &Point { x: x + 24.0, y: y + 12.0 }, label, &style);
            y += item_h;
        }
    }
}
