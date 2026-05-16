//! Image loading cache for `RenderCommand::Image` (#1144).
//!
//! Images are loaded on a background thread and stored as egui `TextureHandle`s.
//! Call `request()` to trigger a load (no-op if already in-flight or done),
//! `poll()` once per frame to drain completed loads into the cache, and `get()`
//! to retrieve a loaded handle for rendering.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

pub(crate) enum CachedImage {
    Loading,
    Loaded(egui::TextureHandle),
    Error(String),
}

pub(crate) struct ImageCache {
    cache: HashMap<String, CachedImage>,
    tx: Sender<(String, Result<egui::ColorImage, String>)>,
    rx: Receiver<(String, Result<egui::ColorImage, String>)>,
    warned: HashSet<String>,
}

impl ImageCache {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            cache: HashMap::new(),
            tx,
            rx,
            warned: HashSet::new(),
        }
    }

    /// Request a remote URL image fetch. No-op if already loading/loaded/errored.
    ///
    /// Requires `net_http_granted`; otherwise inserts an error placeholder
    /// immediately without spawning a thread.
    pub(crate) fn request_url(&mut self, src: &str, net_http_granted: bool) {
        if self.cache.contains_key(src) {
            return;
        }
        if !net_http_granted {
            log::warn!("ImageCache: net.http capability required for remote image '{src}'");
            self.cache.insert(
                src.to_string(),
                CachedImage::Error("net.http capability required for remote images".to_string()),
            );
            return;
        }
        self.cache.insert(src.to_string(), CachedImage::Loading);
        log::info!("ImageCache: fetching remote URL '{src}'");
        let src_key = src.to_string();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let resp = ureq::get(&src_key)
                    .call()
                    .map_err(|e| e.to_string())?;
                let mut bytes: Vec<u8> = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                Ok(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
            })();
            let _ = tx.send((src_key, result));
        });
    }

    /// Request an image load. No-op if already loading/loaded/errored.
    ///
    /// Only relative paths (resolved under `ctx_dir`) are allowed. Absolute
    /// paths and any `src` containing `..` are rejected to prevent path
    /// traversal outside the app's workspace.
    pub(crate) fn request(&mut self, src: &str, ctx_dir: &Path) {
        if self.cache.contains_key(src) {
            return;
        }
        if src.contains("..") || Path::new(src).is_absolute() {
            log::warn!("ImageCache: blocked disallowed image path '{src}'");
            self.cache
                .insert(src.to_string(), CachedImage::Error("Access denied".to_string()));
            return;
        }
        let path: PathBuf = ctx_dir.join(src);
        self.cache.insert(src.to_string(), CachedImage::Loading);
        log::info!("ImageCache: requesting image '{src}'");
        let src_key = src.to_string();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = image::open(&path)
                .map(|img| {
                    let rgba = img.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
                })
                .map_err(|e| e.to_string());
            let _ = tx.send((src_key, result));
        });
    }

    /// Drain the load channel, converting ready images into TextureHandles.
    /// Call once per frame before rendering.
    pub(crate) fn poll(&mut self, egui_ctx: &egui::Context) {
        while let Ok((src, result)) = self.rx.try_recv() {
            match result {
                Ok(color_image) => {
                    let handle = egui_ctx.load_texture(
                        &src,
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    log::info!("ImageCache: loaded '{src}'");
                    self.cache.insert(src, CachedImage::Loaded(handle));
                    egui_ctx.request_repaint();
                }
                Err(e) => {
                    if !self.warned.contains(&src) {
                        log::warn!("ImageCache: failed to load '{src}': {e}");
                        self.warned.insert(src.clone());
                    }
                    self.cache.insert(src, CachedImage::Error(e));
                    egui_ctx.request_repaint();
                }
            }
        }
    }

    /// Returns the texture handle if the image is loaded; None otherwise.
    pub(crate) fn get(&self, src: &str) -> Option<&egui::TextureHandle> {
        match self.cache.get(src) {
            Some(CachedImage::Loaded(h)) => Some(h),
            _ => None,
        }
    }

    /// Returns the cache state for a src key.
    pub(crate) fn state(&self, src: &str) -> Option<&CachedImage> {
        self.cache.get(src)
    }
}
