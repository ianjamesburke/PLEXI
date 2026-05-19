//! Video subsystem (#345 substrate, #346 AVFoundation backing).
//!
//! Three layers, mirroring [`crate::audio`] and [`crate::midi`]:
//!
//!   1. [`VideoDecoder`] — host-facing trait. `open()` returns a
//!      [`VideoOpenAck`] carrying the negotiated `width`/`height`/`fps`/
//!      `duration_ms` plus an opaque `handle_id`. Subsequent operations
//!      (`set_state`, `close`) take that `handle_id`. The trait lives behind
//!      `Arc<dyn VideoDecoder>` on each `ProcessApp` instance.
//!
//!   2. [`AvfVideoDecoder`] — production decoder. macOS: AVFoundation
//!      pipeline (`AVURLAsset` → `AVAssetReader` → `AVAssetReaderTrackOutput`
//!      configured for 32BGRA → worker thread that pushes RGBA frames into
//!      the binary pipe ring at native frame rate). Non-macOS: returns
//!      `Err(NotImplemented)` cleanly. The factory selects this impl unless
//!      `PLEXI_VIDEO=mock://...` is set.
//!
//!   3. [`MockVideoDecoder`] — procedural RGBA gradient at configurable fps.
//!      Frames are pushed via the binary-pipe ring (one frame per packet).
//!      Used by tests and by the POC `examples/video-player/` app driven
//!      with `PLEXI_VIDEO=mock://...`.
//!
//! Wire shape: apps `OpenVideo { request_id, source, pipe_id }` and the host
//! replies with `VideoOpenAck { request_id, handle_id, width, height, fps,
//! duration_ms }` on success or `VideoOpenError { request_id, error }` on
//! failure. Frames travel on the binary pipe as raw RGBA8 packed
//! `[R,G,B,A,R,G,B,A,...]` of length `width * height * 4`. One pipe frame =
//! one video frame; consumers read whole frames using the existing
//! `Pipe.read_frame()` length-prefix protocol.
//!
//! Out of scope (deferred):
//!   - PTS-accurate scheduling — frame timing is "sleep `1.0/fps` between
//!     pushes". Drift accumulates over long clips.
//!   - A/V sync.
//!   - Codec coverage beyond AVFoundation native (H.264, HEVC, ProRes, etc.
//!     are all native).
//!   - Streaming sources (HTTP, RTSP).
//!   - Subtitles / closed-captions.
//!   - Hardware-acceleration tuning.

use std::sync::Arc;

use crossbeam_queue::ArrayQueue;

// ─── Public types ────────────────────────────────────────────────────────────

/// Playback state for a video handle. Encoded on the wire as
/// `{"play": null}` / `{"pause": null}` / `{"seek": <ms>}` via serde's
/// default `untagged`-friendly encoding. The PGAP wire serialises this as a
/// nested struct under `state` in `DrawCommand::SetVideoState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoState {
    Play,
    Pause,
    /// Absolute position in milliseconds from the start of the video.
    Seek { position_ms: u64 },
}

/// Info for one camera device returned by `VideoDecoder::list_camera_devices`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDeviceInfo {
    pub id: String,
    pub name: String,
}

/// Result of `VideoDecoder::open` on success. Reported back to the app as
/// `PlexiEvent::VideoOpenAck`.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoOpenAck {
    pub handle_id: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    /// Returned by [`AvfVideoDecoder`] on non-macOS targets where AVFoundation
    /// is unavailable. macOS builds (post-#346) never produce this variant —
    /// they surface concrete `Decoder(...)` / `InvalidSource(...)` errors
    /// instead. Gated behind `cfg(not(target_os = "macos"))` so dead-code
    /// analysis on macOS doesn't flag it; cross-platform consumers (router,
    /// tests) handle it under the same gate.
    #[cfg(not(target_os = "macos"))]
    #[error("video decoder not implemented on this platform")]
    NotImplemented,
    #[error("invalid video source url: {0}")]
    InvalidSource(String),
    /// Wraps any underlying decoder failure.
    #[error("decoder: {0}")]
    Decoder(String),
}

/// Opaque video session. Drop tears down the worker thread (mock) or the
/// AVFoundation player (#346). Owned by `ProcessApp::video_handles`.
pub struct VideoHandle {
    pub handle_id: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub duration_ms: u64,
    /// Drop-in cleanup. Mock spawns a worker thread; production stub is empty.
    _guard: Box<dyn VideoHandleGuard>,
    /// Per-handle inner control surface for `set_state`. Trait-object so the
    /// production stub (which never opens anything) and the mock (which
    /// drives a worker thread) can coexist behind one `VideoHandle` shape.
    inner: Box<dyn VideoHandleInner>,
}

impl std::fmt::Debug for VideoHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoHandle")
            .field("handle_id", &self.handle_id)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("fps", &self.fps)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

impl VideoHandle {
    pub fn set_state(&mut self, state: VideoState) -> Result<(), VideoError> {
        self.inner.set_state(state)
    }
}

trait VideoHandleGuard: Send {}
trait VideoHandleInner: Send {
    fn set_state(&mut self, state: VideoState) -> Result<(), VideoError>;
}

// ─── Device trait ────────────────────────────────────────────────────────────

pub trait VideoDecoder: Send + Sync {
    /// Open `source` and return its negotiated dimensions / fps / duration
    /// plus an opaque handle id. The handle is returned via the `VideoHandle`
    /// the caller must store; the returned `VideoOpenAck` is what we send to
    /// the app on the wire.
    ///
    /// `frame_ring` is where decoded RGBA frames are pushed. Each push is
    /// `width * height * 4` bytes, packed RGBA8. The mock spawns a worker
    /// thread that pumps frames into the ring at the configured fps; the
    /// production stub returns `Err(NotImplemented)` before touching the ring.
    fn open(
        &self,
        source: &str,
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) -> Result<(VideoOpenAck, VideoHandle), VideoError>;

    /// Enumerate available camera devices. Returns an empty vec on non-macOS
    /// or when no cameras are attached. Capability checking is enforced by
    /// the routing layer, not here.
    fn list_camera_devices(&self) -> Vec<CameraDeviceInfo>;
}

// ─── Production impl: AvfVideoDecoder ────────────────────────────────────────

/// Production video decoder backed by AVFoundation (#346).
///
/// Pipeline: `AVURLAsset` → `AVAssetReader` → `AVAssetReaderTrackOutput`
/// configured for `kCVPixelFormatType_32BGRA`. A worker thread owns the
/// reader, pulls `CMSampleBuffer`s synchronously, locks the underlying
/// `CVPixelBuffer`, swaps BGRA→RGBA, and pushes the frame into the binary
/// pipe ring. Pause / play / seek are signalled via a shared `Mutex<VideoState>`;
/// on seek, the worker rebuilds the reader from `time_range`.
///
/// Frame timing is "sleep `1.0 / fps` between pushes" — drift accumulates
/// across long clips. PTS-accurate scheduling is deferred (issue body, "out
/// of scope").
///
/// All AVFoundation / CoreVideo handles stay confined to the worker thread.
/// Reader-style obj-c objects are not documented thread-safe; we never
/// share them across threads.
pub struct AvfVideoDecoder {
    next_handle_id: std::sync::atomic::AtomicU64,
}

