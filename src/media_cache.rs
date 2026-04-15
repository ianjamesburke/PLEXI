/// Shared media cache for image textures and ffmpeg-generated video thumbnails.
///
/// Owned by `PlexiApp` and borrowed through `AppRenderContext`, so all apps in
/// a session reuse the same decoded textures and thumbnail files. All rendering
/// of the `Image`, `VideoThumbnail`, and `FileGrid` draw commands is funnelled
/// through this type — apps never touch ffmpeg, the `image` crate, or GPU state.
///
/// # Invalidation
///
/// Image textures are keyed by absolute path. On every lookup the stored
/// `SystemTime` is compared against the file's current `mtime`; if it differs
/// the cached texture is dropped and re-decoded.
///
/// Video thumbnails are keyed by `(path, mtime_nanos, timestamp_seconds)` and
/// stored on disk under `~/.cache/plexi/thumbnails/<hash>.jpg`. Extraction is
/// asynchronous: a request returns `ThumbStatus::Pending` until a worker thread
/// writes the jpg; subsequent requests then hit the texture cache.
///
/// The cache is single-threaded from the render loop's perspective — it lives
/// in a `RefCell<MediaCache>` on `PlexiApp` and is borrowed mutably during
/// `ui()`. Only the ffmpeg extraction runs on background threads; completion
/// is reported via an `mpsc::Receiver`.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::SystemTime;

use image::imageops::FilterType;

/// Maximum image dimension we'll upload to the GPU. Images larger than this on
/// their longest side are downscaled before texture upload.
const MAX_TEXTURE_DIMENSION: u32 = 2048;

/// Result of trying to fetch an image texture from the cache.
pub enum ImageStatus {
    /// Texture is ready to be painted.
    Ready(egui::TextureHandle),
    /// File missing or decode failed.
    Error(String),
}

/// Result of trying to fetch a video thumbnail.
pub enum ThumbStatus {
    /// Thumbnail is ready to be painted.
    Ready(egui::TextureHandle),
    /// ffmpeg is still running (or was just kicked off).
    Pending,
    /// ffmpeg failed, video is missing, or thumbnail decode failed.
    Error(String),
}

/// A completed ffmpeg extraction reported by the worker pool.
struct ThumbResult {
    key: String,
    /// Ok = on-disk path to the extracted jpg. Err = failure message.
    result: Result<PathBuf, String>,
}

pub struct MediaCache {
    /// Texture cache keyed by absolute path. The stored SystemTime is the file
    /// mtime at load time; a mismatch on lookup forces a reload.
    textures: HashMap<PathBuf, (egui::TextureHandle, SystemTime)>,

    /// Map from thumbnail cache key (see `thumb_cache_key`) to the on-disk
    /// thumbnail path. Lets us reuse already-extracted thumbnails on restart.
    thumb_paths: HashMap<String, PathBuf>,

    /// Thumbnail errors. Once a video fails to produce a thumbnail we don't
    /// keep retrying every frame — the error sticks until the cache is cleared.
    thumb_errors: HashMap<String, String>,

    /// Cache keys for which an ffmpeg extraction is currently in flight. Used
    /// to de-dupe identical requests that arrive on consecutive frames.
    thumb_in_flight: HashSet<String>,

    /// Channel: worker threads send `ThumbResult` here when ffmpeg finishes.
    thumb_tx: Sender<ThumbResult>,
    thumb_rx: Receiver<ThumbResult>,

    /// Root directory for on-disk thumbnails, e.g. `~/.cache/plexi/thumbnails`.
    thumb_dir: PathBuf,
}

impl MediaCache {
    pub fn new() -> Self {
        let (thumb_tx, thumb_rx) = mpsc::channel();
        let thumb_dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("plexi")
            .join("thumbnails");
        if let Err(e) = std::fs::create_dir_all(&thumb_dir) {
            log::warn!(
                "MediaCache: failed to create thumbnail dir {:?}: {e}",
                thumb_dir
            );
        }
        Self {
            textures: HashMap::new(),
            thumb_paths: HashMap::new(),
            thumb_errors: HashMap::new(),
            thumb_in_flight: HashSet::new(),
            thumb_tx,
            thumb_rx,
            thumb_dir,
        }
    }

