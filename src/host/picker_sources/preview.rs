//! Palette previews: the one classifier that decides a file's preview kind,
//! and the worker that reads text and decodes images off the UI thread
//! (`docs/palette-picker.md`, Preview). No egui types: an image arrives as
//! pixels and the host uploads its texture in `App::logic`.

use crate::file_browser::helpers::{
    decode_image_preview, is_image_extension, read_text_preview, TextPreviewError,
};
use plexi_host_model::picker::{PreviewBody, PreviewRef, RowKey};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// An image file larger than this is not decoded: "Too large to preview".
pub(crate) const PREVIEW_IMAGE_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// A preview still loading after this shows "Preview timed out".
pub(crate) const PREVIEW_TIMEOUT: Duration = Duration::from_secs(3);

const TOO_LARGE: &str = "Too large to preview";

/// The preview kind of a file of `size` bytes: an image by extension, else
/// text (a NUL byte in what is read then says "Binary file").
pub(crate) fn classify(path: &Path, size: Option<u64>) -> PreviewRef {
    let image = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| is_image_extension(&ext.to_ascii_lowercase()));
    if !image {
        return PreviewRef::Text;
    }
    if size.is_some_and(|size| size > PREVIEW_IMAGE_MAX_BYTES) {
        return PreviewRef::Unsupported(TOO_LARGE);
    }
    PreviewRef::Image
}

/// One finished preview: its body, and an image's pixels for upload.
pub(crate) struct LoadedPreview {
    pub row: RowKey,
    pub body: PreviewBody,
    pub pixels: Option<image::RgbaImage>,
    /// The thread that read the file, for the UI-thread test.
    #[cfg(test)]
    pub read_on: thread::ThreadId,
}

struct Request {
    row: RowKey,
    path: PathBuf,
    kind: PreviewRef,
}

/// Reads previews on one worker thread, newest request first: requests a
/// later one replaced before the worker reached them are skipped.
pub(crate) struct PreviewLoader {
    requests: Option<Sender<Request>>,
    loaded: Receiver<LoadedPreview>,
    pending: Option<(RowKey, Instant)>,
    worker: Option<JoinHandle<()>>,
}

impl PreviewLoader {
    pub(crate) fn spawn(wake: Arc<dyn crate::app::ui_mailbox::UiWake>) -> Result<Self, String> {
        let (requests, inbox) = mpsc::channel::<Request>();
        let (done, loaded) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("plexi-palette-preview".to_owned())
            .spawn(move || {
                while let Ok(mut request) = inbox.recv() {
                    while let Ok(newer) = inbox.try_recv() {
                        request = newer;
                    }
                    let preview = load(request);
                    if done.send(preview).is_err() {
                        return;
                    }
                    wake.wake("palette_preview");
                }
            })
            .map_err(|err| format!("spawn palette preview worker: {err}"))?;
        Ok(Self {
            requests: Some(requests),
            loaded,
            pending: None,
            worker: Some(worker),
        })
    }

    /// Ask for `row`'s preview of `path`. Never reads on the caller's thread.
    pub(crate) fn load(&mut self, row: RowKey, path: PathBuf, kind: PreviewRef) {
        self.pending = Some((row.clone(), Instant::now()));
        let sent = self
            .requests
            .as_ref()
            .is_some_and(|requests| requests.send(Request { row, path, kind }).is_ok());
        if !sent {
            log::warn!(
                "palette preview: worker is gone; the preview stays loading until it times out"
            );
        }
    }

    /// Finished previews, plus a timed-out note for a request older than
    /// `PREVIEW_TIMEOUT`. Serviced from `App::logic`.
    pub(crate) fn poll(&mut self, now: Instant) -> Vec<LoadedPreview> {
        let mut out: Vec<LoadedPreview> = self.loaded.try_iter().collect();
        if let Some((row, since)) = &self.pending {
            if out.iter().any(|done| done.row == *row) {
                self.pending = None;
            } else if now.duration_since(*since) >= PREVIEW_TIMEOUT {
                log::warn!("palette preview: timed out after {PREVIEW_TIMEOUT:?}");
                out.push(LoadedPreview {
                    row: row.clone(),
                    body: PreviewBody::Note("Preview timed out".to_owned()),
                    pixels: None,
                    #[cfg(test)]
                    read_on: thread::current().id(),
                });
                self.pending = None;
            }
        }
        out
    }
}

impl Drop for PreviewLoader {
    fn drop(&mut self) {
        drop(self.requests.take());
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                log::warn!("palette preview: worker panicked");
            }
        }
    }
}

