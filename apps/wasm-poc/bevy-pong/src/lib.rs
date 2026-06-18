// Bevy Pong — GPU render-pass POC for the Plexi v2 WASM runtime.
//
// Proves: gpu capability import, WGSL pipeline creation, per-frame render pass
// submission, surface-node lifecycle, and native GPU performance from WASM.
//
// No pixel buffer copy. No host-side blit. The WASM module issues GPU commands
// through the `gpu` interface; the host executes them against its wgpu device.
// This is the same performance tier as a native app — the WASM/host boundary
// only passes command descriptors, not framebuffer data.
//
// WGSL shader draws instanced quads. Each game object (background, paddles,
// ball, centre line segments) is one instance. The vertex buffer holds quad
// positions in clip space; per-instance data (position, size, colour) is in a
// storage buffer read by the vertex shader.

wit_bindgen::generate!({
    world: "plexi-gpu-app",
    path: "wit/world.wit",
});

use bytemuck::{Pod, Zeroable};
use plexi::platform::gpu::{
    self, BindingEntry, BindingResource, BufferUsage, ComputePassDesc, DrawCall,
    RenderPassDesc, RenderPipelineDesc, TextureFormat, VertexAttr,
};
use plexi::platform::host_log;
use plexi::platform::pipes::{self, PipeDirection, PipeType};
use plexi::platform::types::{
    alignment::Alignment, column_node::ColumnNode, effect::Effect, indexed_node::IndexedNode,
    input_event::InputEvent, key_event::KeyEvent, state_snapshot::StateSnapshot,
    surface_event::SurfaceEvent, surface_node::SurfaceNode, text_node::TextNode,
    timer_effect::TimerEffect, ui_node_data::UiNodeData, ui_tree::UiTree,
};

// ── WGSL ──────────────────────────────────────────────────────────────────────

// Instanced quad shader. Each instance is one game object.
// Instance data layout: [x, y, w, h, r, g, b, a] as f32 (32 bytes per instance).
const PONG_WGSL: &str = r#"
struct Instance {
    @location(0) pos:   vec2<f32>,   // top-left in NDC
    @location(1) size:  vec2<f32>,   // width, height in NDC
    @location(2) color: vec4<f32>,
}

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
}

// Unit quad corners (clip-space before per-instance transform)
const CORNERS: array<vec2<f32>, 6> = array(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VOut {
    let corner = CORNERS[vi];
    // NDC: top-left is (-1, 1), bottom-right is (1, -1). Y flipped.
    let ndc = inst.pos + corner * inst.size;
    return VOut(vec4(ndc, 0.0, 1.0), inst.color);
}

@fragment
fn fs_main(v: VOut) -> @location(0) vec4<f32> {
    return v.color;
}
"#;

// ── Instance data layout ──────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Instance {
    pos:   [f32; 2],
    size:  [f32; 2],
    color: [f32; 4],
}

impl Instance {
    // Convert pixel-space rect to NDC. Canvas is CANVAS_W × CANVAS_H pixels.
    fn from_px(x: f32, y: f32, w: f32, h: f32, cw: f32, ch: f32, color: [f32; 4]) -> Self {
        // NDC x: -1 (left) to +1 (right); NDC y: +1 (top) to -1 (bottom)
        let nx = (x / cw) * 2.0 - 1.0;
        let ny = 1.0 - (y / ch) * 2.0;
        let nw = (w / cw) * 2.0;
        let nh = (h / ch) * 2.0;
        Instance { pos: [nx, ny - nh], size: [nw, nh], color }
    }
}

// ── Game constants ────────────────────────────────────────────────────────────

const CANVAS_W: f32 = 480.0;
const CANVAS_H: f32 = 320.0;
const TIMER_TICK: u32 = 1;
const TICK_MS: u32 = 16;
const PADDLE_W: f32 = 10.0;
const PADDLE_H: f32 = 60.0;
const PADDLE_SPEED: f32 = 4.0;
const BALL_SIZE: f32 = 8.0;
const BALL_SPEED: f32 = 3.0;

