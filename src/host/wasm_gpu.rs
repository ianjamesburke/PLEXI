// GPU host backend for the WASM runtime (G7 surface lifecycle, G11 render pass).
//
// The `gpu` WIT import is a WebGPU-aligned subset of wgpu. WASM apps issue GPU
// command descriptors (pipelines, buffers, bind groups, render/compute passes)
// across the component boundary; the host executes them against a real wgpu
// device. No framebuffer data crosses the boundary — only command descriptors.
//
// Surface nodes: the host allocates a wgpu texture (`alloc_surface`), hands the
// guest its opaque handle via a `surface-ready` event, and the guest renders
// into it through `submit-render-pass`. The host reads the texture back
// (`read_texture`) to composite it into egui (the live leg) or to assert pixels
// (the gate leg).
//
// Live panes run on the host's *shared* wgpu device (eframe's `RenderState`,
// registered once at startup via [`register_host_render_state`]). That lets the
// guest's surface texture be sampled directly by egui through
// `register_native_texture` — no per-frame readback, no re-upload. Headless and
// test contexts (no eframe) fall back to a dedicated device via [`GpuDevice::new`].
// `read_texture` survives as the capture path for scene/pixel assertions only.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;
#[cfg(test)]
use std::time::Instant;

use crate::host::wasm_app::bindings::plexi::platform::gpu as wit;

/// The host's shared egui/wgpu render state, registered once at eframe startup.
/// `None` in headless/test processes that never construct an eframe app.
static HOST_RENDER_STATE: OnceLock<egui_wgpu::RenderState> = OnceLock::new();

/// Register eframe's shared render state so live WASM surfaces can be composited
/// zero-copy. Idempotent: a second registration is ignored with a warning.
pub fn register_host_render_state(state: egui_wgpu::RenderState) {
    if HOST_RENDER_STATE.set(state).is_err() {
        log::warn!("host render state already registered; ignoring duplicate");
    }
}

/// The shared render state, if the host shell registered one.
pub fn host_render_state() -> Option<&'static egui_wgpu::RenderState> {
    HOST_RENDER_STATE.get()
}

/// A texture the host owns, with its view, format, and dimensions.
struct GpuTexture {
    texture: wgpu::Texture,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

struct GpuPipeline {
    render: Option<wgpu::RenderPipeline>,
    compute: Option<wgpu::ComputePipeline>,
}

/// Number of GPU copies that may be in flight for a non-shared surface.
/// Three buffers let the renderer display the newest completed frame while the
/// GPU is writing the next two; it never needs to wait for the current frame.
const SURFACE_READBACK_RING: usize = 3;

struct AsyncReadbackSlot {
    buffer: wgpu::Buffer,
    busy: bool,
}

struct AsyncReadbackComplete {
    slot: usize,
    image: Result<image::RgbaImage, String>,
}

/// Nonblocking surface readback for the rare non-shared-device path.
///
/// `map_async` is issued from the UI thread, but the blocking `PollType::Wait`
/// poll and row packing run on a worker. The UI only drains completed images;
/// if every staging buffer is busy it keeps displaying the most recent frame.
struct AsyncSurfaceReadbackRing {
    slots: Vec<AsyncReadbackSlot>,
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
    next_slot: usize,
    completed_tx: Sender<AsyncReadbackComplete>,
    completed_rx: Receiver<AsyncReadbackComplete>,
}

impl AsyncSurfaceReadbackRing {
    fn new() -> Self {
        let (completed_tx, completed_rx) = mpsc::channel();
        Self {
            slots: Vec::new(),
            width: 0,
            height: 0,
            padded_bytes_per_row: 0,
            next_slot: 0,
            completed_tx,
            completed_rx,
        }
    }

