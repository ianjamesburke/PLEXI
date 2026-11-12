// Scene-graph renderer — maps a WASM plugin's arena `UiTree` onto egui widgets.
//
// The tree is a flat list of `IndexedNode`s referenced by u32 id (the WIT model
// avoids recursive types). Rendering walks from `tree.root`, looking children
// up by id. Interactive nodes (buttons, list rows, submitted inputs) collect
// their declared action strings into `RenderResult` so the pane can route them
// back to the guest. A depth cap guards against malformed/cyclic trees.

use egui::{Color32, RichText};
use std::collections::{HashMap, VecDeque};

use crate::ui::style;
use crate::ui::theme::Colors;

use super::wasm_view::bindings::plexi::platform::types::{
    CanvasCommand, FooterKeysNode, PinnedEdge, RowNode, TextNode,
};
use super::wasm_view::{
    Alignment, BadgeColor, ButtonStyle, Color, IndexedNode, Justify, UiNodeData, UiTree,
};

use crate::render::view_chrome::{self, ViewChrome};

mod text_role;
pub(crate) use text_role::TextRole;

const MAX_DEPTH: u32 = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasFit {
    #[default]
    Fill,
    Contain,
}

/// A canvas click reported in the plugin's own declared canvas coordinate
/// space (post `canvas_transform` inversion), not screen pixels. See
/// `MouseEvent` in `sdk/python/plexi_sdk/events.py`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasClick {
    pub x: f32,
    pub y: f32,
    pub button: Option<&'static str>,
    pub pressed: bool,
}

/// Interactions produced by one render pass, to be translated into guest input.
#[derive(Default, Debug)]
pub struct RenderResult {
    /// Action strings from clicked buttons, selected list rows, or submitted inputs.
    pub actions: Vec<String>,
    /// `(on_change action, new value)` pairs from edited text inputs.
    pub value_changes: Vec<(String, String)>,
    /// Canvas clicks, already inverted into the plugin's declared canvas space.
    pub canvas_clicks: Vec<CanvasClick>,
    pub canvas_time: std::time::Duration,
    authored_paint: Vec<std::ops::Range<usize>>,
    /// Painted geometry per node, filled only by a pass started through
    /// [`render_ui_tree_layout`]. `None` on every live-host pass.
    pub node_layout: Option<Vec<NodeLayout>>,
    node_paint: Option<Vec<NodePaintSpan>>,
}

/// What one node painted in a headless pass: read from the pass's shape list,
/// so it reports the pixels the renderer produced and never a second layout.
/// A node with no entry was not visited as its own node (a keypad button, a
/// table cell); a node with an entry and `visible == None` painted nothing a
/// viewer can see.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeLayout {
    pub id: u32,
    /// Union of everything the node painted, before clipping.
    pub rect: Option<egui::Rect>,
    /// The part of `rect` inside the clip rects it was painted under.
    pub visible: Option<egui::Rect>,
    /// Clip rect of the `Ui` the node was laid out in.
    pub clip: egui::Rect,
    /// A text run inside the node broke a line in the middle of a word.
    pub text_wrapped_mid_word: bool,
    /// A text run inside the node extends past its clip rect.
    pub text_cut: bool,
}

#[derive(Debug)]
struct NodePaintSpan {
    id: u32,
    shapes: std::ops::Range<usize>,
    clip: egui::Rect,
}

/// True when consecutive rows of one text run split a word: the first row
/// ends, and the next begins, on a non-whitespace character with no newline
/// between them ("Star" / "t").
pub fn rows_break_mid_word(rows: &[(String, bool)]) -> bool {
    rows.windows(2).any(|pair| {
        let (first, ends_with_newline) = &pair[0];
        !ends_with_newline
            && first.chars().last().is_some_and(|c| !c.is_whitespace())
            && pair[1].0.chars().next().is_some_and(|c| !c.is_whitespace())
    })
}

fn galley_rows(galley: &egui::Galley) -> Vec<(String, bool)> {
    galley
        .rows
        .iter()
        .map(|placed| {
            (
                placed.row.glyphs.iter().map(|g| g.chr).collect(),
                placed.ends_with_newline,
            )
        })
        .collect()
}

/// Slack, in points, before painted geometry counts as cut: strokes and
/// glyph ink routinely sit a fraction of a point outside their widget rect.
const LAYOUT_CUT_SLACK: f32 = 1.0;

fn measure_shape(shape: &egui::Shape, clip: egui::Rect, fact: &mut NodeLayout) {
    match shape {
        egui::Shape::Noop => {}
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                measure_shape(shape, clip, fact);
            }
        }
        other => {
            let bounds = other.visual_bounding_rect();
            if !bounds.is_positive() {
                return;
            }
            fact.rect = Some(fact.rect.map_or(bounds, |r| r.union(bounds)));
            let visible = bounds.intersect(clip);
            if visible.is_positive() {
                fact.visible = Some(fact.visible.map_or(visible, |r| r.union(visible)));
            }
            if let egui::Shape::Text(text) = other {
                if !clip.expand(LAYOUT_CUT_SLACK).contains_rect(bounds) {
                    fact.text_cut = true;
                }
                if rows_break_mid_word(&galley_rows(&text.galley)) {
                    fact.text_wrapped_mid_word = true;
                }
            }
        }
    }
}

fn collect_node_layout(ui: &egui::Ui, spans: &[NodePaintSpan]) -> Vec<NodeLayout> {
    ui.ctx().graphics(|g| {
        let entries: Vec<&egui::epaint::ClippedShape> = g
            .get(ui.layer_id())
            .map(|list| list.all_entries().collect())
            .unwrap_or_default();
        spans
            .iter()
            .map(|span| {
                let mut fact = NodeLayout {
                    id: span.id,
                    rect: None,
                    visible: None,
                    clip: span.clip,
                    text_wrapped_mid_word: false,
                    text_cut: false,
                };
                for entry in entries.get(span.shapes.clone()).unwrap_or_default() {
                    measure_shape(&entry.shape, entry.clip_rect, &mut fact);
                }
                fact
            })
            .collect()
    })
}

/// Render a view tree, compositing `surface` into the first surface-node
/// (the guest's GPU output). Pass `None` to draw a placeholder instead.
///
/// `tree_seq` identifies the *generation* of the tree being painted: callers
/// bump it every time they swap in a newly committed guest tree, and repaint
/// the same tree under the same value. Host-owned edit buffers use it to tell
/// "the plugin pushed me a new value" apart from "I am painting the same stale
/// tree again" — see `reconcile_text_input_buffer`.
#[cfg(test)]
pub fn render_ui_tree_with_surface(
    ui: &mut egui::Ui,
    tree: &UiTree,
    colors: &Colors,
    surface: Option<egui::TextureId>,
    surface_key: Option<crate::ui::focus::SurfaceKey>,
    tree_seq: u64,
) -> RenderResult {
    render_ui_tree_with_canvas_fits(ui, tree, colors, surface, None, None, surface_key, tree_seq)
}

#[derive(Clone, Copy)]
struct TreeRenderEnv<'a> {
    nodes: &'a [IndexedNode],
    colors: &'a Colors,
    surface: Option<egui::TextureId>,
    canvas_fits: Option<&'a HashMap<u32, CanvasFit>>,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
    surface_key: Option<crate::ui::focus::SurfaceKey>,
    tree_seq: u64,
    /// How many `Row`s enclose the node being rendered. Rows deeper than
    /// `MAX_MEASURED_ROW_DEPTH` are laid out without a measuring pass.
    row_depth: u8,
    /// True inside a row's own measuring pass, where nested rows are laid
    /// out in one pass so no row is measured once per enclosing row.
    in_row_measure: bool,
}

/// Rows nested deeper than this, and rows inside another row's measuring
/// pass, are laid out in one pass: their long text is not wrapped and their
/// children are centered whatever `align` says. A measuring pass renders a
/// row's whole subtree once more, and trees are untrusted guest input, so the
/// number of rows that may each do that above any one node is bounded. With
/// the rule that a measuring pass never lays a flex column out, one frame
/// visits a node at most `2 * (MAX_MEASURED_ROW_DEPTH + 1)` times: once to
/// paint and once in its nearest flex column's measurement, each with one
/// more visit per measuring row above it.
const MAX_MEASURED_ROW_DEPTH: u8 = 4;

// `render_node` calls on this thread, for the tests that bound how often one
// frame visits a node.
#[cfg(test)]
thread_local! {
    static RENDER_NODE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[allow(clippy::too_many_arguments)]
pub fn render_ui_tree_with_canvas_fits(
    ui: &mut egui::Ui,
    tree: &UiTree,
    colors: &Colors,
    surface: Option<egui::TextureId>,
    canvas_fits: Option<&HashMap<u32, CanvasFit>>,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
    surface_key: Option<crate::ui::focus::SurfaceKey>,
    tree_seq: u64,
) -> RenderResult {
    render_ui_tree_into(
        ui,
        tree,
        colors,
        surface,
        canvas_fits,
        pending_click,
        surface_key,
        tree_seq,
        RenderResult::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn render_ui_tree_into(
    ui: &mut egui::Ui,
    tree: &UiTree,
    colors: &Colors,
    surface: Option<egui::TextureId>,
    canvas_fits: Option<&HashMap<u32, CanvasFit>>,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
    surface_key: Option<crate::ui::focus::SurfaceKey>,
    tree_seq: u64,
    mut out: RenderResult,
) -> RenderResult {
    let paint_start = paint_len(ui);
    let env = TreeRenderEnv {
        nodes: &tree.nodes,
        colors,
        surface,
        canvas_fits,
        pending_click,
        surface_key,
        tree_seq,
        row_depth: 0,
        in_row_measure: false,
    };
    let render_root = |ui: &mut egui::Ui, out: &mut RenderResult| {
        render_node(ui, env, tree.root, out, 0);
    };
    // Good-by-default spacing (stint 0445): a declarative plugin with no layout
    // code should not render flush against the pane edge. Wrap the whole tree
    // in the host UI kit's standard content inset so body content, plugin bars,
    // and footers all get comfortable breathing room. Full-bleed pixel plugins (a
    // grow Canvas or a GPU Surface anywhere in the tree) own their own layout
    // and are exempt.
    // Paint the plugin-surface fill (`theme.bg`, the color plugins are told their
    // container is) across the entire pane rect first so both inset flow plugins
    // and full-bleed Canvas plugins share the host theme backdrop.
    ui.painter()
        .rect_filled(ui.max_rect(), 0.0, colors.terminal_bg);
    if tree_wants_content_padding(&tree.nodes) {
        // Stint 0674: chrome bands are exempt from the inset. A leading SurfaceHeader
        // and a trailing FooterKeys are the plugin's *edges*, not its content —
        // insetting them floats the title bar off the pane's top-left corner
        // with a strip of background above and beside it. Render those bands
        // against the pane rect and inset only the body between them.
        if !render_root_with_chrome_bands(ui, tree.root, env, &mut out) {
            egui::Frame::NONE
                .inner_margin(root_content_inset())
                .show(ui, |ui| render_root(ui, &mut out));
        }
    } else {
        render_root(ui, &mut out);
    }
    resolve_default_text(ui, paint_start, colors.terminal_bg, &out.authored_paint);
    out.authored_paint.clear();
    if let Some(spans) = out.node_paint.take() {
        out.node_layout = Some(collect_node_layout(ui, &spans));
    }
    out
}

fn paint_len(ui: &egui::Ui) -> usize {
    ui.ctx()
        .graphics(|g| g.get(ui.layer_id()).map_or(0, |p| p.all_entries().len()))
}

/// Default declarative text follows the final painted surface, including
/// nested cards, transparent ghost buttons, selection, press and disabled
/// tints. Literal Text colors and canvas commands remain author-owned.
/// This runs after containers replace their background placeholders, so the
/// text cannot accidentally resolve against a workspace or preset backdrop.
fn resolve_default_text(
    ui: &egui::Ui,
    start: usize,
    backdrop: Color32,
    authored: &[std::ops::Range<usize>],
) {
    fn resolve(
        shape: &mut egui::Shape,
        clip: egui::Rect,
        backdrop: Color32,
        fills: &mut Vec<(egui::Rect, Color32)>,
        authored: bool,
    ) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    resolve(shape, clip, backdrop, fills, authored);
                }
            }
            egui::Shape::Rect(rect) if rect.fill != Color32::TRANSPARENT => {
                fills.push((rect.rect.intersect(clip), rect.fill));
            }
            egui::Shape::Text(text) if !authored && !text.galley.job.text.is_empty() => {
                let bounds = text.galley.rect.translate(text.pos.to_vec2());
                if !clip.intersects(bounds) {
                    return;
                }
                let point = bounds.center();
                let mut layers = Vec::new();
                for (rect, fill) in fills.iter().rev() {
                    if rect.contains(point) {
                        layers.push(*fill);
                        if fill.a() == 255 {
                            break;
                        }
                    }
                }
                let fill = layers.iter().rev().fold(backdrop, |bg, fg| bg.blend(*fg));
                let preferred = text
                    .override_text_color
                    .or_else(|| {
                        text.galley
                            .job
                            .sections
                            .first()
                            .map(|s| s.format.color)
                            .filter(|c| *c != Color32::PLACEHOLDER)
                    })
                    .unwrap_or(text.fallback_color);
                text.override_text_color = Some(
                    crate::ui::foreground::Surface::over(backdrop, fill)
                        .foreground(preferred, crate::ui::foreground::Role::SmallText),
                );
            }
            _ => {}
        }
    }
    ui.ctx().graphics_mut(|g| {
        if let Some(list) = g.get_mut(ui.layer_id()) {
            let end = list.all_entries().len();
            let mut fills = Vec::new();
            for index in start..end {
                list.mutate_shape(egui::layers::ShapeIdx(index), |entry| {
                    resolve(
                        &mut entry.shape,
                        entry.clip_rect,
                        backdrop,
                        &mut fills,
                        authored.iter().any(|range| range.contains(&index)),
                    );
                });
            }
        }
    });
}

/// Render a root `Column` whose leading `SurfaceHeader`s and trailing `FooterKeys`
/// paint full-bleed while its body carries the standard content inset.
/// Returns false when the tree is not a column (the caller then insets the
/// whole tree, which is correct for a bare root widget).
fn render_root_with_chrome_bands(
    ui: &mut egui::Ui,
    root: u32,
    env: TreeRenderEnv<'_>,
    out: &mut RenderResult,
) -> bool {
    let nodes = env.nodes;
    let Some(UiNodeData::Column(c)) = find_node(nodes, root).map(|n| &n.data) else {
        return false;
    };

    let leading = c
        .children
        .iter()
        .take_while(|id| {
            matches!(
                find_node(nodes, **id).map(|n| &n.data),
                Some(UiNodeData::SurfaceHeader(_))
            )
        })
        .count();
    let trailing = c
        .children
        .last()
        .copied()
        .and_then(|last_id| column_bottom_pin(nodes, last_id));
    if leading == 0 && trailing.is_none() {
        return false;
    }

    let stack_size = egui::vec2(ui.available_width(), ui.available_height());
    let (stack_rect, _) = ui.allocate_exact_size(stack_size, egui::Sense::hover());

    let bars_h = if leading > 0 {
        ui.scope_builder(egui::UiBuilder::new().max_rect(stack_rect), |ui| {
            ui.set_clip_rect(stack_rect);
            ui.with_layout(egui::Layout::top_down(cross_align(c.align)), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for &child in &c.children[..leading] {
                    render_node(ui, env, child, out, 1);
                }
            });
        })
        .response
        .rect
        .height()
    } else {
        0.0
    };

    let footer_h = trailing
        .map(|(_, f)| view_chrome::footer_keys_height(ui, &f.entries, f.divider))
        .unwrap_or(0.0);
    let body_end = c.children.len() - usize::from(trailing.is_some());
    let body_rect = egui::Rect::from_min_max(
        egui::pos2(stack_rect.min.x, stack_rect.min.y + bars_h),
        egui::pos2(stack_rect.max.x, stack_rect.max.y - footer_h),
    );
    // Apply the inset by shrinking the body rect rather than wrapping it in a
    // `Frame`: a frame's bottom margin is added *after* its content instead of
    // being withheld from the child's height, so a bottom-pinned child would
    // land flush against the footer band and get painted over by it.
    let inset = root_content_inset();
    let body_inner = body_rect.shrink2(egui::vec2(inset.left as f32, inset.top as f32));
    if body_inner.height() > 0.0 && body_end > leading {
        ui.scope_builder(egui::UiBuilder::new().max_rect(body_inner), |ui| {
            ui.set_clip_rect(body_inner);
            ui.set_min_height(body_inner.height());
            ui.set_max_height(body_inner.height());
            render_column_children(
                ui,
                env,
                &c.children[leading..body_end],
                c.gap,
                c.align,
                c.justify,
                out,
                0,
            );
        });
    }

    if let Some((footer_id, _)) = trailing {
        let footer_rect = egui::Rect::from_min_size(
            egui::pos2(stack_rect.min.x, stack_rect.max.y - footer_h),
            egui::vec2(stack_rect.width(), footer_h),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
            ui.set_clip_rect(footer_rect);
            render_node(ui, env, footer_id, out, 1);
        });
    }
    true
}

/// The host UI kit's standard content inset for a declarative plugin's root.
/// `SPACE_XL` horizontal matches the breathing room modal bodies get; the
/// slightly tighter `SPACE_MD` top/bottom keeps short plugins from feeling
/// bottom-heavy. All values come from `crate::ui::style` design tokens.
fn root_content_inset() -> egui::Margin {
    egui::Margin {
        left: style::SPACE_XL as i8,
        right: style::SPACE_XL as i8,
        top: style::SPACE_MD as i8,
        bottom: style::SPACE_MD as i8,
    }
}

/// A tree earns the default content inset unless it is a full-bleed pixel plugin:
/// a grow `Canvas` (games, visualizers) or a GPU `Surface` (video/3D output)
/// anywhere in the tree signals the plugin owns every pixel, so the host must not
/// inset it. A fixed-size `Canvas` is treated as ordinary flow content.
fn tree_wants_content_padding(nodes: &[IndexedNode]) -> bool {
    !nodes.iter().any(|n| match &n.data {
        UiNodeData::Canvas(c) => c.grow,
        UiNodeData::Surface(_) => true,
        _ => false,
    })
}

/// Headless render of a `UiTree` to PNG bytes via `egui_kittest`'s wgpu
/// offscreen backend — the same rasterization path `PlexiUiHarness` uses for
/// screenshot tests, reused here for `plexi view render --png` /
/// `plexi plugin check --png-dir` so the CLI renders exactly what the live host
/// would paint instead of hand-rolling a second rasterizer for widget chrome
/// (buttons, fonts, footer key chips) that has no flat-primitive form.
/// `pixels_per_point` is explicit: the CLI renders at 1.0; HiDPI screenshot
/// tests render at 2.0 so surface-resolution and pixel-grid regressions are
/// visible in the captured pixels.
pub fn render_ui_tree_to_png(
    tree: &UiTree,
    width: f32,
    height: f32,
    pixels_per_point: f32,
) -> Result<Vec<u8>, String> {
    let mut harness = headless_tree_harness(tree, width, height, pixels_per_point, None);
    harness.run();
    let img = harness
        .render()
        .map_err(|e| format!("offscreen render failed: {e}"))?;
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .map_err(|e| format!("PNG encode failed: {e}"))?;
    Ok(bytes)
}

/// Headless layout pass over a `UiTree`: the same widgets, fonts and pane size
/// as [`render_ui_tree_to_png`], returning what each node painted instead of
/// pixels. Nothing is rasterized, so it needs no GPU.
pub fn render_ui_tree_layout(tree: &UiTree, width: f32, height: f32) -> Vec<NodeLayout> {
    let sink = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut harness = headless_tree_harness(tree, width, height, 1.0, Some(sink.clone()));
    harness.run();
    drop(harness);
    let layout = sink.take();
    log::info!(
        "wasm_render: headless layout pass {width}x{height} measured {} node(s)",
        layout.len()
    );
    layout
}

type NodeLayoutSink = std::rc::Rc<std::cell::RefCell<Vec<NodeLayout>>>;

/// The harness both headless entry points share. With a `layout_sink`, every
/// pass overwrites it with that pass's node geometry, so after `run` it holds
/// the settled frame.
fn headless_tree_harness(
    tree: &UiTree,
    width: f32,
    height: f32,
    pixels_per_point: f32,
    layout_sink: Option<NodeLayoutSink>,
) -> egui_kittest::Harness<'static> {
    let config = crate::config::PlexiConfig::load();
    let colors = crate::ui::theme::colors_from_config(&config);

    let tree = tree.clone();
    // `set_fonts` only takes effect on the *next* frame's `begin_frame`, so the
    // custom `ui-medium` family used by declarative Button/ListRow nodes is not
    // bound during the frame that calls it. Apply fonts on a first, content-less
    // frame and render the tree only once they are live — otherwise laying out
    // any button label on the first frame panics in epaint. The live host does
    // not hit this because `setup_fonts` runs once at startup, long before any
    // plugin frame.
    let mut fonts_ready = false;
    egui_kittest::Harness::builder()
        .with_size(egui::Vec2::new(width, height))
        .with_pixels_per_point(pixels_per_point)
        .build_ui(move |ui| {
            if !fonts_ready {
                crate::ui::theme::setup_fonts(ui.ctx());
                crate::ui::theme::setup_style(ui.ctx(), &colors);
                fonts_ready = true;
                ui.ctx().request_repaint();
                return;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    let out = render_ui_tree_into(
                        ui,
                        &tree,
                        &colors,
                        None,
                        None,
                        None,
                        None,
                        0,
                        RenderResult {
                            node_paint: layout_sink.as_ref().map(|_| Vec::new()),
                            ..RenderResult::default()
                        },
                    );
                    if let (Some(sink), Some(layout)) = (&layout_sink, out.node_layout) {
                        sink.replace(layout);
                    }
                });
        })
}

fn rgba(c: &Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn canvas_align(align: Alignment) -> egui::Align2 {
    match align {
        Alignment::Start | Alignment::Stretch => egui::Align2::LEFT_TOP,
        Alignment::Center => egui::Align2::CENTER_CENTER,
        Alignment::End => egui::Align2::RIGHT_TOP,
    }
}

fn canvas_transform(
    rect: egui::Rect,
    width: f32,
    height: f32,
    fit: CanvasFit,
) -> (egui::Pos2, f32, f32) {
    let sx = if width > 0.0 {
        rect.width() / width
    } else {
        1.0
    };
    let sy = if height > 0.0 {
        rect.height() / height
    } else {
        1.0
    };
    if width <= 0.0 || height <= 0.0 || fit == CanvasFit::Fill {
        return (rect.min, sx, sy);
    }
    let scale = sx.min(sy);
    let content_size = egui::vec2(width * scale, height * scale);
    let origin = rect.center() - content_size / 2.0;
    (origin, scale, scale)
}

fn cross_align(a: Alignment) -> egui::Align {
    match a {
        Alignment::Start | Alignment::Stretch => egui::Align::Min,
        Alignment::Center => egui::Align::Center,
        Alignment::End => egui::Align::Max,
    }
}

/// True when `pending_click` (from `HostRequest::ClickPaneNode`/
/// `HostHarness::inject_node_click`) targets this frame's node `id` by its
/// arena id — the same honest match `PaneClickTarget::Pos` does by rect
/// containment, just keyed on identity instead of geometry.
fn node_click_matches(pending_click: Option<crate::host::pane::PendingPaneClick>, id: u32) -> bool {
    matches!(
        pending_click.map(|c| c.target),
        Some(crate::host::pane::PaneClickTarget::Node(n)) if n == u64::from(id)
    )
}

/// Maximum un-echoed `on_change` values tracked per `TextInput`. A guest that
/// stops answering must not grow this without bound; dropping the oldest entry
/// only costs an adoption of a value the user typed many keystrokes ago.
const TEXT_INPUT_PENDING_LIMIT: usize = 32;

/// Host-owned edit state for one `TextInput`, held in egui temp memory
/// (stints 0456, 0720).
#[derive(Clone, Debug, Default, PartialEq)]
struct TextInputEditState {
    /// The draft the user is editing. Painted, not `ti.value`.
    buf: String,
    /// The last `ti.value` adopted from or echoed by the guest. Used to tell a
    /// pre-echo guest frame (carrying the same pre-edit value the widget had
    /// before an in-flight `on_change` took effect) apart from an external
    /// plugin-side `SetState` reset.
    last_plugin_value: String,
    /// Generation of the committed guest tree this state last reconciled
    /// against. Equal generations mean the same tree is being repainted.
    seen_seq: u64,
    /// Values already sent to the guest through `on_change` that have not yet
    /// come back in a committed tree, oldest first.
    pending: VecDeque<String>,
    /// Set when `on_submit` fired: the guest is about to redefine the value
    /// (clear it, keep it, transform it), so the next genuinely new tree is
    /// authoritative — unless the user has typed since, in which case their
    /// fresh draft wins.
    awaiting_submit: bool,
}

/// Outcome of one reconcile, so the caller can log the lossy branch without
/// re-deriving it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TextInputReconcile {
    /// The local draft survived this frame.
    KeptDraft,
    /// The plugin's value replaced the local draft.
    Adopted,
}

