//! Live-host screenshot capture (`plexi host screenshot`, stint 0461).
//!
//! The CLI sends `AppRequest::Screenshot`; the handler queues a
//! [`PendingScreenshot`] and asks egui for a viewport capture
//! (`ViewportCommand::Screenshot`). The wgpu backend reads the real
//! live paint target back and delivers it as `egui::Event::Screenshot` in a later
//! frame's raw input, where [`PlexiApp::fulfill_screenshot_events`] crops,
//! encodes, and writes the PNG plus the CLI response file. This captures the
//! host UI — chrome, terminals, apps, overlays — with
//! no OS-level screen capture involved. The native backend paints requested
//! frames into a GPU texture without acquiring or presenting a window drawable,
//! including when occluded. It uses the same egui context and renderer.
//! Request timestamps reject unrelated, expired and late readbacks.
//!
//! # The reply invariant (stints 0495, 0504)
//!
//! Every queued request gets exactly one typed reply — a success object or an
//! `{"error": ...}` object — and it always arrives inside the CLI's wait
//! window. Two rules hold that up:
//!
//! 1. [`PlexiApp::service_pending_screenshots`] runs from `App::logic`, never
//!    `App::ui`. eframe skips `ui` entirely while the window is minimized or
//!    fully occluded, so issuing the viewport command from `ui` meant an
//!    occluded host never asked for a capture at all: the PNG landed late
//!    (once something un-occluded the window — stint 0495) or never (asleep
//!    display — stint 0504), and the CLI reported its own generic timeout with
//!    nothing in the host log to explain it.
//! 2. [`SCREENSHOT_DEADLINE`] bounds every request. When the compositor
//!    genuinely cannot produce pixels, the host writes the typed error itself
//!    and logs it at warn, rather than leaving the caller to time out silently.

use super::PlexiApp;

/// How long the host waits for wgpu to deliver a capture before replying with
/// a typed error. Must stay comfortably under `host_screenshot_cli`'s response
/// timeout so the caller surfaces this reason instead of its own generic one.
pub const SCREENSHOT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(6);

#[derive(Debug)]
pub struct PendingScreenshot {
    /// Crop to this pane's screen rect; `None` captures the whole window.
    pub pane_id: Option<u64>,
    pub output_path: String,
    pub response_file: String,
    pub capture_requested_at: Option<std::time::Instant>,
    /// UI frame count at enqueue, used to observe UI activity during the request.
    pub initial_ui_frame: u64,
    /// Reply with a typed error once this instant passes. Set from
    /// [`SCREENSHOT_DEADLINE`] at enqueue time.
    pub expires_at: std::time::Instant,
}

impl PlexiApp {
    /// Drive every outstanding screenshot request one step.
    ///
    /// Call this from `App::logic` only — see the module docs. Expiry runs
    /// first so an overdue request replies even on a pass where the readback
    /// poll would be a no-op.
    pub(crate) fn service_pending_screenshots(
        &mut self,
        ctx: &egui::Context,
        render_state: Option<&egui_wgpu::RenderState>,
    ) {
        if self.pending_screenshots.is_empty() {
            return;
        }
        self.expire_pending_screenshots(ctx);
        if self.pending_screenshots.is_empty() {
            return;
        }
        self.poll_pending_screenshots(render_state);
        self.request_pending_screenshot(ctx);
        // eframe 0.34 handles viewport commands after the current paint, then
        // delivers wgpu's asynchronous result before a later pass. Keep those
        // lifecycle stages moving while a CLI request is outstanding. egui
        // advances delayed requests by predicted_dt, so include that interval
        // to guarantee this targets a subsequent OS frame — and never pass a
        // literal zero, which schedules an extra settling paint.
        let predicted_dt = ctx.input(|input| input.predicted_dt);
        ctx.request_repaint_after(
            std::time::Duration::from_secs_f32(predicted_dt) + std::time::Duration::from_millis(25),
        );
    }