    fn ensure_size(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        if self.width == width && self.height == height && !self.slots.is_empty() {
            return true;
        }
        if self.slots.iter().any(|slot| slot.busy) {
            // Preserve in-flight buffers until their workers finish. A resize
            // gets picked up on the next frame instead of blocking the UI.
            return false;
        }
        let padded_bytes_per_row = width.saturating_mul(4).div_ceil(256) * 256;
        let size = padded_bytes_per_row as u64 * height as u64;
        self.slots = (0..SURFACE_READBACK_RING)
            .map(|_index| AsyncReadbackSlot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("plexi-surface-readback-ring"),
                    size,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                busy: false,
            })
            .collect();
        self.width = width;
        self.height = height;
        self.padded_bytes_per_row = padded_bytes_per_row;
        self.next_slot = 0;
        log::info!(
            "wasm gpu: allocated async readback ring buffers={} size={}x{} padded_row={}",
            SURFACE_READBACK_RING,
            width,
            height,
            padded_bytes_per_row,
        );
        true
    }

    fn submit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) {
        if !self.ensure_size(device, width, height) {
            return;
        }
        let Some(slot_index) = (0..self.slots.len())
            .map(|offset| (self.next_slot + offset) % self.slots.len())
            .find(|&index| !self.slots[index].busy)
        else {
            return;
        };
        self.next_slot = (slot_index + 1) % self.slots.len();
        let slot = &mut self.slots[slot_index];
        slot.busy = true;

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("plexi-async-surface-readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &slot.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let (mapped_tx, mapped_rx) = mpsc::channel();
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped_tx.send(result.map_err(|err| err.to_string()));
            });
        let worker_device = device.clone();
        let worker_buffer = slot.buffer.clone();
        let completed_tx = self.completed_tx.clone();
        let padded_bytes_per_row = self.padded_bytes_per_row;
        std::thread::spawn(move || {
            if let Err(error) = worker_device.poll(wgpu::PollType::wait_indefinitely()) {
                log::error!("async surface readback poll failed: {error}");
            }
            let image = match mapped_rx.recv() {
                Ok(Ok(())) => {
                    let mapped = worker_buffer.slice(..).get_mapped_range();
                    let packed = pack_rgba_rows(&mapped, width, height, padded_bytes_per_row);
                    drop(mapped);
                    worker_buffer.unmap();
                    packed
                }
                Ok(Err(err)) => Err(format!("async surface map failed: {err}")),
                Err(err) => Err(format!("async surface map callback closed: {err}")),
            };
            let _ = completed_tx.send(AsyncReadbackComplete {
                slot: slot_index,
                image,
            });
        });
    }

    fn take_latest(&mut self) -> Option<Result<image::RgbaImage, String>> {
        let mut latest = None;
        while let Ok(completed) = self.completed_rx.try_recv() {
            if let Some(slot) = self.slots.get_mut(completed.slot) {
                slot.busy = false;
            }
            latest = Some(completed.image);
        }
        latest
    }
}

/// Owns a wgpu device and the opaque-handle registries the `gpu` import refers
/// to. One per app with the gpu capability granted; lives inside `HostCtx`.
pub struct GpuDevice {
    device: wgpu::Device,
    queue: wgpu::Queue,
    next_handle: u64,
    buffers: HashMap<u64, wgpu::Buffer>,
    textures: HashMap<u64, GpuTexture>,
    views: HashMap<u64, wgpu::TextureView>,
    /// Source texture of each guest-created surface view (`create_surface_view`),
    /// so freeing a surface also drops the views that retain its texture.
    view_sources: HashMap<u64, u64>,
    /// View handles retired by a surface reallocation (display-scale change).
    /// A render pass targeting one is dropped with a warning instead of
    /// erroring: the guest legitimately renders one more frame against the
    /// old view before it processes the re-delivered `surface-ready`.
    retired_views: HashSet<u64>,
    pipelines: HashMap<u64, GpuPipeline>,
    bind_groups: HashMap<u64, wgpu::BindGroup>,
    /// Surface readbacks performed by this device. The live present path leaves
    /// this at zero; only the capture path (`read_texture`) increments it. The
    /// perf gate reads this per-device count so it is isolated from other tests.
    readbacks: AtomicU64,
    /// Dedicated-device surface presentation uses this bounded asynchronous ring
    /// instead of synchronously polling the GPU on the UI thread.
    async_surface_readbacks: AsyncSurfaceReadbackRing,
}