    /// Drain any completed ffmpeg extractions. Should be called once per frame
    /// before any image/video draw commands are processed.
    pub fn poll_completions(&mut self) -> bool {
        let mut any = false;
        loop {
            match self.thumb_rx.try_recv() {
                Ok(ThumbResult { key, result }) => {
                    self.thumb_in_flight.remove(&key);
                    match result {
                        Ok(path) => {
                            self.thumb_paths.insert(key, path);
                        }
                        Err(msg) => {
                            log::warn!("MediaCache: thumbnail failed for {key}: {msg}");
                            self.thumb_errors.insert(key, msg);
                        }
                    }
                    any = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        any
    }

    /// Get (or load) a texture for an image at `path`.
    ///
    /// The render loop calls this synchronously — image decode blocks the
    /// frame but is cached from then on. `ctx` is the egui context used to
    /// upload the texture.
    pub fn get_image(&mut self, ctx: &egui::Context, path: &Path) -> ImageStatus {
        let abs_path = match std::fs::canonicalize(path) {
            Ok(p) => p,
            Err(_) => {
                return ImageStatus::Error(format!("file not found: {}", path.display()));
            }
        };

        let current_mtime = match std::fs::metadata(&abs_path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(e) => {
                return ImageStatus::Error(format!("stat failed: {e}"));
            }
        };

        // Fast path — cached and fresh.
        if let Some((tex, cached_mtime)) = self.textures.get(&abs_path) {
            if *cached_mtime == current_mtime {
                return ImageStatus::Ready(tex.clone());
            }
        }

        // Cache miss or stale — decode from disk.
        match load_image_from_path(&abs_path) {
            Ok(color_image) => {
                let name = format!("plexi-media:{}", abs_path.display());
                let texture = ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR);
                self.textures
                    .insert(abs_path, (texture.clone(), current_mtime));
                ImageStatus::Ready(texture)
            }
            Err(e) => {
                // Drop any stale texture so we don't keep serving a corrupt one.
                self.textures.remove(&abs_path);
                ImageStatus::Error(format!("decode failed: {e}"))
            }
        }
    }

    /// Get (or request) a thumbnail for a video at `path`.
    ///
    /// On a cache miss this spawns an ffmpeg extraction on a worker thread
    /// and returns `Pending`. The caller should request a repaint so the next
    /// frame picks up the completed jpg.
    pub fn get_video_thumbnail(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        timestamp_seconds: f32,
    ) -> ThumbStatus {
        let abs_path = match std::fs::canonicalize(path) {
            Ok(p) => p,
            Err(_) => {
                return ThumbStatus::Error(format!("file not found: {}", path.display()));
            }
        };

        let mtime = match std::fs::metadata(&abs_path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(e) => {
                return ThumbStatus::Error(format!("stat failed: {e}"));
            }
        };

        let key = thumb_cache_key(&abs_path, mtime, timestamp_seconds);

        if let Some(msg) = self.thumb_errors.get(&key) {
            return ThumbStatus::Error(msg.clone());
        }

        // Check memory-resident jpg path map first.
        let cached_path = self
            .thumb_paths
            .get(&key)
            .cloned()
            .or_else(|| {
                // Fall back to the on-disk location — might exist from a previous run.
                let p = self.thumb_dir.join(format!("{key}.jpg"));
                if p.is_file() {
                    self.thumb_paths.insert(key.clone(), p.clone());
                    Some(p)
                } else {
                    None
                }
            });

        if let Some(thumb_path) = cached_path {
            return match self.get_image(ctx, &thumb_path) {
                ImageStatus::Ready(tex) => ThumbStatus::Ready(tex),
                ImageStatus::Error(e) => ThumbStatus::Error(e),
            };
        }

        // Nothing on disk — kick off an extraction if one isn't already running.
        if self.thumb_in_flight.insert(key.clone()) {
            let tx = self.thumb_tx.clone();
            let src = abs_path.clone();
            let dst = self.thumb_dir.join(format!("{key}.jpg"));
            let ts = timestamp_seconds;
            let key_for_thread = key.clone();
            let egui_ctx = ctx.clone();
            thread::spawn(move || {
                let result = extract_video_frame(&src, ts, &dst);
                let _ = tx.send(ThumbResult {
                    key: key_for_thread,
                    result,
                });
                // Request a repaint so the completed thumbnail is picked up on
                // the next frame without the user needing to move the mouse.
                egui_ctx.request_repaint();
            });
        }

        ThumbStatus::Pending
    }
}

/// Decode an image file into an `egui::ColorImage`, downscaling if it's larger
/// than `MAX_TEXTURE_DIMENSION` on the longest side. Used for both `Image`
/// commands and thumbnail jpgs.
fn load_image_from_path(path: &Path) -> Result<egui::ColorImage, String> {
    let decoded = image::open(path).map_err(|e| e.to_string())?;
    let decoded = if decoded.width().max(decoded.height()) > MAX_TEXTURE_DIMENSION {
        decoded.resize(
            MAX_TEXTURE_DIMENSION,
            MAX_TEXTURE_DIMENSION,
            FilterType::Triangle,
        )
    } else {
        decoded
    };
    let rgba = decoded.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// Build a filename-safe cache key for a video thumbnail.
fn thumb_cache_key(path: &Path, mtime: SystemTime, timestamp_seconds: f32) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    if let Ok(d) = mtime.duration_since(SystemTime::UNIX_EPOCH) {
        d.as_nanos().hash(&mut hasher);
    }
    // f32 isn't Hash — hash the bits so two identical seconds compare equal.
    timestamp_seconds.to_bits().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Shell out to ffmpeg to extract a single frame at `timestamp_seconds` from
/// `src` and write it to `dst`. Returns the final `dst` path on success.
///
/// We put `-ss` BEFORE `-i` for fast seeking. ffmpeg is required; if it's not
/// installed the error is surfaced back through `ThumbStatus::Error`.
fn extract_video_frame(
    src: &Path,
    timestamp_seconds: f32,
    dst: &Path,
) -> Result<PathBuf, String> {
    if let Some(parent) = dst.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(format!("create thumb dir: {e}"));
        }
    }
    let output = Command::new("ffmpeg")
        .arg("-nostdin")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(format!("{timestamp_seconds}"))
        .arg("-i")
        .arg(src)
        .arg("-frames:v")
        .arg("1")
        .arg("-q:v")
        .arg("3")
        .arg("-y")
        .arg(dst)
        .output()
        .map_err(|e| format!("failed to run ffmpeg: {e} (is ffmpeg installed?)"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!("ffmpeg exited {}: {stderr}", output.status));
    }
    if !dst.is_file() {
        return Err("ffmpeg produced no output file".to_string());
    }
    Ok(dst.to_path_buf())
}
