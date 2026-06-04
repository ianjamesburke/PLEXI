use fontdue::{Font, FontSettings};
use serde_json::Value;
use tiny_skia::{Paint, PathBuilder, Pixmap, PremultipliedColorU8, Stroke, Transform};

use crate::protocol::view::{CanvasPrimitive, Color, Point, TextStyle, View};

const FONT_DATA: &[u8] = include_bytes!("../../fonts/DejaVuSans.ttf");

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
                let stroke_style = Stroke { width: stroke.width, ..Stroke::default() };
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
                "layout" => self.pgap_layout(&mut pixmap, cmd, width as f32),
                "responsive" => self.pgap_responsive(&mut pixmap, cmd, width, height),
                _ => {}
            }
        }
        pixmap.encode_png().expect("PNG encoding never fails on a valid Pixmap")
    }

    // ── Layout command support ──────────────────────────────────────────────
    //
    // Resolves a `layout` draw command using taffy for flex positioning, then
    // paints each leaf at its resolved absolute position. Font measurement uses
    // fontdue (DejaVuSans) — metrics approximate egui's harfbuzz output.

    const BADGE_PAD_H: f32 = 8.0;
    const BADGE_PAD_V: f32 = 3.0;
    const BADGE_MIN_W: f32 = 32.0;

    fn measure_text_advance(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|ch| self.font.rasterize(ch, size).0.advance_width)
            .sum()
    }

    fn measure_text_height(&self, size: f32) -> f32 {
        self.font
            .horizontal_line_metrics(size)
            .map(|m| (m.ascent - m.descent).ceil())
            .unwrap_or(size)
    }

    fn measure_leaf_headless(&self, cmd: &Value) -> (f32, f32) {
        let kind = cmd.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "badge" => {
                let label = cmd["label"].as_str().unwrap_or("");
                let font_size = cmd["font_size"].as_f64().unwrap_or(11.0) as f32;
                let tw = self.measure_text_advance(label, font_size);
                let th = self.measure_text_height(font_size);
                let w = (tw + Self::BADGE_PAD_H * 2.0).max(Self::BADGE_MIN_W);
                let h = th + Self::BADGE_PAD_V * 2.0;
                (w, h)
            }
            "text" => {
                let text = cmd["text"].as_str().unwrap_or("");
                let size = cmd["size"].as_f64().unwrap_or(12.0) as f32;
                let w = self.measure_text_advance(text, size);
                let h = self.measure_text_height(size);
                (w, h)
            }
            "key_chip" => {
                let label = cmd["label"].as_str().unwrap_or("");
                let font_size = cmd["font_size"].as_f64().unwrap_or(11.0) as f32;
                let tw = self.measure_text_advance(label, font_size);
                let th = self.measure_text_height(font_size);
                (tw + 10.0, th + 2.0) // KEYCHIP_PAD_H=5*2, PAD_V=1*2
            }
            _ => (0.0, 0.0),
        }
    }

    fn pgap_layout(&self, pixmap: &mut Pixmap, cmd: &Value, available_w: f32) {
        use taffy::prelude::*;

        let origin_x = cmd["x"].as_f64().unwrap_or(0.0) as f32;
        let origin_y = cmd["y"].as_f64().unwrap_or(0.0) as f32;
        let gap = cmd["gap"].as_f64().unwrap_or(0.0) as f32;
        let direction = cmd["direction"].as_str().unwrap_or("row");
        let children = match cmd["children"].as_array() {
            Some(a) => a,
            None => return,
        };

        let mut tree: taffy::TaffyTree<Option<Value>> = taffy::TaffyTree::new();

        fn build_node(
            tree: &mut taffy::TaffyTree<Option<Value>>,
            child: &Value,
            renderer: &HeadlessRenderer,
        ) -> taffy::NodeId {
            let kind = child.get("type").and_then(Value::as_str).unwrap_or("");
            match kind {
                "leaf" => {
                    let command = &child["command"];
                    let (w, h) = renderer.measure_leaf_headless(command);
                    let style = taffy::style::Style {
                        size: taffy::geometry::Size {
                            width: length(w),
                            height: length(h),
                        },
                        ..Default::default()
                    };
                    tree.new_leaf_with_context(style, Some(command.clone())).unwrap()
                }
                "node" => {
                    let dir = child["direction"].as_str().unwrap_or("row");
                    let g = child["gap"].as_f64().unwrap_or(0.0) as f32;
                    let flex_dir = if dir == "column" {
                        taffy::style::FlexDirection::Column
                    } else {
                        taffy::style::FlexDirection::Row
                    };
                    let child_ids: Vec<taffy::NodeId> = child["children"]
                        .as_array()
                        .map(|a| a.iter().map(|c| build_node(tree, c, renderer)).collect())
                        .unwrap_or_default();
                    let gap_size = if dir == "column" {
                        taffy::geometry::Size { width: length(0.0), height: length(g) }
                    } else {
                        taffy::geometry::Size { width: length(g), height: length(0.0) }
                    };
                    let style = taffy::style::Style {
                        display: taffy::style::Display::Flex,
                        flex_direction: flex_dir,
                        gap: gap_size,
                        ..Default::default()
                    };
                    tree.new_with_children(style, &child_ids).unwrap()
                }
                _ => tree.new_leaf(taffy::style::Style::default()).unwrap(),
            }
        }

        let flex_dir = if direction == "column" {
            taffy::style::FlexDirection::Column
        } else {
            taffy::style::FlexDirection::Row
        };
        let gap_size = if direction == "column" {
            taffy::geometry::Size { width: length(0.0), height: length(gap) }
        } else {
            taffy::geometry::Size { width: length(gap), height: length(0.0) }
        };

        let child_ids: Vec<taffy::NodeId> = children
            .iter()
            .map(|c| build_node(&mut tree, c, self))
            .collect();
        let root_style = taffy::style::Style {
            display: taffy::style::Display::Flex,
            flex_direction: flex_dir,
            gap: gap_size,
            ..Default::default()
        };
        let root = tree.new_with_children(root_style, &child_ids).unwrap();

        let avail = taffy::geometry::Size {
            width: taffy::style::AvailableSpace::Definite(available_w - origin_x),
            height: taffy::style::AvailableSpace::MaxContent,
        };
        tree.compute_layout(root, avail).unwrap();

        // Walk the resolved tree and paint each leaf.
        fn paint(
            tree: &taffy::TaffyTree<Option<Value>>,
            node: taffy::NodeId,
            parent_x: f32,
            parent_y: f32,
            pixmap: &mut Pixmap,
            renderer: &HeadlessRenderer,
        ) {
            let layout = tree.layout(node).unwrap();
            let abs_x = parent_x + layout.location.x;
            let abs_y = parent_y + layout.location.y;
            let leaf_h = layout.size.height;

            if let Some(Some(cmd)) = tree.get_node_context(node) {
                let kind = cmd.get("type").and_then(Value::as_str).unwrap_or("");
                match kind {
                    "badge" => {
                        let label = cmd["label"].as_str().unwrap_or("");
                        let fill = cmd["fill"].as_str().unwrap_or("#89b4fa");
                        let fg = cmd["fg"].as_str().unwrap_or("#1e1e2e");
                        let font_size = cmd["font_size"].as_f64().unwrap_or(11.0) as f32;
                        let radius = cmd["radius"].as_f64().unwrap_or(8.0) as f32;
                        let (w, _h) = renderer.measure_leaf_headless(cmd);
                        let pill_rect_cmd = serde_json::json!({
                            "type": "rect", "x": abs_x, "y": abs_y,
                            "w": w, "h": leaf_h, "fill": fill, "radius": radius
                        });
                        renderer.pgap_rect(pixmap, &pill_rect_cmd);
                        let tw = renderer.measure_text_advance(label, font_size);
                        let th = renderer.measure_text_height(font_size);
                        let text_cmd = serde_json::json!({
                            "type": "text",
                            "x": abs_x + (w - tw) / 2.0,
                            "y": abs_y + (leaf_h - th) / 2.0,
                            "text": label, "size": font_size, "color": fg, "bold": false
                        });
                        renderer.pgap_text(pixmap, &text_cmd);
                    }
                    "text" => {
                        let mut text_cmd = cmd.clone();
                        text_cmd["x"] = serde_json::json!(abs_x);
                        text_cmd["y"] = serde_json::json!(abs_y);
                        renderer.pgap_text(pixmap, &text_cmd);
                    }
                    _ => {}
                }
            }

            for child in tree.children(node).unwrap() {
                paint(tree, child, abs_x, abs_y, pixmap, renderer);
            }
        }

        paint(&tree, root, origin_x, origin_y, pixmap, self);
    }

    fn pgap_responsive(&self, pixmap: &mut Pixmap, cmd: &Value, width: u32, height: u32) {
        let tiers = match cmd.get("tiers").and_then(Value::as_array) {
            Some(t) => t,
            None => return,
        };
        let w = width as f32;
        let h = height as f32;
        let ratio = if h > 0.0 { w / h } else { f32::MAX };
        let tier = tiers.iter().find(|t| {
            let aspect = t.get("aspect").and_then(Value::as_str).unwrap_or("");
            match aspect {
                "landscape" => ratio > 1.2,
                "portrait" => ratio < 0.83,
                "square" => true,
                _ => false,
            }
        });
        if let Some(tier) = tier {
            // Synthesize a layout command from the winning tier and delegate
            let mut layout_cmd = tier.clone();
            layout_cmd["type"] = serde_json::json!("layout");
            if layout_cmd.get("x").is_none() {
                layout_cmd["x"] = cmd.get("x").cloned().unwrap_or(serde_json::json!(0.0));
            }
            if layout_cmd.get("y").is_none() {
                layout_cmd["y"] = cmd.get("y").cloned().unwrap_or(serde_json::json!(0.0));
            }
            self.pgap_layout(pixmap, &layout_cmd, w);
        }
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
        let stroke = Stroke { width, ..Stroke::default() };
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