/// Resolve a `TextInput`'s edit buffer for this frame (stints 0456, 0720).
///
/// The plugin's `value` is a controlled input that round-trips asynchronously: a
/// keystroke goes out as `on_change` and only comes back one or more guest
/// frames later. Meanwhile the host repaints the *last committed* tree every
/// frame, so the same stale `value` is presented over and over. Comparing
/// values alone cannot separate that stale repaint from a genuine plugin-side
/// reset, which is why typing faster than the guest's round trip used to drop
/// characters. Three signals fix it:
///
/// - `seq` — the committed tree's generation. Unchanged generation means we are
///   looking at a tree we already reconciled; it can never carry news, so the
///   draft is kept verbatim.
/// - `pending` — the values we sent out and are still waiting to see echoed. A
///   new tree whose value is one of them is our own echo (possibly several
///   keystrokes behind), so the draft is kept and the queue drained past it.
/// - `last_plugin_value` — the baseline guest value before our un-echoed edits. A
///   new tree that arrives while `pending` is non-empty and still carries that
///   unchanged baseline was rendered before our `on_change` took effect (e.g. a
///   startup resize or timer frame in flight), so the draft is kept.
///
/// Anything else on a new tree is genuine plugin news (`SetState`, a post-submit
/// reset) and is adopted.
///
/// Accepted residual: a plugin that *transforms* the value (uppercasing,
/// filtering characters) emits a tree value that was never in `pending`, so it
/// is adopted and the caret can jump to the end. Transforming controlled inputs
/// are not supported by this contract; a plugin that needs one must own its own
/// key handling.
fn reconcile_text_input_buffer(
    stored: Option<TextInputEditState>,
    plugin_value: &str,
    seq: u64,
) -> (TextInputEditState, TextInputReconcile) {
    let adopt = |mut state: TextInputEditState| {
        state.buf = plugin_value.to_string();
        state.last_plugin_value = plugin_value.to_string();
        state.seen_seq = seq;
        state.pending.clear();
        state.awaiting_submit = false;
        (state, TextInputReconcile::Adopted)
    };

    let Some(mut state) = stored else {
        return adopt(TextInputEditState::default());
    };

    // Same committed tree as last frame: no news, never clobber the draft.
    if state.seen_seq == seq {
        return (state, TextInputReconcile::KeptDraft);
    }
    state.seen_seq = seq;

    // A submit hands the value back to the plugin — unless the user has already
    // started a fresh draft in the gap, which outranks the plugin's answer.
    if state.awaiting_submit {
        state.awaiting_submit = false;
        if state.buf.is_empty() {
            return adopt(state);
        }
        if let Some(index) = state.pending.iter().position(|value| value == plugin_value) {
            state.pending.drain(..=index);
            state.last_plugin_value = plugin_value.to_string();
        }
        return (state, TextInputReconcile::KeptDraft);
    }

    // Our own echo, possibly lagging several keystrokes behind.
    if let Some(index) = state.pending.iter().position(|value| value == plugin_value) {
        state.pending.drain(..=index);
        state.last_plugin_value = plugin_value.to_string();
        return (state, TextInputReconcile::KeptDraft);
    }

    // A newly committed guest tree that still carries the exact baseline value
    // from before our un-echoed edits is a pre-echo frame (e.g. a resize or
    // timer render that was already in flight when the user typed), not an
    // plugin-pushed reset.
    if !state.pending.is_empty() && plugin_value == state.last_plugin_value {
        return (state, TextInputReconcile::KeptDraft);
    }

    adopt(state)
}

fn find_node(nodes: &[IndexedNode], id: u32) -> Option<&IndexedNode> {
    nodes
        .get(id as usize)
        .filter(|node| node.id == id)
        .or_else(|| nodes.iter().find(|node| node.id == id))
}

/// Stateful egui controls are keyed by the guest's stable key path rather
/// than its transport arena id. Keyed list reconciliation can therefore move
/// a node without losing its edit buffer, focus, or scroll state.
fn stable_node_id(ui: &egui::Ui, kind: &'static str, node: &IndexedNode) -> egui::Id {
    ui.id().with((kind, node.key.as_str()))
}

/// If `child_id` is a `Pinned{edge: Bottom}` wrapper around a `FooterKeys` node,
/// or is itself a bare trailing `FooterKeys` node, return the id to render and
/// its footer data. Used by `Column` to reserve a flush-bottom footer slot,
/// mirroring the legacy renderer's bottom-pin partition (components.rs
/// `render_stack`'s `StackDirection::Vertical` branch).
fn column_bottom_pin(nodes: &[IndexedNode], child_id: u32) -> Option<(u32, &FooterKeysNode)> {
    let node = find_node(nodes, child_id)?;
    match &node.data {
        UiNodeData::Pinned(p) if p.edge == PinnedEdge::Bottom => {
            let inner = find_node(nodes, p.child)?;
            match &inner.data {
                UiNodeData::FooterKeys(f) => Some((p.child, f)),
                _ => None,
            }
        }
        UiNodeData::FooterKeys(f) => Some((child_id, f)),
        _ => None,
    }
}

/// Indices of the children that are grow `Space` nodes, the flex gaps
/// `Spacer(grow=True)` compiles to.
fn grow_spacer_indices(nodes: &[IndexedNode], children: &[u32]) -> Vec<usize> {
    children
        .iter()
        .enumerate()
        .filter(|(_, id)| {
            matches!(find_node(nodes, **id).map(|n| &n.data), Some(UiNodeData::Space(s)) if s.grow)
        })
        .map(|(idx, _)| idx)
        .collect()
}

/// Where a column hands out its leftover height (stints 1327, 1358). Each
/// range is a flex gap: the grow `Space` child it covers, or an empty range
/// for a gap `justify` places before `children[range.start]`. Grow spacers
/// take all the leftover height, so `justify` only places children of a
/// column that has none.
fn column_flex_gaps(
    nodes: &[IndexedNode],
    children: &[u32],
    justify: Justify,
) -> Vec<std::ops::Range<usize>> {
    let spacers = grow_spacer_indices(nodes, children);
    if !spacers.is_empty() {
        return spacers.into_iter().map(|at| at..at + 1).collect();
    }
    let n = children.len();
    if n == 0 {
        return Vec::new();
    }
    match justify {
        Justify::Start => Vec::new(),
        Justify::Center => vec![0..0, n..n],
        Justify::End => std::iter::once(0..0).collect(),
        Justify::SpaceBetween => (1..n).map(|at| at..at).collect(),
    }
}

/// A row child that takes a share of the row's leftover width instead of
/// its content's width: a grow `Space`, or a `Column` or `Row` declared
/// `grow` (stint 1363).
fn grows_along_row(node: &IndexedNode) -> bool {
    match &node.data {
        UiNodeData::Space(s) => s.grow,
        UiNodeData::Column(c) => c.grow,
        UiNodeData::Row(r) => r.grow,
        _ => false,
    }
}

/// A row child that, laid out directly in the row, would take more width
/// than its content: a `Column` that centers or ends its children spreads
/// over all the width the row has left (stint 1363).
fn fills_row_width(node: &IndexedNode) -> bool {
    matches!(&node.data, UiNodeData::Column(c) if cross_align(c.align) != egui::Align::Min)
}

/// The widest gap a row or column honors. Gaps are guest input; a gap past
/// any pane width only pushes children out of sight.
const MAX_GAP: f32 = 4096.0;

/// A guest gap made safe for layout arithmetic: non-finite is zero, and
/// negative or huge values are clamped.
fn sanitized_gap(gap: f32) -> f32 {
    if gap.is_finite() {
        gap.clamp(0.0, MAX_GAP)
    } else {
        0.0
    }
}

/// How a row places its children along its width (stints 1358, 1363).
#[derive(Debug, PartialEq)]
struct RowPlacement {
    /// Space before the first child.
    lead: f32,
    /// Space between children.
    gap: f32,
    /// Slot width of each child.
    widths: Vec<f32>,
}

/// Place children of the given natural widths along a row `available`
/// wide; `None` is a grow child. Grow children share the leftover width
/// equally, whatever their content; without any, `justify` places the
/// children. `None` when the children do not fit, so the row lays them out
/// as it always has and wraps its text.
fn place_along_row(
    natural: &[Option<f32>],
    gap: f32,
    available: f32,
    justify: Justify,
) -> Option<RowPlacement> {
    let n = natural.len();
    if n == 0 || !available.is_finite() {
        return None;
    }
    let fixed = natural.iter().flatten().sum::<f32>() + gap * (n - 1) as f32;
    let leftover = available - fixed;
    if !leftover.is_finite() || leftover < 0.0 {
        return None;
    }
    let growers = natural.iter().filter(|w| w.is_none()).count();
    if growers > 0 {
        let share = leftover / growers as f32;
        return Some(RowPlacement {
            lead: 0.0,
            gap,
            widths: natural.iter().map(|w| w.unwrap_or(share)).collect(),
        });
    }
    let widths: Vec<f32> = natural.iter().flatten().copied().collect();
    let (lead, gap) = match justify {
        Justify::Start => (0.0, gap),
        Justify::Center => (leftover / 2.0, gap),
        Justify::End => (leftover, gap),
        Justify::SpaceBetween if n > 1 => (0.0, gap + leftover / (n - 1) as f32),
        Justify::SpaceBetween => (0.0, gap),
    };
    Some(RowPlacement { lead, gap, widths })
}

/// One info line the first time a layout rule applies at this place in the
/// tree, so the log shows which rule shaped a pane without a line per frame.
/// Measuring passes stay silent: they repeat the pass that paints.
fn trace_layout_once(ui: &egui::Ui, rule: &'static str, detail: impl FnOnce() -> String) {
    if ui.is_sizing_pass() {
        return;
    }
    let seen_id = ui.id().with(("plexi_layout_trace", rule));
    let seen = ui.ctx().memory_mut(|m| {
        let seen = m.data.get_temp::<bool>(seen_id).unwrap_or(false);
        m.data.insert_temp(seen_id, true);
        seen
    });
    if !seen {
        log::info!("wasm_render: {rule}: {}", detail());
    }
}

/// Greedy line breaking for a row that wraps whole children (stint 1333):
/// each line takes children until the next one, plus the gap before it, would
/// pass `available`. A child wider than the row still gets a line of its own.
fn pack_row_lines(widths: &[f32], gap: f32, available: f32) -> Vec<std::ops::Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0.0;
    for (idx, width) in widths.iter().enumerate() {
        if idx > start && used + gap + width > available {
            lines.push(start..idx);
            start = idx;
            used = 0.0;
        }
        used += if idx > start { gap + width } else { *width };
    }
    if start < widths.len() {
        lines.push(start..widths.len());
    }
    lines
}

const KEYPAD_BUTTON_W: f32 = 64.0;
const KEYPAD_BUTTON_H: f32 = 42.0;

fn is_button_row(nodes: &[IndexedNode], row_id: u32) -> Option<(usize, f32)> {
    let Some(UiNodeData::Row(r)) = find_node(nodes, row_id).map(|n| &n.data) else {
        return None;
    };
    if r.children.is_empty() {
        return None;
    }
    let all_buttons = r.children.iter().all(|cid| {
        matches!(
            find_node(nodes, *cid).map(|n| &n.data),
            Some(UiNodeData::Button(_))
        )
    });
    all_buttons.then_some((r.children.len(), r.gap))
}

fn column_button_grid_spec(nodes: &[IndexedNode], col_id: u32) -> Option<(usize, f32)> {
    let Some(UiNodeData::Column(c)) = find_node(nodes, col_id).map(|n| &n.data) else {
        return None;
    };
    if c.children.len() < 2 {
        return None;
    }
    let mut max_cols = 0usize;
    let mut row_gap = 0.0f32;
    for (idx, &child_id) in c.children.iter().enumerate() {
        let (cols, gap) = is_button_row(nodes, child_id)?;
        max_cols = max_cols.max(cols);
        if idx == 0 {
            row_gap = gap;
        }
    }
    (max_cols >= 2).then(|| {
        let width =
            max_cols as f32 * KEYPAD_BUTTON_W + (max_cols.saturating_sub(1)) as f32 * row_gap;
        (max_cols, width)
    })
}

fn row_parent_button_grid_cols(nodes: &[IndexedNode], row_id: u32) -> Option<usize> {
    let parent = nodes.iter().find(|n| match &n.data {
        UiNodeData::Column(c) => c.children.contains(&row_id),
        _ => false,
    })?;
    column_button_grid_spec(nodes, parent.id).map(|(cols, _)| cols)
}

fn card_keypad_width(nodes: &[IndexedNode], card_child_id: u32) -> Option<f32> {
    if let Some((_, w)) = column_button_grid_spec(nodes, card_child_id) {
        return Some(w);
    }
    let Some(UiNodeData::Column(c)) = find_node(nodes, card_child_id).map(|n| &n.data) else {
        return None;
    };
    c.children
        .iter()
        .find_map(|&cid| column_button_grid_spec(nodes, cid).map(|(_, w)| w))
}

fn subtree_contains(nodes: &[IndexedNode], root_id: u32, target_id: u32) -> bool {
    if root_id == target_id {
        return true;
    }
    let Some(node) = find_node(nodes, root_id) else {
        return false;
    };
    match &node.data {
        UiNodeData::Row(r) => r
            .children
            .iter()
            .any(|&c| subtree_contains(nodes, c, target_id)),
        UiNodeData::Column(c) => c
            .children
            .iter()
            .any(|&c| subtree_contains(nodes, c, target_id)),
        UiNodeData::ListView(l) => l
            .items
            .iter()
            .any(|&c| subtree_contains(nodes, c, target_id)),
        UiNodeData::Scroll(s) => subtree_contains(nodes, s.child, target_id),
        UiNodeData::Padding(p) => subtree_contains(nodes, p.child, target_id),
        UiNodeData::Pinned(p) => subtree_contains(nodes, p.child, target_id),
        _ => false,
    }
}

fn is_nested_card(nodes: &[IndexedNode], card_id: u32) -> bool {
    nodes.iter().any(|n| {
        n.id != card_id
            && n.key.starts_with(CARD_NODE_KEY_PREFIX)
            && match &n.data {
                UiNodeData::Padding(p) => subtree_contains(nodes, p.child, card_id),
                _ => false,
            }
    })
}

/// Width a plugin button needs to paint `label` on one line.
fn button_one_line_width(ui: &egui::Ui, label: &str, min_size: egui::Vec2, colors: &Colors) -> f32 {
    crate::ui::button::KitButton::new(label, colors)
        .min_size(min_size)
        .natural_width(ui)
}

fn render_button_node(
    ui: &mut egui::Ui,
    id: u32,
    b: &super::wasm_view::bindings::plexi::platform::types::ButtonNode,
    min_size: egui::Vec2,
    colors: &Colors,
    out: &mut RenderResult,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
) {
    use crate::ui::button::{ButtonKind, KitButton};
    let kind = match b.style {
        // The SDK encodes selected state for Checkbox/Toggle/Radio/Segmented/Tabs
        // as primary vs secondary (sdk/python/plexi_sdk/ui.py); Primary is the
        // accent-filled kind, visibly distinct from Secondary.
        ButtonStyle::Primary => ButtonKind::Primary,
        ButtonStyle::Secondary => ButtonKind::Secondary,
        ButtonStyle::Danger => ButtonKind::Danger,
        ButtonStyle::Ghost => ButtonKind::Ghost,
    };
    let synthetic_click = !b.disabled && node_click_matches(pending_click, id);
    let add_button = |ui: &mut egui::Ui| {
        ui.add_enabled(
            !b.disabled,
            KitButton::new(&b.label, colors)
                .kind(kind)
                .min_size(min_size)
                .wrap_mode(egui::TextWrapMode::Wrap),
        )
    };
    // A label may wrap between words, never inside one (stint 1333): egui
    // breaks a word that is wider than the space it is given, so a button
    // squeezed by its row siblings painted one letter per line. Give the
    // button the width of its longest word and let it overflow the row.
    let available = ui.available_width();
    let one_line_width = |label: &str| button_one_line_width(ui, label, min_size, colors);
    let word_width = if one_line_width(&b.label) <= available {
        0.0
    } else {
        b.label
            .split_whitespace()
            .map(one_line_width)
            .fold(0.0, f32::max)
    };
    let response = if word_width > available {
        ui.allocate_ui_with_layout(egui::vec2(word_width, min_size.y), *ui.layout(), add_button)
            .inner
    } else {
        add_button(ui)
    };
    if response.clicked() || synthetic_click {
        out.actions.push(b.on_click.clone());
    }
}

/// Lay out a column's children, sharing the leftover height between its grow
/// spacers (stints 0674 and 1327).
///
/// A grow `Space` is a flex gap, not a widget. Rendered in flow it would be
/// `ui.add_space(available_height())`, which consumes every remaining pixel and
/// pushes each later child past the clip edge. The column is split at its grow
/// spacers instead: every span after the first is measured with a sizing pass,
/// the height left over is divided equally between the spacers, and each span
/// is rendered top-down in its own rect. The last span is anchored to the
/// bottom edge, so one spacer is a bottom pin and `[spacer, child, spacer]`
/// centers the child. When the spans are taller than the column the spacers
/// collapse to zero and the last span still holds the bottom edge.
///
/// `justify` (stint 1358) places the children of a column without grow
/// spacers the same way, as if it held spacers: `center` one before and one
/// after the children, `end` one before them, `space-between` one between
/// each pair. `start` places them top-down, as columns always have.
#[allow(clippy::too_many_arguments)]
fn render_column_children(
    ui: &mut egui::Ui,
    env: TreeRenderEnv<'_>,
    children: &[u32],
    gap: f32,
    align: Alignment,
    justify: Justify,
    out: &mut RenderResult,
    depth: u32,
) {
    let render_span = |ui: &mut egui::Ui, span: &[u32], out: &mut RenderResult| {
        ui.spacing_mut().item_spacing.y = gap;
        for &child in span {
            render_node(ui, env, child, out, depth + 1);
        }
    };

    let gaps = column_flex_gaps(env.nodes, children, justify);
    let has_spacer = gaps.iter().any(|gap| !gap.is_empty());
    // A justified column fills the height it is offered; a column of
    // unbounded height (inside a vertical `Scrollable`) has none to place
    // its children in, so it stacks them from the top.
    let Some(first_gap) = gaps
        .first()
        .filter(|_| has_spacer || ui.available_height().is_finite())
    else {
        ui.with_layout(egui::Layout::top_down(cross_align(align)), |ui| {
            render_span(ui, children, out)
        });
        return;
    };
    if !has_spacer {
        trace_layout_once(ui, "column justify places children down the column", || {
            format!("justify={justify:?} children={}", children.len())
        });
    }

    let stack_size = egui::vec2(ui.available_width(), ui.available_height());
    let (stack_rect, _) = ui.allocate_exact_size(stack_size, egui::Sense::hover());
    // A column with a grow spacer always takes the whole rect it is offered,
    // so a measuring pass has its size already. Laying its children out there
    // would measure them again inside every enclosing measurement: two visits
    // per level, doubling with each level of nesting.
    if ui.is_sizing_pass() {
        return;
    }

    let head = &children[..first_gap.start];
    ui.scope_builder(egui::UiBuilder::new().max_rect(stack_rect), |ui| {
        ui.set_clip_rect(stack_rect);
        ui.with_layout(egui::Layout::top_down(cross_align(align)), |ui| {
            render_span(ui, head, out)
        });
    });

    // The spans that follow each flex gap.
    let spans: Vec<&[u32]> = gaps
        .iter()
        .enumerate()
        .map(|(n, gap)| {
            let end = gaps.get(n + 1).map_or(children.len(), |next| next.start);
            &children[gap.end..end]
        })
        .collect();
    if spans.iter().all(|span| span.is_empty()) {
        return;
    }

    // Measure, then render top-down in a placed rect. A `bottom_up` layout
    // cannot be used directly: a `Row` child is an `ui.horizontal`, whose own
    // ui grows *downward* from wherever bottom-up seated it, so the row
    // overflows the column's bottom edge and its lower half is clipped away.
    // The sizing pass lays a span out without painting, which gives an exact
    // height with no frame of lag.
    let measure = |ui: &mut egui::Ui, span: &[u32]| {
        if span.is_empty() {
            return 0.0;
        }
        let mut sizing = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(stack_rect)
                .sizing_pass()
                .invisible()
                .layout(egui::Layout::top_down(cross_align(align))),
        );
        let mut sizing_out = RenderResult::default();
        render_span(&mut sizing, span, &mut sizing_out);
        sizing.min_rect().height()
    };
    // With one spacer the tail is anchored to the bottom edge and the head's
    // height is never needed, so the head is not laid out a second time.
    let head_h = if spans.len() > 1 {
        measure(ui, head)
    } else {
        0.0
    };
    let heights: Vec<f32> = spans.iter().map(|span| measure(ui, span)).collect();
    let leftover = stack_rect.height() - head_h - heights.iter().sum::<f32>();
    let share = (leftover / spans.len() as f32).max(0.0);
    if spans.len() > 1 {
        trace_layout_once(
            ui,
            "column shares leftover height between grow spacers",
            || {
                format!(
                    "spacers={} column_h={:.0} leftover={leftover:.0} share={share:.0}",
                    spans.len(),
                    stack_rect.height()
                )
            },
        );
    }

    let mut cursor = stack_rect.min.y + head_h;
    for (n, (span, &measured)) in spans.iter().zip(&heights).enumerate() {
        cursor += share;
        let top = cursor;
        cursor += measured;
        if span.is_empty() {
            continue;
        }
        let height = measured.min(stack_rect.height());
        let top = if n + 1 == spans.len() {
            stack_rect.max.y - height
        } else {
            top
        };
        let span_rect = egui::Rect::from_min_max(
            egui::pos2(stack_rect.min.x, top),
            egui::pos2(stack_rect.max.x, top + height),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(span_rect), |ui| {
            ui.set_clip_rect(span_rect.intersect(stack_rect));
            ui.with_layout(egui::Layout::top_down(cross_align(align)), |ui| {
                render_span(ui, span, out)
            });
        });
    }
}

/// Font size of a `Text` node. The size is guest input and is not validated
/// when the tree is decoded: a NaN or infinite size falls back to the body
/// size and any other is clamped to a range a font can be laid out at.
fn text_size(t: &TextNode) -> f32 {
    const MIN_TEXT_SIZE: f32 = 1.0;
    const MAX_TEXT_SIZE: f32 = 512.0;
    t.size
        .filter(|size| size.is_finite())
        .map_or(style::TEXT_BODY, |size| {
            size.clamp(MIN_TEXT_SIZE, MAX_TEXT_SIZE)
        })
}

/// The styled text of a `Text` node. `text` is the node's own text, or one
/// word of it when a caller measures how narrow the node can wrap.
fn text_rich(
    ui: &egui::Ui,
    node: &IndexedNode,
    t: &TextNode,
    text: &str,
    colors: &Colors,
) -> RichText {
    let default_color = TextRole::from_key(&node.key)
        .map(|role| role.preferred(colors))
        .or(ui.visuals().override_text_color)
        .unwrap_or(colors.text_primary);
    let mut rich = RichText::new(text).size(text_size(t)).color(default_color);
    if t.bold {
        rich = rich.strong();
    }
    if let Some(c) = &t.color {
        rich = rich.color(rgba(c));
    }
    rich
}

