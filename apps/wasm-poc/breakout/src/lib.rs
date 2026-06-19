// Breakout benchmark - GPU render-pass POC for the Plexi v2 WASM runtime.
//
// A classic Breakout layout ported into Plexi's component/WASM
// surface runtime. It keeps the same core shape: paddle, ball, arena walls,
// brick grid, collision response, and score. Rendering is through Plexi's GPU
// import so the benchmark measures the host surface path with no framebuffer
// copy across the WASM boundary.

wit_bindgen::generate!({
    world: "plexi-gpu-app",
    path: "wit/world.wit",
});

use bytemuck::{Pod, Zeroable};
use exports::plexi::platform::lifecycle::Guest;
use plexi::platform::gpu::{
    self, BufferUsage, DrawCall, RenderPassDesc, RenderPipelineDesc, TextureFormat, VertexAttr,
};
use plexi::platform::host_log;
use plexi::platform::types::{
    Alignment, ColumnNode, Effect, IndexedNode, InputEvent, KeyEvent, StateSnapshot, SurfaceEvent,
    SurfaceNode, TextNode, TimerEffect, UiNodeData, UiTree,
};

const BREAKOUT_WGSL: &str = r#"
struct Instance {
    @location(0) pos:   vec2<f32>,
    @location(1) size:  vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
}

const CORNERS: array<vec2<f32>, 6> = array(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VOut {
    let corner = CORNERS[vi];
    let ndc = inst.pos + corner * inst.size;
    return VOut(vec4(ndc, 0.0, 1.0), inst.color);
}

@fragment
fn fs_main(v: VOut) -> @location(0) vec4<f32> {
    return v.color;
}
"#;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Instance {
    pos: [f32; 2],
    size: [f32; 2],
    color: [f32; 4],
}

impl Instance {
    fn from_px(x: f32, y: f32, w: f32, h: f32, cw: f32, ch: f32, color: [f32; 4]) -> Self {
        let nx = (x / cw) * 2.0 - 1.0;
        let ny = 1.0 - (y / ch) * 2.0;
        let nw = (w / cw) * 2.0;
        let nh = (h / ch) * 2.0;
        Instance {
            pos: [nx, ny - nh],
            size: [nw, nh],
            color,
        }
    }
}

#[derive(Clone, Copy)]
struct Vec2 {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct Brick {
    x: f32,
    y: f32,
    alive: bool,
}

#[derive(Clone, Copy)]
enum Collision {
    Left,
    Right,
    Top,
    Bottom,
}

const CANVAS_W: f32 = 900.0;
const CANVAS_H: f32 = 600.0;
const TIMER_TICK: u32 = 1;
const TICK_MS: u32 = 16;

const PADDLE_W: f32 = 120.0;
const PADDLE_H: f32 = 20.0;
const PADDLE_SPEED: f32 = 8.0;
const PADDLE_Y: f32 = CANVAS_H - 60.0;
const PADDLE_PADDING: f32 = 10.0;

const BALL_SIZE: f32 = 18.0;
const BALL_SPEED: f32 = 5.8;

const WALL: f32 = 10.0;
const BRICK_W: f32 = 100.0;
const BRICK_H: f32 = 30.0;
const BRICK_GAP: f32 = 5.0;
const BRICK_COLS: usize = 8;
const BRICK_ROWS: usize = 5;
const MAX_INSTANCES: usize = 96;

const BG: [f32; 4] = [0.055, 0.067, 0.090, 1.0];
const WALL_COLOR: [f32; 4] = [0.310, 0.337, 0.396, 1.0];
const PADDLE_COLOR: [f32; 4] = [0.325, 0.423, 0.996, 1.0];
const BALL_COLOR: [f32; 4] = [0.992, 0.439, 0.439, 1.0];
const BRICK_COLOR: [f32; 4] = [0.522, 0.580, 0.996, 1.0];
const BRICK_HIT_COLOR: [f32; 4] = [0.553, 0.827, 0.780, 1.0];

struct Breakout {
    pipeline: Option<u64>,
    instance_buf: Option<u64>,
    surface_view: Option<u64>,
    surface_w: f32,
    surface_h: f32,

    ball: Vec2,
    vel: Vec2,
    paddle_x: f32,
    left: bool,
    right: bool,
    score: u32,
    lives: u32,
    bricks: Vec<Brick>,
    last_hit_frame: u64,