impl AvfVideoDecoder {
    pub fn new() -> Self {
        Self {
            next_handle_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Resolve a wire `source` string to a local filesystem path. Accepts
    /// either an absolute path or a `file://` URL. Returns the path string
    /// the caller passes to `NSURL::fileURLWithPath:`.
    fn resolve_path(source: &str) -> Result<String, VideoError> {
        if source.is_empty() {
            return Err(VideoError::InvalidSource(
                "source URL is empty".to_owned(),
            ));
        }
        if let Some(rest) = source.strip_prefix("file://") {
            // Strip the //hostname/ prefix if present (file:///foo/bar → /foo/bar).
            let path = rest.trim_start_matches('/');
            let absolute = format!("/{path}");
            return Ok(absolute);
        }
        if source.starts_with('/') {
            return Ok(source.to_owned());
        }
        Err(VideoError::InvalidSource(format!(
            "expected absolute path or file:// URL, got {source:?}"
        )))
    }
}

impl Default for AvfVideoDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
impl VideoDecoder for AvfVideoDecoder {
    fn open(
        &self,
        source: &str,
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) -> Result<(VideoOpenAck, VideoHandle), VideoError> {
        avf_impl::open(self, source, frame_ring)
    }

    fn list_camera_devices(&self) -> Vec<CameraDeviceInfo> {
        unsafe { avf_impl::enumerate_cameras() }
    }
}

#[cfg(not(target_os = "macos"))]
impl VideoDecoder for AvfVideoDecoder {
    fn open(
        &self,
        _source: &str,
        _frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) -> Result<(VideoOpenAck, VideoHandle), VideoError> {
        // AVFoundation is macOS-only. On other platforms the production stub
        // surfaces an explicit error rather than panicking.
        Err(VideoError::NotImplemented)
    }

    fn list_camera_devices(&self) -> Vec<CameraDeviceInfo> {
        vec![]
    }
}


#[cfg(target_os = "macos")]
#[allow(deprecated)]
// `AVAsset::tracksWithMediaType` is deprecated in favour of the async
// `loadTracksWithMediaType:completionHandler:`. We deliberately use the sync
// variant: this code runs on a dedicated worker thread that already owns the
// AVAssetReader, so blocking-on-track-load is correct. Switching to the
// async API would force a Tokio / dispatch_queue wrapper for no behavioural
// gain.
//
// `AVCaptureDevice::devicesWithMediaType` is deprecated in favour of
// `AVCaptureDeviceDiscoverySession` on newer OS versions. We use the sync
// variant for simplicity; the discovery session requires more setup.
// Localised `#[allow]` keeps the rest of the crate honest.
mod avf_impl {
    //! macOS-only AVFoundation backing. Isolated so non-mac builds don't see
    //! the obj-c symbols and the rest of `video.rs` stays portable.

    use super::{
        AvfVideoDecoder, CameraDeviceInfo, VideoError, VideoHandle, VideoHandleGuard,
        VideoHandleInner, VideoOpenAck, VideoState,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};

    use crossbeam_queue::ArrayQueue;
    use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
    use objc2::declare::ClassBuilder;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, Sel};
    use objc2::{sel, ClassType};
    use objc2_av_foundation::{
        AVAssetReader, AVAssetReaderTrackOutput, AVCaptureDevice, AVCaptureDeviceInput,
        AVCaptureSession, AVCaptureVideoDataOutput, AVMediaTypeVideo, AVURLAsset,
    };
    use objc2_core_media::{CMTime, CMTimeRange};
    use objc2_core_video::{
        kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA,
        CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
        CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
        CVPixelBufferUnlockBaseAddress,
    };
    use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

    /// Top-level entry from the trait `open()`. Dispatches on `camera://`
    /// sources to the camera capture path; otherwise resolves the file path
    /// and opens the AVFoundation asset pipeline.
    pub(super) fn open(
        decoder: &AvfVideoDecoder,
        source: &str,
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) -> Result<(VideoOpenAck, VideoHandle), VideoError> {
        if source.starts_with("camera://") {
            return open_camera(decoder, source, frame_ring);
        }

        let path = AvfVideoDecoder::resolve_path(source)?;

        // Construct AVURLAsset on the calling thread to query metadata
        // synchronously. The asset itself is dropped at function exit; the
        // worker constructs its own asset (cheap; the file handle is held
        // by the asset reader).
        let metadata = unsafe { query_metadata(&path)? };

        let handle_id = decoder
            .next_handle_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let state = Arc::new(std::sync::Mutex::new(VideoState::Play));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let path_thread = path.clone();
        let state_thread = Arc::clone(&state);
        let stop_thread = Arc::clone(&stop);
        let width = metadata.width;
        let height = metadata.height;
        let fps = metadata.fps.max(1.0);

        let join = std::thread::Builder::new()
            .name(format!("avf-video-{handle_id}"))
            .spawn(move || {
                worker_loop(
                    path_thread,
                    width,
                    height,
                    fps,
                    state_thread,
                    stop_thread,
                    frame_ring,
                );
            })
            .map_err(|e| VideoError::Decoder(format!("avf worker spawn: {e}")))?;

        let ack = VideoOpenAck {
            handle_id,
            width: metadata.width,
            height: metadata.height,
            fps: metadata.fps,
            duration_ms: metadata.duration_ms,
        };

        struct AvfGuard {
            stop: Arc<std::sync::atomic::AtomicBool>,
            join: Option<std::thread::JoinHandle<()>>,
        }
        impl VideoHandleGuard for AvfGuard {}
        impl Drop for AvfGuard {
            fn drop(&mut self) {
                self.stop
                    .store(true, std::sync::atomic::Ordering::Release);
                if let Some(h) = self.join.take() {
                    let _ = h.join();
                }
            }
        }

        struct AvfInner {
            state: Arc<std::sync::Mutex<VideoState>>,
        }
        impl VideoHandleInner for AvfInner {
            fn set_state(&mut self, new_state: VideoState) -> Result<(), VideoError> {
                let mut s = self
                    .state
                    .lock()
                    .expect("avf-video state mutex poisoned on set_state");
                *s = new_state;
                Ok(())
            }
        }

        let video_handle = VideoHandle {
            handle_id,
            width: metadata.width,
            height: metadata.height,
            fps: metadata.fps,
            duration_ms: metadata.duration_ms,
            _guard: Box::new(AvfGuard {
                stop,
                join: Some(join),
            }),
            inner: Box::new(AvfInner { state }),
        };

        Ok((ack, video_handle))
    }

    struct AssetMetadata {
        width: u32,
        height: u32,
        fps: f32,
        duration_ms: u64,
    }

    /// Build an `AVURLAsset` for `path` and pull `width`, `height`, `fps`,
    /// `duration_ms` off the first video track. Returns `Decoder` on any
    /// AVFoundation failure (file missing, no video track, etc.).
    unsafe fn query_metadata(path: &str) -> Result<AssetMetadata, VideoError> {
        let asset = build_asset(path)?;
        let video_type = AVMediaTypeVideo
            .ok_or_else(|| VideoError::Decoder("AVMediaTypeVideo unbound".to_owned()))?;
        let tracks = asset.tracksWithMediaType(video_type);
        if tracks.count() == 0 {
            return Err(VideoError::Decoder(format!(
                "no video track in asset: {path}"
            )));
        }
        let track = tracks.objectAtIndex(0);
        let size = track.naturalSize();
        let fps = track.nominalFrameRate();
        let duration_seconds = asset.duration().seconds();
        let duration_ms = if duration_seconds.is_finite() && duration_seconds >= 0.0 {
            (duration_seconds * 1000.0) as u64
        } else {
            0
        };
        Ok(AssetMetadata {
            width: size.width.round().max(0.0) as u32,
            height: size.height.round().max(0.0) as u32,
            fps,
            duration_ms,
        })
    }

    /// Construct `AVURLAsset` from a filesystem path. Validates the file
    /// exists; AVFoundation otherwise returns a reader that fails on
    /// `start_reading()` with a less helpful error.
    unsafe fn build_asset(
        path: &str,
    ) -> Result<objc2::rc::Retained<AVURLAsset>, VideoError> {
        if !std::path::Path::new(path).exists() {
            return Err(VideoError::Decoder(format!("file does not exist: {path}")));
        }
        let ns_path = NSString::from_str(path);
        let url = NSURL::fileURLWithPath(&ns_path);
        let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
        Ok(asset)
    }

    /// Build the output-settings dictionary `{kCVPixelBufferPixelFormatTypeKey:
    /// kCVPixelFormatType_32BGRA}`. The key is a `CFString` and toll-free
    /// bridged with `NSString`; we cast the pointer.
    unsafe fn build_bgra_settings(
    ) -> objc2::rc::Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> {
        // kCVPixelFormatType_32BGRA is OSType (u32 alias). NSNumber wraps it
        // verbatim — the AVFoundation pixel-format key compares the numeric
        // value, not the bit representation, so signed/unsigned NSNumber
        // boxing is interchangeable here.
        let format_value = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
        let cf_key: &objc2_core_foundation::CFString =
            kCVPixelBufferPixelFormatTypeKey;
        // Toll-free bridge: CFString and NSString share an ABI-compatible
        // representation. The cast is safe because both are opaque
        // `[u8; 0]`-shaped types pointing at the same Core Foundation object.
        let key: &NSString = &*(cf_key as *const _ as *const NSString);
        let value: &objc2::runtime::AnyObject =
            &*(format_value.as_ref() as *const _ as *const objc2::runtime::AnyObject);
        NSDictionary::from_slices::<NSString>(&[key], &[value])
    }

    /// Build an `AVAssetReader` + `AVAssetReaderTrackOutput` configured for
    /// 32BGRA, optionally seeked to `start_ms`. Returns the reader, the
    /// track output, and the asset (kept alive for the duration of the
    /// reader). Caller drops all three when the reader is exhausted or
    /// being rebuilt for seek.
    unsafe fn build_reader(
        path: &str,
        start_ms: Option<u64>,
    ) -> Result<
        (
            objc2::rc::Retained<AVURLAsset>,
            objc2::rc::Retained<AVAssetReader>,
            objc2::rc::Retained<AVAssetReaderTrackOutput>,
        ),
        VideoError,
    > {
        let asset = build_asset(path)?;
        let video_type = AVMediaTypeVideo
            .ok_or_else(|| VideoError::Decoder("AVMediaTypeVideo unbound".to_owned()))?;
        let tracks = asset.tracksWithMediaType(video_type);
        if tracks.count() == 0 {
            return Err(VideoError::Decoder(format!(
                "no video track in asset: {path}"
            )));
        }
        let track = tracks.objectAtIndex(0);

        let settings = build_bgra_settings();

        let track_output =
            AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
                &track,
                Some(&settings),
            );

        let reader = AVAssetReader::assetReaderWithAsset_error(&asset)
            .map_err(|e| VideoError::Decoder(format!("AVAssetReader init: {:?}", e)))?;

        // Seek by setting time_range on the reader before adding outputs.
        if let Some(ms) = start_ms {
            let start = CMTime::new(ms as i64, 1000);
            let duration = asset.duration();
            // Range from start to "positive infinity" represented as the
            // remaining duration. CMTimeRange is value/duration.
            let remaining_ms = ((duration.seconds() * 1000.0) as i64).saturating_sub(ms as i64);
            let dur = CMTime::new(remaining_ms.max(0), 1000);
            let range = CMTimeRange {
                start,
                duration: dur,
            };
            reader.setTimeRange(range);
        }

        if !reader.canAddOutput(&track_output) {
            return Err(VideoError::Decoder(format!(
                "reader cannot add 32BGRA output for {path}"
            )));
        }
        reader.addOutput(&track_output);

        if !reader.startReading() {
            let err = reader.error();
            return Err(VideoError::Decoder(format!(
                "AVAssetReader startReading failed: {:?}",
                err
            )));
        }

        Ok((asset, reader, track_output))
    }

