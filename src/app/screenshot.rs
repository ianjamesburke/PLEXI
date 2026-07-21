//! Live-host screenshot capture (`plexi host screenshot`, stint 0461).
//!
//! The CLI sends `AppRequest::Screenshot`; the handler queues a
//! [`PendingScreenshot`] and asks egui for a viewport capture
//! (`ViewportCommand::Screenshot`). The wgpu backend reads the real
//! swapchain back and delivers it as `egui::Event::Screenshot` in a later
//! frame's raw input, where [`PlexiApp::fulfill_pending_screenshots`] crops,
//! encodes, and writes the PNG plus the CLI response file. This captures the
//! pixels the user actually sees — chrome, terminals, apps, overlays — with
//! no OS-level screen capture involved.

use super::PlexiApp;

#[derive(Debug)]
pub struct PendingScreenshot {
    /// Crop to this pane's screen rect; `None` captures the whole window.
    pub pane_id: Option<u64>,
    pub output_path: String,
    pub response_file: String,
}

impl PlexiApp {
    /// Drain any `Event::Screenshot` deliveries from this frame's raw input
    /// and fulfill every queued request with the captured image.
    pub(crate) fn fulfill_pending_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_screenshots.is_empty() {
            return;
        }
        // wgpu 29 no longer advances map_async callbacks without an explicit
        // device poll. egui-wgpu queues the readback after the frame render,
        // so poll non-blockingly on subsequent passes until its Screenshot
        // event reaches egui's raw input.
        if let Some(render_state) = crate::host::wasm_gpu::host_render_state() {
            if let Err(error) = render_state.device.poll(wgpu::PollType::Poll) {
                log::warn!("screenshot: wgpu device poll failed: {error}");
            }
        }
        let images: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .collect()
        });
        let Some(image) = images.last() else {
            ctx.request_repaint_after(std::time::Duration::from_millis(10));
            return;
        };
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
            if let Err(error) = std::fs::write(&request.response_file, response.to_string()) {
                log::warn!(
                    "screenshot: could not write response file {}: {error}",
                    request.response_file
                );
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