const BG:     [f32; 4] = [0.118, 0.118, 0.180, 1.0]; // Catppuccin base
const WHITE:  [f32; 4] = [0.804, 0.839, 0.957, 1.0]; // text
const PINK:   [f32; 4] = [0.953, 0.545, 0.659, 1.0]; // ball (red)
const DIM:    [f32; 4] = [0.271, 0.278, 0.353, 1.0]; // centre line

// ── App state ─────────────────────────────────────────────────────────────────

struct Vec2 { x: f32, y: f32 }

struct Pong {
    // GPU handles
    pipeline: Option<u64>,
    instance_buf: Option<u64>,
    surface_view: Option<u64>,
    surface_w: f32,
    surface_h: f32,

    // Pipe for waveform/score data to external visualiser
    score_pipe: Option<u32>,

    // Game
    ball: Vec2,
    vel: Vec2,
    left_y: f32,
    right_y: f32,
    left_up: bool, left_down: bool,
    right_up: bool, right_down: bool,
    score_left: u32,
    score_right: u32,

    node_id: u32,
}

impl Pong {
    fn new() -> Self {
        Pong {
            pipeline: None, instance_buf: None,
            surface_view: None, surface_w: CANVAS_W, surface_h: CANVAS_H,
            score_pipe: None,
            ball: Vec2 { x: CANVAS_W / 2.0, y: CANVAS_H / 2.0 },
            vel: Vec2 { x: BALL_SPEED, y: BALL_SPEED * 0.7 },
            left_y: CANVAS_H / 2.0, right_y: CANVAS_H / 2.0,
            left_up: false, left_down: false,
            right_up: false, right_down: false,
            score_left: 0, score_right: 0,
            node_id: 0,
        }
    }

    // ── GPU setup ─────────────────────────────────────────────────────────────

    fn setup_gpu(&mut self, surface_handle: u64, w: u32, h: u32) {
        self.surface_w = w as f32;
        self.surface_h = h as f32;

        // Create a view into the surface-node texture.
        let view = gpu::create_surface_view(surface_handle)
            .expect("create_surface_view");
        self.surface_view = Some(view);

        // Compile the instanced quad pipeline once.
        let desc = RenderPipelineDesc {
            vs_entry: "vs_main".to_string(),
            fs_entry: "fs_main".to_string(),
            vertex_stride: std::mem::size_of::<Instance>() as u32,
            attrs: vec![
                VertexAttr { location: 0, format: "float32x2".to_string(), offset: 0 },
                VertexAttr { location: 1, format: "float32x2".to_string(), offset: 8 },
                VertexAttr { location: 2, format: "float32x4".to_string(), offset: 16 },
            ],
            output_format: TextureFormat::Rgba8Unorm,
            blend_alpha: true,
        };
        self.pipeline = Some(
            gpu::create_render_pipeline("pong", PONG_WGSL, desc)
                .expect("create_render_pipeline")
        );

        // Allocate instance buffer (32 objects × 32 bytes).
        self.instance_buf = Some(
            gpu::create_buffer(
                "instances",
                (32 * std::mem::size_of::<Instance>()) as u64,
                BufferUsage::VERTEX | BufferUsage::COPY_DST,
            ).expect("create_buffer")
        );

        host_log::info(&format!("bevy-pong: GPU ready {}×{}", w, h));
    }

    // ── Tick ─────────────────────────────────────────────────────────────────