/// Width and height a row's `Text` child wraps to when it would run past the
/// row's edge (stint 1333), or `None` when it is laid out as usual: it fits on
/// one line, it truncates, or the width left is narrower than its longest
/// word. `reserved` is the width the siblings after it need.
fn row_text_wrap_size(
    ui: &egui::Ui,
    env: TreeRenderEnv<'_>,
    node: &IndexedNode,
    t: &TextNode,
    reserved: f32,
) -> Option<egui::Vec2> {
    let galley = |text: &str, mode: egui::TextWrapMode, width: f32| {
        egui::WidgetText::from(text_rich(ui, node, t, text, env.colors)).into_galley(
            ui,
            Some(mode),
            width,
            egui::FontSelection::Default,
        )
    };
    let one_line = |text: &str| {
        galley(text, egui::TextWrapMode::Extend, f32::INFINITY)
            .size()
            .x
    };
    let wrap_width = ui.available_width() - reserved;
    let longest_word = t.text.split_whitespace().map(one_line).fold(0.0, f32::max);
    if wrap_width < longest_word {
        return None;
    }
    let height = galley(&t.text, egui::TextWrapMode::Wrap, wrap_width)
        .size()
        .y;
    Some(egui::vec2(wrap_width, height))
}

/// True when a row's `Text` child is too long for the width left in the row.
fn row_text_overflows(
    ui: &egui::Ui,
    env: TreeRenderEnv<'_>,
    node: &IndexedNode,
    t: &TextNode,
) -> bool {
    let available = ui.available_width();
    if t.truncate || !available.is_finite() {
        return false;
    }
    egui::WidgetText::from(text_rich(ui, node, t, &t.text, env.colors))
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::FontSelection::Default,
        )
        .size()
        .x
        > available
}

/// What one measuring pass over a row found: its height and the rect each
/// child took, in the measuring pass's own coordinates.
struct RowMeasure {
    height: f32,
    child_rects: Vec<egui::Rect>,
}

impl RowMeasure {
    /// A row measurement from each child's natural size, the children laid
    /// side by side `gap` apart; a grow child (`None` width) takes no width.
    fn from_sizes(widths: &[Option<f32>], sizes: &[egui::Vec2], gap: f32, height: f32) -> Self {
        let mut x = 0.0;
        let child_rects = widths
            .iter()
            .zip(sizes)
            .map(|(width, size)| {
                let size = egui::vec2(width.unwrap_or(0.0), size.y);
                let rect = egui::Rect::from_min_size(egui::pos2(x, 0.0), size);
                x += size.x + gap;
                rect
            })
            .collect();
        Self {
            height,
            child_rects,
        }
    }

    /// Width the children after `idx` need, including the gap before them.
    fn reserved_after(&self, idx: usize, gap: f32) -> f32 {
        match (self.child_rects.get(idx + 1), self.child_rects.last()) {
            (Some(next), Some(last)) => last.max.x - next.min.x + gap,
            _ => 0.0,
        }
    }
}

/// One child of a row: a keypad cell when the row belongs to a button grid,
/// any other node through `render_node`.
fn render_row_child(
    ui: &mut egui::Ui,
    env: TreeRenderEnv<'_>,
    r: &RowNode,
    idx: usize,
    grid_cols: Option<usize>,
    out: &mut RenderResult,
    depth: u32,
) {
    let child = r.children[idx];
    if let Some(cols) = grid_cols {
        if let Some(UiNodeData::Button(b)) = find_node(env.nodes, child).map(|n| &n.data) {
            let span = if idx == 0 && r.children.len() < cols {
                cols - r.children.len() + 1
            } else {
                1
            };
            let btn_w = span as f32 * KEYPAD_BUTTON_W + (span.saturating_sub(1)) as f32 * r.gap;
            render_button_node(
                ui,
                child,
                b,
                egui::vec2(btn_w, KEYPAD_BUTTON_H),
                env.colors,
                out,
                env.pending_click,
            );
            return;
        }
    }
    render_node(ui, env, child, out, depth + 1);
}

/// Lay a row out once without painting. Rows inside this pass do not measure
/// and flex columns are not laid out, so it visits each node of the row's
/// subtree at most once.
fn measure_row(
    ui: &mut egui::Ui,
    env: TreeRenderEnv<'_>,
    r: &RowNode,
    grid_cols: Option<usize>,
    depth: u32,
) -> RowMeasure {
    let mut sizing = ui.new_child(
        egui::UiBuilder::new()
            .sizing_pass()
            .invisible()
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
    );
    sizing.spacing_mut().item_spacing.x = r.gap;
    let env = TreeRenderEnv {
        in_row_measure: true,
        ..env
    };
    let mut sizing_out = RenderResult::default();
    let child_rects = (0..r.children.len())
        .map(|idx| {
            sizing
                .scope(|ui| render_row_child(ui, env, r, idx, grid_cols, &mut sizing_out, depth))
                .response
                .rect
        })
        .collect();
    RowMeasure {
        height: sizing.min_rect().height(),
        child_rects,
    }
}

/// Natural size of each child of a row that hands out its leftover width
/// (stint 1363). Each child is laid out alone, without painting, in a
/// measuring pass as wide as the row, so its width is its content's whatever
/// its siblings take: a centered `Column` measures as wide as its widest
/// child, not as the row. A grow child's width is a share of the leftover
/// instead, but its height is still measured. Like `measure_row`, rows
/// inside this pass do not measure and flex columns are not laid out, so it
/// visits each node of the row's subtree at most once.
fn measure_row_children(
    ui: &mut egui::Ui,
    env: TreeRenderEnv<'_>,
    r: &RowNode,
    grid_cols: Option<usize>,
    depth: u32,
) -> Vec<egui::Vec2> {
    let env = TreeRenderEnv {
        in_row_measure: true,
        ..env
    };
    let rect = ui.available_rect_before_wrap();
    let mut sizing_out = RenderResult::default();
    r.children
        .iter()
        .enumerate()
        .map(|(idx, _)| {
            let mut sizing = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .sizing_pass()
                    .invisible()
                    .layout(egui::Layout::left_to_right(egui::Align::Min)),
            );
            render_row_child(&mut sizing, env, r, idx, grid_cols, &mut sizing_out, depth);
            sizing.min_rect().size()
        })
        .collect()
}

/// A horizontal run of a row. `egui` aligns each child against the height the
/// run has when the child is added, so a run that places its children at the
/// top or bottom is given the measured row height to place them in.
fn row_line<R>(
    ui: &mut egui::Ui,
    valign: egui::Align,
    height: Option<f32>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let size = egui::vec2(
        ui.available_size_before_wrap().x,
        height.unwrap_or(ui.spacing().interact_size.y),
    );
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(valign), add_contents)
        .inner
}

/// Lay out a `Row` (stints 1327 and 1333).
///
/// `align` is the cross axis, as it is on a `Column`: it places children at
/// the top, the center or the bottom of the row. A row made only of buttons
/// (`Actions`, `Select`, `Tabs`) wraps whole buttons onto further lines when
/// they do not fit; keypad grids keep their fixed cells. A `Text` child too
/// long for the row wraps in the width the children after it leave.
fn render_row_node(
    ui: &mut egui::Ui,
    env: TreeRenderEnv<'_>,
    id: u32,
    r: &RowNode,
    out: &mut RenderResult,
    depth: u32,
) {
    let nodes = env.nodes;
    let grid_cols = row_parent_button_grid_cols(nodes, id);
    let available = ui.available_width();
    let valign = cross_align(r.align);
    let may_measure = !env.in_row_measure && env.row_depth < MAX_MEASURED_ROW_DEPTH;
    let env = TreeRenderEnv {
        row_depth: env.row_depth.saturating_add(1),
        ..env
    };

    if grid_cols.is_none() && is_button_row(nodes, id).is_some() {
        let widths: Vec<f32> = r
            .children
            .iter()
            .map(|child| match find_node(nodes, *child).map(|n| &n.data) {
                Some(UiNodeData::Button(b)) => button_one_line_width(
                    ui,
                    &b.label,
                    egui::vec2(style::BUTTON_H_MD, style::BUTTON_H_MD),
                    env.colors,
                ),
                _ => 0.0,
            })
            .collect();
        let lines = pack_row_lines(&widths, r.gap, available);
        if lines.len() > 1 {
            trace_layout_once(ui, "button row wraps whole buttons", || {
                format!(
                    "row={id} buttons={} lines={} available={available:.0}",
                    widths.len(),
                    lines.len()
                )
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = r.gap;
                for line in lines {
                    // Each line is placed along the row by `justify`.
                    let natural: Vec<Option<f32>> =
                        widths[line.clone()].iter().copied().map(Some).collect();
                    let placement =
                        place_along_row(&natural, sanitized_gap(r.gap), available, r.justify);
                    // Buttons on a line share one height, so `align` has
                    // nothing to place.
                    row_line(ui, egui::Align::Center, None, |ui| {
                        match &placement {
                            Some(placement) => {
                                ui.spacing_mut().item_spacing.x = placement.gap;
                                ui.add_space(placement.lead);
                            }
                            None => ui.spacing_mut().item_spacing.x = r.gap,
                        }
                        for child in &r.children[line] {
                            render_node(ui, env, *child, out, depth + 1);
                        }
                    });
                }
            });
            return;
        }
    }

    // One measuring pass per row at most, and only when the row needs it: to
    // hand out its leftover width, to place children at its top or bottom
    // edge, or to wrap a long text. A row that may not measure centers its
    // children, as rows always have.
    let mut measure: Option<RowMeasure> = None;
    let placed = may_measure && valign != egui::Align::Center;
    let along = may_measure
        && available.is_finite()
        && (r.justify != Justify::Start
            || r.children.iter().any(|child| {
                find_node(nodes, *child).is_some_and(|n| grows_along_row(n) || fills_row_width(n))
            }));
    if along {
        let natural = measure_row_children(ui, env, r, grid_cols, depth);
        let widths: Vec<Option<f32>> = r
            .children
            .iter()
            .zip(&natural)
            .map(|(child, size)| {
                let grows = find_node(nodes, *child).is_some_and(grows_along_row);
                (!grows).then_some(size.x)
            })
            .collect();
        let row_h = natural.iter().map(|s| s.y).fold(0.0, f32::max);
        if let Some(placement) =
            place_along_row(&widths, sanitized_gap(r.gap), available, r.justify)
        {
            trace_layout_once(ui, "row places children along the row", || {
                format!(
                    "row={id} justify={:?} lead={:.0} gap={:.0} widths={:?}",
                    r.justify, placement.lead, placement.gap, placement.widths
                )
            });
            let line_align = if placed { valign } else { egui::Align::Center };
            row_line(ui, line_align, placed.then_some(row_h), |ui| {
                ui.spacing_mut().item_spacing.x = placement.gap;
                ui.add_space(placement.lead);
                for (idx, &width) in placement.widths.iter().enumerate() {
                    // Each child gets a slot exactly its share wide and lays
                    // itself out centered inside it, as in a measured slot.
                    // A grow `Space` paints nothing in its slot.
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, natural[idx].y),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(width);
                            render_row_child(ui, env, r, idx, grid_cols, out, depth);
                        },
                    );
                }
            });
            return;
        }
        // The children do not fit: lay the row out as rows always have, from
        // this measurement, so it is still measured once.
        measure = Some(RowMeasure::from_sizes(&widths, &natural, r.gap, row_h));
    }
    if placed && measure.is_none() {
        let measured = measure_row(ui, env, r, grid_cols, depth);
        trace_layout_once(ui, "row align places children across the row", || {
            format!("row={id} align={:?} row_h={:.0}", r.align, measured.height)
        });
        measure = Some(measured);
    }
    let height = measure.as_ref().map(|m| m.height);
    let line_align = if placed { valign } else { egui::Align::Center };
    row_line(ui, line_align, height, |ui| {
        ui.spacing_mut().item_spacing.x = r.gap;
        for (idx, child) in r.children.iter().enumerate() {
            let text = find_node(nodes, *child).and_then(|n| match &n.data {
                UiNodeData::Text(t) => Some((n, t)),
                _ => None,
            });
            if let Some((node, t)) = text {
                if may_measure && row_text_overflows(ui, env, node, t) {
                    let reserved = measure
                        .get_or_insert_with(|| measure_row(ui, env, r, grid_cols, depth))
                        .reserved_after(idx, r.gap);
                    if let Some(size) = row_text_wrap_size(ui, env, node, t, reserved) {
                        trace_layout_once(ui, "row text wraps at the row edge", || {
                            format!(
                                "node={child} wrap_width={:.0} reserved={reserved:.0}",
                                size.x
                            )
                        });
                        // A vertical child scope as wide as the wrap width:
                        // the `Text` arm wraps there as it does in a column.
                        ui.allocate_ui_with_layout(
                            size,
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_max_width(size.x);
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                render_node(ui, env, *child, out, depth + 1);
                            },
                        );
                        continue;
                    }
                }
            }
            // A grow child that did not fit has no measured slot.
            let grows = find_node(nodes, *child).is_some_and(grows_along_row);
            let slot = measure
                .as_ref()
                .filter(|_| placed && !grows)
                .and_then(|m| m.child_rects.get(idx))
                .map(|rect| rect.size());
            match slot {
                // The row places the child's measured slot at its top or
                // bottom; inside the slot the child lays itself out centered,
                // so a button's label stays in the middle of the button.
                Some(size) => {
                    ui.allocate_ui_with_layout(
                        size,
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| render_row_child(ui, env, r, idx, grid_cols, out, depth),
                    );
                }
                None => render_row_child(ui, env, r, idx, grid_cols, out, depth),
            }
        }
    });
}

fn render_node(
    ui: &mut egui::Ui,
    env: TreeRenderEnv<'_>,
    id: u32,
    out: &mut RenderResult,
    depth: u32,
) {
    #[cfg(test)]
    RENDER_NODE_CALLS.with(|calls| calls.set(calls.get() + 1));
    let layout_start = node_paint_start(ui, out);
    render_node_inner(ui, env, id, out, depth);
    record_node_paint(ui, out, id, layout_start);
}

/// Where a node's paint span starts, when this pass records node geometry.
fn node_paint_start(ui: &egui::Ui, out: &RenderResult) -> Option<usize> {
    out.node_paint.is_some().then(|| paint_len(ui))
}

/// Close the paint span `node_paint_start` opened for node `id`.
fn record_node_paint(ui: &egui::Ui, out: &mut RenderResult, id: u32, start: Option<usize>) {
    if let (Some(start), Some(spans)) = (start, out.node_paint.as_mut()) {
        spans.push(NodePaintSpan {
            id,
            shapes: start..paint_len(ui),
            clip: ui.clip_rect(),
        });
    }
}

