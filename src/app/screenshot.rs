//! Live-host screenshot capture (`plexi host screenshot`, stint 0461).
//!
//! The CLI sends `AppRequest::Screenshot`; the handler queues a
//! [`PendingScreenshot`] and asks egui for a viewport capture
//! (`ViewportCommand::Screenshot`). The wgpu backend reads the real
//! swapchain back and delivers it as `egui::Event::Screenshot` in a later
//! frame's raw input, where [`PlexiApp::fulfill_screenshot_events`] crops,
//! encodes, and writes the PNG plus the CLI response file. This captures the
//! pixels the user actually sees — chrome, terminals, apps, overlays — with
//! no OS-level screen capture involved.
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

/// How often an undelivered capture is re-requested from the viewport.
const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Debug)]
pub struct PendingScreenshot {
    /// Crop to this pane's screen rect; `None` captures the whole window.
    pub pane_id: Option<u64>,
    pub output_path: String,
    pub response_file: String,
    pub capture_requested_at: Option<std::time::Instant>,
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
        self.expire_pending_screenshots();
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
    fn expire_pending_screenshots(&mut self) {
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
        for request in expired {
            let error = format!(
                "screenshot readback did not complete within {}s: the host window produced no \
                 frame (display asleep, window minimized or fully occluded, or GPU readback \
                 stalled)",
                SCREENSHOT_DEADLINE.as_secs()
            );
            log::warn!(
                "screenshot: pane_id={:?} output={} timed out: {error}",
                request.pane_id,
                request.output_path
            );
            crate::rpc::write_json_response(
                &request.response_file,
                serde_json::json!({ "error": error }),
            );
        }
    }

    /// Ask the viewport for a capture, at most once per [`RETRY_INTERVAL`].
    fn request_pending_screenshot(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        let should_capture = self.pending_screenshots.iter().any(|request| {
            request
                .capture_requested_at
                .is_none_or(|requested_at| now.duration_since(requested_at) >= RETRY_INTERVAL)
        });
        if !should_capture {
            return;
        }
        for request in &mut self.pending_screenshots {
            request.capture_requested_at = Some(now);
        }
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::Screenshot(egui::UserData::default()),
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
        let images: Vec<std::sync::Arc<egui::ColorImage>> = raw_input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
            .collect();
        let Some(image) = images.last() else {
            return;
        };
        if self.pending_screenshots.is_empty() {
            return;
        }
        let pixels_per_point = ctx.pixels_per_point();
        for request in std::mem::take(&mut self.pending_screenshots) {
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

    fn write_screenshot(
        &mut self,
        request: &PendingScreenshot,
        image: &egui::ColorImage,
        pixels_per_point: f32,
    ) -> Result<(u32, u32), String> {
        let region = match request.pane_id {
            None => None,
            Some(pane_id) => {
                let Some((win_idx, tile_id)) = self.find_pane_in_any_window(pane_id) else {
                    return Err(format!("pane {pane_id} not found"));
                };
                let Some(rect) = self.windows[win_idx].tree.tiles.rect(tile_id) else {
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
    use super::crop_color_image;

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
