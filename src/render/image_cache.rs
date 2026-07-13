//! Image loading cache for `RenderCommand::Image` (#1144, #1354).
//!
//! Loads are keyed by their raw path or URL.

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

/// Fetch a remote URL and decode it as an image. Enforces a 10 MB cap to
/// prevent OOM from malicious or oversized images.
fn fetch_url_image(url: &str) -> Result<egui::ColorImage, String> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes: Vec<u8> = Vec::new();
    // 10 MB cap — prevents OOM from malicious or oversized images.
    resp.into_reader()
        .take(10 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
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

    /// Request a remote URL image fetch keyed by raw src string. No-op if already loading/loaded/errored.
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
            let result = fetch_url_image(&src_key);
            let _ = tx.send((src_key, result));
        });
    }

    /// Request an image load keyed by raw src path. No-op if already loading/loaded/errored.
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
            self.cache.insert(
                src.to_string(),
                CachedImage::Error("Access denied".to_string()),
            );
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

    /// Drain the load channel. Call once per frame before rendering.
    ///
    pub(crate) fn poll(&mut self, egui_ctx: &egui::Context) {
        while let Ok((key, result)) = self.rx.try_recv() {
            match result {
                Ok(color_image) => {
                    let handle =
                        egui_ctx.load_texture(&key, color_image, egui::TextureOptions::LINEAR);
                    log::info!("ImageCache: loaded '{key}'");
                    self.cache.insert(key, CachedImage::Loaded(handle));
                    crate::platform::frame_diag::note(
                        crate::platform::frame_diag::RepaintCause::ImageCacheCompletion,
                    );
                    egui_ctx.request_repaint();
                }
                Err(e) => {
                    if !self.warned.contains(&key) {
                        log::warn!("ImageCache: failed to load '{key}': {e}");
                        self.warned.insert(key.clone());
                    }
                    self.cache.insert(key, CachedImage::Error(e));
                    crate::platform::frame_diag::note(
                        crate::platform::frame_diag::RepaintCause::ImageCacheCompletion,
                    );
                    egui_ctx.request_repaint();
                }
            }
        }
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.cache
            .values()
            .any(|entry| matches!(entry, CachedImage::Loading))
    }

    /// Returns the texture handle if the image is loaded; None otherwise.
    pub(crate) fn get(&self, src: &str) -> Option<&egui::TextureHandle> {
        match self.cache.get(src) {
            Some(CachedImage::Loaded(h)) => Some(h),
            _ => None,
        }
    }

    /// Returns the cache state for a key.
    pub(crate) fn state(&self, src: &str) -> Option<&CachedImage> {
        self.cache.get(src)
    }
}