    /// Decode loop. Owns the reader; rebuilds it on seek. Exits when `stop`
    /// is set, when the reader returns no more samples, or on a permanent
    /// error.
    fn worker_loop(
        path: String,
        width: u32,
        height: u32,
        fps: f32,
        state: Arc<std::sync::Mutex<VideoState>>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) {
        let frame_period = std::time::Duration::from_secs_f32(1.0 / fps);

        // Initial reader at offset 0.
        let mut current = match unsafe { build_reader(&path, None) } {
            Ok(r) => Some(r),
            Err(e) => {
                log::error!("avf worker[{path}]: initial reader build failed: {e}");
                return;
            }
        };

        loop {
            if stop.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }

            // Snapshot state. If a seek arrived, drop the current reader and
            // rebuild at the new offset, then resume play.
            let action = {
                let mut st = state.lock().expect("avf state mutex poisoned");
                match *st {
                    VideoState::Play => StateAction::Play,
                    VideoState::Pause => StateAction::Pause,
                    VideoState::Seek { position_ms } => {
                        *st = VideoState::Play;
                        StateAction::Seek(position_ms)
                    }
                }
            };

            match action {
                StateAction::Pause => {
                    std::thread::sleep(frame_period);
                    continue;
                }
                StateAction::Seek(ms) => {
                    drop(current.take());
                    current = match unsafe { build_reader(&path, Some(ms)) } {
                        Ok(r) => Some(r),
                        Err(e) => {
                            log::error!("avf worker[{path}]: seek rebuild failed: {e}");
                            return;
                        }
                    };
                    continue;
                }
                StateAction::Play => {}
            }

            let Some((_asset, _reader, track_output)) = current.as_ref() else {
                return;
            };

            // Pull and push a frame inside an autoreleasepool so the
            // CMSampleBuffer / CVPixelBuffer don't accumulate.
            let pushed = objc2::rc::autoreleasepool(|_pool| unsafe {
                let Some(sample) = track_output.copyNextSampleBuffer() else {
                    return PullResult::EndOfStream;
                };
                let Some(image_buffer) = sample.image_buffer() else {
                    return PullResult::Skip;
                };
                // CVPixelBuffer = CVImageBuffer = CVBuffer (type aliases). The
                // CV functions all accept the same `&CVBuffer` shape.
                let pixel_buffer: &objc2_core_video::CVPixelBuffer = &image_buffer;

                let lock_flags = CVPixelBufferLockFlags::ReadOnly;
                let lock_status = CVPixelBufferLockBaseAddress(pixel_buffer, lock_flags);
                // CVReturn is i32; 0 == kCVReturnSuccess.
                if lock_status != 0 {
                    return PullResult::Skip;
                }

                let buf_width = CVPixelBufferGetWidth(pixel_buffer) as u32;
                let buf_height = CVPixelBufferGetHeight(pixel_buffer) as u32;
                let bytes_per_row = CVPixelBufferGetBytesPerRow(pixel_buffer);
                let base = CVPixelBufferGetBaseAddress(pixel_buffer);

                let result = if base.is_null() || buf_width == 0 || buf_height == 0 {
                    PullResult::Skip
                } else {
                    let frame = copy_bgra_to_rgba(
                        base.cast::<u8>(),
                        bytes_per_row,
                        buf_width,
                        buf_height,
                        width,
                        height,
                    );
                    let _ = frame_ring.push(frame);
                    PullResult::Pushed
                };

                let _ = CVPixelBufferUnlockBaseAddress(pixel_buffer, lock_flags);
                result
            });

            match pushed {
                PullResult::EndOfStream => {
                    // Worker idles after EOS; the host closes the handle on
                    // its own. Sleep at the frame cadence so we don't spin.
                    std::thread::sleep(frame_period);
                }
                PullResult::Skip => {
                    // Decode hiccup; retry next tick at frame cadence.
                    std::thread::sleep(frame_period);
                }
                PullResult::Pushed => {
                    std::thread::sleep(frame_period);
                }
            }
        }
    }

    enum StateAction {
        Play,
        Pause,
        Seek(u64),
    }

    enum PullResult {
        Pushed,
        Skip,
        EndOfStream,
    }

    // ── Camera capture (#1505) ────────────────────────────────────────────

    /// Shared state threaded from the camera worker into the ObjC delegate.
    /// Heap-allocated so we can pass a raw pointer into the ObjC ivar.
    struct CameraShared {
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
        paused: Arc<AtomicBool>,
        out_width: u32,
        out_height: u32,
    }

    /// Enumerate connected camera devices via AVCaptureDevice. Returns an
    /// empty vec when no cameras are found or `AVMediaTypeVideo` is unbound.
    pub(super) unsafe fn enumerate_cameras() -> Vec<CameraDeviceInfo> {
        let video_type = match AVMediaTypeVideo {
            Some(t) => t,
            None => return vec![],
        };
        let devices = AVCaptureDevice::devicesWithMediaType(video_type);
        let count = devices.count();
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            let dev = devices.objectAtIndex(i);
            let id = dev.uniqueID().to_string();
            let name = dev.localizedName().to_string();
            result.push(CameraDeviceInfo { id, name });
        }
        result
    }

    /// Register `PlexiCameraDelegate` with the ObjC runtime once.
    ///
    /// The delegate stores a `*mut CameraShared` in its ivar. On each
    /// `captureOutput:didOutputSampleBuffer:fromConnection:` callback it
    /// reads the shared state, converts the pixel buffer BGRA→RGBA, and
    /// pushes the frame into the ring. The shared pointer is valid for the
    /// lifetime of the camera worker thread (which owns the `Box<CameraShared>`).
    fn get_camera_delegate_class() -> &'static objc2::runtime::AnyClass {
        static CLASS: OnceLock<&'static objc2::runtime::AnyClass> = OnceLock::new();
        CLASS.get_or_init(|| {
            let superclass = NSObject::class();
            let mut builder = ClassBuilder::new(c"PlexiCameraDelegate", superclass)
                .expect("PlexiCameraDelegate already registered");

            builder.add_ivar::<*mut std::ffi::c_void>(c"sharedPtr");

            unsafe extern "C" fn did_output_sample_buffer(
                this: &AnyObject,
                _cmd: Sel,
                _output: *mut AnyObject,
                sample_buffer: *mut AnyObject,
                _connection: *mut AnyObject,
            ) {
                // Wrap the entire callback in catch_unwind. Panics must not
                // cross the extern "C" boundary — they become abort() otherwise.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let cls = match objc2::runtime::AnyClass::get(c"PlexiCameraDelegate") {
                        Some(c) => c,
                        None => return,
                    };
                    let ivar = match cls.instance_variable(c"sharedPtr") {
                        Some(v) => v,
                        None => return,
                    };
                    let raw_ptr: *mut std::ffi::c_void =
                        unsafe { *ivar.load_ptr::<*mut std::ffi::c_void>(this) };
                    if raw_ptr.is_null() {
                        return;
                    }
                    let shared = unsafe { &*(raw_ptr as *const CameraShared) };

                    if shared.paused.load(Ordering::Acquire) {
                        return;
                    }

                    if sample_buffer.is_null() {
                        return;
                    }

                    // Extract pixel buffer from CMSampleBuffer.
                    let pixel_buffer: *mut AnyObject =
                        unsafe { objc2::msg_send![&*sample_buffer, imageBuffer] };
                    if pixel_buffer.is_null() {
                        return;
                    }

                    let lock_flags = CVPixelBufferLockFlags::ReadOnly;
                    let lock_status = unsafe {
                        CVPixelBufferLockBaseAddress(
                            &*(pixel_buffer as *const objc2_core_video::CVPixelBuffer),
                            lock_flags,
                        )
                    };
                    if lock_status != 0 {
                        return;
                    }

                    let pxbuf = unsafe {
                        &*(pixel_buffer as *const objc2_core_video::CVPixelBuffer)
                    };
                    let buf_width = CVPixelBufferGetWidth(pxbuf) as u32;
                    let buf_height = CVPixelBufferGetHeight(pxbuf) as u32;
                    let bytes_per_row = CVPixelBufferGetBytesPerRow(pxbuf);
                    let base = CVPixelBufferGetBaseAddress(pxbuf);

                    if !base.is_null() && buf_width > 0 && buf_height > 0 {
                        let frame = unsafe {
                            copy_bgra_to_rgba(
                                base.cast::<u8>(),
                                bytes_per_row,
                                buf_width,
                                buf_height,
                                shared.out_width,
                                shared.out_height,
                            )
                        };
                        let _ = shared.frame_ring.push(frame);
                    }

                    unsafe { let _ = CVPixelBufferUnlockBaseAddress(pxbuf, lock_flags); }
                }));
            }

            unsafe {
                builder.add_method(
                    sel!(captureOutput:didOutputSampleBuffer:fromConnection:),
                    did_output_sample_buffer as unsafe extern "C" fn(_, _, _, _, _),
                );
            }

            builder.register()
        })
    }

    /// Open a `camera://` source and return a `VideoHandle` that delivers
    /// live RGBA8 frames via the binary pipe ring.
    ///
    /// `camera://0` or `camera://` → the system default video device.
    /// `camera://<device-id>` → the device with that unique ID.
    pub(super) fn open_camera(
        decoder: &AvfVideoDecoder,
        source: &str,
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) -> Result<(VideoOpenAck, VideoHandle), VideoError> {
        let (width, height, fps) = (640u32, 480u32, 30.0f32);
        let handle_id = decoder
            .next_handle_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let paused_thread = Arc::clone(&paused);
        let stop_thread = Arc::clone(&stop);

        // Parse device id from `camera://<id>`. Empty string / "0" → default.
        let device_id = source
            .strip_prefix("camera://")
            .unwrap_or("")
            .to_string();

        let join = std::thread::Builder::new()
            .name(format!("avf-camera-{handle_id}"))
            .spawn(move || {
                camera_worker(
                    device_id,
                    width,
                    height,
                    fps,
                    handle_id,
                    paused_thread,
                    stop_thread,
                    frame_ring,
                );
            })
            .map_err(|e| VideoError::Decoder(format!("camera worker spawn: {e}")))?;

        let ack = VideoOpenAck {
            handle_id,
            width,
            height,
            fps,
            duration_ms: 0,
        };

        struct CameraGuard {
            stop: Arc<AtomicBool>,
            join: Option<std::thread::JoinHandle<()>>,
        }
        impl VideoHandleGuard for CameraGuard {}
        impl Drop for CameraGuard {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::Release);
                if let Some(h) = self.join.take() {
                    let _ = h.join();
                }
            }
        }

        struct CameraInner {
            paused: Arc<AtomicBool>,
        }
        impl VideoHandleInner for CameraInner {
            fn set_state(&mut self, new_state: VideoState) -> Result<(), VideoError> {
                match new_state {
                    VideoState::Play => self.paused.store(false, Ordering::Release),
                    VideoState::Pause => self.paused.store(true, Ordering::Release),
                    VideoState::Seek { .. } => {
                        log::warn!("camera: Seek is a no-op for live sources");
                    }
                }
                Ok(())
            }
        }

        let video_handle = VideoHandle {
            handle_id,
            width,
            height,
            fps,
            duration_ms: 0,
            _guard: Box::new(CameraGuard { stop, join: Some(join) }),
            inner: Box::new(CameraInner { paused }),
        };

        Ok((ack, video_handle))
    }

    /// Camera worker thread. Creates AVCaptureSession with the specified
    /// device, registers an ObjC delegate that forwards frames into the
    /// frame ring, then polls until the stop flag is set.
    ///
    /// Delegate callbacks are delivered on a private serial dispatch queue,
    /// so no NSRunLoop spin is required on this thread.
    fn camera_worker(
        device_id: String,
        width: u32,
        height: u32,
        fps: f32,
        handle_id: u64,
        paused: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) {
        // Box the shared state and keep it alive for the duration of the
        // session. The raw pointer is stored in the ObjC delegate ivar.
        let shared = Box::new(CameraShared {
            frame_ring,
            paused,
            out_width: width,
            out_height: height,
        });
        let shared_ptr = Box::into_raw(shared) as *mut std::ffi::c_void;

        let result = unsafe {
            run_capture_session(device_id, handle_id, fps, shared_ptr, &stop)
        };

        // Restore ownership so the Box is dropped and its Arc members
        // are decremented.
        let _shared = unsafe { Box::from_raw(shared_ptr as *mut CameraShared) };

        if let Err(e) = result {
            log::error!("camera worker[{handle_id}]: {e}");
        }
    }

    /// Builds and runs the AVCaptureSession, then polls until `stop` is set.
    unsafe fn run_capture_session(
        device_id: String,
        handle_id: u64,
        fps: f32,
        shared_ptr: *mut std::ffi::c_void,
        stop: &AtomicBool,
    ) -> Result<(), VideoError> {
        objc2::rc::autoreleasepool(|_pool| {
            let video_type = AVMediaTypeVideo
                .ok_or_else(|| VideoError::Decoder("AVMediaTypeVideo unbound".to_owned()))?;

            // Find the camera device.
            let device: Retained<AVCaptureDevice> = if device_id.is_empty() || device_id == "0" {
                AVCaptureDevice::defaultDeviceWithMediaType(video_type)
                    .ok_or_else(|| VideoError::Decoder("no default camera device".to_owned()))?
            } else {
                let devices = AVCaptureDevice::devicesWithMediaType(video_type);
                let mut found: Option<Retained<AVCaptureDevice>> = None;
                for i in 0..devices.count() {
                    let dev = devices.objectAtIndex(i);
                    if dev.uniqueID().to_string() == device_id {
                        found = Some(dev);
                        break;
                    }
                }
                found.ok_or_else(|| VideoError::Decoder(format!(
                    "camera device not found: {device_id:?}"
                )))?
            };

            log::info!(
                "camera worker[{handle_id}]: using device '{}'",
                device.localizedName()
            );

            // Build session.
            let session = AVCaptureSession::new();

            // Add device input.
            let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
                .map_err(|e| VideoError::Decoder(format!("AVCaptureDeviceInput: {e:?}")))?;

            if session.canAddInput(&input) {
                session.addInput(&input);
            } else {
                return Err(VideoError::Decoder(
                    "cannot add device input to capture session".to_owned(),
                ));
            }

            // Build video data output.
            let video_output = AVCaptureVideoDataOutput::new();
            video_output.setAlwaysDiscardsLateVideoFrames(true);

            // Create delegate object.
            let delegate_cls = get_camera_delegate_class();
            let delegate: Retained<NSObject> = {
                let obj: Retained<NSObject> = objc2::msg_send_id![delegate_cls, new];
                // Store the shared pointer in the ivar.
                let ivar = delegate_cls
                    .instance_variable(c"sharedPtr")
                    .expect("sharedPtr ivar");
                ivar.load_ptr::<*mut std::ffi::c_void>(obj.as_ref()).write(shared_ptr);
                obj
            };

            // Create a serial dispatch queue for delegate callbacks.
            let queue: DispatchRetained<DispatchQueue> =
                DispatchQueue::new("plexi.camera.delegate", DispatchQueueAttr::SERIAL);

            if session.canAddOutput(&video_output) {
                session.addOutput(&video_output);
            } else {
                return Err(VideoError::Decoder(
                    "cannot add video output to capture session".to_owned(),
                ));
            }

            // Wire the delegate.
            let delegate_ptr = Retained::as_ptr(&delegate) as *const AnyObject;
            video_output.setSampleBufferDelegate_queue(
                Some(&*(delegate_ptr
                    as *const objc2::runtime::ProtocolObject<
                        dyn objc2_av_foundation::AVCaptureVideoDataOutputSampleBufferDelegate,
                    >)),
                Some(&queue),
            );

            let _ = fps; // fps is advisory; AVCapture delivers at device rate
            session.startRunning();
            log::info!("camera worker[{handle_id}]: session started");

            // Poll until stop is signalled.
            let poll = std::time::Duration::from_millis(50);
            loop {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(poll);
            }

            session.stopRunning();
            log::info!("camera worker[{handle_id}]: session stopped");

            // Nil out the delegate ivar before the shared pointer is freed.
            let ivar = delegate_cls
                .instance_variable(c"sharedPtr")
                .expect("sharedPtr ivar");
            ivar.load_ptr::<*mut std::ffi::c_void>(delegate.as_ref())
                .write(std::ptr::null_mut());

            Ok(())
        })
    }

    /// Copy a BGRA pixel buffer into an `[R,G,B,A,...]` packed `Vec<u8>` of
    /// size `out_width * out_height * 4`. `bytes_per_row` may be larger
    /// than `width * 4` due to row alignment — we only copy `width * 4`
    /// bytes per row. If the buffer's intrinsic dimensions disagree with
    /// the negotiated `out_width / out_height`, we use the smaller of each
    /// to avoid reading past the buffer.
    unsafe fn copy_bgra_to_rgba(
        base: *const u8,
        bytes_per_row: usize,
        buf_width: u32,
        buf_height: u32,
        out_width: u32,
        out_height: u32,
    ) -> Vec<u8> {
        let w = out_width.min(buf_width) as usize;
        let h = out_height.min(buf_height) as usize;
        let mut out = vec![0u8; (out_width * out_height * 4) as usize];
        let out_row = (out_width * 4) as usize;
        for y in 0..h {
            let src_row = base.add(y * bytes_per_row);
            let dst_row_off = y * out_row;
            for x in 0..w {
                let src = src_row.add(x * 4);
                let b = *src;
                let g = *src.add(1);
                let r = *src.add(2);
                let a = *src.add(3);
                let dst = dst_row_off + x * 4;
                out[dst] = r;
                out[dst + 1] = g;
                out[dst + 2] = b;
                out[dst + 3] = a;
            }
        }
        out
    }
}