    /// Reply with a typed error for any request the viewport never fulfilled.
    ///
    /// A silent stall is indistinguishable from a hung host, so this is the
    /// backstop that makes "no pixels available" an observable, actionable
    /// failure instead of a CLI-side timeout with an empty host log.
    fn expire_pending_screenshots(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        if !self
            .pending_screenshots
            .iter()
            .any(|request| now >= request.expires_at)
        {
            return;
        }
        let (expired, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_screenshots)
            .into_iter()
            .partition(|request| now >= request.expires_at);
        self.pending_screenshots = live;
        let viewport = ctx.input(|input| input.viewport().clone());
        let ui_frame = self.frame_tick.load(std::sync::atomic::Ordering::Relaxed);
        for request in expired {
            let error = screenshot_timeout_error(
                viewport.minimized,
                viewport.occluded,
                viewport.focused,
                ui_frame.saturating_sub(request.initial_ui_frame),
            );
            log::warn!(
                "screenshot: pane_id={:?} timed out: {error}",
                request.pane_id,
            );
            crate::rpc::write_json_response(
                &request.response_file,
                serde_json::json!({ "error": error }),
            );
        }
    }

    /// Issue one capture per newly queued batch. Keep its identity stable while
    /// GPU readback is outstanding; repaint/poll and the deadline bound progress.
    fn request_pending_screenshot(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        let should_capture = self
            .pending_screenshots
            .iter()
            .any(|request| request.capture_requested_at.is_none());
        if !should_capture {
            return;
        }
        for request in &mut self.pending_screenshots {
            request.capture_requested_at.get_or_insert(now);
        }
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::Screenshot(egui::UserData::new(now)),
        );
    }

    /// Advance wgpu's asynchronous screenshot readback.
    fn poll_pending_screenshots(&self, render_state: Option<&egui_wgpu::RenderState>) {
        // wgpu 29 no longer advances map_async callbacks without an explicit
        // device poll. Keep this non-blocking: a blocking wait stalls the UI
        // before eframe has painted and registered the capture callback.
        if let Some(render_state) = render_state {
            if let Err(error) = render_state.device.poll(wgpu::PollType::Poll) {
                log::warn!("screenshot: wgpu device poll failed: {error}");
            }
        } else {
            log::warn!("screenshot: eframe wgpu render state unavailable for readback polling");
        }
    }

    /// Fulfill queued requests from eframe's raw-input hook.
    ///
    /// eframe 0.34 injects completed wgpu captures immediately before this
    /// hook. Consuming them here avoids depending on egui's later input-state
    /// lifecycle.
    pub(crate) fn fulfill_screenshot_events(
        &mut self,
        ctx: &egui::Context,
        raw_input: &egui::RawInput,
    ) {
        for event in &raw_input.events {
            let egui::Event::Screenshot {
                viewport_id,
                user_data,
                image,
            } = event
            else {
                continue;
            };
            if *viewport_id != egui::ViewportId::ROOT {
                continue;
            }
            let Some(captured_at) = user_data
                .data
                .as_ref()
                .and_then(|data| data.downcast_ref::<std::time::Instant>())
            else {
                continue;
            };
            // Match the request generation, not merely the arrival of any
            // image. A late GPU result must not satisfy a newer CLI request.
            let now = std::time::Instant::now();
            let (matched, remaining): (Vec<_>, Vec<_>) =
                std::mem::take(&mut self.pending_screenshots)
                    .into_iter()
                    .partition(|request| {
                        request.capture_requested_at == Some(*captured_at)
                            && now < request.expires_at
                    });
            self.pending_screenshots = remaining;
            let pixels_per_point = ctx.pixels_per_point();
            for request in matched {
                let result = self.write_screenshot(&request, image, pixels_per_point);
                let response = match &result {
                    Ok((width, height)) => serde_json::json!({
                        "ok": true,
                        "path": request.output_path,
                        "width": width,
                        "height": height,
                    }),
                    Err(error) => {
                        log::warn!(
                            "screenshot: pane_id={:?} output={} failed: {error}",
                            request.pane_id,
                            request.output_path
                        );
                        serde_json::json!({ "error": error })
                    }
                };
                crate::rpc::write_json_response(&request.response_file, response);
            }
        }
    }

    fn write_screenshot(
        &mut self,
        request: &PendingScreenshot,
        image: &egui::ColorImage,
        pixels_per_point: f32,
    ) -> Result<(u32, u32), String> {
        let region = match request.pane_id {
            None => None,
            Some(pane_id) => {
                if self.find_pane_in_any_window(pane_id).is_none() {
                    return Err(format!("pane {pane_id} not found"));
                };
                let Some(rect) = self.visible_pane_rect(pane_id) else {
                    return Err(format!(
                        "pane {pane_id}: no known screen rect (pane has not rendered yet)"
                    ));
                };
                Some(rect)
            }
        };
        let cropped = crop_color_image(image, region, pixels_per_point)?;
        let (width, height) = (cropped.width() as u32, cropped.height() as u32);
        let mut rgba = Vec::with_capacity(cropped.pixels.len() * 4);
        for pixel in &cropped.pixels {
            rgba.extend_from_slice(&pixel.to_array());
        }
        let buffer = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| "could not assemble RGBA buffer from capture".to_string())?;
        if let Some(parent) = std::path::Path::new(&request.output_path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        buffer
            .save_with_format(&request.output_path, image::ImageFormat::Png)
            .map_err(|e| format!("could not write {}: {e}", request.output_path))?;
        log::info!(
            "screenshot: wrote {}x{} png to {} (pane_id={:?})",
            width,
            height,
            request.output_path,
            request.pane_id
        );
        Ok((width, height))
    }
}