fn render_node_inner(
    ui: &mut egui::Ui,
    env: TreeRenderEnv<'_>,
    id: u32,
    out: &mut RenderResult,
    depth: u32,
) {
    let TreeRenderEnv {
        nodes,
        colors,
        surface,
        canvas_fits,
        pending_click,
        surface_key,
        tree_seq,
        row_depth: _,
        in_row_measure: _,
    } = env;
    if depth > MAX_DEPTH {
        return;
    }
    let Some(node) = find_node(nodes, id) else {
        return;
    };

    let authored_start = match &node.data {
        UiNodeData::Canvas(_) => Some(paint_len(ui)),
        UiNodeData::Text(text) if text.color.is_some() => Some(paint_len(ui)),
        _ => None,
    };

    match &node.data {
        UiNodeData::Empty => {}

        UiNodeData::Text(t) => {
            let rich = text_rich(ui, node, t, &t.text, colors);
            let add_label = |ui: &mut egui::Ui| {
                let mut label = egui::Label::new(rich);
                if t.truncate {
                    label = label.truncate();
                }
                ui.add(label);
            };
            if !ui.layout().is_horizontal() {
                match t.align {
                    Alignment::Center => {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), add_label);
                    }
                    Alignment::End => {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Max), add_label);
                    }
                    Alignment::Start | Alignment::Stretch => {
                        add_label(ui);
                    }
                }
            } else {
                add_label(ui);
            }
        }

        UiNodeData::Button(b) => {
            render_button_node(
                ui,
                id,
                b,
                egui::vec2(style::BUTTON_H_MD, style::BUTTON_H_MD),
                colors,
                out,
                pending_click,
            );
        }

        UiNodeData::TextInput(ti) => {
            // Host-owned edit buffer (stint 0456): the plugin's `value` is a
            // controlled input that round-trips asynchronously (on_change →
            // guest update → SetState → next tree), so painting `ti.value`
            // directly would clobber keystrokes typed while the echo is in
            // flight. The buffer lives in egui temp memory keyed by the
            // widget id; a plugin-pushed value change (reset after submit,
            // external SetState) still wins over any local draft.
            let widget_id = stable_node_id(ui, "l1_text_input", node);
            let state_id = widget_id.with("edit_state");
            let stored: Option<TextInputEditState> =
                ui.ctx().memory_mut(|m| m.data.get_temp(state_id));
            let previous_draft = stored.as_ref().map(|s| s.buf.clone());
            let (mut state, outcome) = reconcile_text_input_buffer(stored, &ti.value, tree_seq);
            if outcome == TextInputReconcile::Adopted {
                if let Some(draft) = previous_draft.filter(|d| !d.is_empty() && *d != ti.value) {
                    log::info!(
                        "l1_text_input: adopting plugin value node={id} plugin={:?} discarding draft={draft:?}",
                        ti.value
                    );
                }
            }
            let mut buf = std::mem::take(&mut state.buf);
            let resp = crate::ui::text_field::TextField::singleline(widget_id, &ti.placeholder)
                .password(ti.password)
                .log_name("l1_text_input")
                .show(ui, &mut buf, colors);
            if let Some(key) = surface_key {
                // `autofocus` declares this field the pane's default text
                // surface: the reconciler focuses it when the pane owns input
                // and holds no focused surface, so a plugin that reveals an
                // entry form gets a live cursor with no imperative focus
                // command and no key-routing code of its own (stint 0674).
                if ti.autofocus {
                    // Edge-triggered: one line the first time this widget id
                    // claims the pane default, so a duplicate registration for
                    // one node in one frame is visible in the log.
                    let logged_id = resp.id.with("plexi_autofocus_logged");
                    let already: bool = ui.ctx().memory_mut(|m| {
                        let seen = m.data.get_temp::<bool>(logged_id).unwrap_or(false);
                        m.data.insert_temp(logged_id, true);
                        seen
                    });
                    if !already {
                        log::info!(
                            "l1_text_input: autofocus claiming pane default surface pane={key:?} id={:?}",
                            resp.id
                        );
                    }
                    crate::ui::focus::register_default_text_surface(ui.ctx(), key, resp.id);
                } else {
                    crate::ui::focus::register_text_surface(ui.ctx(), key, resp.id);
                }
                // Node-targeted clicks focus the field; once focused, the
                // dispatch gate (`focused_pane_text_surface`, stint 0456)
                // routes keystrokes here instead of the plugin's KeyEvent path.
                // The claim routes through the reconciler (stint 0429), which
                // grants it while this pane owns input.
                if node_click_matches(pending_click, id) {
                    crate::ui::focus::claim_text_surface(ui.ctx(), key, resp.id);
                }
            }
            if resp.changed() {
                out.value_changes.push((ti.on_change.clone(), buf.clone()));
                // Remember what we sent so the guest's echo — however many
                // keystrokes behind it arrives — is recognized as ours and
                // never mistaken for a plugin-pushed reset.
                if state.pending.len() >= TEXT_INPUT_PENDING_LIMIT {
                    state.pending.pop_front();
                }
                state.pending.push_back(buf.clone());
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                out.actions.push(ti.on_submit.clone());
                // Submitting hands the value back to the plugin, so the draft
                // ends here: clearing it locally means a character typed
                // before the guest's cleared tree arrives starts the next
                // entry instead of re-extending the submitted one. If the plugin
                // keeps its value instead of clearing it (a search field), the
                // next committed tree is adopted and restores it.
                buf.clear();
                state.last_plugin_value.clear();
                state.pending.clear();
                state.awaiting_submit = true;
                // Keep focus for consecutive entries — Enter submits, it
                // doesn't dismiss the field. Escape leaves the field via
                // egui's native focus surrender in `begin_pass`; the
                // dispatch gate swallows that same-frame Escape so it
                // can't fire the ViewActive CloseView binding.
                if let Some(key) = surface_key {
                    crate::ui::focus::claim_text_surface(ui.ctx(), key, resp.id);
                }
            }
            state.buf = buf;
            ui.ctx().memory_mut(|m| m.data.insert_temp(state_id, state));
        }

        UiNodeData::Row(r) => render_row_node(ui, env, id, r, out, depth),

        UiNodeData::Column(c) => {
            if let Some(table) = parse_table_node_key(&node.key) {
                render_data_table_node(ui, &table, id, &c.children, colors, out, pending_click);
                return;
            }
            let footer = c
                .children
                .last()
                .copied()
                .and_then(|last_id| column_bottom_pin(nodes, last_id));
            if let Some((footer_id, footer_data)) = footer {
                let footer_h =
                    view_chrome::footer_keys_height(ui, &footer_data.entries, footer_data.divider);
                let stack_size = egui::vec2(ui.available_width(), ui.available_height());
                let (stack_rect, _) = ui.allocate_exact_size(stack_size, egui::Sense::hover());
                let body_h = (stack_rect.height() - footer_h).max(0.0);
                let body_rect = egui::Rect::from_min_size(
                    stack_rect.min,
                    egui::vec2(stack_rect.width(), body_h),
                );

                ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                    ui.set_clip_rect(body_rect);
                    ui.set_min_height(body_h);
                    ui.set_max_height(body_h);
                    render_column_children(
                        ui,
                        env,
                        &c.children[..c.children.len() - 1],
                        c.gap,
                        c.align,
                        c.justify,
                        out,
                        depth,
                    );
                });

                let footer_rect = egui::Rect::from_min_size(
                    egui::pos2(stack_rect.min.x, stack_rect.max.y - footer_h),
                    egui::vec2(stack_rect.width(), footer_h),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                    ui.set_clip_rect(footer_rect);
                    ui.set_min_height(footer_h);
                    ui.set_max_height(footer_h);
                    render_node(ui, env, footer_id, out, depth + 1);
                });
            } else {
                render_column_children(ui, env, &c.children, c.gap, c.align, c.justify, out, depth);
            }
        }

        UiNodeData::ProgressBar(p) => {
            let frac = if p.max > 0.0 {
                (p.value / p.max).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut bar = egui::ProgressBar::new(frac);
            if let Some(label) = &p.label {
                bar = bar.text(label);
            }
            if let Some(c) = &p.color {
                bar = bar.fill(rgba(c));
            }
            ui.add(bar);
        }

        UiNodeData::Badge(b) => {
            let fill = match b.color {
                BadgeColor::Accent => colors.accent,
                BadgeColor::Success => colors.success,
                BadgeColor::Warning => colors.warning,
                BadgeColor::Danger => colors.danger,
                BadgeColor::Neutral => colors.bg_active,
            };
            crate::ui::badge::badge(ui, &b.text, fill, colors);
        }

        UiNodeData::ListView(l) => {
            egui::ScrollArea::vertical()
                .id_salt(stable_node_id(ui, "l1_list_view", node))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let has_on_select = l.on_select.as_deref().is_some_and(|a| !a.is_empty());
                    for (i, item_id) in l.items.iter().enumerate() {
                        let selected = l.selected == Some(i as u32);
                        let bg_idx = ui.painter().add(egui::Shape::Noop);
                        let bar_idx = ui.painter().add(egui::Shape::Noop);
                        let inner_resp = egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(8, 6))
                            .corner_radius(style::RADIUS_SMALL)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                if selected {
                                    ui.visuals_mut().override_text_color = Some(colors.accent);
                                }
                                render_node(ui, env, *item_id, out, depth + 1);
                            });
                        let row_rect = inner_resp.response.rect;
                        let resp = inner_resp.response.interact(egui::Sense::click());
                        if resp.hovered() && has_on_select {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if selected {
                            ui.painter().set(
                                bg_idx,
                                egui::Shape::rect_filled(
                                    row_rect,
                                    crate::ui::style::radius_fit(row_rect),
                                    colors.accent.gamma_multiply(0.18),
                                ),
                            );
                            let bar_rect = egui::Rect::from_min_size(
                                egui::pos2(row_rect.left(), row_rect.top() + 2.0),
                                egui::vec2(3.0, (row_rect.height() - 4.0).max(2.0)),
                            );
                            ui.painter().set(
                                bar_idx,
                                egui::Shape::rect_filled(
                                    bar_rect,
                                    crate::ui::style::radius_fit(bar_rect),
                                    colors.accent,
                                ),
                            );
                        } else if resp.hovered() && has_on_select {
                            ui.painter().set(
                                bg_idx,
                                egui::Shape::rect_filled(
                                    row_rect,
                                    crate::ui::style::radius_fit(row_rect),
                                    colors.bg_hover,
                                ),
                            );
                        }
                        let list_clicked = i == 0 && node_click_matches(pending_click, id);
                        if resp.clicked()
                            || node_click_matches(pending_click, *item_id)
                            || list_clicked
                        {
                            if let Some(action) = l.on_select.as_deref().filter(|a| !a.is_empty()) {
                                out.actions.push(format!("{action}:{i}"));
                            }
                        }
                    }
                });
        }

        UiNodeData::Scroll(s) => {
            let area = if s.horizontal {
                egui::ScrollArea::horizontal()
            } else {
                egui::ScrollArea::vertical()
            };
            area.show(ui, |ui| {
                render_node(ui, env, s.child, out, depth + 1);
            });
        }

        UiNodeData::Padding(p) => {
            let is_card = node.key.starts_with(CARD_NODE_KEY_PREFIX);
            let nested_card = is_card && is_nested_card(nodes, id);
            let keypad_w = is_card.then(|| card_keypad_width(nodes, p.child)).flatten();
            let mut frame = egui::Frame::new().inner_margin(egui::Margin {
                left: p.left as i8,
                right: p.right as i8,
                top: p.top as i8,
                bottom: p.bottom as i8,
            });
            if is_card {
                frame = frame
                    .fill(if nested_card {
                        colors.bg_darkest
                    } else {
                        colors.bg_active
                    })
                    .stroke(egui::Stroke::new(1.0_f32, colors.border))
                    .corner_radius(if nested_card {
                        style::RADIUS_SM
                    } else {
                        style::RADIUS_MD
                    });
            }
            frame.show(ui, |ui| {
                if let Some(w) = keypad_w {
                    let target_w = w.min(ui.available_width());
                    ui.set_min_width(target_w);
                    ui.set_max_width(target_w);
                } else if is_card {
                    ui.set_min_width(ui.available_width());
                }
                render_node(ui, env, p.child, out, depth + 1);
            });
        }

        UiNodeData::Canvas(c) => {
            let canvas_started = std::time::Instant::now();
            let width = if c.grow {
                ui.available_width().max(1.0)
            } else if c.width > 0.0 {
                c.width.min(ui.available_width()).max(1.0)
            } else {
                ui.available_width().max(1.0)
            };
            let height = if c.grow {
                ui.available_height().max(1.0)
            } else {
                c.height
            };
            let (rect, resp) = ui.allocate_exact_size(
                egui::vec2(width, height.max(1.0)),
                egui::Sense::click_and_drag(),
            );
            let fit = canvas_fits
                .and_then(|fits| fits.get(&id))
                .copied()
                .unwrap_or_default();
            let (origin, sx, sy) = canvas_transform(rect, c.width, c.height, fit);
            let to_canvas =
                |pos: egui::Pos2, button: Option<&'static str>, pressed: bool| CanvasClick {
                    x: (pos.x - origin.x) / sx,
                    y: (pos.y - origin.y) / sy,
                    button,
                    pressed,
                };
            // A real click is detected by egui's own `Sense` resolution
            // (resolved once per pass, inside `Context::begin_pass`, from that
            // pass's actual `RawInput` — it cannot be faked by mutating
            // `ctx.input_mut()` after the pass has started). `plexi pane
            // click`/`HostHarness::inject_click` deliver a `PendingPaneClick`
            // instead, matched against this frame's freshly-computed `rect` —
            // the same honest hit-test a real click would need, just resolved
            // explicitly rather than via egui's internal interact_widgets.
            // Drag samples (stint 0510) arrive the same way, one per frame,
            // and map to press/move/release mouse events in canvas space.
            let synthetic = pending_click.filter(|c| {
                matches!(c.target, crate::host::pane::PaneClickTarget::Pos(pos) if rect.contains(pos))
            });
            if let Some(c) = synthetic {
                if let crate::host::pane::PaneClickTarget::Pos(pos) = c.target {
                    use crate::host::pane::PointerPhase;
                    out.canvas_clicks.push(match c.phase {
                        // `Click` keeps the pre-drag contract: one
                        // pressed=true event, no synthetic release.
                        PointerPhase::Click | PointerPhase::Press => {
                            to_canvas(pos, Some(c.button), true)
                        }
                        PointerPhase::Move => to_canvas(pos, None, false),
                        PointerPhase::Release => to_canvas(pos, Some(c.button), false),
                    });
                }
            }
            // Real pointer interactions, through egui's own hit-testing. A
            // plain click stays a single pressed=true event (the existing plugin
            // contract); a genuine drag delivers press → move… → release so
            // scrub/trim interactions work with a physical mouse too.
            let real_button = [
                (egui::PointerButton::Primary, "left"),
                (egui::PointerButton::Secondary, "right"),
                (egui::PointerButton::Middle, "middle"),
            ];
            if let Some(pos) = resp.interact_pointer_pos() {
                if resp.clicked() {
                    let button = real_button
                        .iter()
                        .find(|(b, _)| resp.clicked_by(*b))
                        .map_or("left", |(_, name)| *name);
                    out.canvas_clicks.push(to_canvas(pos, Some(button), true));
                } else if let Some((_, button)) =
                    real_button.iter().find(|(b, _)| resp.drag_started_by(*b))
                {
                    out.canvas_clicks.push(to_canvas(pos, Some(button), true));
                } else if let Some((_, button)) =
                    real_button.iter().find(|(b, _)| resp.drag_stopped_by(*b))
                {
                    out.canvas_clicks.push(to_canvas(pos, Some(button), false));
                } else if resp.dragged() {
                    out.canvas_clicks.push(to_canvas(pos, None, false));
                }
            }
            for command in &c.commands {
                match command {
                    CanvasCommand::Rect(r) => {
                        let min = egui::pos2(origin.x + r.x * sx, origin.y + r.y * sy);
                        let size = egui::vec2(r.width * sx, r.height * sy);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(min, size),
                            r.radius * sx.min(sy),
                            rgba(&r.fill),
                        );
                    }
                    CanvasCommand::Circle(circle) => {
                        ui.painter().circle_filled(
                            egui::pos2(origin.x + circle.x * sx, origin.y + circle.y * sy),
                            circle.radius * sx.min(sy),
                            rgba(&circle.fill),
                        );
                    }
                    CanvasCommand::Line(line) => {
                        ui.painter().line_segment(
                            [
                                egui::pos2(origin.x + line.x1 * sx, origin.y + line.y1 * sy),
                                egui::pos2(origin.x + line.x2 * sx, origin.y + line.y2 * sy),
                            ],
                            egui::Stroke::new(line.width * sx.min(sy), rgba(&line.color)),
                        );
                    }
                    CanvasCommand::Text(text) => {
                        // Rasterize at the final effective size snapped to the
                        // physical pixel grid (stint 0527): a raw
                        // `size × scale` yields fractional per-frame font
                        // sizes that re-rasterize soft at every canvas resize.
                        let ppp = ui.painter().pixels_per_point();
                        let effective = (text.size * sx.min(sy)).max(1.0);
                        let snapped = (effective * ppp).round().max(1.0) / ppp;
                        ui.painter().text(
                            egui::pos2(origin.x + text.x * sx, origin.y + text.y * sy),
                            canvas_align(text.align),
                            &text.text,
                            egui::FontId::proportional(snapped),
                            rgba(&text.color),
                        );
                    }
                }
            }
            out.canvas_time += canvas_started.elapsed();
        }

        UiNodeData::Divider => {
            ui.separator();
        }

        // A grow `Space` directly inside a `Column` never reaches this arm:
        // `render_column_children` treats it as a flex gap. Inside a row it
        // reaches here only when the row does not hand out its leftover
        // width (it is too deep to measure, or its children do not fit), and
        // then it takes none: its share is the row's to give, and the row's
        // height is no measure of width (stint 1363).
        UiNodeData::Space(s) => {
            let amount = if !s.grow {
                s.size
            } else if ui.layout().is_horizontal() {
                0.0
            } else {
                ui.available_height().max(0.0)
            };
            ui.add_space(amount);
        }

        // GPU surface: composite the guest's render (read back into an egui
        // texture by the live pane). Without one (e.g. no gpu grant), draw a
        // labelled placeholder so the layout still reserves the footprint.
        UiNodeData::Surface(s) => {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(s.width as f32, s.height as f32),
                egui::Sense::hover(),
            );
            // The backing texture is allocated at `logical × ppp` physical
            // pixels (stint 0527); snapping the composite rect to the pixel
            // grid makes the texel→pixel mapping the identity at integer ppp,
            // so no resampling blurs the guest's render.
            let rect = {
                use egui::emath::GuiRounding;
                rect.round_to_pixels(ui.painter().pixels_per_point())
            };
            match surface {
                Some(tex) => {
                    ui.painter().image(
                        tex,
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                None => {
                    ui.painter()
                        .rect_filled(rect, style::RADIUS_MD, colors.bg_active);
                    crate::ui::snap::text_snapped(
                        ui.painter(),
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "GPU surface",
                        egui::FontId::proportional(style::TEXT_CAPTION),
                        colors.text_dim,
                    );
                }
            }
        }

        UiNodeData::SurfaceHeader(a) => {
            ViewChrome::new(colors).paint_view_bar(ui, &a.title, &a.subtitle);
        }

        UiNodeData::FooterKeys(f) => {
            ViewChrome::new(colors).paint_footer_keys(ui, &f.entries, f.divider);
        }

        // Bottom-pinned nodes are partitioned and rendered by the enclosing
        // `Column` (see `column_bottom_pin`). When `Pinned` appears outside a
        // `Column` (e.g. at the tree root), render its child inline.
        UiNodeData::Pinned(p) => {
            render_node(ui, env, p.child, out, depth + 1);
        }

        UiNodeData::Spinner(sp) => {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                let t = ui.input(|i| i.time) as f32;
                ui.ctx().request_repaint();
                let angle = t * 4.0;
                let center = rect.center();
                for k in 0..8 {
                    let a = angle + k as f32 * std::f32::consts::TAU / 8.0;
                    let alpha = (k as f32 / 8.0 * 200.0) as u8 + 40;
                    let p = center + egui::vec2(a.cos(), a.sin()) * 6.0;
                    ui.painter().circle_filled(
                        p,
                        1.6,
                        colors.accent.linear_multiply(alpha as f32 / 255.0),
                    );
                }
                if !sp.label.is_empty() {
                    ui.add_space(style::SPACE_SM);
                    ViewChrome::new(colors).text_label(
                        ui,
                        &sp.label,
                        view_chrome::TextLabelStyle {
                            size: style::TEXT_CAPTION,
                            color: colors.text_dim,
                            bold: false,
                            monospace: false,
                            wrap: false,
                        },
                    );
                }
            });
        }
    }
    if let Some(start) = authored_start {
        out.authored_paint.push(start..paint_len(ui));
    }
}

pub(crate) const CARD_NODE_KEY_PREFIX: &str = "__plexi_card__:";
pub(crate) const TABLE_NODE_KEY_PREFIX: &str = "__plexi_table__:";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(crate) struct TableNodeMeta {
    pub identity_key: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub selected: Option<u32>,
    pub on_select: Option<String>,
}

pub(crate) fn parse_table_node_key(key: &str) -> Option<TableNodeMeta> {
    let payload = key.strip_prefix(TABLE_NODE_KEY_PREFIX)?;
    serde_json::from_str(payload).ok()
}