// ─── Mock impl: MockVideoDecoder ─────────────────────────────────────────────

/// Configuration for the mock decoder. Parsed out of `PLEXI_VIDEO=mock://...`
/// at factory time; constructed directly in tests.
#[derive(Debug, Clone, PartialEq)]
pub struct MockVideoDecoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    /// Reported back as `duration_ms` on `open`. The mock never stops
    /// streaming on its own — the caller closes the handle when done. The
    /// duration is advisory metadata for UI seekbars.
    pub duration_ms: u64,
}

impl Default for MockVideoDecoderConfig {
    fn default() -> Self {
        Self {
            width: 320,
            height: 180,
            fps: 30.0,
            duration_ms: 30_000,
        }
    }
}

/// Procedural-gradient video decoder. Drives a worker thread that paints an
/// animated RGBA gradient and pushes frames into the binary pipe ring at
/// `fps`. Used by the POC and tests.
pub struct MockVideoDecoder {
    config: MockVideoDecoderConfig,
    next_handle_id: std::sync::atomic::AtomicU64,
}

impl MockVideoDecoder {
    pub fn new(config: MockVideoDecoderConfig) -> Self {
        Self {
            config,
            next_handle_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Validate a `mock://...` source URL and parse out any per-call
    /// overrides. Today the URL is opaque ("mock://gradient") and we simply
    /// reject empty / non-`mock://` strings; the config carries the
    /// dimensions. Future: accept query-string fps/width/height overrides.
    fn validate_source(source: &str) -> Result<(), VideoError> {
        if source.is_empty() {
            return Err(VideoError::InvalidSource(
                "source URL is empty".to_owned(),
            ));
        }
        // Mock decoder accepts mock:// URLs and camera:// sources (for tests
        // that exercise the camera path without real hardware).
        if !source.starts_with("mock://") && !source.starts_with("camera://") {
            return Err(VideoError::InvalidSource(format!(
                "MockVideoDecoder requires a mock:// or camera:// URL, got {source:?}"
            )));
        }
        Ok(())
    }
}

impl VideoDecoder for MockVideoDecoder {
    fn open(
        &self,
        source: &str,
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) -> Result<(VideoOpenAck, VideoHandle), VideoError> {
        Self::validate_source(source)?;

        let handle_id = self
            .next_handle_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let width = self.config.width;
        let height = self.config.height;
        let fps = self.config.fps.max(1.0);
        let duration_ms = self.config.duration_ms;

        // Per-handle control surface. The worker thread reads `state` on each
        // tick; the host writes via `set_state`.
        let state = Arc::new(std::sync::Mutex::new(VideoState::Play));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let state_thread = Arc::clone(&state);
        let stop_thread = Arc::clone(&stop);
        let frame_period = std::time::Duration::from_secs_f32(1.0 / fps);

        let handle = std::thread::Builder::new()
            .name(format!("mock-video-{handle_id}"))
            .spawn(move || {
                let mut frame_index: u64 = 0;
                loop {
                    if stop_thread.load(std::sync::atomic::Ordering::Acquire) {
                        return;
                    }
                    // Snapshot the current state and apply any seek. We
                    // drain the seek to avoid re-applying it on the next tick.
                    let paused = {
                        let mut st = state_thread
                            .lock()
                            .expect("mock-video state mutex poisoned");
                        match *st {
                            VideoState::Play => false,
                            VideoState::Pause => true,
                            VideoState::Seek { position_ms } => {
                                // Convert ms → frame index at `fps`; clamp to
                                // duration so we don't overshoot.
                                let total_frames = ((duration_ms as f32 / 1000.0) * fps) as u64;
                                let target = ((position_ms as f32 / 1000.0) * fps) as u64;
                                frame_index = target.min(total_frames.max(1) - 1);
                                // After applying the seek, resume play.
                                *st = VideoState::Play;
                                false
                            }
                        }
                    };
                    if !paused {
                        let buf = render_gradient_frame(width, height, frame_index);
                        // Drop on full ring — never block the worker.
                        let _ = frame_ring.push(buf);
                        frame_index = frame_index.wrapping_add(1);
                    }
                    std::thread::sleep(frame_period);
                }
            })
            .map_err(|e| VideoError::Decoder(format!("mock thread spawn: {e}")))?;

        struct MockGuard {
            stop: Arc<std::sync::atomic::AtomicBool>,
            handle: Option<std::thread::JoinHandle<()>>,
        }
        impl VideoHandleGuard for MockGuard {}
        impl Drop for MockGuard {
            fn drop(&mut self) {
                self.stop
                    .store(true, std::sync::atomic::Ordering::Release);
                if let Some(h) = self.handle.take() {
                    let _ = h.join();
                }
            }
        }

        struct MockInner {
            state: Arc<std::sync::Mutex<VideoState>>,
        }
        impl VideoHandleInner for MockInner {
            fn set_state(&mut self, new_state: VideoState) -> Result<(), VideoError> {
                let mut s = self
                    .state
                    .lock()
                    .expect("mock-video state mutex poisoned on set_state");
                *s = new_state;
                Ok(())
            }
        }

        let ack = VideoOpenAck {
            handle_id,
            width,
            height,
            fps,
            duration_ms,
        };

        let video_handle = VideoHandle {
            handle_id,
            width,
            height,
            fps,
            duration_ms,
            _guard: Box::new(MockGuard {
                stop,
                handle: Some(handle),
            }),
            inner: Box::new(MockInner { state }),
        };

        Ok((ack, video_handle))
    }

    fn list_camera_devices(&self) -> Vec<CameraDeviceInfo> {
        vec![CameraDeviceInfo {
            id: "mock-cam-0".into(),
            name: "Mock Camera".into(),
        }]
    }
}

/// Paint a procedural RGBA8 gradient that animates with `frame_index`. The
/// horizontal axis encodes a hue sweep; the vertical axis encodes the frame
/// index modulated through brightness. Pure CPU, no allocations beyond the
/// returned buffer.
fn render_gradient_frame(width: u32, height: u32, frame_index: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity((width * height * 4) as usize);
    let phase = (frame_index % 256) as u8;
    for y in 0..height {
        let row_base = ((y as u32 * 255) / height.max(1)) as u8;
        for x in 0..width {
            let r = ((x as u32 * 255) / width.max(1)) as u8;
            let g = row_base.wrapping_add(phase);
            let b = phase.wrapping_add(row_base / 2);
            buf.push(r);
            buf.push(g);
            buf.push(b);
            buf.push(0xff);
        }
    }
    buf
}

// ─── Factory ────────────────────────────────────────────────────────────────

/// Build the production video decoder. Reads `PLEXI_VIDEO` for a
/// `mock://...` opt-in (parametrised via `PLEXI_VIDEO_FPS`,
/// `PLEXI_VIDEO_WIDTH`, `PLEXI_VIDEO_HEIGHT`, `PLEXI_VIDEO_DURATION_MS`);
/// otherwise instantiates the production [`AvfVideoDecoder`] stub.
///
/// Tests inject `Arc::new(MockVideoDecoder::new(cfg))` directly into
/// `ProcessApp::video_device` and skip the env var altogether.
#[cfg(not(test))]
pub fn default_video_device() -> Arc<dyn VideoDecoder> {
    if let Ok(url) = std::env::var("PLEXI_VIDEO") {
        if url.starts_with("mock://") {
            let cfg = MockVideoDecoderConfig {
                width: parse_env("PLEXI_VIDEO_WIDTH", 320),
                height: parse_env("PLEXI_VIDEO_HEIGHT", 180),
                fps: parse_env_f32("PLEXI_VIDEO_FPS", 30.0),
                duration_ms: parse_env("PLEXI_VIDEO_DURATION_MS", 30_000),
            };
            log::info!(
                "video: PLEXI_VIDEO={url} — using MockVideoDecoder ({}x{} @ {} fps)",
                cfg.width,
                cfg.height,
                cfg.fps
            );
            return Arc::new(MockVideoDecoder::new(cfg));
        }
    }
    Arc::new(AvfVideoDecoder::new())
}

#[cfg(test)]
pub fn default_video_device() -> Arc<dyn VideoDecoder> {
    // Tests that exercise the routing path inject their own MockVideoDecoder
    // directly into `ProcessApp::video_device`. The harness factory returns
    // the production stub so the no-injection path errors loudly with
    // `NotImplemented` rather than silently producing frames.
    Arc::new(AvfVideoDecoder::new())
}

#[cfg(not(test))]
fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<T>().ok())
        .unwrap_or(default)
}