impl GpuDevice {
    /// Acquire a headless wgpu device. Errors if no adapter is available
    /// (e.g. CI without a GPU) so the caller can fail the gpu grant cleanly.
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|error| format!("no wgpu adapter available for the gpu capability: {error}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("plexi-wasm-gpu"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: Default::default(),
            experimental_features: Default::default(),
            trace: Default::default(),
        }))
        .map_err(|e| format!("failed to acquire wgpu device: {e}"))?;

        Ok(Self::from_shared(device, queue))
    }

    /// Build a device backed by host-provided wgpu handles. Live panes pass
    /// eframe's shared `RenderState` device/queue so surface textures live on
    /// the same device egui renders with, enabling zero-copy compositing.
    pub fn from_shared(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            device,
            queue,
            next_handle: 1,
            buffers: HashMap::new(),
            textures: HashMap::new(),
            views: HashMap::new(),
            view_sources: HashMap::new(),
            retired_views: HashSet::new(),
            pipelines: HashMap::new(),
            bind_groups: HashMap::new(),
            readbacks: AtomicU64::new(0),
            async_surface_readbacks: AsyncSurfaceReadbackRing::new(),
        }
    }

    /// Surface readbacks this device has performed (capture path only).
    pub fn readback_count(&self) -> u64 {
        self.readbacks.load(Ordering::Relaxed)
    }

    fn next(&mut self) -> u64 {
        let h = self.next_handle;
        self.next_handle += 1;
        h
    }

    // ── Host-side surface management (not part of the WIT trait) ──────────────

    /// Allocate a surface texture for a `surface-node`. Returns the opaque
    /// texture handle the host sends to the guest in a `surface-ready` event.
    pub fn alloc_surface(&mut self, width: u32, height: u32) -> u64 {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("plexi-surface"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            // egui samples the surface through an sRGB view of these same bytes
            // (see `surface_srgb_view`), reproducing the legacy readback look.
            view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
        });
        let handle = self.next();
        self.textures.insert(
            handle,
            GpuTexture {
                texture,
                width,
                height,
                format,
            },
        );
        handle
    }

    /// Drop a surface texture allocated by [`Self::alloc_surface`] — used when
    /// the display scale changes and the surface reallocates at the new
    /// physical resolution (stint 0527). Unknown handles are a no-op; a guest
    /// render against the freed handle gets the ordinary "unknown texture
    /// handle" error until it handles the re-delivered `surface-ready`.
    pub fn free_surface(&mut self, handle: u64) {
        if self.textures.remove(&handle).is_none() {
            log::warn!("wasm gpu: free_surface on unknown texture handle {handle}");
        }
        // Guest-created views retain the texture; drop them too or repeated
        // display-scale changes leak the old texture for the pane's lifetime.
        // Their handles go to `retired_views` so an in-flight guest frame
        // that still targets one drops gracefully.
        let stale: Vec<u64> = self
            .view_sources
            .iter()
            .filter(|(_, tex)| **tex == handle)
            .map(|(view, _)| *view)
            .collect();
        for view in stale {
            self.views.remove(&view);
            self.view_sources.remove(&view);
            self.retired_views.insert(view);
        }
    }

    /// Read a texture (typically a surface) back to a tightly-packed RGBA8
    /// image for pixel assertions. Test-only: live composition uses the
    /// zero-copy shared-device path or the async readback ring, never this
    /// synchronous UI-thread-blocking call.
    #[cfg(test)]
    pub fn read_texture(&self, handle: u64) -> Result<image::RgbaImage, String> {
        self.readbacks.fetch_add(1, Ordering::Relaxed);
        let tex = self.textures.get(&handle).ok_or("unknown texture handle")?;
        if !matches!(
            tex.format,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
        ) {
            return Err("read_texture only supports rgba8 surfaces".to_string());
        }
        let (w, h) = (tex.width, tex.height);
        // copy_texture_to_buffer needs bytes_per_row aligned to 256.
        let unpadded = w * 4;
        let padded = unpadded.div_ceil(256) * 256;
        let tight_bytes = unpadded as u64 * h as u64;
        let padded_bytes = padded as u64 * h as u64;
        let total_start = Instant::now();
        let encode_start = Instant::now();
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plexi-surface-readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("plexi-readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &tex.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let encode_submit_us = encode_start.elapsed().as_micros();

        let map_start = Instant::now();
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("buffer readback poll failed: {error}"))?;
        let map_wait_us = map_start.elapsed().as_micros();

        let pack_start = Instant::now();
        let data = slice.get_mapped_range();
        let out = pack_rgba_rows(&data, w, h, padded)?;
        let pack_us = pack_start.elapsed().as_micros();
        drop(data);
        buf.unmap();
        // Capture path only (scene/pixel assertions) — never the live present
        // path — so this stays off `info` to keep hot-path logs clean.
        log::debug!(
            "wasm gpu readback: texture={handle} size={}x{} bytes={} padded_bytes={} encode_submit_us={} map_wait_us={} pack_us={} total_us={}",
            w,
            h,
            tight_bytes,
            padded_bytes,
            encode_submit_us,
            map_wait_us,
            pack_us,
            total_start.elapsed().as_micros()
        );
        Ok(out)
    }

    /// An sRGB-reinterpreting view of a surface texture, for handing to egui's
    /// renderer via `register_native_texture`. The guest renders into the base
    /// `Rgba8Unorm` texture; egui samples the identical bytes as sRGB, matching
    /// how the old readback→`ColorImage` path displayed them. Live present path.
    pub fn surface_srgb_view(&self, handle: u64) -> Option<wgpu::TextureView> {
        let tex = self.textures.get(&handle)?;
        Some(tex.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("plexi-surface-egui-srgb"),
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            ..Default::default()
        }))
    }

    /// Queue a surface copy into the nonblocking staging ring. Completion is
    /// obtained with [`Self::take_surface_readback`]; neither method waits for
    /// GPU work on the caller thread.
    pub fn request_surface_readback(&mut self, handle: u64) -> Result<(), String> {
        let tex = self.textures.get(&handle).ok_or("unknown texture handle")?;
        if !matches!(
            tex.format,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
        ) {
            return Err("async readback only supports rgba8 surfaces".to_string());
        }
        self.async_surface_readbacks.submit(
            &self.device,
            &self.queue,
            &tex.texture,
            tex.width,
            tex.height,
        );
        Ok(())
    }

    /// Return the newest completed asynchronous surface frame, if one is ready.
    pub fn take_surface_readback(&mut self) -> Option<Result<image::RgbaImage, String>> {
        self.async_surface_readbacks.take_latest()
    }

    // ── WIT-facing operations ────────────────────────────────────────────────

    pub fn create_surface_view(&mut self, texture: u64) -> Result<u64, String> {
        let tex = self
            .textures
            .get(&texture)
            .ok_or("unknown texture handle")?;
        let view = tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let handle = self.next();
        self.views.insert(handle, view);
        self.view_sources.insert(handle, texture);
        Ok(handle)
    }

    pub fn create_buffer(&mut self, label: &str, size: u64, usage: wit::BufferUsage) -> u64 {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: map_buffer_usage(usage),
            mapped_at_creation: false,
        });
        let handle = self.next();
        self.buffers.insert(handle, buffer);
        handle
    }

    pub fn write_buffer(&mut self, handle: u64, offset: u64, data: &[u8]) -> Result<(), String> {
        let buffer = self.buffers.get(&handle).ok_or("unknown buffer handle")?;
        self.queue.write_buffer(buffer, offset, data);
        Ok(())
    }

    pub fn read_buffer(&self, handle: u64, offset: u64, size: u64) -> Result<Vec<u8>, String> {
        let src = self.buffers.get(&handle).ok_or("unknown buffer handle")?;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plexi-buffer-readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("plexi-buf-readback"),
            });
        encoder.copy_buffer_to_buffer(src, offset, &staging, 0, size);
        self.queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("buffer readback poll failed: {error}"))?;
        let out = slice.get_mapped_range().to_vec();
        staging.unmap();
        Ok(out)
    }

    pub fn create_texture(&mut self, label: &str, desc: wit::TextureDesc) -> Result<u64, String> {
        let format = map_texture_format(desc.format);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: desc.width,
                height: desc.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: desc.mip_levels.max(1),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: map_texture_usage(desc.usage),
            view_formats: &[],
        });
        let handle = self.next();
        self.textures.insert(
            handle,
            GpuTexture {
                texture,
                width: desc.width,
                height: desc.height,
                format,
            },
        );
        Ok(handle)
    }

    pub fn create_render_pipeline(
        &mut self,
        label: &str,
        wgsl: &str,
        desc: wit::RenderPipelineDesc,
    ) -> Result<u64, String> {
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });

        let attrs: Vec<wgpu::VertexAttribute> = desc
            .attrs
            .iter()
            .map(|a| {
                Ok(wgpu::VertexAttribute {
                    format: map_vertex_format(&a.format)?,
                    offset: a.offset as u64,
                    shader_location: a.location,
                })
            })
            .collect::<Result<_, String>>()?;

        // Per-instance vertex buffer (the POC pattern: corners from
        // vertex_index, per-object data from instance-rate attributes).
        let buffers: Vec<wgpu::VertexBufferLayout> = if attrs.is_empty() {
            vec![]
        } else {
            vec![wgpu::VertexBufferLayout {
                array_stride: desc.vertex_stride as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &attrs,
            }]
        };

        let blend = if desc.blend_alpha {
            Some(wgpu::BlendState::ALPHA_BLENDING)
        } else {
            None
        };

        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(&desc.vs_entry),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(&desc.fs_entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: map_texture_format(desc.output_format),
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let handle = self.next();
        self.pipelines.insert(
            handle,
            GpuPipeline {
                render: Some(pipeline),
                compute: None,
            },
        );
        Ok(handle)
    }

    pub fn create_compute_pipeline(
        &mut self,
        label: &str,
        wgsl: &str,
        entry: &str,
    ) -> Result<u64, String> {
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            });
        let handle = self.next();
        self.pipelines.insert(
            handle,
            GpuPipeline {
                render: None,
                compute: Some(pipeline),
            },
        );
        Ok(handle)
    }

    pub fn create_bind_group(
        &mut self,
        pipeline: u64,
        bindings: &[wit::BindingEntry],
    ) -> Result<u64, String> {
        let pl = self
            .pipelines
            .get(&pipeline)
            .ok_or("unknown pipeline handle")?;
        let layout = match (&pl.render, &pl.compute) {
            (Some(r), _) => r.get_bind_group_layout(0),
            (_, Some(c)) => c.get_bind_group_layout(0),
            _ => return Err("pipeline has no stages".to_string()),
        };
        let entries: Vec<wgpu::BindGroupEntry> = bindings
            .iter()
            .map(|b| {
                let resource = match &b.resource_ref {
                    wit::BindingResource::Buffer(h) => {
                        let buffer = self.buffers.get(h).ok_or("unknown buffer in binding")?;
                        buffer.as_entire_binding()
                    }
                    wit::BindingResource::Texture(h) => {
                        // Texture bindings need a long-lived view; not exercised
                        // by the current POCs. Reject clearly until needed.
                        let _ = h;
                        return Err("texture bindings not yet supported".to_string());
                    }
                };
                Ok(wgpu::BindGroupEntry {
                    binding: b.binding,
                    resource,
                })
            })
            .collect::<Result<_, String>>()?;
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("plexi-bind-group"),
            layout: &layout,
            entries: &entries,
        });
        let handle = self.next();
        self.bind_groups.insert(handle, bind_group);
        Ok(handle)
    }

    pub fn submit_render_pass(&mut self, pass: wit::RenderPassDesc) -> Result<(), String> {
        // A frame drawn against a view retired by a surface reallocation is
        // dropped, not an error: the guest hasn't processed the re-delivered
        // `surface-ready` yet and re-targets on its next frame.
        if self.retired_views.contains(&pass.target) {
            log::warn!(
                "wasm gpu: dropped render pass targeting retired view {} \
                 (surface reallocated for a display-scale change)",
                pass.target
            );
            return Ok(());
        }
        let view = self
            .views
            .get(&pass.target)
            .ok_or("unknown surface view handle")?;
        let pipeline = self
            .pipelines
            .get(&pass.pipeline)
            .and_then(|p| p.render.as_ref())
            .ok_or("unknown render pipeline handle")?;
        let vbuf = match pass.vertex_buffer {
            Some(h) => Some(self.buffers.get(&h).ok_or("unknown vertex buffer handle")?),
            None => None,
        };
        let groups: Vec<(u32, &wgpu::BindGroup)> = pass
            .bind_groups
            .iter()
            .map(|(idx, h)| {
                self.bind_groups
                    .get(h)
                    .map(|bg| (*idx, bg))
                    .ok_or_else(|| "unknown bind group handle".to_string())
            })
            .collect::<Result<_, String>>()?;

        let load = match pass.clear_color {
            Some((r, g, b, a)) => wgpu::LoadOp::Clear(wgpu::Color {
                r: r as f64,
                g: g as f64,
                b: b as f64,
                a: a as f64,
            }),
            None => wgpu::LoadOp::Load,
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("plexi-render-pass"),
            });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("plexi-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(pipeline);
            for (idx, bg) in &groups {
                rpass.set_bind_group(*idx, *bg, &[]);
            }
            if let Some(vbuf) = vbuf {
                rpass.set_vertex_buffer(0, vbuf.slice(..));
            }
            for d in &pass.draws {
                rpass.draw(
                    d.first_vertex..d.first_vertex + d.vertices,
                    d.first_instance..d.first_instance + d.instances,
                );
            }
        }
        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    pub fn submit_compute_pass(&mut self, pass: wit::ComputePassDesc) -> Result<(), String> {
        let pipeline = self
            .pipelines
            .get(&pass.pipeline)
            .and_then(|p| p.compute.as_ref())
            .ok_or("unknown compute pipeline handle")?;
        let groups: Vec<(u32, &wgpu::BindGroup)> = pass
            .bind_groups
            .iter()
            .map(|(idx, h)| {
                self.bind_groups
                    .get(h)
                    .map(|bg| (*idx, bg))
                    .ok_or_else(|| "unknown bind group handle".to_string())
            })
            .collect::<Result<_, String>>()?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("plexi-compute-pass"),
            });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("plexi-compute-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            for (idx, bg) in &groups {
                cpass.set_bind_group(*idx, *bg, &[]);
            }
            for d in &pass.dispatches {
                cpass.dispatch_workgroups(d.x, d.y, d.z);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    pub fn copy_texture(&mut self, src: u64, dst: u64) -> Result<(), String> {
        let s = self.textures.get(&src).ok_or("unknown src texture")?;
        let d = self.textures.get(&dst).ok_or("unknown dst texture")?;
        let (w, h) = (s.width.min(d.width), s.height.min(d.height));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("plexi-copy-texture"),
            });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &s.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &d.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    pub fn destroy_buffer(&mut self, handle: u64) {
        self.buffers.remove(&handle);
    }

    pub fn destroy_texture(&mut self, handle: u64) {
        self.textures.remove(&handle);
        self.views.remove(&handle);
    }

    pub fn destroy_pipeline(&mut self, handle: u64) {
        self.pipelines.remove(&handle);
    }

    pub fn destroy_bind_group(&mut self, handle: u64) {
        self.bind_groups.remove(&handle);
    }
}

