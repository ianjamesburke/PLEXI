//! Headless app rendering — spawns an app process, collects one frame, renders to PNG.
//!
//! Used by `plexi app render <id>` to produce screenshots without a display.

use crate::app_protocol::{ControlCommand, DrawCommand, PlexiEvent, Rect as ProtoRect, RenderCommand};
use crate::ui::theme::Colors;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BOOT_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
const PROTOCOL: &str = "pgap/3";

/// Spawn the app, collect one frame, return raw RenderCommands as JSON.
pub fn render_app_to_json(
    app_id: &str,
    bin_path: &Path,
    width: u32,
    height: u32,
    seed_state: Option<serde_json::Value>,
) -> Result<String, String> {
    let commands = spawn_and_collect_frame(app_id, bin_path, width, height, seed_state)?;
    serde_json::to_string_pretty(&commands)
        .map_err(|e| format!("failed to serialize frame: {e}"))
}

/// Spawn the app at `bin_path`, collect one frame at `width`×`height`, render to PNG bytes.
pub fn render_app_to_png(
    app_id: &str,
    bin_path: &Path,
    width: u32,
    height: u32,
    seed_state: Option<serde_json::Value>,
) -> Result<Vec<u8>, String> {
    let commands = spawn_and_collect_frame(app_id, bin_path, width, height, seed_state)?;
    render_commands_to_png(&commands, width, height)
}

/// Spawn the app, exchange Init/Ready, send one Render, collect until FrameDone.
fn spawn_and_collect_frame(
    app_id: &str,
    bin_path: &Path,
    width: u32,
    height: u32,
    seed_state: Option<serde_json::Value>,
) -> Result<Vec<RenderCommand>, String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    // Mirror the env setup from ProcessApp::launch so Python apps can import plexi_sdk.
    // .py entries are launched via python3 — no shebang or execute bit required (mirrors ProcessApp::launch).
    const ENV_WHITELIST: &[&str] = &["HOME", "PATH", "LANG", "LC_ALL", "TERM", "USER", "SHELL"];
    let bundle_contents = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()));
    let bundled_py_bin = bundle_contents.as_ref()
        .map(|c| c.join("Resources").join("assets").join("python").join("bin"));
    let bundle_sdk = bundle_contents.as_ref()
        .map(|c| c.join("Resources").join("sdk").join("python"));
    let is_python = bin_path.extension().and_then(|e| e.to_str()) == Some("py");
    let mut cmd = if is_python {
        let venv_python = bin_path
            .parent()
            .map(|app_dir| app_dir.join(".venv").join("bin").join("python"))
            .filter(|p| p.exists());
        let py = venv_python
            .map(std::ffi::OsString::from)
            .or_else(|| {
                bundled_py_bin.as_ref()
                    .map(|b| b.join("python3"))
                    .filter(|p| p.exists())
                    .map(std::ffi::OsString::from)
            })
            .unwrap_or_else(|| std::ffi::OsString::from("python3"));
        log::info!("app_render[{app_id}]: launching .py entry via {:?}", py);
        let mut c = Command::new(py);
        c.arg(bin_path);
        c
    } else {
        Command::new(bin_path)
    };
    cmd.current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear();
    for var in ENV_WHITELIST {
        if let Ok(v) = std::env::var(var) {
            cmd.env(var, v);
        }
    }
    for (k, v) in std::env::vars() {
        if k.starts_with("PLEXI_") {
            cmd.env(k, v);
        }
    }
    let pythonpath = crate::config::build_pythonpath(bundle_sdk.as_deref());
    cmd.env("PYTHONPATH", &pythonpath);
    log::info!("app_render[{app_id}]: PYTHONPATH={pythonpath}");

    let mut child = cmd.spawn()
        .map_err(|e| format!("failed to spawn '{app_id}' at {}: {e}", bin_path.display()))?;

    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut reader = BufReader::new(stdout);

    // Send Init (with optional seed state embedded)
    let init = PlexiEvent::Init {
        protocol: PROTOCOL.to_string(),
        app_id: app_id.to_string(),
        workspace_root: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        capabilities: vec![],
        feature_flags: vec![],
        compact_threshold: 280.0,
        regular_threshold: 480.0,
        theme: render_colors().to_theme_map(),
        args: vec![],
        state: seed_state,
    };
    let init_json = serde_json::to_string(&init)
        .map_err(|e| format!("failed to serialize Init: {e}"))?;
    writeln!(stdin, "{init_json}").map_err(|e| format!("failed to write Init: {e}"))?;
    log::info!("app_render[{app_id}]: sent Init");

    // Wait for Ready
    let deadline = Instant::now() + BOOT_TIMEOUT;
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(format!("app '{app_id}' did not send Ready within {BOOT_TIMEOUT:?}"));
        }
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| format!("read error waiting for Ready: {e}"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<DrawCommand>(line) {
            Ok(DrawCommand::Control(ControlCommand::Ready { .. })) => {
                log::info!("app_render[{app_id}]: got Ready");
                break;
            }
            Ok(_) => { /* ignore other commands before Ready */ }
            Err(e) => {
                log::warn!("app_render[{app_id}]: parse error before Ready: {e} (line: {line})");
            }
        }
    }

    // Send Render
    let render = PlexiEvent::Render {
        frame_id: 1,
        rect: ProtoRect { x: 0.0, y: 0.0, w: width as f32, h: height as f32 },
    };
    let render_json = serde_json::to_string(&render)
        .map_err(|e| format!("failed to serialize Render: {e}"))?;
    writeln!(stdin, "{render_json}").map_err(|e| format!("failed to write Render: {e}"))?;
    log::info!("app_render[{app_id}]: sent Render {width}×{height}");

    // Collect RenderCommands until FrameDone
    let mut render_commands: Vec<RenderCommand> = Vec::new();
    let deadline = Instant::now() + FRAME_TIMEOUT;
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(format!("app '{app_id}' did not emit FrameDone within {FRAME_TIMEOUT:?}"));
        }
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| format!("read error collecting frame: {e}"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<DrawCommand>(line) {
            Ok(DrawCommand::Render(rc)) => {
                render_commands.push(rc);
            }
            Ok(DrawCommand::Control(ControlCommand::FrameDone { .. })) => {
                log::info!("app_render[{app_id}]: FrameDone — {} commands", render_commands.len());
                break;
            }
            Ok(_) => { /* ignore host/control commands during frame collection */ }
            Err(e) => {
                log::warn!("app_render[{app_id}]: parse error in frame: {e}");
            }
        }
    }

    let _ = child.kill();
    Ok(render_commands)
}

