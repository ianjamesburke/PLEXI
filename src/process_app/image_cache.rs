//! Image loading cache for `RenderCommand::Image` (#1144, #1354).
//!
//! Supports two loading modes:
//! - **Src-keyed** (`request` / `request_url`): fire-and-forget, keyed by raw path/URL.
//! - **Handle-keyed** (`request_by_handle`): async lifecycle keyed by opaque UUID.
//!   When a handle load completes, `poll()` returns the completed handles so the
//!   caller can emit `PlexiEvent::ImageLoaded` to the app.

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
    /// Keys in `cache` that are opaque handle UUIDs (from `request_by_handle`).
    /// When these complete, `poll()` includes them in its return value so the
    /// caller can emit `PlexiEvent::ImageLoaded`. Keys are removed on completion
    /// to prevent unbounded growth.
    handle_keys: HashSet<String>,
    /// Immediate completions for handle requests that failed synchronously
    /// (e.g. capability denied). Drained by `poll()` alongside channel completions.
    immediate_completions: Vec<(String, Result<(), String>)>,
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
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

impl ImageCache {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            cache: HashMap::new(),
            tx,
            rx,
            warned: HashSet::new(),
            handle_keys: HashSet::new(),
            immediate_completions: Vec::new(),
        }
    }

    /// Request an async image fetch keyed by an opaque handle UUID.
    /// No-op if the handle is already in the cache.
    ///
    /// Requires `net_http_granted`; otherwise records an immediate error
    /// completion so the caller can emit `PlexiEvent::ImageLoaded` with
    /// `status = "error"` without waiting for the background thread.
    ///
    /// On success, the handle resolves via the same channel as src-keyed
    /// requests and appears in `poll()`'s return value.
    pub(crate) fn request_by_handle(&mut self, handle: &str, src: &str, net_http_granted: bool) {
        if self.cache.contains_key(handle) {
            return;
        }
        if !net_http_granted {
            log::warn!("ImageCache: load_image handle={handle} denied — net.http capability required");
            self.cache.insert(
                handle.to_string(),
                CachedImage::Error("net.http capability required".to_string()),
            );
            self.immediate_completions.push((
                handle.to_string(),
                Err("capability_denied: net.http not granted".to_string()),
            ));
            return;
        }
        self.handle_keys.insert(handle.to_string());
        self.cache.insert(handle.to_string(), CachedImage::Loading);
        log::info!("ImageCache: load_image handle={handle} src={src}");
        let handle_key = handle.to_string();
        let src_str = src.to_string();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = fetch_url_image(&src_str);
            let _ = tx.send((handle_key, result));
        });
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

    /// Drain the load channel. Call once per frame before rendering.
    ///
    /// Returns completed handle loads as `(handle, Ok(()) | Err(message))`.
    /// The caller should push `PlexiEvent::ImageLoaded` for each.
    pub(crate) fn poll(&mut self, egui_ctx: &egui::Context) -> Vec<(String, Result<(), String>)> {
        let mut completed: Vec<(String, Result<(), String>)> =
            std::mem::take(&mut self.immediate_completions);

        while let Ok((key, result)) = self.rx.try_recv() {
            // `remove` cleans up the set entry, preventing unbounded growth.
            let is_handle = self.handle_keys.remove(&key);
            match result {
                Ok(color_image) => {
                    let handle = egui_ctx.load_texture(
                        &key,
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    if is_handle {
                        log::info!("ImageCache: handle={key} loaded");
                        completed.push((key.clone(), Ok(())));
                    } else {
                        log::info!("ImageCache: loaded '{key}'");
                    }
                    self.cache.insert(key, CachedImage::Loaded(handle));
                    egui_ctx.request_repaint();
                }
                Err(e) => {
                    if is_handle {
                        log::warn!("ImageCache: handle={key} failed: {e}");
                        completed.push((key.clone(), Err(e.clone())));
                    } else if !self.warned.contains(&key) {
                        log::warn!("ImageCache: failed to load '{key}': {e}");
                        self.warned.insert(key.clone());
                    }
                    self.cache.insert(key, CachedImage::Error(e));
                    egui_ctx.request_repaint();
                }
            }
        }
        completed
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