fn pack_rgba_rows(
    data: &[u8],
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
) -> Result<image::RgbaImage, String> {
    let unpadded = width
        .checked_mul(4)
        .ok_or_else(|| format!("rgba row too wide: {width}"))?;
    let expected_len = padded_bytes_per_row as usize * height as usize;
    if data.len() < expected_len {
        return Err(format!(
            "readback buffer too small: got {} bytes, expected at least {expected_len}",
            data.len()
        ));
    }
    let mut packed = Vec::with_capacity(unpadded as usize * height as usize);
    for y in 0..height {
        let row_start = padded_bytes_per_row as usize * y as usize;
        let row_end = row_start + unpadded as usize;
        packed.extend_from_slice(&data[row_start..row_end]);
    }
    image::RgbaImage::from_raw(width, height, packed)
        .ok_or_else(|| format!("failed to build rgba image {width}x{height} from readback"))
}

// ── WIT type mappers ─────────────────────────────────────────────────────────

fn map_buffer_usage(u: wit::BufferUsage) -> wgpu::BufferUsages {
    let mut out = wgpu::BufferUsages::empty();
    if u.contains(wit::BufferUsage::VERTEX) {
        out |= wgpu::BufferUsages::VERTEX;
    }
    if u.contains(wit::BufferUsage::INDEX) {
        out |= wgpu::BufferUsages::INDEX;
    }
    if u.contains(wit::BufferUsage::UNIFORM) {
        out |= wgpu::BufferUsages::UNIFORM;
    }
    if u.contains(wit::BufferUsage::STORAGE) {
        out |= wgpu::BufferUsages::STORAGE;
    }
    if u.contains(wit::BufferUsage::COPY_SRC) {
        out |= wgpu::BufferUsages::COPY_SRC;
    }
    if u.contains(wit::BufferUsage::COPY_DST) {
        out |= wgpu::BufferUsages::COPY_DST;
    }
    out
}