    frames: u64,
    ticks: u64,
    fps_estimate: u32,
    node_id: u32,
}

impl Breakout {
    fn new() -> Self {
        let mut game = Breakout {
            pipeline: None,
            instance_buf: None,
            surface_view: None,
            surface_w: CANVAS_W,
            surface_h: CANVAS_H,
            ball: Vec2 {
                x: CANVAS_W / 2.0,
                y: CANVAS_H / 2.0,
            },
            vel: Vec2 {
                x: BALL_SPEED * 0.65,
                y: -BALL_SPEED,
            },
            paddle_x: CANVAS_W / 2.0,
            left: false,
            right: false,
            score: 0,
            lives: 3,
            bricks: Vec::new(),
            last_hit_frame: 0,
            frames: 0,
            ticks: 0,
            fps_estimate: 60,
            node_id: 0,
        };
        game.reset_bricks();
        game
    }

    fn setup_gpu(&mut self, surface_handle: u64, w: u32, h: u32) {
        self.surface_w = w as f32;
        self.surface_h = h as f32;

        let view = gpu::create_surface_view(surface_handle).expect("create_surface_view");
        self.surface_view = Some(view);

        let desc = RenderPipelineDesc {
            vs_entry: "vs_main".to_string(),
            fs_entry: "fs_main".to_string(),
            vertex_stride: core::mem::size_of::<Instance>() as u32,
            attrs: vec![
                VertexAttr {
                    location: 0,
                    format: "float32x2".to_string(),
                    offset: 0,
                },
                VertexAttr {
                    location: 1,
                    format: "float32x2".to_string(),
                    offset: 8,
                },
                VertexAttr {
                    location: 2,
                    format: "float32x4".to_string(),
                    offset: 16,
                },
            ],
            output_format: TextureFormat::Rgba8Unorm,
            blend_alpha: true,
        };
        self.pipeline = Some(
            gpu::create_render_pipeline("breakout", BREAKOUT_WGSL, &desc)
                .expect("create_render_pipeline"),
        );
        self.instance_buf = Some(
            gpu::create_buffer(
                "breakout-instances",
                (MAX_INSTANCES * core::mem::size_of::<Instance>()) as u64,
                BufferUsage::VERTEX | BufferUsage::COPY_DST,
            )
            .expect("create_buffer"),
        );
        host_log::info(&format!("breakout: GPU ready {}x{}", w, h));
    }

    fn reset_bricks(&mut self) {
        self.bricks.clear();
        let total_w = BRICK_COLS as f32 * BRICK_W + (BRICK_COLS - 1) as f32 * BRICK_GAP;
        let start_x = (CANVAS_W - total_w) * 0.5;
        let start_y = 64.0;
        for row in 0..BRICK_ROWS {
            for col in 0..BRICK_COLS {
                self.bricks.push(Brick {
                    x: start_x + col as f32 * (BRICK_W + BRICK_GAP),
                    y: start_y + row as f32 * (BRICK_H + BRICK_GAP),
                    alive: true,
                });
            }
        }
    }

    fn tick(&mut self) {
        self.ticks += 1;
        if self.ticks % 60 == 0 {
            self.fps_estimate = ((self.frames % 60) as u32).max(1);
        }

        let direction = (self.right as i8 - self.left as i8) as f32;
        self.paddle_x = (self.paddle_x + direction * PADDLE_SPEED).clamp(
            WALL + PADDLE_PADDING + PADDLE_W / 2.0,
            self.surface_w - WALL - PADDLE_PADDING - PADDLE_W / 2.0,
        );

        self.ball.x += self.vel.x;
        self.ball.y += self.vel.y;

        if self.ball.x <= WALL + BALL_SIZE / 2.0 {
            self.ball.x = WALL + BALL_SIZE / 2.0;
            self.vel.x = self.vel.x.abs();
        }
        if self.ball.x >= self.surface_w - WALL - BALL_SIZE / 2.0 {
            self.ball.x = self.surface_w - WALL - BALL_SIZE / 2.0;
            self.vel.x = -self.vel.x.abs();
        }
        if self.ball.y <= WALL + BALL_SIZE / 2.0 {
            self.ball.y = WALL + BALL_SIZE / 2.0;
            self.vel.y = self.vel.y.abs();
        }
        if self.ball.y > self.surface_h + BALL_SIZE {
            self.lives = self.lives.saturating_sub(1);
            self.reset_ball();
        }

        self.collide_paddle();
        self.collide_bricks();

        if self.bricks.iter().all(|brick| !brick.alive) {
            self.reset_bricks();
            self.reset_ball();
        }
    }