/// Report observations at the deadline, without asserting a cause for the timeout.
fn screenshot_timeout_error(
    minimized: Option<bool>,
    occluded: Option<bool>,
    focused: Option<bool>,
    ui_frames: u64,
) -> String {
    fn state(value: Option<bool>) -> &'static str {
        match value {
            Some(true) => "true",
            Some(false) => "false",
            None => "unknown",
        }
    }
    format!(
        "screenshot readback did not complete within {}s: no screenshot frame was received; \
         viewport at timeout: minimized={}, occluded={}, focused={}; UI frames during request={ui_frames}",
        SCREENSHOT_DEADLINE.as_secs(),
        state(minimized),
        state(occluded),
        state(focused),
    )
}

/// Crop a captured frame to `region` (logical points; `None` = full frame).
/// The capture is in physical pixels, so the rect scales by
/// `pixels_per_point` and clamps to the frame bounds.
fn crop_color_image(
    image: &egui::ColorImage,
    region: Option<egui::Rect>,
    pixels_per_point: f32,
) -> Result<egui::ColorImage, String> {
    let Some(rect) = region else {
        return Ok(image.clone());
    };
    let (frame_w, frame_h) = (image.width(), image.height());
    let x0 = ((rect.min.x * pixels_per_point).floor().max(0.0) as usize).min(frame_w);
    let y0 = ((rect.min.y * pixels_per_point).floor().max(0.0) as usize).min(frame_h);
    let x1 = ((rect.max.x * pixels_per_point).ceil().max(0.0) as usize).min(frame_w);
    let y1 = ((rect.max.y * pixels_per_point).ceil().max(0.0) as usize).min(frame_h);
    if x1 <= x0 || y1 <= y0 {
        return Err(format!(
            "pane rect {rect:?} is outside the captured frame ({frame_w}x{frame_h} px)"
        ));
    }
    let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0..y1 {
        pixels.extend_from_slice(&image.pixels[y * frame_w + x0..y * frame_w + x1]);
    }
    Ok(egui::ColorImage::new([x1 - x0, y1 - y0], pixels))
}