fn load(request: Request) -> LoadedPreview {
    let Request { row, path, kind } = request;
    let failed = |error: &dyn std::fmt::Display| {
        log::warn!(
            "palette preview: could not read '{}': {error}",
            path.display()
        );
        PreviewBody::Note(format!("Could not read: {error}"))
    };
    let (body, pixels) = match kind {
        PreviewRef::Text => match read_text_preview(&path) {
            Ok(text) => (
                PreviewBody::Text {
                    text: text.body,
                    truncated: text.truncated,
                },
                None,
            ),
            Err(TextPreviewError::Binary) => (PreviewBody::Note("Binary file".to_owned()), None),
            Err(TextPreviewError::Io(error)) => (failed(&error), None),
        },
        PreviewRef::Image => match std::fs::metadata(&path) {
            // The file may have grown since the walk measured it.
            Ok(meta) if meta.len() > PREVIEW_IMAGE_MAX_BYTES => {
                (PreviewBody::Note(TOO_LARGE.to_owned()), None)
            }
            Ok(_) => match decode_image_preview(&path) {
                Ok(pixels) => (
                    PreviewBody::Image {
                        width: pixels.width(),
                        height: pixels.height(),
                    },
                    Some(pixels),
                ),
                Err(error) => (failed(&error), None),
            },
            Err(error) => (failed(&error), None),
        },
        PreviewRef::Unsupported(reason) => (PreviewBody::Note(reason.to_owned()), None),
    };
    log::debug!("palette preview: loaded one {kind:?} preview");
    LoadedPreview {
        row,
        body,
        pixels,
        #[cfg(test)]
        read_on: thread::current().id(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoWake;
    impl crate::app::ui_mailbox::UiWake for NoWake {
        fn wake(&self, _source: &'static str) {}
    }

    fn loader() -> PreviewLoader {
        PreviewLoader::spawn(Arc::new(NoWake)).expect("spawn preview worker")
    }

    fn wait(loader: &mut PreviewLoader) -> LoadedPreview {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(done) = loader.poll(Instant::now()).pop() {
                return done;
            }
            assert!(Instant::now() < deadline, "no preview arrived");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn preview_never_loads_on_the_ui_thread() {
        let dir = tempfile::tempdir().expect("dir");
        let text = dir.path().join("notes.md");
        std::fs::write(&text, "# Title\nbody").expect("write text");
        let picture = dir.path().join("dot.png");
        image::RgbaImage::new(3, 2)
            .save(&picture)
            .expect("write png");
        let mut loader = loader();
        let ui_thread = thread::current().id();

        loader.load(RowKey::new("files", "notes"), text, PreviewRef::Text);
        let done = wait(&mut loader);
        assert_ne!(done.read_on, ui_thread, "text read on the calling thread");
        assert_eq!(
            done.body,
            PreviewBody::Text {
                text: "# Title\nbody".to_owned(),
                truncated: false
            }
        );

        loader.load(RowKey::new("files", "dot"), picture, PreviewRef::Image);
        let done = wait(&mut loader);
        assert_ne!(
            done.read_on, ui_thread,
            "image decoded on the calling thread"
        );
        assert_eq!(
            done.body,
            PreviewBody::Image {
                width: 3,
                height: 2
            }
        );
        assert!(done.pixels.is_some());
    }

    #[test]
    fn oversized_image_shows_no_preview_with_reason() {
        let dir = tempfile::tempdir().expect("dir");
        let big = dir.path().join("huge.png");
        assert_eq!(
            classify(&big, Some(PREVIEW_IMAGE_MAX_BYTES + 1)),
            PreviewRef::Unsupported("Too large to preview")
        );
        assert_eq!(
            classify(&big, Some(PREVIEW_IMAGE_MAX_BYTES)),
            PreviewRef::Image
        );
        assert_eq!(
            classify(&dir.path().join("a.rs"), Some(1)),
            PreviewRef::Text
        );

        // Grown past the limit after the walk measured it: the loader
        // refuses to decode it too.
        let file = std::fs::File::create(&big).expect("create");
        file.set_len(PREVIEW_IMAGE_MAX_BYTES + 1).expect("grow");
        let mut loader = loader();
        loader.load(RowKey::new("files", "huge"), big, PreviewRef::Image);
        let done = wait(&mut loader);
        assert_eq!(
            done.body,
            PreviewBody::Note("Too large to preview".to_owned())
        );
        assert!(done.pixels.is_none());
    }

    #[test]
    fn binary_and_unreadable_files_say_why() {
        let dir = tempfile::tempdir().expect("dir");
        let binary = dir.path().join("blob.bin");
        std::fs::write(&binary, b"ab\0cd").expect("write");
        let mut loader = loader();
        loader.load(RowKey::new("files", "blob"), binary, PreviewRef::Text);
        assert_eq!(
            wait(&mut loader).body,
            PreviewBody::Note("Binary file".to_owned())
        );
        loader.load(
            RowKey::new("files", "gone"),
            dir.path().join("missing.txt"),
            PreviewRef::Text,
        );
        match wait(&mut loader).body {
            PreviewBody::Note(note) => assert!(note.starts_with("Could not read: "), "{note}"),
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[test]
    fn a_stuck_preview_times_out() {
        let mut loader = loader();
        let row = RowKey::new("files", "slow");
        loader.pending = Some((row.clone(), Instant::now()));
        assert!(loader.poll(Instant::now()).is_empty());
        let late = loader.poll(Instant::now() + PREVIEW_TIMEOUT);
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].row, row);
        assert_eq!(
            late[0].body,
            PreviewBody::Note("Preview timed out".to_owned())
        );
    }
}