    fn collide_paddle(&mut self) {
        let paddle_x = self.paddle_x - PADDLE_W / 2.0;
        let paddle_y = PADDLE_Y - PADDLE_H / 2.0;
        if self.vel.y > 0.0
            && rect_collision(
                self.ball,
                BALL_SIZE / 2.0,
                paddle_x,
                paddle_y,
                PADDLE_W,
                PADDLE_H,
            )
            .is_some()
        {
            self.vel.y = -self.vel.y.abs();
            let offset = ((self.ball.x - self.paddle_x) / (PADDLE_W / 2.0)).clamp(-1.0, 1.0);
            self.vel.x = offset * BALL_SPEED;
        }
    }

    fn collide_bricks(&mut self) {
        for brick in &mut self.bricks {
            if !brick.alive {
                continue;
            }
            let Some(side) = rect_collision(
                self.ball,
                BALL_SIZE / 2.0,
                brick.x,
                brick.y,
                BRICK_W,
                BRICK_H,
            ) else {
                continue;
            };
            brick.alive = false;
            self.score += 1;
            self.last_hit_frame = self.frames;
            match side {
                Collision::Left | Collision::Right => self.vel.x = -self.vel.x,
                Collision::Top | Collision::Bottom => self.vel.y = -self.vel.y,
            }
            break;
        }
    }

    fn reset_ball(&mut self) {
        self.ball = Vec2 {
            x: self.surface_w / 2.0,
            y: self.surface_h / 2.0,
        };
        self.vel = Vec2 {
            x: BALL_SPEED * 0.65,
            y: -BALL_SPEED,
        };
        if self.lives == 0 {
            self.score = 0;
            self.lives = 3;
            self.reset_bricks();
        }
    }

    fn render(&mut self) {
        let (pipeline, buf, view) = match (self.pipeline, self.instance_buf, self.surface_view) {
            (Some(pipeline), Some(buf), Some(view)) => (pipeline, buf, view),
            _ => return,
        };
        let cw = self.surface_w;
        let ch = self.surface_h;
        let mut instances = Vec::with_capacity(MAX_INSTANCES);

        instances.push(Instance::from_px(0.0, 0.0, cw, ch, cw, ch, BG));
        instances.push(Instance::from_px(0.0, 0.0, WALL, ch, cw, ch, WALL_COLOR));
        instances.push(Instance::from_px(
            cw - WALL,
            0.0,
            WALL,
            ch,
            cw,
            ch,
            WALL_COLOR,
        ));
        instances.push(Instance::from_px(0.0, 0.0, cw, WALL, cw, ch, WALL_COLOR));
        instances.push(Instance::from_px(
            0.0,
            ch - WALL,
            cw,
            WALL,
            cw,
            ch,
            WALL_COLOR,
        ));

        for brick in &self.bricks {
            if brick.alive {
                let color = if self.frames.saturating_sub(self.last_hit_frame) < 8 {
                    BRICK_HIT_COLOR
                } else {
                    BRICK_COLOR
                };
                instances.push(Instance::from_px(
                    brick.x, brick.y, BRICK_W, BRICK_H, cw, ch, color,
                ));
            }
        }

        instances.push(Instance::from_px(
            self.paddle_x - PADDLE_W / 2.0,
            PADDLE_Y - PADDLE_H / 2.0,
            PADDLE_W,
            PADDLE_H,
            cw,
            ch,
            PADDLE_COLOR,
        ));
        instances.push(Instance::from_px(
            self.ball.x - BALL_SIZE / 2.0,
            self.ball.y - BALL_SIZE / 2.0,
            BALL_SIZE,
            BALL_SIZE,
            cw,
            ch,
            BALL_COLOR,
        ));

        let bytes: &[u8] = bytemuck::cast_slice(&instances);
        gpu::write_buffer(buf, 0, bytes).expect("write_buffer");
        gpu::submit_render_pass(&RenderPassDesc {
            target: view,
            clear_color: None,
            pipeline,
            vertex_buffer: Some(buf),
            index_buffer: None,
            bind_groups: vec![],
            draws: vec![DrawCall {
                vertices: 6,
                instances: instances.len() as u32,
                first_vertex: 0,
                first_instance: 0,
            }],
        })
        .expect("submit_render_pass");
        self.frames += 1;
    }