#[cfg(test)]
mod tests {
    use super::{crop_color_image, screenshot_timeout_error};

    /// Real GPU readback without any window/surface. Odd width exercises row
    /// padding; successive colors ensure a reused capture returns fresh pixels.
    #[test]
    fn screenshot_gpu_readback_without_compositor_drawable() {
        let config = egui_wgpu::WgpuConfiguration::default();
        let instance = config.wgpu_setup.new_instance();
        let instance = pollster::block_on(instance);
        let state = pollster::block_on(egui_wgpu::RenderState::create(
            &config,
            &instance,
            None,
            egui_wgpu::RendererOptions::default(),
        ))
        .expect("headless GPU");
        let template = state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot regression"),
            size: wgpu::Extent3d {
                width: 73,
                height: 41,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: state.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let mut capture = egui_wgpu::capture::CaptureState::new(&state.device, &template);
        let (tx, rx) = egui_wgpu::capture::capture_channel();
        let timeout = crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
        for (color, expected) in [
            (wgpu::Color::RED, egui::Color32::RED),
            (wgpu::Color::BLUE, egui::Color32::BLUE),
        ] {
            let mut encoder = state.device.create_command_encoder(&Default::default());
            let view = capture.texture.create_view(&Default::default());
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(color),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                });
            }
            let buffer = capture.copy_textures(&state.device, None, &mut encoder);
            state.queue.submit([encoder.finish()]);
            capture.read_screen_rgba(
                egui::Context::default(),
                buffer,
                vec![],
                tx.clone(),
                egui::ViewportId::ROOT,
            );
            state
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(timeout),
                })
                .expect("GPU readback");
            let (_, _, image) = rx.recv_timeout(timeout).expect("capture reply");
            assert_eq!(image.size, [73, 41]);
            assert!(image.pixels.iter().all(|pixel| *pixel == expected));
        }
    }

    #[test]
    fn timeout_reports_observed_viewport_and_ui_frames_without_assuming_cause() {
        let error = screenshot_timeout_error(Some(false), Some(true), Some(false), 0);
        assert!(error.contains("minimized=false"));
        assert!(error.contains("occluded=true"));
        assert!(error.contains("focused=false"));
        assert!(error.contains("UI frames during request=0"));

        let unknown = screenshot_timeout_error(None, None, None, 12);
        assert!(unknown.contains("minimized=unknown"));
        assert!(unknown.contains("occluded=unknown"));
        assert!(unknown.contains("focused=unknown"));
        assert!(unknown.contains("UI frames during request=12"));
    }

    #[test]
    fn crop_scales_by_pixels_per_point_and_clamps_to_frame() {
        let mut image = egui::ColorImage::filled([100, 80], egui::Color32::BLACK);
        // Mark a known pixel inside the crop region: logical (10,5) @2x = (20,10).
        image.pixels[10 * 100 + 20] = egui::Color32::WHITE;
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 5.0), egui::pos2(30.0, 25.0));
        let cropped = crop_color_image(&image, Some(rect), 2.0).expect("valid crop");
        assert_eq!(cropped.size, [40, 40]);
        assert_eq!(cropped.pixels[0], egui::Color32::WHITE);

        // A rect hanging past the frame edge clamps instead of failing.
        let edge = egui::Rect::from_min_max(egui::pos2(45.0, 35.0), egui::pos2(60.0, 50.0));
        let clamped = crop_color_image(&image, Some(edge), 2.0).expect("clamped crop");
        assert_eq!(clamped.size, [10, 10]);

        // Fully outside the frame is a loud error.
        let outside = egui::Rect::from_min_max(egui::pos2(60.0, 45.0), egui::pos2(70.0, 50.0));
        assert!(crop_color_image(&image, Some(outside), 2.0).is_err());

        // No region = full frame.
        let full = crop_color_image(&image, None, 2.0).expect("full frame");
        assert_eq!(full.size, [100, 80]);
    }
}