fn render_data_table_node(
    ui: &mut egui::Ui,
    table: &TableNodeMeta,
    id: u32,
    child_ids: &[u32],
    colors: &Colors,
    out: &mut RenderResult,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
) {
    let col_count = table
        .columns
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0))
        .max(1);
    let table_width = ui.available_width().max(1.0);
    let col_w = table_width / col_count as f32;

    if !table.columns.is_empty() {
        let (header_rect, _) =
            ui.allocate_exact_size(egui::vec2(table_width, 24.0), egui::Sense::hover());
        crate::ui::list::paint_table_header_background(ui, header_rect, colors);
        for (col_idx, label) in table.columns.iter().enumerate() {
            let left = header_rect.left() + col_idx as f32 * col_w;
            let right = (left + col_w).min(header_rect.right());
            let cell = egui::Rect::from_min_max(
                egui::pos2(left, header_rect.top()),
                egui::pos2(right, header_rect.bottom()),
            );
            crate::ui::list::paint_table_header_label(
                ui,
                cell,
                label.clone(),
                egui::FontId::proportional(style::TEXT_HINT),
                colors,
            );
            if col_idx + 1 < col_count {
                ui.painter().line_segment(
                    [
                        egui::pos2(cell.right(), cell.top() + 5.0),
                        egui::pos2(cell.right(), cell.bottom() - 5.0),
                    ],
                    egui::Stroke::new(1.0_f32, colors.border),
                );
            }
        }
        ui.add_space(2.0);
    }

    egui::ScrollArea::vertical()
        .id_salt(ui.id().with(("l1_data_table", table.identity_key.as_str())))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let row_w = ui.available_width().max(1.0);
            let row_col_w = row_w / col_count as f32;
            let row_offset = usize::from(!table.columns.is_empty());
            for (row_idx, row_cells) in table.rows.iter().enumerate() {
                let (row_rect, resp) = ui.allocate_exact_size(
                    egui::vec2(row_w, style::LIST_ROW_DENSE_H),
                    egui::Sense::click(),
                );
                let selected = table.selected == Some(row_idx as u32);
                crate::ui::list::paint_table_row_background(
                    ui,
                    row_rect,
                    selected,
                    resp.hovered(),
                    colors,
                );
                let font = egui::FontId::proportional(style::TEXT_CAPTION);
                for (col_idx, cell_text) in row_cells.iter().enumerate() {
                    let left = row_rect.left() + col_idx as f32 * row_col_w;
                    let right = (left + row_col_w).min(row_rect.right());
                    let cell = egui::Rect::from_min_max(
                        egui::pos2(left, row_rect.top()),
                        egui::pos2(right, row_rect.bottom()),
                    );
                    let color = if selected || col_idx == 0 {
                        colors.text_primary
                    } else {
                        colors.text_dim
                    };
                    crate::ui::list::paint_table_cell_text(
                        ui,
                        cell,
                        cell_text.clone(),
                        font.clone(),
                        color,
                    );
                }
                let child_clicked = child_ids
                    .get(row_offset + row_idx)
                    .is_some_and(|cid| node_click_matches(pending_click, *cid));
                let table_clicked = row_idx == 0 && node_click_matches(pending_click, id);
                if resp.clicked() || child_clicked || table_clicked {
                    if let Some(action) = table.on_select.as_deref().filter(|a| !a.is_empty()) {
                        out.actions.push(format!("{action}:{row_idx}"));
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::wasm_view::bindings::plexi::platform::types::{
        ButtonNode, CanvasNode, CanvasRect, CanvasText, ColumnNode, FooterKeyEntry, FooterKeysNode,
        RowNode, SpaceNode, SurfaceHeaderNode, SurfaceNode, TextInputNode, TextNode,
    };
    use crate::host::wasm_view::{InputEvent, StateSnapshot, StateStore, SystemStats, WasmView};

    #[test]
    fn plain_text_uses_host_theme_when_egui_visuals_differ() {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::light());
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );
        let tree = UiTree {
            root: 0,
            nodes: vec![node(
                0,
                UiNodeData::Text(TextNode {
                    text: "Calculator".into(),
                    size: None,
                    bold: false,
                    color: None,
                    truncate: false,
                    align: Alignment::Start,
                }),
            )],
        };
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = render_ui_tree_with_surface(ui, &tree, &colors, None, None, 0);
        });
        let text = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Calculator" => Some(text),
                _ => None,
            })
            .expect("painted Calculator text");
        let color = text.galley.job.sections[0].format.color;
        let color = if color == Color32::PLACEHOLDER {
            text.fallback_color
        } else {
            color
        };
        assert_eq!(
            color, colors.text_primary,
            "plugin text must follow the host palette"
        );

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 160.0))
            .with_theme(egui::Theme::Light)
            .build_ui(move |ui| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(colors.bg_darkest))
                    .show_inside(ui, |ui| {
                        let _ = render_ui_tree_with_surface(ui, &tree, &colors, None, None, 0);
                    });
            });
        harness.run();
        harness
            .render()
            .expect("theme regression render")
            .save("/tmp/plexi-distribution-text-theme.png")
            .expect("theme screenshot");
    }

    /// Stint 0456: the declarative TextInput renders through the styled
    /// host field (`crate::ui::text_field`) — bg_active fill, border
    /// stroke, dimmed hint — not egui's default TextEdit chrome. Visual
    /// review artifact: /tmp/plexi-render-0456-textinput.png.
    #[test]
    fn screenshot_declarative_text_input_styled_field() {
        let tree = UiTree {
            root: 0,
            nodes: vec![
                node(
                    0,
                    UiNodeData::Column(ColumnNode {
                        children: vec![1, 2],
                        gap: 8.0,
                        align: Alignment::Start,
                        justify: Justify::Start,
                        grow: true,
                    }),
                ),
                node(
                    1,
                    UiNodeData::Text(TextNode {
                        text: "I'm thinking of a number between 1 and 100.".to_string(),
                        size: None,
                        bold: false,
                        color: None,
                        truncate: false,
                        align: Alignment::Start,
                    }),
                ),
                node(
                    2,
                    UiNodeData::TextInput(TextInputNode {
                        value: String::new(),
                        placeholder: "Enter your guess (1-100)".to_string(),
                        on_change: "guess".to_string(),
                        on_submit: "guess".to_string(),
                        password: false,
                        autofocus: false,
                    }),
                ),
            ],
        };
        let png = render_ui_tree_to_png(&tree, 420.0, 180.0, 1.0).expect("render TextInput tree");
        std::fs::write("/tmp/plexi-render-0456-textinput.png", png)
            .expect("write screenshot for visual review");
    }

    /// Stints 0527/0528/0530 evidence: a text-heavy plugin (SurfaceHeader with
    /// title+subtitle, flow text, canvas text commands) rendered at ppp 1.0
    /// and 2.0. The ppp-2.0 capture must show 2× pixel detail — crisp canvas
    /// text at the snapped effective size, pixel-grid SurfaceHeader galleys, and the
    /// contrast-floored subtitle tone. Review artifacts:
    /// /tmp/plexi-render-0527-plugin-text-ppp{1,2}.png.
    #[test]
    fn screenshot_text_heavy_plugin_at_ppp_1_and_2() {
        let white = Color {
            r: 0xe6,
            g: 0xe6,
            b: 0xf0,
            a: 0xff,
        };
        let canvas_text = |x: f32, y: f32, size: f32, text: &str| {
            CanvasCommand::Text(CanvasText {
                x,
                y,
                text: text.to_string(),
                size,
                color: white,
                bold: false,
                align: Alignment::Start,
            })
        };
        let tree = UiTree {
            root: 0,
            nodes: vec![
                node(
                    0,
                    UiNodeData::Column(ColumnNode {
                        children: vec![1, 2, 3],
                        gap: 8.0,
                        align: Alignment::Start,
                        justify: Justify::Start,
                        grow: true,
                    }),
                ),
                node(
                    1,
                    UiNodeData::SurfaceHeader(SurfaceHeaderNode {
                        title: "Pixel Grid Fixture".to_string(),
                        subtitle: "surface resolution · typography evidence".to_string(),
                    }),
                ),
                node(
                    2,
                    UiNodeData::Text(TextNode {
                        text: "The quick brown fox jumps over the lazy dog 0123456789".to_string(),
                        size: None,
                        bold: false,
                        color: None,
                        truncate: false,
                        align: Alignment::Start,
                    }),
                ),
                node(
                    3,
                    UiNodeData::Canvas(CanvasNode {
                        width: 420.0,
                        height: 160.0,
                        grow: false,
                        commands: vec![
                            canvas_text(8.0, 12.0, 11.0, "canvas 11pt: waveform ruler 0 dB"),
                            canvas_text(8.0, 40.0, 14.0, "canvas 14pt: The quick brown fox"),
                            canvas_text(8.0, 74.0, 18.0, "canvas 18pt: 120.0 BPM 44.1 kHz"),
                        ],
                    }),
                ),
            ],
        };
        for (ppp, path) in [
            (1.0, "/tmp/plexi-render-0527-app-text-ppp1.png"),
            (2.0, "/tmp/plexi-render-0527-app-text-ppp2.png"),
        ] {
            let png = render_ui_tree_to_png(&tree, 480.0, 300.0, ppp)
                .unwrap_or_else(|e| panic!("render text-heavy tree at ppp {ppp}: {e}"));
            std::fs::write(path, png).expect("write screenshot for visual review");
        }
    }

    /// Stint 0750 evidence: every `BadgeColor` rendered through the one
    /// `ui::badge` primitive, beside a single-glyph badge that exercises the
    /// width floor. Confirms the WIT `Badge` arm now picks up `BADGE_PAD_H/V`
    /// and `RADIUS_BADGE` instead of the `Margin::symmetric(6, 2)` /
    /// `RADIUS_SM` it painted before. Review artifact:
    /// /tmp/plexi-render-0750-badges.png.
    #[test]
    fn screenshot_badge_colors_and_width_floor() {
        let badge = |id: u32, text: &str, color: BadgeColor| {
            node(
                id,
                UiNodeData::Badge(
                    crate::host::wasm_view::bindings::plexi::platform::types::BadgeNode {
                        text: text.to_string(),
                        color,
                    },
                ),
            )
        };
        let tree = UiTree {
            root: 0,
            nodes: vec![
                node(
                    0,
                    UiNodeData::Column(ColumnNode {
                        children: vec![1, 2],
                        gap: style::SPACE_MD,
                        align: Alignment::Start,
                        justify: Justify::Start,
                        grow: false,
                    }),
                ),
                node(
                    1,
                    UiNodeData::Row(RowNode {
                        children: vec![3, 4, 5, 6, 7],
                        gap: style::SPACE_SM,
                        align: Alignment::Start,
                        justify: Justify::Start,
                        grow: false,
                    }),
                ),
                // A one-character badge: the pill must stay round rather than
                // shrink-wrapping the glyph.
                node(
                    2,
                    UiNodeData::Row(RowNode {
                        children: vec![8, 9],
                        gap: style::SPACE_SM,
                        align: Alignment::Start,
                        justify: Justify::Start,
                        grow: false,
                    }),
                ),
                badge(3, "accent", BadgeColor::Accent),
                badge(4, "success", BadgeColor::Success),
                badge(5, "warning", BadgeColor::Warning),
                badge(6, "danger", BadgeColor::Danger),
                badge(7, "neutral", BadgeColor::Neutral),
                badge(8, "1", BadgeColor::Accent),
                badge(9, "42", BadgeColor::Danger),
            ],
        };
        let png = render_ui_tree_to_png(&tree, 480.0, 120.0, 2.0).expect("render badge tree");
        std::fs::write("/tmp/plexi-render-0750-badges.png", png)
            .expect("write screenshot for visual review");
    }

    /// Build an edit state as the render arm would leave it.
    fn edit_state(
        buf: &str,
        last_plugin_value: &str,
        seen_seq: u64,
        pending: &[&str],
    ) -> TextInputEditState {
        TextInputEditState {
            buf: buf.to_string(),
            last_plugin_value: last_plugin_value.to_string(),
            seen_seq,
            pending: pending.iter().map(|v| v.to_string()).collect(),
            awaiting_submit: false,
        }
    }

    /// Stints 0456/0720/0864: the TextInput edit buffer only resets when the plugin
    /// pushes news — a repaint of the same committed tree, a pre-echo guest
    /// frame carrying the unchanged baseline value, and the echo of our own
    /// `on_change` never clobber a local draft mid-round-trip.
    #[test]
    fn text_input_buffer_reconciliation() {
        // Fresh widget: adopt the plugin's value.
        let (state, outcome) = reconcile_text_input_buffer(None, "seed", 7);
        assert_eq!(outcome, TextInputReconcile::Adopted);
        assert_eq!(state, edit_state("seed", "seed", 7, &[]));

        // Stint 0720 regression: the SAME committed tree repainted, carrying a
        // value that differs from the draft because the guest has not echoed
        // yet. Value comparison alone called this a plugin reset and dropped the
        // keystroke; the generation says there is no news.
        let (state, outcome) =
            reconcile_text_input_buffer(Some(edit_state("hi", "", 3, &["h", "hi"])), "", 3);
        assert_eq!(
            outcome,
            TextInputReconcile::KeptDraft,
            "a repaint of an already-reconciled tree must never clobber the draft"
        );
        assert_eq!(state, edit_state("hi", "", 3, &["h", "hi"]));

        // Stint 0864 regression: a NEW guest tree (e.g. from a startup resize
        // render that was already in flight before `on_change("h")` reached the
        // guest) commits with the same baseline value (`""`) the widget had
        // before the pending edits started. That is a pre-echo frame, not an
        // plugin-pushed reset.
        let (state, outcome) =
            reconcile_text_input_buffer(Some(edit_state("hi", "", 3, &["h", "hi"])), "", 4);
        assert_eq!(
            outcome,
            TextInputReconcile::KeptDraft,
            "a pre-echo guest frame carrying the unchanged baseline value must never clobber the draft"
        );
        assert_eq!(state, edit_state("hi", "", 4, &["h", "hi"]));

        // A new tree carrying an echo we are still waiting on: keep the draft
        // and drain the queue through that entry.
        let (state, outcome) = reconcile_text_input_buffer(Some(state), "h", 5);
        assert_eq!(outcome, TextInputReconcile::KeptDraft);
        assert_eq!(state, edit_state("hi", "h", 5, &["hi"]));

        // A new tree with a value we never sent and that differs from the
        // baseline is genuine plugin news (external SetState, post-submit reset)
        // and wins over the draft.
        let (state, outcome) =
            reconcile_text_input_buffer(Some(edit_state("typed", "seed", 3, &["typed"])), "", 4);
        assert_eq!(outcome, TextInputReconcile::Adopted);
        assert_eq!(state, edit_state("", "", 4, &[]));
    }

    /// Stint 0720: after `on_submit` the plugin redefines the value, but a
    /// character typed before its tree lands starts the next entry rather than
    /// re-extending the submitted one.
    #[test]
    fn text_input_submit_hands_the_value_back_unless_the_user_typed_first() {
        let submitted = TextInputEditState {
            buf: String::new(),
            last_plugin_value: String::new(),
            seen_seq: 3,
            pending: VecDeque::new(),
            awaiting_submit: true,
        };

        // Untouched since submit: whatever the plugin decided is authoritative —
        // cleared here, but a search field that keeps its query is restored the
        // same way.
        let (state, outcome) = reconcile_text_input_buffer(Some(submitted.clone()), "query", 4);
        assert_eq!(outcome, TextInputReconcile::Adopted);
        assert_eq!(state, edit_state("query", "query", 4, &[]));

        // Typed in the gap: the fresh draft outranks the plugin's answer.
        let mut typed_after = submitted;
        typed_after.buf = "z".to_string();
        typed_after.pending.push_back("z".to_string());
        let (state, outcome) = reconcile_text_input_buffer(Some(typed_after), "hi", 4);
        assert_eq!(outcome, TextInputReconcile::KeptDraft);
        assert_eq!(state, edit_state("z", "", 4, &["z"]));
    }
    use std::path::PathBuf;

    fn node(id: u32, data: UiNodeData) -> IndexedNode {
        IndexedNode {
            id,
            key: String::new(),
            data,
        }
    }

    #[test]
    fn keyed_text_input_id_survives_list_reorder() {
        let ctx = egui::Context::default();
        let mut before = egui::Id::NULL;
        let mut after = egui::Id::NULL;
        ctx.begin_pass(egui::RawInput::default());
        egui::Area::new(egui::Id::new("keyed-id-test")).show(&ctx, |ui| {
            before = stable_node_id(
                ui,
                "l1_text_input",
                &IndexedNode {
                    id: 2,
                    key: "0/@row-b/0".to_string(),
                    data: UiNodeData::Empty,
                },
            );
            // A keyed row can occupy another arena slot after its siblings
            // move; its path stays the same and so does the egui state id.
            after = stable_node_id(
                ui,
                "l1_text_input",
                &IndexedNode {
                    id: 19,
                    key: "0/@row-b/0".to_string(),
                    data: UiNodeData::Empty,
                },
            );
        });
        let _ = ctx.end_pass();
        assert_eq!(before, after);
    }

    /// Bounding box of everything painted when `tree` renders into a `pane`-sized
    /// screen. Used to observe where the good-by-default content inset places
    /// real content vs. where a full-bleed plugin draws.
    fn painted_bounds(tree: &UiTree, pane: egui::Vec2) -> egui::Rect {
        let mut bounds = egui::Rect::NOTHING;
        for r in painted_shapes(tree, pane) {
            bounds = bounds.union(r);
        }
        bounds
    }

    /// Every finite, positive shape bounding rect painted by `tree` into a
    /// `pane`-sized screen (the host panel is `Frame::NONE`, so shapes are
    /// exactly what the tree render draws).
    fn painted_shapes(tree: &UiTree, pane: egui::Vec2) -> Vec<egui::Rect> {
        paint(tree, pane)
            .into_iter()
            .map(|clipped| clipped.shape.visual_bounding_rect())
            .filter(|r| r.is_finite() && r.is_positive())
            .collect()
    }

    /// One frame of `tree` in a `pane`-sized screen, as clipped shapes.
    fn paint(tree: &UiTree, pane: egui::Vec2) -> Vec<egui::epaint::ClippedShape> {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );
        // The host style, as the live host and the headless harness apply it.
        // Kit button widths depend on it: egui takes the style's widget stroke
        // out of the button padding, and bare egui has no such stroke, which
        // makes every button 2px wider than `KitButton::natural_width`.
        crate::ui::theme::setup_style(&ctx, &colors);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, pane)),
            ..Default::default()
        };
        let output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    let _ = render_ui_tree_with_surface(ui, tree, &colors, None, None, 0);
                });
        });
        output.shapes
    }

    /// Split painted shapes into the pane-filling background surface (any shape
    /// that covers the whole `pane` rect) and the union of everything else
    /// (the actual content). Used to prove the surface fills the pane while
    /// content stays inset.
    fn surface_and_content(tree: &UiTree, pane: egui::Vec2) -> (Option<egui::Rect>, egui::Rect) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, pane);
        let mut surface: Option<egui::Rect> = None;
        let mut content = egui::Rect::NOTHING;
        for r in painted_shapes(tree, pane) {
            let fills_pane = r.min.x <= screen.min.x + 0.5
                && r.min.y <= screen.min.y + 0.5
                && r.max.x >= screen.max.x - 0.5
                && r.max.y >= screen.max.y - 0.5;
            if fills_pane {
                surface = Some(surface.map_or(r, |s| s.union(r)));
            } else {
                content = content.union(r);
            }
        }
        (surface, content)
    }

    // Stint 0445: a declarative flow plugin (no grow Canvas / Surface) earns the
    // host's standard content inset, so its content never renders flush against
    // the pane edge, and buttons get a comfortable minimum click target.
    #[test]
    fn flow_root_is_inset_and_buttons_meet_minimum_size() {
        let tree = UiTree {
            root: 0,
            nodes: vec![node(
                0,
                UiNodeData::Button(ButtonNode {
                    label: "X".to_string(),
                    on_click: "x".to_string(),
                    style: ButtonStyle::Primary,
                    disabled: false,
                }),
            )],
        };
        let pane = egui::vec2(400.0, 300.0);
        let (surface, content) = surface_and_content(&tree, pane);
        // Stint 0448: the plugin surface fills the whole pane; only content insets.
        let surface = surface.expect("flow plugin must paint a pane-filling surface");
        assert!(
            surface.min.x <= 0.5
                && surface.min.y <= 0.5
                && surface.max.x >= pane.x - 0.5
                && surface.max.y >= pane.y - 0.5,
            "flow surface {surface:?} should fill the pane {pane:?}"
        );
        assert!(
            content.min.x >= style::SPACE_XL - 0.5,
            "flow content left edge {} should be inset by ~SPACE_XL ({})",
            content.min.x,
            style::SPACE_XL
        );
        assert!(
            content.min.y >= style::SPACE_MD - 0.5,
            "flow content top edge {} should be inset by ~SPACE_MD ({})",
            content.min.y,
            style::SPACE_MD
        );
        assert!(
            content.height() >= style::BUTTON_H_MD - 0.5,
            "button height {} should meet the minimum target ({})",
            content.height(),
            style::BUTTON_H_MD
        );
    }

    // Stint 0448: a flow plugin's surface/background fill must span the entire
    // pane rect — the content inset moves only the layout cursor, never the
    // visible surface. Regression guard against the 0445 black-border bug.
    #[test]
    fn flow_root_background_fills_the_pane() {
        let tree = UiTree {
            root: 0,
            nodes: vec![
                node(
                    0,
                    UiNodeData::Column(ColumnNode {
                        children: vec![1],
                        gap: 0.0,
                        align: Alignment::Start,
                        justify: Justify::Start,
                        grow: true,
                    }),
                ),
                node(
                    1,
                    UiNodeData::Text(TextNode {
                        text: "hello".to_string(),
                        size: None,
                        bold: false,
                        color: None,
                        truncate: false,
                        align: Alignment::Start,
                    }),
                ),
            ],
        };
        let pane = egui::vec2(400.0, 300.0);
        let (surface, content) = surface_and_content(&tree, pane);
        let surface = surface.expect("flow plugin must paint a pane-filling surface");
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, pane);
        assert!(
            surface.min.x <= 0.5
                && surface.min.y <= 0.5
                && surface.max.x >= pane.x - 0.5
                && surface.max.y >= pane.y - 0.5,
            "surface {surface:?} must equal the full pane rect {screen:?}"
        );
        // Content still lives inside the inset.
        assert!(
            content.min.x >= style::SPACE_XL - 0.5 && content.min.y >= style::SPACE_MD - 0.5,
            "content {content:?} must stay inset while the surface fills the pane"
        );
    }

    // A grow Canvas signals a full-bleed pixel plugin; the host must not inset it,
    // so its drawing reaches the pane edge.
    #[test]
    fn grow_canvas_plugin_is_not_inset() {
        let fill = Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        let tree = UiTree {
            root: 0,
            nodes: vec![node(
                0,
                UiNodeData::Canvas(CanvasNode {
                    width: 100.0,
                    height: 100.0,
                    grow: true,
                    commands: vec![CanvasCommand::Rect(CanvasRect {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 100.0,
                        radius: 0.0,
                        fill,
                    })],
                }),
            )],
        };
        let bounds = painted_bounds(&tree, egui::vec2(400.0, 300.0));
        assert!(
            bounds.min.x <= 1.0 && bounds.min.y <= 1.0,
            "full-bleed canvas should reach the pane origin, got {:?}",
            bounds.min
        );
    }

    #[test]
    fn content_padding_exemption_tracks_full_bleed_nodes() {
        let flow = vec![
            node(
                0,
                UiNodeData::Column(ColumnNode {
                    children: vec![1],
                    gap: 0.0,
                    align: Alignment::Start,
                    justify: Justify::Start,
                    grow: false,
                }),
            ),
            node(
                1,
                UiNodeData::Text(TextNode {
                    text: "hi".to_string(),
                    size: None,
                    bold: false,
                    color: None,
                    truncate: false,
                    align: Alignment::Start,
                }),
            ),
        ];
        assert!(
            tree_wants_content_padding(&flow),
            "a text/column tree is flow content and wants the inset"
        );

        let mut with_grow_canvas = flow.clone();
        with_grow_canvas[1] = node(
            1,
            UiNodeData::Canvas(CanvasNode {
                width: 10.0,
                height: 10.0,
                grow: true,
                commands: vec![],
            }),
        );
        assert!(
            !tree_wants_content_padding(&with_grow_canvas),
            "a grow canvas makes the plugin full-bleed"
        );

        let mut with_fixed_canvas = flow.clone();
        with_fixed_canvas[1] = node(
            1,
            UiNodeData::Canvas(CanvasNode {
                width: 10.0,
                height: 10.0,
                grow: false,
                commands: vec![],
            }),
        );
        assert!(
            tree_wants_content_padding(&with_fixed_canvas),
            "a fixed-size canvas is ordinary flow content and still wants the inset"
        );

        let mut with_surface = flow.clone();
        with_surface[1] = node(
            1,
            UiNodeData::Surface(SurfaceNode {
                width: 8,
                height: 8,
                texture_handle: None,
            }),
        );
        assert!(
            !tree_wants_content_padding(&with_surface),
            "a GPU surface makes the plugin full-bleed"
        );
    }

    #[test]
    fn root_content_inset_uses_design_tokens() {
        let inset = root_content_inset();
        assert_eq!(inset.left, style::SPACE_XL as i8);
        assert_eq!(inset.right, style::SPACE_XL as i8);
        assert_eq!(inset.top, style::SPACE_MD as i8);
        assert_eq!(inset.bottom, style::SPACE_MD as i8);
    }

    /// A `Column([SurfaceHeader, body…, FooterKeys])` — the shape every flow plugin has.
    fn chrome_tree() -> UiTree {
        UiTree {
            root: 0,
            nodes: vec![
                node(
                    0,
                    UiNodeData::Column(ColumnNode {
                        children: vec![1, 2, 3, 4, 7],
                        gap: style::SPACE_MD,
                        align: Alignment::Start,
                        justify: Justify::Start,
                        grow: true,
                    }),
                ),
                node(
                    1,
                    UiNodeData::SurfaceHeader(SurfaceHeaderNode {
                        title: "Todo".to_string(),
                        subtitle: "2 open".to_string(),
                    }),
                ),
                node(
                    2,
                    UiNodeData::Text(TextNode {
                        text: "buy milk".to_string(),
                        size: None,
                        bold: false,
                        color: None,
                        truncate: false,
                        align: Alignment::Start,
                    }),
                ),
                node(
                    3,
                    UiNodeData::Space(SpaceNode {
                        size: 0.0,
                        grow: true,
                    }),
                ),
                // The tail is a Row of buttons — an `Actions` bar, the shape the
                // overflow bug actually took (a bare child never reproduced it).
                node(
                    4,
                    UiNodeData::Row(RowNode {
                        children: vec![5, 6],
                        gap: style::SPACE_SM,
                        align: Alignment::Start,
                        justify: Justify::Start,
                        grow: false,
                    }),
                ),
                node(
                    5,
                    UiNodeData::Button(ButtonNode {
                        label: "Add item".to_string(),
                        on_click: "add".to_string(),
                        style: ButtonStyle::Primary,
                        disabled: false,
                    }),
                ),
                node(
                    6,
                    UiNodeData::Button(ButtonNode {
                        label: "Delete".to_string(),
                        on_click: "delete".to_string(),
                        style: ButtonStyle::Danger,
                        disabled: false,
                    }),
                ),
                node(
                    7,
                    UiNodeData::FooterKeys(FooterKeysNode {
                        entries: vec![FooterKeyEntry {
                            keys: vec!["a".to_string()],
                            description: "add".to_string(),
                        }],
                        divider: true,
                    }),
                ),
            ],
        }
    }

    /// Stint 0674: an `SurfaceHeader` is the plugin's top edge, not its content. Insetting
    /// it floated the title band off the pane's corner — a strip of background
    /// above and beside every flow plugin's chrome. The band paints against the
    /// pane rect while the body below it keeps the standard inset.
    #[test]
    fn leading_view_bar_paints_full_bleed_while_body_stays_inset() {
        let pane = egui::vec2(400.0, 300.0);
        let shapes = painted_shapes(&chrome_tree(), pane);
        let fills_pane = |r: &egui::Rect| r.max.y >= pane.y - 0.5;

        let band = shapes
            .iter()
            .filter(|r| !fills_pane(r))
            .find(|r| r.min.y <= 0.5 && r.min.x <= 0.5 && r.max.x >= pane.x - 0.5)
            .copied();
        assert!(
            band.is_some(),
            "plugin bar should paint a full-width band at the pane's top edge; painted {shapes:?}"
        );

        // Body content is everything that is not a full-width chrome band
        // (the plugin bar's own band, the footer's band, its divider rule).
        let body_top = band.expect("band").max.y;
        let body = shapes
            .iter()
            .filter(|r| {
                let full_width = r.min.x <= 0.5 && r.max.x >= pane.x - 0.5;
                !fills_pane(r) && !full_width && r.min.y > body_top
            })
            .fold(egui::Rect::NOTHING, |acc, r| acc.union(*r));
        assert!(
            body.min.x >= style::SPACE_XL - 0.5,
            "body content left edge {} should keep the ~SPACE_XL inset",
            body.min.x
        );
    }

    /// Stint 0674: a grow `Space` consumed `available_height()`, so every child
    /// declared after it was pushed past the pane's bottom edge and silently
    /// vanished (todo's Add/Toggle/Delete controls). The tail now pins to the
    /// bottom, and — the second half of the same bug — it must clear the
    /// footer band rather than sliding under it and losing its lower half to
    /// the clip. Both halves are asserted against the footer's own top edge,
    /// because a clipped shape still reports its full pre-clip bounds and a
    /// bounds-only check cannot see the overlap.
    #[test]
    fn column_children_after_a_grow_spacer_pin_above_the_footer_band() {
        let pane = egui::vec2(400.0, 300.0);
        let shapes = painted_shapes(&chrome_tree(), pane);
        let full_width = |r: &egui::Rect| r.min.x <= 0.5 && r.max.x >= pane.x - 0.5;

        // The footer band is the full-width chrome strip in the lower half.
        let footer_top = shapes
            .iter()
            .filter(|r| full_width(r) && r.min.y > pane.y / 2.0 && r.max.y < pane.y + 0.5)
            .map(|r| r.min.y)
            .fold(f32::INFINITY, f32::min);
        assert!(
            footer_top.is_finite(),
            "footer keys should paint a full-width band; painted {shapes:?}"
        );

        // Body content: shapes that are neither a full-width chrome band nor
        // part of the footer's own contents (its key chips start below the
        // band's top edge). A tail sliding under the footer still *starts*
        // above it, so this keeps the failing case in scope.
        let body = shapes
            .iter()
            .filter(|r| !full_width(r) && r.min.y < footer_top)
            .fold(egui::Rect::NOTHING, |acc, r| acc.union(*r));
        assert!(
            body.max.y <= footer_top + 0.5,
            "body content ends at {} but the footer band starts at {footer_top} — \
             the bottom-pinned tail is sliding under the footer and being clipped",
            body.max.y
        );
        assert!(
            body.max.y >= footer_top - (style::BUTTON_H_MD + style::SPACE_MD + 8.0),
            "the child after the grow spacer should sit just above the footer, \
             body ends at {} with the footer at {footer_top}",
            body.max.y
        );
    }

    // ── Stints 1327 and 1333: column flex, row align, narrow-pane rows ──────

    /// One painted text run: its content, the rows it was laid out in, where
    /// it landed and the clip it was painted under.
    struct PaintedText {
        text: String,
        rows: Vec<String>,
        rect: egui::Rect,
        clip: egui::Rect,
    }

    impl PaintedText {
        /// Fully inside both its clip rect and the pane.
        fn visible_in(&self, pane: egui::Vec2) -> bool {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, pane);
            self.clip
                .intersect(screen)
                .expand(0.5)
                .contains_rect(self.rect)
        }

        /// Every laid-out row holds only whole words of the source text.
        fn breaks_only_between_words(&self) -> bool {
            let words: Vec<&str> = self.text.split_whitespace().collect();
            self.rows
                .iter()
                .all(|row| row.split_whitespace().all(|w| words.contains(&w)))
        }
    }

    fn painted_texts(tree: &UiTree, pane: egui::Vec2) -> Vec<PaintedText> {
        fn collect(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<PaintedText>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, clip, out);
                    }
                }
                egui::Shape::Text(text) if !text.galley.job.text.is_empty() => {
                    out.push(PaintedText {
                        text: text.galley.job.text.clone(),
                        rows: text.galley.rows.iter().map(|r| r.row.text()).collect(),
                        rect: text.galley.rect.translate(text.pos.to_vec2()),
                        clip,
                    });
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in paint(tree, pane) {
            collect(&clipped.shape, clipped.clip_rect, &mut out);
        }
        out
    }

    fn painted_text<'a>(texts: &'a [PaintedText], text: &str) -> &'a PaintedText {
        texts.iter().find(|t| t.text == text).unwrap_or_else(|| {
            panic!(
                "{text:?} was not painted; painted {:?}",
                texts.iter().map(|t| &t.text).collect::<Vec<_>>()
            )
        })
    }

    fn text_node(id: u32, text: &str) -> IndexedNode {
        node(
            id,
            UiNodeData::Text(TextNode {
                text: text.to_string(),
                size: None,
                bold: false,
                color: None,
                truncate: false,
                align: Alignment::Start,
            }),
        )
    }

    fn button_node(id: u32, label: &str) -> IndexedNode {
        node(
            id,
            UiNodeData::Button(ButtonNode {
                label: label.to_string(),
                on_click: label.to_string(),
                style: ButtonStyle::Secondary,
                disabled: false,
            }),
        )
    }

    fn grow_node(id: u32) -> IndexedNode {
        node(
            id,
            UiNodeData::Space(SpaceNode {
                size: 0.0,
                grow: true,
            }),
        )
    }

    fn column_node(id: u32, children: Vec<u32>) -> IndexedNode {
        node(
            id,
            UiNodeData::Column(ColumnNode {
                children,
                gap: style::SPACE_MD,
                align: Alignment::Start,
                justify: Justify::Start,
                grow: true,
            }),
        )
    }

    fn row_node(id: u32, children: Vec<u32>, gap: f32, align: Alignment) -> IndexedNode {
        node(
            id,
            UiNodeData::Row(RowNode {
                children,
                gap,
                align,
                justify: Justify::Start,
                grow: false,
            }),
        )
    }

    fn badge_node(id: u32, text: &str) -> IndexedNode {
        node(
            id,
            UiNodeData::Badge(
                crate::host::wasm_view::bindings::plexi::platform::types::BadgeNode {
                    text: text.to_string(),
                    color: BadgeColor::Warning,
                },
            ),
        )
    }

    /// The sizes `plexi plugin check` renders by default.
    const CHECK_SIZES: [egui::Vec2; 3] = [
        egui::vec2(320.0, 240.0),
        egui::vec2(800.0, 600.0),
        egui::vec2(300.0, 900.0),
    ];

    /// `Column([SurfaceHeader, Spacer(grow), Text, Spacer(grow), FooterKeys])`.
    fn centered_tree() -> UiTree {
        UiTree {
            root: 0,
            nodes: vec![
                column_node(0, vec![1, 2, 3, 4, 5]),
                node(
                    1,
                    UiNodeData::SurfaceHeader(SurfaceHeaderNode {
                        title: "Timer".to_string(),
                        subtitle: "running".to_string(),
                    }),
                ),
                grow_node(2),
                text_node(3, "CENTER"),
                grow_node(4),
                node(
                    5,
                    UiNodeData::FooterKeys(FooterKeysNode {
                        entries: vec![FooterKeyEntry {
                            keys: vec!["s".to_string()],
                            description: "start".to_string(),
                        }],
                        divider: true,
                    }),
                ),
            ],
        }
    }

    /// Stint 1327: only the first grow `Space` split a column, so a second one
    /// landed in the bottom-pinned tail and the child between them sat at the
    /// top. Leftover height is shared by every grow spacer, which centres it.
    #[test]
    fn column_two_grow_spacers_center_the_child_between_them() {
        let tree = centered_tree();
        for pane in CHECK_SIZES {
            let shapes = painted_shapes(&tree, pane);
            let full_width = |r: &egui::Rect| r.min.x <= 0.5 && r.max.x >= pane.x - 0.5;
            let header_bottom = shapes
                .iter()
                .filter(|r| full_width(r) && r.min.y <= 0.5 && r.max.y < pane.y / 2.0)
                .map(|r| r.max.y)
                .fold(0.0, f32::max);
            let footer_top = shapes
                .iter()
                .filter(|r| full_width(r) && r.min.y > pane.y / 2.0 && r.max.y < pane.y + 0.5)
                .map(|r| r.min.y)
                .fold(f32::INFINITY, f32::min);
            assert!(
                header_bottom > 0.0 && footer_top.is_finite(),
                "{pane:?}: header and footer bands should both paint; painted {shapes:?}"
            );
            let texts = painted_texts(&tree, pane);
            let text = painted_text(&texts, "CENTER");
            let body_center = (header_bottom + footer_top) / 2.0;
            assert!(
                (text.rect.center().y - body_center).abs() <= 2.0,
                "{pane:?}: text centre y {} should be within 2px of the body centre {body_center} \
                 (body {header_bottom}..{footer_top})",
                text.rect.center().y
            );
        }
    }

    /// Stint 1327: with two grow spacers the tail was measured at the full
    /// stack height, laid out from the top over the head, and everything after
    /// the second spacer was pushed past the clip edge.
    #[test]
    fn column_children_after_a_second_grow_spacer_are_painted_without_overlap() {
        let tree = UiTree {
            root: 0,
            nodes: vec![
                column_node(0, vec![1, 2, 3, 4, 5]),
                text_node(1, "HEAD"),
                grow_node(2),
                text_node(3, "MIDDLE"),
                grow_node(4),
                row_node(5, vec![6, 7], style::SPACE_SM, Alignment::Start),
                button_node(6, "Add item"),
                button_node(7, "Delete"),
            ],
        };
        for pane in CHECK_SIZES {
            let texts = painted_texts(&tree, pane);
            let head = painted_text(&texts, "HEAD");
            let middle = painted_text(&texts, "MIDDLE");
            let add = painted_text(&texts, "Add item");
            let delete = painted_text(&texts, "Delete");
            for t in [head, middle, add, delete] {
                assert!(
                    t.visible_in(pane),
                    "{pane:?}: {:?} at {:?} is outside its clip {:?}",
                    t.text,
                    t.rect,
                    t.clip
                );
            }
            assert!(
                head.rect.max.y <= middle.rect.min.y && middle.rect.max.y <= add.rect.min.y,
                "{pane:?}: head {:?}, middle {:?} and actions {:?} must stack without overlap",
                head.rect,
                middle.rect,
                add.rect
            );
            let above = middle.rect.min.y - head.rect.max.y;
            let below = add.rect.min.y - middle.rect.max.y;
            assert!(
                (above - below).abs() <= style::BUTTON_H_MD,
                "{pane:?}: the two spacers should share the leftover height, got {above} and {below}"
            );
        }
    }

    /// Stint 1327: the Row arm never read `align`. It is the cross axis, as on
    /// a `Column`: children sit at the top, the center or the bottom of the
    /// row, and the row itself does not move sideways.
    #[test]
    fn row_align_places_children_across_the_row() {
        let pane = egui::vec2(400.0, 200.0);
        // A short label beside a tall one: the tall text sets the row height.
        let rects = |align: Alignment| {
            let tree = UiTree {
                root: 0,
                nodes: vec![
                    row_node(0, vec![1, 2], style::SPACE_SM, align),
                    text_node(1, "small"),
                    node(
                        2,
                        UiNodeData::Text(TextNode {
                            text: "TALL".to_string(),
                            size: Some(48.0),
                            bold: false,
                            color: None,
                            truncate: false,
                            align: Alignment::Start,
                        }),
                    ),
                ],
            };
            let texts = painted_texts(&tree, pane);
            (
                painted_text(&texts, "small").rect,
                painted_text(&texts, "TALL").rect,
            )
        };
        let (small, tall) = rects(Alignment::Start);
        assert!(
            (small.min.y - tall.min.y).abs() <= 1.0,
            "start puts children at the row's top: {small:?} vs {tall:?}"
        );
        let (small, tall) = rects(Alignment::End);
        assert!(
            (small.max.y - tall.max.y).abs() <= 1.0,
            "end puts children at the row's bottom: {small:?} vs {tall:?}"
        );
        assert!(
            small.min.x < pane.x / 4.0,
            "align is the cross axis: the row stays at the left edge, got {small:?}"
        );

        // The row places the button, not the label inside it: in a top-aligned
        // row the label is still in the middle of its button.
        for align in [Alignment::Start, Alignment::End] {
            let tree = UiTree {
                root: 0,
                nodes: vec![
                    row_node(0, vec![1, 2], style::SPACE_SM, align),
                    button_node(1, "Save"),
                    node(
                        2,
                        UiNodeData::Text(TextNode {
                            text: "TALL".to_string(),
                            size: Some(48.0),
                            bold: false,
                            color: None,
                            truncate: false,
                            align: Alignment::Start,
                        }),
                    ),
                ],
            };
            let texts = painted_texts(&tree, pane);
            let label = painted_text(&texts, "Save").rect;
            let tall = painted_text(&texts, "TALL").rect;
            let button_center = match align {
                Alignment::End => tall.max.y - style::BUTTON_H_MD / 2.0,
                _ => tall.min.y + style::BUTTON_H_MD / 2.0,
            };
            assert!(
                (label.center().y - button_center).abs() <= 2.0,
                "{align:?}: label centre {} should be the button centre {button_center}",
                label.center().y
            );
        }
    }

    /// The Row default is center: a row that says nothing about `align`, as
    /// every SDK row that is not an explicit `HStack(align=...)` does, centers
    /// its children vertically, and `align="start"` puts them at the top.
    #[test]
    fn row_without_align_centers_and_start_aligns_to_the_top() {
        let rects = |align: &str| {
            let tree = crate::host::wasm_python::decode_ui_tree(&format!(
                r#"{{"root":0,"nodes":[
                {{"id":0,"key":"0","data":{{"type":"row","children":[1,2]{align}}}}},
                {{"id":1,"key":"1","data":{{"type":"text","text":"TALL","size":48.0}}}},
                {{"id":2,"key":"2","data":{{"type":"text","text":"small"}}}}
            ]}}"#
            ))
            .expect("row tree decodes");
            let texts = painted_texts(&tree, egui::vec2(400.0, 200.0));
            (
                painted_text(&texts, "TALL").rect,
                painted_text(&texts, "small").rect,
            )
        };
        let (tall, small) = rects("");
        assert!(
            (small.center().y - tall.center().y).abs() <= 1.0,
            "a row with no align centers its children: {small:?} vs {tall:?}"
        );
        let (tall, small) = rects(r#","align":"start""#);
        assert!(
            (small.min.y - tall.min.y).abs() <= 1.0,
            "align start puts children at the top: {small:?} vs {tall:?}"
        );
    }

    /// A tree from `(id, data)` pairs through the bridge decoder, as a Python
    /// plugin's view arrives; node 0 is the root.
    fn wire_tree(nodes: &[(u32, serde_json::Value)]) -> UiTree {
        let nodes: Vec<serde_json::Value> = nodes
            .iter()
            .map(|(id, data)| serde_json::json!({"id": id, "key": id.to_string(), "data": data}))
            .collect();
        crate::host::wasm_python::decode_ui_tree(
            &serde_json::json!({"root": 0, "nodes": nodes}).to_string(),
        )
        .expect("test tree decodes")
    }

    fn wire_button(label: &str) -> serde_json::Value {
        serde_json::json!({"type": "button", "label": label, "on_click": label})
    }

    fn wire_grow_spacer() -> serde_json::Value {
        serde_json::json!({"type": "spacer", "grow": true})
    }

    /// The rect each node painted in one headless layout pass.
    fn laid_out(tree: &UiTree, pane: egui::Vec2) -> HashMap<u32, egui::Rect> {
        render_ui_tree_layout(tree, pane.x, pane.y)
            .into_iter()
            .filter_map(|fact| fact.rect.map(|rect| (fact.id, rect)))
            .collect()
    }

    fn rect_of(rects: &HashMap<u32, egui::Rect>, id: u32) -> egui::Rect {
        *rects
            .get(&id)
            .unwrap_or_else(|| panic!("node {id} painted nothing: {rects:?}"))
    }

    /// The margin egui's default `CentralPanel` frame (`Frame::central_panel`)
    /// adds around the ui that `egui_kittest`'s `build_ui` hands the headless
    /// layout pass.
    const LAYOUT_HARNESS_MARGIN: f32 = 8.0;

    /// Left and right edges of the content inside the default root inset.
    fn content_x(pane: egui::Vec2) -> (f32, f32) {
        let inset = style::SPACE_XL + LAYOUT_HARNESS_MARGIN;
        (inset, pane.x - inset)
    }

    /// Top and bottom edges of the content inside the default root inset.
    fn content_y(pane: egui::Vec2) -> (f32, f32) {
        let inset = style::SPACE_MD + LAYOUT_HARNESS_MARGIN;
        (inset, pane.y - inset)
    }

    const ROW_PANES: [egui::Vec2; 2] = [egui::vec2(320.0, 240.0), egui::vec2(800.0, 600.0)];

    /// Stint 1363: a grow `Spacer` in a row added the row's height as
    /// horizontal space. It takes the width the other children leave, so the
    /// child after it sits at the row's right edge.
    #[test]
    fn row_grow_spacer_pins_the_last_child_to_the_right_edge() {
        let tree = wire_tree(&[
            (0, serde_json::json!({"type": "row", "children": [1, 2, 3]})),
            (1, wire_button("Back")),
            (2, wire_grow_spacer()),
            (3, wire_button("Next")),
        ]);
        for pane in ROW_PANES {
            let rects = laid_out(&tree, pane);
            let (left, right) = content_x(pane);
            let back = rect_of(&rects, 1);
            let next = rect_of(&rects, 3);
            assert!(
                (back.min.x - left).abs() <= 1.5,
                "{pane:?}: Back starts the row at {left}, got {back:?}"
            );
            assert!(
                (next.max.x - right).abs() <= 1.5,
                "{pane:?}: Next ends at the row's right edge {right}, got {next:?}"
            );
            assert!(
                (back.center().y - next.center().y).abs() <= 1.0,
                "{pane:?}: both sit on one line: {back:?} vs {next:?}"
            );
        }
    }

    /// Stint 1363: two grow spacers share the leftover width, which centers
    /// the child between them.
    #[test]
    fn row_two_grow_spacers_center_the_child_between_them() {
        let tree = wire_tree(&[
            (0, serde_json::json!({"type": "row", "children": [1, 2, 3]})),
            (1, wire_grow_spacer()),
            (2, wire_button("Start")),
            (3, wire_grow_spacer()),
        ]);
        for pane in ROW_PANES {
            let start = rect_of(&laid_out(&tree, pane), 2);
            let (left, right) = content_x(pane);
            let middle = (left + right) / 2.0;
            assert!(
                (start.center().x - middle).abs() <= 1.5,
                "{pane:?}: Start is centered at {middle}, got {start:?}"
            );
        }
    }

    /// Stint 1363, gauntlet card 07: `HStack([Column(grow=True),
    /// Column(grow=True)])`. The first column took the whole row and pushed
    /// the second out of the pane. Grow children share the row's width.
    #[test]
    fn row_grow_columns_share_the_width() {
        let gap = style::SPACE_SM;
        // Card 07's row says nothing about `align` (center); the taller
        // column sets the row's height, so both start at its top either way.
        for align in [None, Some("start")] {
            let mut row = serde_json::json!({"type": "row", "children": [1, 2], "gap": gap});
            if let Some(align) = align {
                row["align"] = serde_json::json!(align);
            }
            let tree = wire_tree(&[
                (0, row),
                (
                    1,
                    serde_json::json!({"type": "column", "children": [3, 4], "grow": true}),
                ),
                (
                    2,
                    serde_json::json!({"type": "column", "children": [5], "grow": true}),
                ),
                (3, wire_button("Convert")),
                (
                    4,
                    serde_json::json!({"type": "text", "text": "Amount and unit"}),
                ),
                (5, wire_button("Result")),
            ]);
            for pane in ROW_PANES {
                let rects = laid_out(&tree, pane);
                let (left, right) = content_x(pane);
                let (top, _) = content_y(pane);
                let half = (right - left - gap) / 2.0;
                let convert = rect_of(&rects, 3);
                let result = rect_of(&rects, 5);
                assert!(
                    (convert.min.x - left).abs() <= 1.5 && (convert.min.y - top).abs() <= 1.5,
                    "{align:?} {pane:?}: the first column starts the row at its top: {convert:?}"
                );
                assert!(
                    (result.min.x - (left + half + gap)).abs() <= 1.5,
                    "{align:?} {pane:?}: the second column starts halfway, at {}, got {result:?}",
                    left + half + gap
                );
                assert!(
                    result.max.x <= right + 0.5,
                    "{align:?} {pane:?}: {result:?} is past the row"
                );
            }
        }
    }

    /// Stint 1363, gauntlet card 10: centered `Column` cells in a row each
    /// took the full row width, so the second and third cell were pushed out
    /// of the pane. A column that does not grow takes its content's width.
    #[test]
    fn row_column_children_take_their_content_width() {
        let gap = 12.0;
        let cell = |badge: u32, button: u32| serde_json::json!({"type": "column", "children": [badge, button], "align": "center"});
        let tree = wire_tree(&[
            (
                0,
                serde_json::json!({"type": "row", "children": [1, 2, 3], "gap": gap}),
            ),
            (1, cell(4, 5)),
            (2, cell(6, 7)),
            (3, cell(8, 9)),
            (4, serde_json::json!({"type": "badge", "text": "Happy"})),
            (5, wire_button("Happy")),
            (6, serde_json::json!({"type": "badge", "text": "Meh"})),
            (7, wire_button("Meh")),
            (8, serde_json::json!({"type": "badge", "text": "Sad"})),
            (9, wire_button("Sad")),
        ]);
        for pane in ROW_PANES {
            let rects = laid_out(&tree, pane);
            let (left, right) = content_x(pane);
            let buttons = [rect_of(&rects, 5), rect_of(&rects, 7), rect_of(&rects, 9)];
            assert!(
                (buttons[0].min.x - left).abs() <= 1.5,
                "{pane:?}: the first cell starts the row: {buttons:?}"
            );
            for pair in buttons.windows(2) {
                assert!(
                    pair[1].min.x - pair[0].max.x <= gap + 1.5,
                    "{pane:?}: cells sit side by side, {gap}px apart: {buttons:?}"
                );
            }
            assert!(buttons[2].max.x <= right + 0.5, "{pane:?}: {buttons:?}");
        }
    }

    /// Main-axis placement of three buttons in a row by `justify`.
    fn justified_row(justify: &str) -> UiTree {
        wire_tree(&[
            (
                0,
                serde_json::json!({"type": "row", "children": [1, 2, 3], "justify": justify}),
            ),
            (1, wire_button("One")),
            (2, wire_button("Two")),
            (3, wire_button("Three")),
        ])
    }

    /// Stint 1358: `justify` places a row's children along the row.
    #[test]
    fn row_justify_places_children_along_the_row() {
        for pane in ROW_PANES {
            let (left, right) = content_x(pane);
            let buttons = |justify: &str| {
                let rects = laid_out(&justified_row(justify), pane);
                [rect_of(&rects, 1), rect_of(&rects, 2), rect_of(&rects, 3)]
            };

            let start = buttons("start");
            assert!(
                (start[0].min.x - left).abs() <= 1.5,
                "{pane:?} start: {start:?}"
            );
            assert!(start[2].max.x < right - 10.0, "{pane:?} start: {start:?}");

            let center = buttons("center");
            assert!(
                ((center[0].min.x - left) - (right - center[2].max.x)).abs() <= 1.5,
                "{pane:?} center leaves equal room on both sides: {center:?}"
            );

            let end = buttons("end");
            assert!((end[2].max.x - right).abs() <= 1.5, "{pane:?} end: {end:?}");
            assert!(end[0].min.x > left + 10.0, "{pane:?} end: {end:?}");

            let between = buttons("space-between");
            assert!(
                (between[0].min.x - left).abs() <= 1.5,
                "{pane:?}: {between:?}"
            );
            assert!(
                (between[2].max.x - right).abs() <= 1.5,
                "{pane:?}: {between:?}"
            );
            let first_gap = between[1].min.x - between[0].max.x;
            let second_gap = between[2].min.x - between[1].max.x;
            assert!(
                (first_gap - second_gap).abs() <= 1.5 && first_gap > style::SPACE_SM,
                "{pane:?} space-between spreads the gaps evenly: {between:?}"
            );
        }
    }

    /// Stint 1358: the bridge rejects a `justify` it does not know and names
    /// the values it accepts.
    #[test]
    fn wire_justify_rejects_an_unknown_value() {
        for kind in ["row", "column"] {
            let json = serde_json::json!({"root": 0, "nodes": [
                {"id": 0, "key": "0", "data": {"type": kind, "children": [], "justify": "sideways"}}
            ]});
            let err = crate::host::wasm_python::decode_ui_tree(&json.to_string())
                .expect_err("an unknown justify must not decode")
                .to_string();
            assert!(
                err.contains("sideways") && err.contains("space-between"),
                "{kind}: the error names the value and the accepted set: {err}"
            );
        }
    }

    /// Stint 1358: `justify` places a column's children down the column.
    #[test]
    fn column_justify_places_children_down_the_column() {
        let column = |justify: &str| {
            wire_tree(&[
                (
                    0,
                    serde_json::json!({"type": "column", "children": [1, 2], "justify": justify}),
                ),
                (1, wire_button("Top")),
                (2, wire_button("Bottom")),
            ])
        };
        for pane in ROW_PANES {
            let (top, bottom) = content_y(pane);
            let buttons = |justify: &str| {
                let rects = laid_out(&column(justify), pane);
                (rect_of(&rects, 1), rect_of(&rects, 2))
            };

            let (first, second) = buttons("start");
            assert!(
                (first.min.y - top).abs() <= 1.5,
                "{pane:?} start: {first:?}"
            );
            assert!(second.max.y < bottom - 10.0, "{pane:?} start: {second:?}");

            let (first, second) = buttons("center");
            assert!(
                ((first.min.y - top) - (bottom - second.max.y)).abs() <= 1.5,
                "{pane:?} center leaves equal room above and below: {first:?} {second:?}"
            );

            let (first, second) = buttons("end");
            assert!(
                (second.max.y - bottom).abs() <= 1.5,
                "{pane:?} end: {second:?}"
            );
            assert!(first.min.y > top + 10.0, "{pane:?} end: {first:?}");

            let (first, second) = buttons("space-between");
            assert!((first.min.y - top).abs() <= 1.5, "{pane:?}: {first:?}");
            assert!((second.max.y - bottom).abs() <= 1.5, "{pane:?}: {second:?}");
        }
    }

    /// Run `render` and return how many `render_node` calls it made.
    fn render_node_calls(render: impl FnOnce()) -> usize {
        RENDER_NODE_CALLS.with(|calls| calls.set(0));
        render();
        RENDER_NODE_CALLS.with(|calls| calls.get())
    }

    /// Trees are untrusted guest input and measuring passes re-render a
    /// subtree. Columns that each hold two grow spacers, nested forty deep,
    /// cost two visits per level if every level measures its children again:
    /// 2^40 calls. A measuring pass never lays a flex column out, so one frame
    /// visits each node a bounded number of times.
    #[test]
    fn nested_flex_columns_visit_each_node_a_bounded_number_of_times() {
        const DEPTH: u32 = 40;
        let mut nodes = Vec::new();
        for level in 0..DEPTH {
            let base = level * 4;
            // [spacer, text, spacer, next column]
            nodes.push(column_node(
                base,
                vec![base + 1, base + 2, base + 3, base + 4],
            ));
            nodes.push(grow_node(base + 1));
            nodes.push(text_node(base + 2, "level"));
            nodes.push(grow_node(base + 3));
        }
        nodes.push(text_node(DEPTH * 4, "leaf"));
        let tree = UiTree { root: 0, nodes };
        let count = tree.nodes.len();
        let calls = render_node_calls(|| {
            painted_shapes(&tree, egui::vec2(800.0, 600.0));
        });
        assert!(
            calls <= count * 3,
            "{calls} render_node calls for {count} nodes"
        );
    }

    /// A row of N texts that each overflow the row measured the siblings after
    /// every one of them: N^2 calls. One measuring pass serves the whole row.
    #[test]
    fn wide_row_of_overflowing_texts_is_measured_once() {
        const TEXTS: u32 = 60;
        let mut nodes = vec![row_node(
            0,
            (1..=TEXTS).collect(),
            style::SPACE_SM,
            Alignment::Start,
        )];
        for id in 1..=TEXTS {
            nodes.push(text_node(id, BANNER_TEXT));
        }
        let tree = UiTree { root: 0, nodes };
        let count = tree.nodes.len();
        let calls = render_node_calls(|| {
            painted_shapes(&tree, egui::vec2(320.0, 240.0));
        });
        assert!(
            calls <= count * 3,
            "{calls} render_node calls for {count} nodes"
        );
    }

    /// Rows that each hold a long text and the next row, nested sixty deep:
    /// only the outermost `MAX_MEASURED_ROW_DEPTH` rows measure, so the cost
    /// stays linear in the node count instead of growing with the depth.
    #[test]
    fn nested_rows_stop_measuring_past_the_depth_limit() {
        const DEPTH: u32 = 60;
        let mut nodes = Vec::new();
        for level in 0..DEPTH {
            let base = level * 2;
            nodes.push(row_node(
                base,
                vec![base + 1, base + 2],
                style::SPACE_SM,
                Alignment::End,
            ));
            nodes.push(text_node(base + 1, BANNER_TEXT));
        }
        nodes.push(text_node(DEPTH * 2, "leaf"));
        let tree = UiTree { root: 0, nodes };
        let count = tree.nodes.len();
        let calls = render_node_calls(|| {
            painted_shapes(&tree, egui::vec2(320.0, 240.0));
        });
        assert!(
            calls <= count * (usize::from(MAX_MEASURED_ROW_DEPTH) + 1),
            "{calls} render_node calls for {count} nodes"
        );
    }

    fn flex_column(id: u32, children: Vec<u32>, align: Alignment, justify: Justify) -> IndexedNode {
        node(
            id,
            UiNodeData::Column(ColumnNode {
                children,
                gap: style::SPACE_SM,
                align,
                justify,
                grow: false,
            }),
        )
    }

    fn justified_row_node(id: u32, children: Vec<u32>, justify: Justify) -> IndexedNode {
        node(
            id,
            UiNodeData::Row(RowNode {
                children,
                gap: style::SPACE_SM,
                align: Alignment::Center,
                justify,
                grow: false,
            }),
        )
    }

    /// Stint 1363: a row that hands out its leftover width measures each of
    /// its children once, however many grow spacers and centered columns it
    /// holds.
    #[test]
    fn row_with_grow_children_is_measured_once() {
        const CELLS: u32 = 40;
        let mut nodes = vec![row_node(
            0,
            (0..CELLS).flat_map(|c| [1 + c * 4, 2 + c * 4]).collect(),
            style::SPACE_SM,
            Alignment::End,
        )];
        for c in 0..CELLS {
            let base = 1 + c * 4;
            nodes.push(flex_column(
                base,
                vec![base + 2, base + 3],
                Alignment::Center,
                Justify::Start,
            ));
            nodes.push(grow_node(base + 1));
            nodes.push(text_node(base + 2, "cell"));
            nodes.push(button_node(base + 3, "Go"));
        }
        let tree = UiTree { root: 0, nodes };
        let count = tree.nodes.len();
        let calls = render_node_calls(|| {
            painted_shapes(&tree, egui::vec2(800.0, 600.0));
        });
        assert!(
            calls <= count * 3,
            "{calls} render_node calls for {count} nodes"
        );
    }

    /// Justified rows nested sixty deep, each holding a centered column and
    /// the next row: only the outermost `MAX_MEASURED_ROW_DEPTH` rows measure.
    #[test]
    fn nested_justified_rows_stop_measuring_past_the_depth_limit() {
        const DEPTH: u32 = 60;
        let mut nodes = Vec::new();
        for level in 0..DEPTH {
            let base = level * 3;
            nodes.push(justified_row_node(
                base,
                vec![base + 1, base + 3],
                Justify::SpaceBetween,
            ));
            nodes.push(flex_column(
                base + 1,
                vec![base + 2],
                Alignment::Center,
                Justify::Start,
            ));
            nodes.push(text_node(base + 2, "level"));
        }
        nodes.push(text_node(DEPTH * 3, "leaf"));
        let tree = UiTree { root: 0, nodes };
        let count = tree.nodes.len();
        let calls = render_node_calls(|| {
            painted_shapes(&tree, egui::vec2(320.0, 240.0));
        });
        assert!(
            calls <= count * (usize::from(MAX_MEASURED_ROW_DEPTH) + 1),
            "{calls} render_node calls for {count} nodes"
        );
    }

    /// Stint 1358: a justified column is a flex column, so a measuring pass
    /// never lays it out. Forty nested centered columns, each inside a
    /// grow-spacer column, stay linear in the node count.
    #[test]
    fn nested_justified_columns_visit_each_node_a_bounded_number_of_times() {
        const DEPTH: u32 = 40;
        let mut nodes = Vec::new();
        for level in 0..DEPTH {
            let base = level * 3;
            let justify = if level % 2 == 0 {
                Justify::Center
            } else {
                Justify::SpaceBetween
            };
            nodes.push(flex_column(
                base,
                vec![base + 1, base + 2, base + 3],
                Alignment::Center,
                justify,
            ));
            nodes.push(text_node(base + 1, "level"));
            nodes.push(button_node(base + 2, "Go"));
        }
        nodes.push(text_node(DEPTH * 3, "leaf"));
        let tree = UiTree { root: 0, nodes };
        let count = tree.nodes.len();
        let calls = render_node_calls(|| {
            painted_shapes(&tree, egui::vec2(800.0, 600.0));
        });
        assert!(
            calls <= count * 3,
            "{calls} render_node calls for {count} nodes"
        );
    }

    /// Centered columns inside justified rows inside centered columns, thirty
    /// levels: each row measures its column once, and rows inside that
    /// measurement do not measure, so the visits stay within the frame bound.
    #[test]
    fn rows_of_columns_nested_deep_stay_within_the_visit_bound() {
        const DEPTH: u32 = 30;
        let mut nodes = Vec::new();
        for level in 0..DEPTH {
            let base = level * 4;
            nodes.push(justified_row_node(
                base,
                vec![base + 1, base + 2],
                Justify::Center,
            ));
            nodes.push(text_node(base + 1, BANNER_TEXT));
            nodes.push(flex_column(
                base + 2,
                vec![base + 3, base + 4],
                Alignment::End,
                Justify::Start,
            ));
            nodes.push(button_node(base + 3, "Go"));
        }
        nodes.push(text_node(DEPTH * 4, "leaf"));
        let tree = UiTree { root: 0, nodes };
        let count = tree.nodes.len();
        let calls = render_node_calls(|| {
            painted_shapes(&tree, egui::vec2(320.0, 240.0));
        });
        assert!(
            calls <= count * 2 * (usize::from(MAX_MEASURED_ROW_DEPTH) + 1),
            "{calls} render_node calls for {count} nodes"
        );
    }

    /// The arithmetic of a row's main axis: grow children share the leftover
    /// width, `justify` places fixed children, children that do not fit and
    /// hostile widths fall back to the row's usual layout.
    #[test]
    fn place_along_row_hands_out_the_leftover_width() {
        let fixed = [Some(40.0), Some(60.0)];
        let place =
            |natural: &[Option<f32>], justify| place_along_row(natural, 10.0, 210.0, justify);
        assert_eq!(
            place(&fixed, Justify::Start),
            Some(RowPlacement {
                lead: 0.0,
                gap: 10.0,
                widths: vec![40.0, 60.0]
            })
        );
        assert_eq!(place(&fixed, Justify::Center).map(|p| p.lead), Some(50.0));
        assert_eq!(place(&fixed, Justify::End).map(|p| p.lead), Some(100.0));
        assert_eq!(
            place(&fixed, Justify::SpaceBetween).map(|p| p.gap),
            Some(110.0)
        );
        assert_eq!(
            place(&[Some(40.0)], Justify::SpaceBetween).map(|p| p.lead),
            Some(0.0)
        );
        // Grow children share the leftover equally and override `justify`.
        assert_eq!(
            place(&[Some(40.0), None, Some(60.0), None], Justify::End),
            Some(RowPlacement {
                lead: 0.0,
                gap: 10.0,
                widths: vec![40.0, 40.0, 60.0, 40.0]
            })
        );
        assert_eq!(place(&[Some(400.0), None], Justify::Start), None);
        assert_eq!(
            place_along_row(&fixed, 10.0, f32::INFINITY, Justify::Center),
            None
        );
        assert_eq!(
            place_along_row(&fixed, 10.0, f32::NAN, Justify::Center),
            None
        );
        assert_eq!(place_along_row(&[], 10.0, 100.0, Justify::Center), None);
        for hostile in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -50.0, 1.0e12] {
            let gap = sanitized_gap(hostile);
            assert!((0.0..=MAX_GAP).contains(&gap), "{hostile} -> {gap}");
        }
    }

    /// Stint 1358: an absent `justify` decodes to `start`, which places
    /// children as rows and columns always have; every value round-trips.
    #[test]
    fn wire_justify_defaults_to_start_and_decodes_every_value() {
        let justify_of = |data: serde_json::Value| match wire_tree(&[(0, data)]).nodes[0].data {
            UiNodeData::Row(ref r) => r.justify,
            UiNodeData::Column(ref c) => c.justify,
            ref other => panic!("not a container: {other:?}"),
        };
        for kind in ["row", "column"] {
            assert_eq!(
                justify_of(serde_json::json!({"type": kind, "children": []})),
                Justify::Start
            );
            for (wire, justify) in [
                ("start", Justify::Start),
                ("center", Justify::Center),
                ("end", Justify::End),
                ("space-between", Justify::SpaceBetween),
            ] {
                assert_eq!(
                    justify_of(serde_json::json!({"type": kind, "children": [], "justify": wire})),
                    justify
                );
            }
        }
    }

    /// The two measurements together: a flex column measures its spans, and
    /// the rows inside them wrap their text in that pass too, so the column
    /// sees the wrapped height. Still linear in the node count.
    #[test]
    fn flex_column_over_nested_rows_stays_within_the_visit_bound() {
        const DEPTH: u32 = 30;
        let mut nodes = vec![column_node(0, vec![1, 2, 3, 4])];
        nodes.push(grow_node(1));
        nodes.push(text_node(2, "value"));
        nodes.push(grow_node(3));
        for level in 0..DEPTH {
            let base = 4 + level * 2;
            nodes.push(row_node(
                base,
                vec![base + 1, base + 2],
                style::SPACE_SM,
                Alignment::End,
            ));
            nodes.push(text_node(base + 1, BANNER_TEXT));
        }
        nodes.push(text_node(4 + DEPTH * 2, "leaf"));
        let tree = UiTree { root: 0, nodes };
        let count = tree.nodes.len();
        let calls = render_node_calls(|| {
            painted_shapes(&tree, egui::vec2(320.0, 240.0));
        });
        assert!(
            calls <= count * 2 * (usize::from(MAX_MEASURED_ROW_DEPTH) + 1),
            "{calls} render_node calls for {count} nodes"
        );
    }

    /// A column measures its spans before placing them, and a row in a span
    /// wraps its text in that measurement exactly as it will when painted:
    /// the span after a wrapped banner starts below all of its lines.
    #[test]
    fn column_measures_a_wrapped_row_at_its_wrapped_height() {
        let tree = UiTree {
            root: 0,
            nodes: vec![
                column_node(0, vec![1, 4, 5, 6, 7]),
                row_node(1, vec![2, 3], 8.0, Alignment::Start),
                badge_node(2, "warning"),
                text_node(3, BANNER_TEXT),
                grow_node(4),
                text_node(5, "MIDDLE"),
                grow_node(6),
                text_node(7, "TAIL"),
            ],
        };
        let pane = egui::vec2(320.0, 240.0);
        let texts = painted_texts(&tree, pane);
        let banner = painted_text(&texts, BANNER_TEXT);
        let middle = painted_text(&texts, "MIDDLE");
        assert!(banner.rows.len() > 1, "the banner must wrap at {pane:?}");
        assert!(
            middle.rect.min.y >= banner.rect.max.y,
            "the middle span must start below the wrapped banner: {:?} then {:?}",
            banner.rect,
            middle.rect
        );
    }

    /// A `Text` size is guest input: NaN and infinity fall back to the body
    /// size and absurd sizes are clamped before any font is laid out.
    #[test]
    fn text_size_is_finite_and_bounded() {
        let sized = |size: Option<f32>| {
            text_size(&TextNode {
                text: String::new(),
                size,
                bold: false,
                color: None,
                truncate: false,
                align: Alignment::Start,
            })
        };
        assert_eq!(sized(None), style::TEXT_BODY);
        assert_eq!(sized(Some(f32::NAN)), style::TEXT_BODY);
        assert_eq!(sized(Some(f32::INFINITY)), style::TEXT_BODY);
        assert_eq!(sized(Some(f32::NEG_INFINITY)), style::TEXT_BODY);
        assert_eq!(sized(Some(18.0)), 18.0);
        assert_eq!(sized(Some(-4.0)), 1.0);
        assert_eq!(sized(Some(1.0e9)), 512.0);
        // The whole tree still renders with a hostile size in it.
        let tree = UiTree {
            root: 0,
            nodes: vec![node(
                0,
                UiNodeData::Text(TextNode {
                    text: "hostile".to_string(),
                    size: Some(f32::NAN),
                    bold: false,
                    color: None,
                    truncate: false,
                    align: Alignment::Start,
                }),
            )],
        };
        let texts = painted_texts(&tree, egui::vec2(320.0, 240.0));
        assert!(painted_text(&texts, "hostile").rect.is_finite());
    }

    /// Stint 1333 must not touch keypad grids: a calc-shaped grid keeps its
    /// fixed 64px cells at every check size instead of wrapping or resizing.
    #[test]
    fn keypad_grid_keeps_fixed_cells_at_the_check_sizes() {
        let keys = [
            ["7", "8", "9", "/"],
            ["4", "5", "6", "*"],
            ["1", "2", "3", "-"],
        ];
        let mut nodes = vec![column_node(0, vec![1, 2, 3])];
        for (row, labels) in keys.iter().enumerate() {
            let row_id = 1 + row as u32;
            let first = 10 + row as u32 * 10;
            nodes.push(row_node(
                row_id,
                (first..first + 4).collect(),
                style::SPACE_SM,
                Alignment::Start,
            ));
            for (col, label) in labels.iter().enumerate() {
                nodes.push(button_node(first + col as u32, label));
            }
        }
        let tree = UiTree { root: 0, nodes };
        assert!(column_button_grid_spec(&tree.nodes, 0).is_some());
        for pane in CHECK_SIZES {
            let texts = painted_texts(&tree, pane);
            for labels in keys {
                let centers: Vec<egui::Pos2> = labels
                    .iter()
                    .map(|label| painted_text(&texts, label).rect.center())
                    .collect();
                for pair in centers.windows(2) {
                    assert!(
                        (pair[1].x - pair[0].x - (KEYPAD_BUTTON_W + style::SPACE_SM)).abs() <= 1.0,
                        "{pane:?}: keypad cells must stay {KEYPAD_BUTTON_W}px apart plus the gap, got {centers:?}"
                    );
                    assert!(
                        (pair[1].y - pair[0].y).abs() <= 0.5,
                        "{pane:?}: a keypad row must not wrap, got {centers:?}"
                    );
                }
            }
        }
    }

    /// Content taller than a column with two grow spacers: the spacers collapse
    /// and no span is dropped. The head starts at the top, the middle span is
    /// laid out after it, and the last span still holds the bottom edge.
    #[test]
    fn overfull_column_with_two_grow_spacers_keeps_every_span() {
        let tall = |id: u32, text: &str| {
            node(
                id,
                UiNodeData::Text(TextNode {
                    text: text.to_string(),
                    size: Some(60.0),
                    bold: false,
                    color: None,
                    truncate: false,
                    align: Alignment::Start,
                }),
            )
        };
        let tree = UiTree {
            root: 0,
            nodes: vec![
                column_node(0, vec![1, 2, 3, 4, 5]),
                tall(1, "HEAD"),
                grow_node(2),
                tall(3, "MIDDLE"),
                grow_node(4),
                tall(5, "TAIL"),
            ],
        };
        let pane = egui::vec2(320.0, 150.0);
        let texts = painted_texts(&tree, pane);
        let head = painted_text(&texts, "HEAD");
        let middle = painted_text(&texts, "MIDDLE");
        let tail = painted_text(&texts, "TAIL");
        assert!(
            head.rect.min.y < style::SPACE_MD + 2.0,
            "head starts at the top: {:?}",
            head.rect
        );
        assert!(
            (middle.rect.min.y - head.rect.max.y).abs() <= 2.0,
            "collapsed spacers put the middle span right after the head: {:?} then {:?}",
            head.rect,
            middle.rect
        );
        assert!(
            (tail.rect.max.y - (pane.y - style::SPACE_MD)).abs() <= 2.0,
            "the last span holds the bottom edge: {:?}",
            tail.rect
        );
        for t in [head, middle, tail] {
            assert!(
                t.clip.intersects(t.rect),
                "{:?} must be at least partly painted: rect {:?}, clip {:?}",
                t.text,
                t.rect,
                t.clip
            );
        }
    }

    /// The layout pass behind `plugin check` records what each node painted.
    /// Measuring passes render the same nodes invisibly and must not be
    /// recorded, and a wrapped row text is painted by the `Text` arm like any
    /// other, so it has one entry that is neither cut nor broken mid-word.
    #[test]
    fn layout_pass_records_each_wrapped_row_node_once() {
        for (name, tree) in [
            ("banner", banner_tree(true)),
            (
                "actions",
                button_row_tree(
                    &["Start timer", "Pause", "Reset", "Add a lap", "Settings"],
                    style::SPACE_SM,
                ),
            ),
            ("flex", centered_tree()),
        ] {
            let layout = render_ui_tree_layout(&tree, 320.0, 240.0);
            for node in &tree.nodes {
                let entries: Vec<&NodeLayout> =
                    layout.iter().filter(|fact| fact.id == node.id).collect();
                assert!(
                    entries.len() <= 1,
                    "{name}: node {} was recorded more than once: {entries:?}",
                    node.id
                );
                let paints = matches!(
                    node.data,
                    UiNodeData::Text(_) | UiNodeData::Button(_) | UiNodeData::Badge(_)
                );
                if paints {
                    assert_eq!(
                        entries.len(),
                        1,
                        "{name}: node {} painted and should have a layout entry",
                        node.id
                    );
                    let fact = entries[0];
                    assert!(
                        fact.visible.is_some() && !fact.text_cut && !fact.text_wrapped_mid_word,
                        "{name}: node {} should be fully painted, got {fact:?}",
                        node.id
                    );
                }
            }
        }
    }

    fn button_row_tree(labels: &[&str], gap: f32) -> UiTree {
        let ids: Vec<u32> = (2..2 + labels.len() as u32).collect();
        let mut nodes = vec![
            column_node(0, vec![1]),
            row_node(1, ids.clone(), gap, Alignment::Start),
        ];
        for (id, label) in ids.iter().zip(labels) {
            nodes.push(button_node(*id, label));
        }
        UiTree { root: 0, nodes }
    }

    /// Every label is painted whole, on one line, inside the pane; returns the
    /// number of distinct lines the buttons landed on.
    fn assert_whole_buttons(tree: &UiTree, labels: &[&str], pane: egui::Vec2) -> usize {
        let texts = painted_texts(tree, pane);
        let mut lines: Vec<i32> = Vec::new();
        for label in labels {
            let t = painted_text(&texts, label);
            assert_eq!(
                t.rows.len(),
                1,
                "{pane:?}: button label {label:?} wrapped into {:?}",
                t.rows
            );
            assert!(
                t.visible_in(pane),
                "{pane:?}: button {label:?} at {:?} is cut off (clip {:?})",
                t.rect,
                t.clip
            );
            let y = t.rect.center().y.round() as i32;
            if !lines.contains(&y) {
                lines.push(y);
            }
        }
        lines.len()
    }

    /// Stint 1333: a button row was one `ui.horizontal`, so in a narrow pane the
    /// last button wrapped its label mid-word and later buttons were cut off.
    #[test]
    fn button_row_wraps_whole_buttons_in_a_narrow_pane() {
        let labels = ["Start timer", "Pause", "Reset", "Add a lap", "Settings"];
        let tree = button_row_tree(&labels, style::SPACE_SM);
        for pane in [egui::vec2(320.0, 240.0), egui::vec2(300.0, 900.0)] {
            let lines = assert_whole_buttons(&tree, &labels, pane);
            assert!(
                lines >= 2,
                "{pane:?}: five buttons cannot fit one line, yet they landed on {lines}"
            );
        }
        // Wide enough for one line: the row must not wrap.
        let lines = assert_whole_buttons(&tree, &labels, egui::vec2(800.0, 600.0));
        assert_eq!(lines, 1, "a row that fits stays on one line");
    }

    /// Stint 1333: a seven-option `Select` lost its last option at 320px.
    #[test]
    fn seven_option_select_shows_every_option_in_a_narrow_pane() {
        let labels = [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ];
        let tree = button_row_tree(&labels, 4.0);
        for pane in [egui::vec2(320.0, 240.0), egui::vec2(300.0, 900.0)] {
            assert_whole_buttons(&tree, &labels, pane);
        }
    }

    const BANNER_TEXT: &str =
        "Sync paused: the remote copy changed while you were offline, review before saving";

    fn banner_tree(dismissible: bool) -> UiTree {
        let mut children = vec![2, 3];
        let mut nodes = vec![badge_node(2, "warning"), text_node(3, BANNER_TEXT)];
        if dismissible {
            children.push(4);
            nodes.push(button_node(4, "×"));
        }
        let mut all = vec![
            column_node(0, vec![1]),
            row_node(1, children, 8.0, Alignment::Start),
        ];
        all.append(&mut nodes);
        UiTree {
            root: 0,
            nodes: all,
        }
    }

    /// Stint 1333: a `Banner` is a row of badge, text and an optional dismiss
    /// button; its text ran past the pane edge instead of wrapping.
    #[test]
    fn banner_text_wraps_inside_a_narrow_pane() {
        for pane in [egui::vec2(320.0, 240.0), egui::vec2(300.0, 900.0)] {
            for dismissible in [false, true] {
                let texts = painted_texts(&banner_tree(dismissible), pane);
                let text = painted_text(&texts, BANNER_TEXT);
                assert!(
                    text.visible_in(pane),
                    "{pane:?} dismissible={dismissible}: banner text at {:?} is cut off (clip {:?})",
                    text.rect,
                    text.clip
                );
                assert!(
                    text.rows.len() > 1 && text.breaks_only_between_words(),
                    "{pane:?}: banner text should wrap between words, got rows {:?}",
                    text.rows
                );
                if dismissible {
                    let dismiss = painted_text(&texts, "×");
                    assert!(
                        dismiss.visible_in(pane),
                        "{pane:?}: dismiss button at {:?} must stay in the pane",
                        dismiss.rect
                    );
                    assert!(
                        dismiss.rect.min.x >= text.rect.max.x,
                        "{pane:?}: the text must leave room for the dismiss button"
                    );
                }
            }
        }
    }

    /// A banner that fits is laid out exactly as before: one unwrapped line.
    #[test]
    fn banner_text_that_fits_stays_on_one_line() {
        let texts = painted_texts(&banner_tree(true), egui::vec2(1200.0, 300.0));
        assert_eq!(painted_text(&texts, BANNER_TEXT).rows.len(), 1);
    }

    /// Stint 1333: a button squeezed by its row siblings broke its label one
    /// letter per line. A button is never narrower than its longest word.
    #[test]
    fn button_label_never_breaks_mid_word() {
        let tree = UiTree {
            root: 0,
            nodes: vec![
                row_node(0, vec![1, 2], style::SPACE_SM, Alignment::Start),
                badge_node(1, "live"),
                button_node(2, "Recalibrate sensors"),
            ],
        };
        let texts = painted_texts(&tree, egui::vec2(130.0, 240.0));
        let label = painted_text(&texts, "Recalibrate sensors");
        assert!(
            label.breaks_only_between_words(),
            "label broke inside a word: {:?}",
            label.rows
        );
    }

    #[test]
    fn pack_row_lines_breaks_before_the_child_that_does_not_fit() {
        assert_eq!(pack_row_lines(&[40.0, 40.0, 40.0], 10.0, 200.0), vec![0..3]);
        assert_eq!(
            pack_row_lines(&[40.0, 40.0, 40.0], 10.0, 100.0),
            vec![0..2, 2..3]
        );
        // A child wider than the row still gets a line of its own.
        assert_eq!(
            pack_row_lines(&[40.0, 300.0, 40.0], 10.0, 100.0),
            vec![0..1, 1..2, 2..3]
        );
        assert_eq!(
            pack_row_lines(&[40.0, 40.0], 10.0, f32::INFINITY),
            vec![0..2]
        );
        assert!(pack_row_lines(&[], 10.0, 100.0).is_empty());
    }

    /// Visual evidence for stints 1327 and 1333 at the check matrix sizes: a
    /// value centered between two grow spacers above an `Actions` row,
    /// and a banner, a seven-option select and an action row in flow.
    #[test]
    fn screenshot_column_flex_and_narrow_rows() {
        let labels = ["Start timer", "Pause", "Reset", "Add a lap", "Settings"];
        let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let header = |title: &str| {
            node(
                1,
                UiNodeData::SurfaceHeader(SurfaceHeaderNode {
                    title: title.to_string(),
                    subtitle: "paused".to_string(),
                }),
            )
        };
        let footer = node(
            9,
            UiNodeData::FooterKeys(FooterKeysNode {
                entries: vec![FooterKeyEntry {
                    keys: vec!["s".to_string()],
                    description: "start".to_string(),
                }],
                divider: true,
            }),
        );
        let buttons = |first: u32, labels: &[&str]| -> Vec<IndexedNode> {
            labels
                .iter()
                .enumerate()
                .map(|(i, label)| button_node(first + i as u32, label))
                .collect()
        };

        let mut flex = vec![
            column_node(0, vec![1, 2, 3, 4, 5, 9]),
            header("Sand timer"),
            grow_node(2),
            node(
                3,
                UiNodeData::Text(TextNode {
                    text: "04:32".to_string(),
                    size: Some(28.0),
                    bold: true,
                    color: None,
                    truncate: false,
                    align: Alignment::Center,
                }),
            ),
            grow_node(4),
            row_node(5, (30..35).collect(), style::SPACE_SM, Alignment::Start),
            footer.clone(),
        ];
        flex.extend(buttons(30, &labels));

        let mut rows = vec![
            column_node(0, vec![1, 2, 3, 4, 9]),
            header("Narrow rows"),
            row_node(2, vec![10, 11, 12], 8.0, Alignment::Start),
            row_node(3, (20..27).collect(), 4.0, Alignment::Start),
            row_node(4, (30..35).collect(), style::SPACE_SM, Alignment::Start),
            footer,
            badge_node(10, "warning"),
            text_node(11, BANNER_TEXT),
            button_node(12, "×"),
        ];
        rows.extend(buttons(20, &days));
        rows.extend(buttons(30, &labels));

        for (name, nodes) in [("flex", flex), ("rows", rows)] {
            let tree = UiTree { root: 0, nodes };
            for pane in CHECK_SIZES {
                let png = render_ui_tree_to_png(&tree, pane.x, pane.y, 2.0).expect("render tree");
                let path = crate::ui_tests::evidence_png!(format!("_{name}_{}x{}", pane.x, pane.y));
                std::fs::write(&path, png).expect("write screenshot for visual review");
            }
        }
    }

    /// The rows stints 1358 and 1363 fix, in one plugin view: a grow spacer
    /// between two buttons, a centered pair, `space-between`, two growing
    /// columns (gauntlet card 07) and centered column cells (card 10).
    fn rows_along_the_main_axis_tree() -> UiTree {
        wire_tree(&[
            (
                0,
                serde_json::json!({"type": "column", "children": [1, 2, 3, 4, 5]}),
            ),
            (
                1,
                serde_json::json!({"type": "row", "children": [10, 11, 12]}),
            ),
            (10, wire_button("Back")),
            (11, wire_grow_spacer()),
            (12, wire_button("Next")),
            (
                2,
                serde_json::json!({"type": "row", "children": [20, 21], "justify": "center"}),
            ),
            (20, wire_button("Pause")),
            (21, wire_button("Reset")),
            (
                3,
                serde_json::json!({"type": "row", "children": [22, 23, 24], "justify": "space-between"}),
            ),
            (22, wire_button("1 min")),
            (23, wire_button("5 min")),
            (24, wire_button("10 min")),
            (
                4,
                serde_json::json!({"type": "row", "children": [30, 31], "align": "start"}),
            ),
            (
                30,
                serde_json::json!({"type": "column", "children": [32, 33], "grow": true}),
            ),
            (
                31,
                serde_json::json!({"type": "column", "children": [34, 35], "grow": true}),
            ),
            (32, wire_button("Convert")),
            (33, serde_json::json!({"type": "text", "text": "12 in"})),
            (34, wire_button("Copy")),
            (35, serde_json::json!({"type": "text", "text": "30.48 cm"})),
            (
                5,
                serde_json::json!({"type": "row", "children": [40, 41, 42], "gap": 12.0}),
            ),
            (
                40,
                serde_json::json!({"type": "column", "children": [43, 44], "align": "center"}),
            ),
            (
                41,
                serde_json::json!({"type": "column", "children": [45, 46], "align": "center"}),
            ),
            (
                42,
                serde_json::json!({"type": "column", "children": [47, 48], "align": "center"}),
            ),
            (
                43,
                serde_json::json!({"type": "badge", "text": "good", "color": "success"}),
            ),
            (44, wire_button("Happy")),
            (
                45,
                serde_json::json!({"type": "badge", "text": "fine", "color": "warning"}),
            ),
            (46, wire_button("Meh")),
            (
                47,
                serde_json::json!({"type": "badge", "text": "low", "color": "danger"}),
            ),
            (48, wire_button("Sad")),
        ])
    }

    /// Visual evidence for stints 1358 and 1363 at the smallest check size
    /// and a normal one, plus a column centered by `justify`.
    #[test]
    fn screenshot_rows_along_the_main_axis() {
        let centered = wire_tree(&[
            (
                0,
                serde_json::json!({"type": "column", "children": [1, 2], "align": "center", "justify": "center"}),
            ),
            (
                1,
                serde_json::json!({"type": "text", "text": "04:32", "size": 28.0, "bold": true}),
            ),
            (
                2,
                serde_json::json!({"type": "row", "children": [3, 4], "justify": "center"}),
            ),
            (3, wire_button("Start")),
            (4, wire_button("Reset")),
        ]);
        for (name, tree) in [
            ("rows", rows_along_the_main_axis_tree()),
            ("centered", centered),
        ] {
            for pane in ROW_PANES {
                let png = render_ui_tree_to_png(&tree, pane.x, pane.y, 2.0).expect("render tree");
                let path = crate::ui_tests::evidence_png!(format!("_{name}_{}x{}", pane.x, pane.y));
                std::fs::write(&path, png).expect("write screenshot for visual review");
            }
        }
    }

    /// Stint 0674: `autofocus` makes a field its pane's *default* text surface,
    /// which the focus reconciler grants when the pane owns input and nothing
    /// else is focused. A field that did not ask for focus only registers.
    #[test]
    fn autofocus_text_input_registers_as_the_panes_default_surface() {
        fn default_surface_exists(autofocus: bool) -> bool {
            let ctx = egui::Context::default();
            crate::ui::theme::setup_fonts(&ctx);
            let colors = Colors::from_config(
                &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
            );
            let key = crate::ui::focus::SurfaceKey::Pane(7);
            let tree = UiTree {
                root: 0,
                nodes: vec![node(
                    0,
                    UiNodeData::TextInput(TextInputNode {
                        value: String::new(),
                        placeholder: "What needs doing?".to_string(),
                        on_change: "draft".to_string(),
                        on_submit: "draft".to_string(),
                        password: false,
                        autofocus,
                    }),
                )],
            };
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 200.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        let _ = render_ui_tree_with_surface(ui, &tree, &colors, None, Some(key), 0);
                    });
            });
            let registry = crate::ui::focus::drain_text_surfaces(&ctx);
            assert!(
                registry.default_for(key).is_some() || !registry.surfaces.is_empty(),
                "the field must register as a text surface either way"
            );
            registry.default_for(key).is_some()
        }

        assert!(default_surface_exists(true));
        assert!(!default_surface_exists(false));
    }

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/sysmon.wasm")
    }

    #[test]
    fn contained_canvas_preserves_source_aspect_ratio_and_centers_content() {
        let container = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 700.0));

        let (origin, sx, sy) = canvas_transform(container, 360.0, 440.0, CanvasFit::Contain);

        let expected_scale = 300.0 / 360.0;
        let expected_height = 440.0 * expected_scale;
        assert!((sx - expected_scale).abs() < f32::EPSILON);
        assert!((sy - expected_scale).abs() < f32::EPSILON);
        assert!((origin.x - container.left()).abs() < f32::EPSILON);
        assert!((origin.y - (container.top() + (700.0 - expected_height) / 2.0)).abs() < 0.001);
    }

    #[test]
    fn fill_canvas_uses_the_entire_allocated_rect() {
        let container = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 700.0));

        let (origin, sx, sy) = canvas_transform(container, 640.0, 360.0, CanvasFit::Fill);

        assert_eq!(origin, container.min);
        assert!((sx - 300.0 / 640.0).abs() < f32::EPSILON);
        assert!((sy - 700.0 / 360.0).abs() < f32::EPSILON);
    }

    // Stint 0390: a grow canvas's height must track the pane like its width
    // does, not stay floored at the plugin's declared height. Shrinking the pane
    // below the declared height previously left the canvas rect taller than
    // the pane, painting content below the visible edge.
    #[test]
    fn grow_canvas_height_tracks_pane_when_shrunk_below_declared_height() {
        let declared_height = 360.0;
        let pane_height = 100.0;

        let tree = UiTree {
            root: 0,
            nodes: vec![IndexedNode {
                id: 0,
                key: String::new(),
                data: UiNodeData::Canvas(CanvasNode {
                    width: 0.0,
                    height: declared_height,
                    grow: true,
                    commands: vec![],
                }),
            }],
        };

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );

        let mut raw_input = egui::RawInput::default();
        raw_input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(200.0, pane_height),
        ));

        let mut allocated_height = 0.0f32;
        let _ = ctx.run_ui(raw_input, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    let _ = render_ui_tree_with_surface(ui, &tree, &colors, None, None, 0);
                    allocated_height = ui.min_rect().height();
                });
        });

        assert!(
            allocated_height <= pane_height + 1.0,
            "grow canvas allocated {allocated_height}px tall in a {pane_height}px pane, \
             declared height was {declared_height}px"
        );
    }

    // Stint 0397: a click at a known screen pixel inside a scaled `grow`
    // canvas must arrive as the correct canvas-space coordinate, not the raw
    // pixel. This is the transform-inversion regression guard — it exercises
    // the exact `canvas_transform` math the painter uses, inverted.
    #[test]
    fn canvas_click_inverts_fit_transform_to_canvas_space() {
        let declared_width = 360.0;
        let declared_height = 440.0;
        let pane_width = 200.0;
        let pane_height = 400.0;

        let tree = UiTree {
            root: 0,
            nodes: vec![IndexedNode {
                id: 0,
                key: String::new(),
                data: UiNodeData::Canvas(CanvasNode {
                    width: declared_width,
                    height: declared_height,
                    grow: true,
                    commands: vec![],
                }),
            }],
        };
        let mut fits = HashMap::new();
        fits.insert(0, CanvasFit::Contain);

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );

        // sx = sy = 200/360 = 5/9; origin.y = 200 - (440 * 5/9)/2 = 700/9.
        // A click at screen pixel (50, 100) inverts to canvas space (90, 40).
        let click_px = egui::pos2(50.0, 100.0);
        let screen_rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(pane_width, pane_height));

        // Warm-up frame with no input: egui hit-tests a press/release pair
        // against the *previous* frame's widget rects, so the canvas must
        // already have been laid out once before a click can resolve.
        let warmup = egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        };
        let _ = ctx.run_ui(warmup, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    let _ = render_ui_tree_with_canvas_fits(
                        ui,
                        &tree,
                        &colors,
                        None,
                        Some(&fits),
                        None,
                        None,
                        0,
                    );
                });
        });

        // Move + press + release delivered together as one frame's input,
        // mirroring the click-simulation pattern proven in `src/ui_tests.rs`
        // (`PlexiUiHarness` sidebar tests).
        let click_frame = egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(click_px),
                egui::Event::PointerButton {
                    pos: click_px,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: click_px,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        };
        let mut captured = RenderResult::default();
        let _ = ctx.run_ui(click_frame, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    captured = render_ui_tree_with_canvas_fits(
                        ui,
                        &tree,
                        &colors,
                        None,
                        Some(&fits),
                        None,
                        None,
                        0,
                    );
                });
        });

        assert_eq!(
            captured.canvas_clicks.len(),
            1,
            "one click at a known pixel should produce exactly one canvas click"
        );
        let click = captured.canvas_clicks[0];
        assert!(
            (click.x - 90.0).abs() < 0.01,
            "expected canvas-space x 90.0, got {}",
            click.x
        );
        assert!(
            (click.y - 40.0).abs() < 0.01,
            "expected canvas-space y 40.0, got {}",
            click.y
        );
        assert_eq!(click.button, Some("left"));
        assert!(click.pressed);
    }

    // G4 foundation: a real sysmon view tree (after delivering stats) renders
    // through the egui pipeline headlessly without panicking, and produces no
    // spurious actions.
    #[test]
    fn renders_sysmon_tree_headless() -> wasmtime::Result<()> {
        let mut app =
            WasmView::load_ephemeral_run("sysmon-render", &fixture(), StateStore::ephemeral())?;
        app.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), &[])?;
        app.update(&InputEvent::SystemStatsResult(SystemStats {
            cpu_usage_pct: 42.0,
            memory_used_bytes: 8u64 << 30,
            memory_total_bytes: 16u64 << 30,
            disk_read_bps: 0,
            disk_write_bps: 0,
            net_rx_bps: 0,
            net_tx_bps: 0,
            uptime_secs: 0,
            load_avg_one_min: 0.0,
        }))?;
        let tree = app.view()?;

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );

        let mut captured = RenderResult::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                captured = render_ui_tree_with_surface(ui, &tree, &colors, None, None, 0);
            });
        });

        assert!(
            captured.actions.is_empty(),
            "sysmon view declares no actions"
        );
        Ok(())
    }

    #[test]
    fn card_data_table_list_view_and_text_align_render_and_dispatch_on_select() {
        let tree = crate::host::wasm_python::decode_ui_tree(
            &serde_json::json!({
                "root": 0,
                "nodes": [
                    {
                        "id": 0,
                        "key": "root",
                        "data": {
                            "type": "Column",
                            "children": [1, 2, 4, 5],
                            "gap": 8.0,
                            "align": "start",
                            "grow": true
                        }
                    },
                    {
                        "id": 1,
                        "key": "bar",
                        "data": {
                            "type": "SurfaceHeader",
                            "title": "SDK v3 Hardening",
                            "subtitle": "Card · DataTable · SelectList · Text"
                        }
                    },
                    {
                        "id": 2,
                        "key": "card",
                        "data": {
                            "type": "Padding",
                            "card": true,
                            "top": 12.0,
                            "right": 12.0,
                            "bottom": 12.0,
                            "left": 12.0,
                            "child": 3
                        }
                    },
                    {
                        "id": 3,
                        "key": "card-text",
                        "data": {
                            "type": "Text",
                            "text": "Right-aligned accent summary inside Card",
                            "size": 14.0,
                            "bold": true,
                            "color": "accent",
                            "truncate": true,
                            "align": "end"
                        }
                    },
                    {
                        "id": 4,
                        "key": "table",
                        "data": {
                            "type": "DataTable",
                            "columns": ["Service", "Status", "Latency"],
                            "rows": [
                                ["api-gateway", "healthy", "12ms"],
                                ["worker-pool", "degraded", "184ms"]
                            ],
                            "selected": 0,
                            "on_select": "select_row",
                            "children": []
                        }
                    },
                    {
                        "id": 5,
                        "key": "list",
                        "data": {
                            "type": "ListView",
                            "items": [6, 7],
                            "selected": 1,
                            "item_height": 28.0,
                            "on_select": "select_item"
                        }
                    },
                    {
                        "id": 6,
                        "key": "item-0",
                        "data": {
                            "type": "Text",
                            "text": "First selectable row"
                        }
                    },
                    {
                        "id": 7,
                        "key": "item-1",
                        "data": {
                            "type": "Text",
                            "text": "Second selectable row (selected)"
                        }
                    }
                ]
            })
            .to_string(),
        )
        .expect("decode widget showcase tree");

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );

        // Render and verify Card surface rect + accent text + table/list actions on click.
        let mut table_click = RenderResult::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                table_click = render_ui_tree_with_canvas_fits(
                    ui,
                    &tree,
                    &colors,
                    None,
                    None,
                    Some(crate::host::pane::PendingPaneClick {
                        target: crate::host::pane::PaneClickTarget::Node(4),
                        button: "left",
                        phase: crate::host::pane::PointerPhase::Click,
                    }),
                    None,
                    0,
                );
            });
        });
        assert_eq!(table_click.actions, vec!["select_row:0".to_string()]);

        let has_card_surface = output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => {
                rect.fill == colors.bg_active && rect.stroke.color == colors.border
            }
            _ => false,
        });
        assert!(
            has_card_surface,
            "Card node must paint bg_active surface with border stroke"
        );

        let mut list_click = RenderResult::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                list_click = render_ui_tree_with_canvas_fits(
                    ui,
                    &tree,
                    &colors,
                    None,
                    None,
                    Some(crate::host::pane::PendingPaneClick {
                        target: crate::host::pane::PaneClickTarget::Node(6),
                        button: "left",
                        phase: crate::host::pane::PointerPhase::Click,
                    }),
                    None,
                    0,
                );
            });
        });
        assert_eq!(list_click.actions, vec!["select_item:0".to_string()]);

        let png = render_ui_tree_to_png(&tree, 520.0, 360.0, 2.0)
            .expect("render 0816 widget tree to PNG");
        assert!(!png.is_empty());
    }

    #[test]
    fn list_view_paints_selected_row_chrome_and_respects_override_text_color() {
        fn collect_shapes(
            shape: &egui::Shape,
            rects: &mut Vec<egui::epaint::RectShape>,
            texts: &mut Vec<egui::epaint::TextShape>,
        ) {
            match shape {
                egui::Shape::Rect(r) => rects.push(r.clone()),
                egui::Shape::Text(t) => texts.push(t.clone()),
                egui::Shape::Vec(v) => {
                    for s in v {
                        collect_shapes(s, rects, texts);
                    }
                }
                _ => {}
            }
        }

        let tree = crate::host::wasm_python::decode_ui_tree(
            &serde_json::json!({
                "root": 0,
                "nodes": [
                    {
                        "id": 0,
                        "key": "list",
                        "data": {
                            "type": "ListView",
                            "items": [1, 4],
                            "selected": 0,
                            "on_select": "pick"
                        }
                    },
                    {
                        "id": 1,
                        "key": "row-0",
                        "data": {
                            "type": "Row",
                            "children": [2, 3],
                            "gap": 8.0
                        }
                    },
                    {
                        "id": 2,
                        "key": "row-0-title",
                        "data": {
                            "type": "Text",
                            "text": "Selected Title"
                        }
                    },
                    {
                        "id": 3,
                        "key": "row-0-sub",
                        "data": {
                            "type": "Text",
                            "text": "Muted Trailing",
                            "color": "muted",
                            "size": 12.0
                        }
                    },
                    {
                        "id": 4,
                        "key": "row-1",
                        "data": {
                            "type": "Text",
                            "text": "Unselected Title"
                        }
                    }
                ]
            })
            .to_string(),
        )
        .expect("decode list_view tree");

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );
        crate::ui::theme::setup_style(&ctx, &colors);

        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                let _ = render_ui_tree_with_surface(ui, &tree, &colors, None, None, 0);
            });
        });

        let mut rects = Vec::new();
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect_shapes(&clipped.shape, &mut rects, &mut texts);
        }

        let selected_fill = colors.accent.gamma_multiply(0.18);
        assert!(
            rects
                .iter()
                .any(|r| r.fill == selected_fill && r.rect.width() > 20.0),
            "selected ListView row must paint accent-tinted background fill"
        );
        assert!(
            rects
                .iter()
                .any(|r| r.fill == colors.accent && (r.rect.width() - 3.0).abs() < 0.1),
            "selected ListView row must paint a 3px left accent indicator bar"
        );

        let color_of = |label: &str| {
            let t = texts
                .iter()
                .find(|t| t.galley.job.text == label)
                .unwrap_or_else(|| panic!("missing text {label}"));
            let c = t.galley.job.sections[0].format.color;
            if c == Color32::PLACEHOLDER {
                t.fallback_color
            } else {
                c
            }
        };

        assert_eq!(
            color_of("Selected Title"),
            colors.accent,
            "selected row primary text without explicit color must respect override_text_color"
        );
        assert_eq!(
            color_of("Unselected Title"),
            colors.text_primary,
            "unselected row primary text must use text_primary"
        );
        assert_ne!(
            color_of("Muted Trailing"),
            colors.accent,
            "explicit muted color on secondary/trailing text must stay muted inside selected row"
        );
    }

    #[test]
    fn nested_card_and_keypad_grid_align_and_contrast_secondary_buttons() {
        let tree = crate::host::wasm_python::decode_ui_tree(
            &serde_json::json!({
                "root": 0,
                "nodes": [
                    {
                        "id": 0,
                        "key": "__plexi_card__:outer",
                        "data": {
                            "type": "Padding",
                            "card": true,
                            "top": 16.0,
                            "right": 16.0,
                            "bottom": 16.0,
                            "left": 16.0,
                            "child": 1
                        }
                    },
                    {
                        "id": 1,
                        "key": "outer-col",
                        "data": {
                            "type": "Column",
                            "children": [2, 4],
                            "gap": 12.0,
                            "align": "start",
                            "grow": false
                        }
                    },
                    {
                        "id": 2,
                        "key": "__plexi_card__:readout",
                        "data": {
                            "type": "Padding",
                            "card": true,
                            "top": 12.0,
                            "right": 12.0,
                            "bottom": 12.0,
                            "left": 12.0,
                            "child": 3
                        }
                    },
                    {
                        "id": 3,
                        "key": "display",
                        "data": {
                            "type": "Text",
                            "text": "0",
                            "size": 28.0,
                            "bold": true,
                            "align": "end"
                        }
                    },
                    {
                        "id": 4,
                        "key": "keypad",
                        "data": {
                            "type": "Column",
                            "children": [5, 10],
                            "gap": 8.0,
                            "align": "start",
                            "grow": false
                        }
                    },
                    {
                        "id": 5,
                        "key": "row-top",
                        "data": {
                            "type": "Row",
                            "children": [6, 7, 8, 9],
                            "gap": 8.0
                        }
                    },
                    {
                        "id": 6,
                        "key": "btn-1",
                        "data": { "type": "Button", "label": "1", "on_click": "k1", "style": "secondary" }
                    },
                    {
                        "id": 7,
                        "key": "btn-2",
                        "data": { "type": "Button", "label": "2", "on_click": "k2", "style": "secondary" }
                    },
                    {
                        "id": 8,
                        "key": "btn-3",
                        "data": { "type": "Button", "label": "3", "on_click": "k3", "style": "secondary" }
                    },
                    {
                        "id": 9,
                        "key": "btn-plus",
                        "data": { "type": "Button", "label": "+", "on_click": "k+", "style": "primary" }
                    },
                    {
                        "id": 10,
                        "key": "row-bot",
                        "data": {
                            "type": "Row",
                            "children": [11, 12, 13],
                            "gap": 8.0
                        }
                    },
                    {
                        "id": 11,
                        "key": "btn-0",
                        "data": { "type": "Button", "label": "0", "on_click": "k0", "style": "secondary" }
                    },
                    {
                        "id": 12,
                        "key": "btn-dot",
                        "data": { "type": "Button", "label": ".", "on_click": "k.", "style": "secondary" }
                    },
                    {
                        "id": 13,
                        "key": "btn-eq",
                        "data": { "type": "Button", "label": "=", "on_click": "k=", "style": "primary" }
                    }
                ]
            })
            .to_string(),
        )
        .expect("decode keypad card tree");

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );
        crate::ui::theme::setup_style(&ctx, &colors);

        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                let _ = render_ui_tree_with_surface(ui, &tree, &colors, None, None, 0);
            });
        });

        let mut rects = Vec::new();
        for clipped in &output.shapes {
            if let egui::Shape::Rect(r) = &clipped.shape {
                rects.push(r.clone());
            }
        }

        // Outer Card uses bg_active and stays compact (< 400px in a wide panel).
        let outer_card = rects
            .iter()
            .find(|r| r.fill == colors.bg_active && r.stroke.color == colors.border)
            .expect("outer Card rect");
        assert!(
            outer_card.rect.width() < 360.0,
            "keypad Card must wrap keypad width compactly, got {}",
            outer_card.rect.width()
        );

        // Nested readout Card uses bg_darkest for dark inset contrast.
        assert!(
            rects
                .iter()
                .any(|r| r.fill == colors.bg_darkest && r.stroke.color == colors.border),
            "nested Card must paint dark inset bg_darkest surface"
        );

        // Secondary buttons must NOT use bg_active (which would blend into the outer
        // Card): find the '0' button's own background rect and compare it to the Card.
        let zero_center = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(text) if text.galley.job.text == "0" => {
                    Some(text.visual_bounding_rect().center())
                }
                _ => None,
            })
            .expect("'0' button label");
        let zero_btn = rects
            .iter()
            .filter(|r| r.rect.contains(zero_center) && r.rect.width() > 100.0)
            .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
            .expect("double-wide '0' button rect");
        assert_ne!(
            zero_btn.fill, colors.bg_active,
            "Secondary must not use the bg_active Card fill"
        );
        assert_ne!(
            zero_btn.fill, colors.accent,
            "Secondary must stay distinct from Primary"
        );
        let primary_btns: Vec<_> = rects
            .iter()
            .filter(|r| r.fill == colors.accent && (r.rect.height() - KEYPAD_BUTTON_H).abs() < 1.0)
            .collect();
        assert_eq!(
            primary_btns.len(),
            2,
            "expected '+' and '=' primary buttons"
        );
        assert!(
            (primary_btns[0].rect.left() - primary_btns[1].rect.left()).abs() < 1.0,
            "'=' button in 3-button bottom row must align with '+' in column 4"
        );
        assert!(
            zero_btn.rect.width() > KEYPAD_BUTTON_W * 1.8,
            "'0' button must span 2 columns"
        );
    }
}

#[cfg(test)]
#[path = "wasm_render/contrast_tests.rs"]
mod contrast_tests;