fn map_texture_usage(u: wit::TextureUsage) -> wgpu::TextureUsages {
    let mut out = wgpu::TextureUsages::empty();
    if u.contains(wit::TextureUsage::TEXTURE_BINDING) {
        out |= wgpu::TextureUsages::TEXTURE_BINDING;
    }
    if u.contains(wit::TextureUsage::STORAGE_BINDING) {
        out |= wgpu::TextureUsages::STORAGE_BINDING;
    }
    if u.contains(wit::TextureUsage::RENDER_ATTACHMENT) {
        out |= wgpu::TextureUsages::RENDER_ATTACHMENT;
    }
    if u.contains(wit::TextureUsage::COPY_SRC) {
        out |= wgpu::TextureUsages::COPY_SRC;
    }
    if u.contains(wit::TextureUsage::COPY_DST) {
        out |= wgpu::TextureUsages::COPY_DST;
    }
    out
}

fn map_texture_format(f: wit::TextureFormat) -> wgpu::TextureFormat {
    match f {
        wit::TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        wit::TextureFormat::Rgba8Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        wit::TextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        wit::TextureFormat::R32Float => wgpu::TextureFormat::R32Float,
        wit::TextureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
    }
}

fn map_vertex_format(s: &str) -> Result<wgpu::VertexFormat, String> {
    Ok(match s {
        "float32" => wgpu::VertexFormat::Float32,
        "float32x2" => wgpu::VertexFormat::Float32x2,
        "float32x3" => wgpu::VertexFormat::Float32x3,
        "float32x4" => wgpu::VertexFormat::Float32x4,
        "uint32" => wgpu::VertexFormat::Uint32,
        "uint32x2" => wgpu::VertexFormat::Uint32x2,
        "uint32x4" => wgpu::VertexFormat::Uint32x4,
        "sint32" => wgpu::VertexFormat::Sint32,
        other => return Err(format!("unsupported vertex format '{other}'")),
    })
}

#[cfg(test)]
mod tests {
    use super::pack_rgba_rows;

    #[test]
    fn pack_rgba_rows_strips_row_padding() {
        let row0 = [1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99];
        let row1 = [9, 10, 11, 12, 13, 14, 15, 16, 88, 88, 88, 88];
        let mut data = Vec::new();
        data.extend_from_slice(&row0);
        data.extend_from_slice(&row1);

        let img = pack_rgba_rows(&data, 2, 2, 12).expect("pack rows");

        assert_eq!(
            img.as_raw(),
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }
}