    fn tick(&mut self) {
        let half_p = PADDLE_H / 2.0;
        if self.left_up   { self.left_y  = (self.left_y  - PADDLE_SPEED).max(half_p); }
        if self.left_down { self.left_y  = (self.left_y  + PADDLE_SPEED).min(self.surface_h - half_p); }
        if self.right_up  { self.right_y = (self.right_y - PADDLE_SPEED).max(half_p); }
        if self.right_down{ self.right_y = (self.right_y + PADDLE_SPEED).min(self.surface_h - half_p); }

        self.ball.x += self.vel.x;
        self.ball.y += self.vel.y;

        if self.ball.y <= 0.0 || self.ball.y >= self.surface_h { self.vel.y = -self.vel.y; }

        let left_x = 20.0 + PADDLE_W;
        if self.ball.x <= left_x && (self.ball.y - self.left_y).abs() < half_p + BALL_SIZE / 2.0 {
            self.vel.x = self.vel.x.abs();
            self.vel.y += (self.ball.y - self.left_y) / half_p * 1.5;
        }
        let right_x = self.surface_w - 20.0 - PADDLE_W;
        if self.ball.x >= right_x && (self.ball.y - self.right_y).abs() < half_p + BALL_SIZE / 2.0 {
            self.vel.x = -self.vel.x.abs();
            self.vel.y += (self.ball.y - self.right_y) / half_p * 1.5;
        }

        if self.ball.x < 0.0 { self.score_right += 1; self.reset_ball(-1.0); self.push_score(); }
        if self.ball.x > self.surface_w { self.score_left += 1; self.reset_ball(1.0); self.push_score(); }

        let speed = (self.vel.x.powi(2) + self.vel.y.powi(2)).sqrt().max(1.0);
        if speed > 10.0 { self.vel.x = self.vel.x / speed * 10.0; self.vel.y = self.vel.y / speed * 10.0; }
    }

    fn reset_ball(&mut self, dir: f32) {
        self.ball = Vec2 { x: self.surface_w / 2.0, y: self.surface_h / 2.0 };
        self.vel = Vec2 { x: BALL_SPEED * dir, y: BALL_SPEED * 0.7 };
    }

    // Push score update to the score pipe (if a visualiser is connected).
    fn push_score(&self) {
        if let Some(h) = self.score_pipe {
            let msg = format!("{{\"left\":{},\"right\":{}}}", self.score_left, self.score_right);
            let _ = pipes::send_json(h, &msg);
        }
    }

    // ── Render ────────────────────────────────────────────────────────────────

    fn render(&self) {
        let (pipeline, buf, view) = match (self.pipeline, self.instance_buf, self.surface_view) {
            (Some(p), Some(b), Some(v)) => (p, b, v),
            _ => return,
        };
        let cw = self.surface_w;
        let ch = self.surface_h;

        let mut instances: Vec<Instance> = Vec::with_capacity(32);

        // Background
        instances.push(Instance::from_px(0.0, 0.0, cw, ch, cw, ch, BG));

        // Centre line (dashed — 10 segments)
        let seg_h = ch / 20.0;
        let cx = cw / 2.0 - 1.0;
        for i in 0..10u32 {
            let y = i as f32 * seg_h * 2.0;
            instances.push(Instance::from_px(cx, y, 2.0, seg_h, cw, ch, DIM));
        }

        // Paddles
        instances.push(Instance::from_px(20.0, self.left_y - PADDLE_H / 2.0, PADDLE_W, PADDLE_H, cw, ch, WHITE));
        instances.push(Instance::from_px(cw - 20.0 - PADDLE_W, self.right_y - PADDLE_H / 2.0, PADDLE_W, PADDLE_H, cw, ch, WHITE));

        // Ball
        instances.push(Instance::from_px(self.ball.x - BALL_SIZE / 2.0, self.ball.y - BALL_SIZE / 2.0, BALL_SIZE, BALL_SIZE, cw, ch, PINK));

        let bytes = bytemuck::cast_slice(&instances);
        gpu::write_buffer(buf, 0, bytes.to_vec()).expect("write_buffer");

        let bind_group = gpu::create_bind_group(pipeline, vec![
            BindingEntry { binding: 0, resource: BindingResource::Buffer(buf) },
        ]).expect("create_bind_group");

        gpu::submit_render_pass(RenderPassDesc {
            target: view,
            clear_color: None, // background is the first instance
            pipeline,
            vertex_buffer: Some(buf),
            index_buffer: None,
            bind_groups: vec![(0, bind_group)],
            draws: vec![DrawCall {
                vertices: 6,
                instances: instances.len() as u32,
                first_vertex: 0,
                first_instance: 0,
            }],
        }).expect("submit_render_pass");

        gpu::destroy_bind_group(bind_group);
    }