    fn nid(&mut self, key: &str, data: UiNodeData) -> IndexedNode {
        let id = self.node_id;
        self.node_id += 1;
        IndexedNode {
            id,
            key: key.to_string(),
            data,
        }
    }

    fn build_tree(&mut self) -> UiTree {
        self.node_id = 0;
        let mut nodes = Vec::new();

        let alive = self.bricks.iter().filter(|brick| brick.alive).count();
        let title = self.nid(
            "title",
            UiNodeData::Text(TextNode {
                text: format!(
                    "Score {}   Lives {}   Bricks {}   GPU objects {}   FPS~{}",
                    self.score,
                    self.lives,
                    alive,
                    alive + 7,
                    self.fps_estimate
                ),
                size: Some(16.0),
                bold: true,
                color: None,
                truncate: false,
                align: Alignment::Center,
            }),
        );
        let title_id = title.id;
        nodes.push(title);

        let canvas = self.nid(
            "canvas",
            UiNodeData::Surface(SurfaceNode {
                width: self.surface_w as u32,
                height: self.surface_h as u32,
                texture_handle: self.surface_view,
            }),
        );
        let canvas_id = canvas.id;
        nodes.push(canvas);

        let hint = self.nid(
            "hint",
            UiNodeData::Text(TextNode {
                text: "Left/Right or A/D to move   R reset   Q quit".to_string(),
                size: Some(11.0),
                bold: false,
                color: None,
                truncate: false,
                align: Alignment::Center,
            }),
        );
        let hint_id = hint.id;
        nodes.push(hint);

        let root = self.nid(
            "root",
            UiNodeData::Column(ColumnNode {
                children: vec![title_id, canvas_id, hint_id],
                gap: 8.0,
                align: Alignment::Center,
                grow: true,
            }),
        );
        let root_id = root.id;
        nodes.push(root);
        UiTree {
            root: root_id,
            nodes,
        }
    }
}

fn rect_collision(ball: Vec2, radius: f32, x: f32, y: f32, w: f32, h: f32) -> Option<Collision> {
    let closest_x = ball.x.clamp(x, x + w);
    let closest_y = ball.y.clamp(y, y + h);
    let dx = ball.x - closest_x;
    let dy = ball.y - closest_y;
    if dx * dx + dy * dy > radius * radius {
        return None;
    }

    if dx.abs() > dy.abs() {
        if dx < 0.0 {
            Some(Collision::Left)
        } else {
            Some(Collision::Right)
        }
    } else if dy > 0.0 {
        Some(Collision::Top)
    } else {
        Some(Collision::Bottom)
    }
}

struct Component;
static mut GAME: Option<Breakout> = None;

fn game() -> &'static mut Breakout {
    unsafe { (*core::ptr::addr_of_mut!(GAME)).as_mut().unwrap() }
}

impl Guest for Component {
    fn init(_state: StateSnapshot, _size: (f32, f32)) -> Vec<Effect> {
        unsafe {
            GAME = Some(Breakout::new());
        }
        host_log::info("breakout: init (GPU benchmark)");
        vec![
            Effect::SetTitle("Breakout Benchmark".to_string()),
            Effect::SetTimer(TimerEffect {
                id: TIMER_TICK,
                delay_ms: TICK_MS,
                repeat: true,
            }),
        ]
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        match event {
            InputEvent::SurfaceReady(SurfaceEvent {
                texture_handle,
                width,
                height,
            }) => {
                game().setup_gpu(texture_handle, width, height);
                game().render();
                vec![]
            }
            InputEvent::SurfaceResized(SurfaceEvent {
                texture_handle,
                width,
                height,
            }) => {
                game().setup_gpu(texture_handle, width, height);
                game().render();
                vec![]
            }
            InputEvent::TimerFired(TIMER_TICK) => {
                let g = game();
                g.tick();
                g.render();
                vec![]
            }
            InputEvent::Key(KeyEvent { key, pressed, .. }) => {
                let g = game();
                match key.as_str() {
                    "left" | "a" => g.left = pressed,
                    "right" | "d" => g.right = pressed,
                    "r" if pressed => {
                        g.score = 0;
                        g.lives = 3;
                        g.reset_bricks();
                        g.reset_ball();
                    }
                    "q" | "escape" if pressed => return vec![Effect::CloseSelf],
                    _ => {}
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn view() -> UiTree {
        game().build_tree()
    }
}

export!(Component);