/// Render RenderCommands to PNG bytes via a headless egui context + wgpu offscreen surface.
fn render_commands_to_png(commands: &[RenderCommand], width: u32, height: u32) -> Result<Vec<u8>, String> {
    // ── 1. Build egui shapes via a headless egui context ──────────────────
    let ctx = egui::Context::default();
    ctx.set_fonts(egui::FontDefinitions::default());

    let rect = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(width as f32, height as f32),
    );
    let mut viewport_info = egui::ViewportInfo::default();
    viewport_info.native_pixels_per_point = Some(1.0);
    viewport_info.inner_rect = Some(rect);
    let viewports: egui::ViewportIdMap<egui::ViewportInfo> =
        std::iter::once((egui::ViewportId::ROOT, viewport_info)).collect();
    let raw_input = egui::RawInput {
        screen_rect: Some(rect),
        viewports,
        ..Default::default()
    };

    let colors = render_colors();
    let mut cm_cache = egui_commonmark::CommonMarkCache::default();
    let peaks: HashMap<String, f32> = HashMap::new();
    let mut img_cache = crate::process_app::image_cache::ImageCache::new();

    let full_output = ctx.run(raw_input, |ctx| {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let mut lv_offsets = std::collections::HashMap::new();
                let mut lv_last_sel = std::collections::HashMap::new();
                let mut te_buffers = std::collections::HashMap::new();
                let mut te_focus_ctx = crate::render::components::TextEditFocusCtx::new();
                crate::process_app::render::render_draw_commands(
                    ui, rect, commands, &colors, &mut cm_cache, &peaks,
                    &mut img_cache, &std::env::temp_dir(), false,
                    &mut lv_offsets,
                    &mut lv_last_sel,
                    &mut Vec::new(),
                    &mut te_buffers,
                    &mut te_focus_ctx,
                );
            });
    });

    let pixels_per_point = full_output.pixels_per_point;
    let clipped_primitives = ctx.tessellate(full_output.shapes, pixels_per_point);
    let textures_delta = full_output.textures_delta;

    // ── 2. Set up wgpu offscreen ──────────────────────────────────────────
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok_or("no wgpu adapter available for headless rendering")?;

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("plexi-headless"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: Default::default(),
        },
        None,
    ))
    .map_err(|e| format!("failed to acquire wgpu device: {e}"))?;

    let texture_format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("plexi-offscreen"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // ── 3. egui_wgpu renderer ─────────────────────────────────────────────
    let mut renderer = egui_wgpu::Renderer::new(&device, texture_format, None, 1, false);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("plexi-render"),
    });

    for (id, delta) in &textures_delta.set {
        renderer.update_texture(&device, &queue, *id, delta);
    }

    let screen_descriptor = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point,
    };
    renderer.update_buffers(&device, &queue, &mut encoder, &clipped_primitives, &screen_descriptor);

    {
        let mut rpass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("plexi-egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            })
            .forget_lifetime();
        renderer.render(&mut rpass, &clipped_primitives, &screen_descriptor);
    }

    // ── 4. Readback ───────────────────────────────────────────────────────
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let bytes_per_row_raw = width * 4;
    let bytes_per_row = bytes_per_row_raw.next_multiple_of(align);

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("plexi-readback"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);

    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|_| "readback channel closed unexpectedly".to_string())?
        .map_err(|e| format!("GPU readback failed: {e}"))?;

    let mut pixels: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);
    {
        let mapped = readback.slice(..).get_mapped_range();
        for row in 0..height {
            let start = (row * bytes_per_row) as usize;
            let end = start + (width * 4) as usize;
            pixels.extend_from_slice(&mapped[start..end]);
        }
    }
    readback.unmap();

    // ── 5. Encode PNG ─────────────────────────────────────────────────────
    let mut png_data: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_data, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .and_then(|mut w| w.write_image_data(&pixels))
            .map_err(|e| format!("PNG encoding failed: {e}"))?;
    }
    log::info!("app_render: encoded {} bytes for {width}×{height}", png_data.len());
    Ok(png_data)
}

/// Resolve the colors used for headless render: the user's actual configured
/// theme when a config exists, falling back to the built-in dark defaults.
/// This makes `app render` output match what the GUI shows (light/dark + overrides).
fn render_colors() -> Colors {
    let config = crate::config::PlexiConfig::load_with_workspace(None);
    crate::ui::theme::colors_from_config(&config)
}
