use std::path::PathBuf;

use crate::app::app_trait::{App, AppRenderContext};

pub struct ImageViewerApp {
    path: PathBuf,
    texture: Option<egui::TextureHandle>,
    image_size: Option<[usize; 2]>,
    error: Option<String>,
    zoom: f32,
    fit_to_view: bool,
}

impl ImageViewerApp {
    pub fn new(path: PathBuf) -> Self {
        log::info!(
            "ImageViewerApp: launch app_id=image-viewer path={}",
            path.display()
        );
        Self {
            path,
            texture: None,
            image_size: None,
            error: None,
            zoom: 1.0,
            fit_to_view: true,
        }
    }

    fn filename(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Image")
            .to_string()
    }

    fn load_texture(&mut self, ctx: &egui::Context) {
        if self.texture.is_some() || self.error.is_some() {
            return;
        }
        match image::open(&self.path) {
            Ok(decoded) => {
                let rgba = decoded.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.texture = Some(ctx.load_texture(
                    format!("image-viewer:{}", self.path.display()),
                    color,
                    egui::TextureOptions::LINEAR,
                ));
                self.image_size = Some(size);
                log::info!(
                    "ImageViewerApp: decoded path={} size={}x{}",
                    self.path.display(),
                    size[0],
                    size[1]
                );
            }
            Err(err) => {
                let msg = format!("Unable to load image: {err}");
                log::warn!("ImageViewerApp: {} path={}", msg, self.path.display());
                self.error = Some(msg);
            }
        }
    }
}

impl App for ImageViewerApp {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn type_id(&self) -> &'static str {
        "image-viewer"
    }

    fn display_name(&self) -> String {
        format!("Image - {}", self.filename())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _ctx: &AppRenderContext<'_>) {
        self.load_texture(ui.ctx());

        ui.horizontal(|ui| {
            if ui.selectable_label(self.fit_to_view, "Fit").clicked() {
                self.fit_to_view = true;
            }
            if ui.button("100%").clicked() {
                self.fit_to_view = false;
                self.zoom = 1.0;
            }
            let zoom_response = ui.add(
                egui::Slider::new(&mut self.zoom, 0.1..=8.0)
                    .logarithmic(true)
                    .text("Zoom"),
            );
            if zoom_response.changed() {
                self.fit_to_view = false;
            }
        });

        ui.separator();
        if let Some(error) = &self.error {
            ui.label(error);
            return;
        }
        let Some(texture) = &self.texture else {
            ui.label("Loading image...");
            return;
        };
        let tex_size = texture.size_vec2();
        let available = ui.available_size().max(egui::vec2(1.0, 1.0));
        let scale = if self.fit_to_view {
            (available.x / tex_size.x)
                .min(available.y / tex_size.y)
                .min(1.0)
        } else {
            self.zoom
        };
        let display_size = tex_size * scale.max(0.1);
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let response = ui.add(egui::Image::new((texture.id(), display_size)));
                if response.hovered() {
                    let zoom_delta = ui.input(|i| i.zoom_delta());
                    if (zoom_delta - 1.0).abs() > f32::EPSILON {
                        self.fit_to_view = false;
                        self.zoom = (self.zoom * zoom_delta).clamp(0.1, 8.0);
                    }
                }
            });
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "path": self.path.display().to_string(),
            "zoom": self.zoom,
            "fit_to_view": self.fit_to_view,
        }))
    }
}