#[cfg(not(test))]
fn parse_env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(default)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ring() -> Arc<ArrayQueue<Vec<u8>>> {
        Arc::new(ArrayQueue::new(64))
    }

    /// Convenience: pull whatever the ring drained in `wait_ms` and return
    /// the count of frames + the first frame for shape checks.
    fn drain_after(
        ring: &Arc<ArrayQueue<Vec<u8>>>,
        wait_ms: u64,
    ) -> (usize, Option<Vec<u8>>) {
        std::thread::sleep(std::time::Duration::from_millis(wait_ms));
        let mut first: Option<Vec<u8>> = None;
        let mut count = 0;
        while let Some(f) = ring.pop() {
            if first.is_none() {
                first = Some(f);
            } else {
                let _ = f;
            }
            count += 1;
        }
        (count, first)
    }

    #[test]
    fn open_with_invalid_path_returns_error_not_panic() {
        // Mock decoder rejects non-mock URLs. The contract: bad source → Err,
        // never panic. (Production stub returns NotImplemented for any
        // source, which is its own test below.)
        let dev = MockVideoDecoder::new(MockVideoDecoderConfig::default());
        let ring = make_ring();
        let res = dev.open("/no/such/file.mp4", Arc::clone(&ring));
        match res {
            Err(VideoError::InvalidSource(msg)) => {
                assert!(
                    msg.contains("/no/such/file.mp4"),
                    "error must name the bad source: {msg}"
                );
            }
            other => panic!("expected InvalidSource, got {other:?}"),
        }
        let res = dev.open("", Arc::clone(&ring));
        match res {
            Err(VideoError::InvalidSource(_)) => {}
            other => panic!("expected InvalidSource for empty source, got {other:?}"),
        }
    }

    #[test]
    fn mock_decoder_round_trips_frames() {
        // Open the mock at a small size and a high fps so we get several
        // frames in a short test window. Drain the ring and assert each
        // frame is exactly width*height*4 bytes RGBA8.
        let cfg = MockVideoDecoderConfig {
            width: 16,
            height: 8,
            fps: 60.0,
            duration_ms: 1_000,
        };
        let dev = MockVideoDecoder::new(cfg.clone());
        let ring = make_ring();
        let (ack, handle) = dev
            .open("mock://gradient", Arc::clone(&ring))
            .expect("mock open must succeed");
        assert_eq!(ack.width, cfg.width);
        assert_eq!(ack.height, cfg.height);
        assert!((ack.fps - cfg.fps).abs() < 0.01);
        assert_eq!(ack.duration_ms, cfg.duration_ms);
        assert_eq!(ack.handle_id, 1);

        let (count, first) = drain_after(&ring, 250);
        drop(handle); // join worker
        assert!(count > 0, "mock must deliver at least one frame in 250ms");
        let first = first.expect("first frame must be present");
        assert_eq!(
            first.len(),
            (cfg.width * cfg.height * 4) as usize,
            "frame size must equal width*height*4 (RGBA8)"
        );
    }

    #[test]
    fn avf_decoder_open_with_invalid_path_returns_error_not_panic() {
        // Production decoder against a missing file MUST return an error and
        // never panic. The #![deny(clippy::todo)] lint at the crate root
        // catches todo!() / unimplemented!() — this test pins the runtime
        // contract for the AVFoundation path.
        let dev = AvfVideoDecoder::new();
        let ring = make_ring();
        let res = dev.open("/tmp/definitely-does-not-exist-plexi-test.mp4", ring);
        match res {
            Err(VideoError::Decoder(_)) => {}
            #[cfg(not(target_os = "macos"))]
            Err(VideoError::NotImplemented) => {}
            other => panic!("expected Decoder error for missing file, got {other:?}"),
        }
    }

    #[test]
    fn avf_decoder_open_with_empty_source_returns_invalid_source() {
        let dev = AvfVideoDecoder::new();
        let ring = make_ring();
        let res = dev.open("", ring);
        match res {
            Err(VideoError::InvalidSource(_)) => {}
            #[cfg(not(target_os = "macos"))]
            Err(VideoError::NotImplemented) => {}
            other => panic!("expected InvalidSource for empty source, got {other:?}"),
        }
    }

    #[test]
    fn avf_decoder_open_with_relative_path_returns_invalid_source() {
        let dev = AvfVideoDecoder::new();
        let ring = make_ring();
        let res = dev.open("relative/path.mp4", ring);
        match res {
            Err(VideoError::InvalidSource(_)) => {}
            #[cfg(not(target_os = "macos"))]
            Err(VideoError::NotImplemented) => {}
            other => panic!("expected InvalidSource for relative path, got {other:?}"),
        }
    }

    #[test]
    fn set_state_pause_then_play_round_trips() {
        // Open the mock; pause it; assert the ring drains down. Resume; assert
        // frames flow again. This pins the pause path through the worker
        // thread without flaking on tight timing.
        let cfg = MockVideoDecoderConfig {
            width: 4,
            height: 4,
            fps: 60.0,
            duration_ms: 5_000,
        };
        let dev = MockVideoDecoder::new(cfg);
        let ring = make_ring();
        let (_ack, mut handle) = dev
            .open("mock://gradient", Arc::clone(&ring))
            .expect("mock open must succeed");

        // Let it produce a few frames.
        std::thread::sleep(std::time::Duration::from_millis(120));
        handle
            .set_state(VideoState::Pause)
            .expect("pause must succeed");
        // Drain whatever was already queued; further frames should not arrive.
        std::thread::sleep(std::time::Duration::from_millis(60));
        while ring.pop().is_some() {}
        std::thread::sleep(std::time::Duration::from_millis(120));
        let stalled = {
            let mut n = 0;
            while ring.pop().is_some() {
                n += 1;
            }
            n
        };
        assert_eq!(
            stalled, 0,
            "while paused the ring must not gain frames, got {stalled}"
        );

        // Resume.
        handle
            .set_state(VideoState::Play)
            .expect("play must succeed");
        std::thread::sleep(std::time::Duration::from_millis(120));
        let resumed = {
            let mut n = 0;
            while ring.pop().is_some() {
                n += 1;
            }
            n
        };
        assert!(resumed > 0, "after resume frames must flow again");

        // Seek anywhere — must not error.
        handle
            .set_state(VideoState::Seek { position_ms: 1_000 })
            .expect("seek must succeed");
        drop(handle);
    }

    #[test]
    fn frame_layout_is_rgba8_packed() {
        // Spot-check that the gradient produces a buffer of the expected
        // shape and that the alpha byte is fully opaque (0xff). This pins
        // the wire format consumers parse against.
        let buf = render_gradient_frame(8, 4, 0);
        assert_eq!(buf.len(), 8 * 4 * 4);
        // Every 4th byte (the A channel) must be 0xff.
        for i in 0..(8 * 4) {
            assert_eq!(buf[i * 4 + 3], 0xff, "alpha at pixel {i} must be 0xff");
        }
    }

    #[test]
    fn factory_default_returns_avf_in_tests() {
        // Under `cfg(test)`, the factory always returns the production
        // AvfVideoDecoder — tests that need the mock inject
        // `Arc::new(MockVideoDecoder::new(...))` into
        // `ProcessApp::video_device` directly. The default decoder rejects
        // a non-existent file via Decoder/InvalidSource — never NotImplemented
        // (post-#346) on macOS, never panic anywhere.
        let dev = default_video_device();
        let ring = make_ring();
        let res = dev.open("/tmp/definitely-does-not-exist-plexi-factory-test.mp4", ring);
        let ok = match &res {
            Err(VideoError::Decoder(_)) => true,
            #[cfg(not(target_os = "macos"))]
            Err(VideoError::NotImplemented) => true,
            _ => false,
        };
        assert!(ok, "expected Decoder error (or NotImplemented off-mac), got {res:?}");
    }

    #[test]
    fn prod_methods_no_panic_smoke() {
        // Production no-panic guarantee, mirrors the audio/midi pattern:
        // every input shape (missing file, empty, relative, mock://) must
        // surface an error and never panic.
        let dev = AvfVideoDecoder::new();
        let _ = dev.open("/tmp/missing.mp4", make_ring());
        let _ = dev.open("", make_ring());
        let _ = dev.open("mock://gradient", make_ring());
        let _ = dev.open("relative.mp4", make_ring());
    }
}