    // ── View ─────────────────────────────────────────────────────────────────

    fn nid(&mut self, key: &str, data: UiNodeData) -> IndexedNode {
        let id = self.node_id;
        self.node_id += 1;
        IndexedNode { id, key: key.to_string(), data }
    }

    fn build_tree(&mut self) -> UiTree {
        self.node_id = 0;
        let mut nodes: Vec<IndexedNode> = Vec::new();

        let score = self.nid("score", UiNodeData::Text(TextNode {
            text: format!("{}  —  {}", self.score_left, self.score_right),
            size: Some(22.0), bold: true, color: None, truncate: false,
            align: Alignment::Center,
        }));
        let score_id = score.id;
        nodes.push(score);

        let canvas = self.nid("canvas", UiNodeData::Surface(SurfaceNode {
            width: self.surface_w as u32,
            height: self.surface_h as u32,
            texture_handle: self.surface_view,
        }));
        let canvas_id = canvas.id;
        nodes.push(canvas);

        let hint = self.nid("hint", UiNodeData::Text(TextNode {
            text: "W/S — left   ↑/↓ — right   q to quit".to_string(),
            size: Some(11.0), bold: false, color: None, truncate: false,
            align: Alignment::Center,
        }));
        let hint_id = hint.id;
        nodes.push(hint);

        let root = self.nid("root", UiNodeData::Column(ColumnNode {
            children: vec![score_id, canvas_id, hint_id],
            gap: 8.0, align: Alignment::Center, grow: true,
        }));
        let root_id = root.id;
        nodes.push(root);
        UiTree { root: root_id, nodes }
    }
}

// ── Lifecycle ─────────────────────────────────────────────────────────────────

struct Component;
static mut GAME: Option<Pong> = None;
fn game() -> &'static mut Pong { unsafe { GAME.as_mut().unwrap() } }

impl Guest for Component {
    fn init(_state: StateSnapshot, _size: (f32, f32)) -> Vec<Effect> {
        unsafe { GAME = Some(Pong::new()); }
        host_log::info("bevy-pong: init (GPU path)");

        // Open a JSON pipe for score updates — a visualiser pane can connect.
        if let Ok(h) = pipes::open("pong-scores", PipeType::Json, PipeDirection::Out) {
            game().score_pipe = Some(h);
            host_log::info("bevy-pong: score pipe open");
        }

        vec![
            Effect::SetTitle("Pong".to_string()),
            Effect::SetTimer(TimerEffect { id: TIMER_TICK, delay_ms: TICK_MS, repeat: true }),
        ]
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        match event {
            InputEvent::SurfaceReady(SurfaceEvent { texture_handle, width, height }) => {
                game().setup_gpu(texture_handle, width, height);
                game().render();
                vec![]
            }

            InputEvent::SurfaceResized(SurfaceEvent { texture_handle, width, height }) => {
                game().setup_gpu(texture_handle, width, height);
                vec![]
            }

            InputEvent::TimerFired(TIMER_TICK) => {
                game().tick();
                game().render();
                vec![]
            }

            InputEvent::Key(KeyEvent { key, pressed, .. }) => {
                let g = game();
                match key.as_str() {
                    "w"      => g.left_up    = pressed,
                    "s"      => g.left_down  = pressed,
                    "up"     => g.right_up   = pressed,
                    "down"   => g.right_down = pressed,
                    "q" | "escape" if pressed => return vec![Effect::CloseSelf],
                    _ => {}
                }
                vec![]
            }

            _ => vec![],
        }
    }

    fn view() -> UiTree { game().build_tree() }
}

export!(Component);
