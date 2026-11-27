use super::helpers::{decode_image_preview, read_text_preview, DirStats, TextPreview};
use crate::app::ui_mailbox::UiWake;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

const DIR_PREVIEW_CAP: usize = 500;

/// One in-flight read per preview kind. Drawing records the latest requested
/// path, and the view's external service starts reads and accepts results.
pub(super) struct PreviewTask<T> {
    requested: Option<PathBuf>,
    loading: Option<Receiver<Result<T, String>>>,
}

impl<T> Default for PreviewTask<T> {
    fn default() -> Self {
        Self {
            requested: None,
            loading: None,
        }
    }
}

impl<T: Send + 'static> PreviewTask<T> {
    pub(super) fn request(&mut self, path: &Path) {
        self.requested = Some(path.to_path_buf());
    }

    pub(super) fn service(
        &mut self,
        wake: &Arc<dyn UiWake>,
        read: fn(&Path) -> Result<T, String>,
    ) -> Option<Result<T, String>> {
        if let Some(loading) = &self.loading {
            let result = match loading.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    let message = "Preview reader exited without an answer";
                    log::warn!("file_browser: {message}");
                    Err(message.into())
                }
            };
            self.loading = None;
            if self.requested.is_none() {
                return Some(result);
            }
        }
        let path = self.requested.take()?;
        let (done, loading) = mpsc::channel();
        let wake = Arc::clone(wake);
        let worker = std::thread::Builder::new()
            .name("plexi-explorer-preview".into())
            .spawn(move || {
                let result = read(&path);
                if let Err(error) = &result {
                    log::warn!(
                        "file_browser: preview failed for {}: {error}",
                        path.display()
                    );
                }
                if done.send(result).is_ok() {
                    wake.wake("file_browser_preview");
                }
            });
        match worker {
            Ok(_) => {
                self.loading = Some(loading);
                None
            }
            Err(error) => {
                log::warn!("file_browser: could not start preview reader: {error}");
                Some(Err(error.to_string()))
            }
        }
    }
}

pub(super) fn read_image(path: &Path) -> Result<egui::ColorImage, String> {
    let rgba = decode_image_preview(path)?;
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

pub(super) fn read_text(path: &Path) -> Result<TextPreview, String> {
    read_text_preview(path).map_err(|error| error.to_string())
}

pub(super) fn read_dir(path: &Path) -> Result<DirStats, String> {
    let mut stats = DirStats {
        file_count: 0,
        dir_count: 0,
        total_bytes: 0,
        truncated: false,
    };
    let entries = std::fs::read_dir(path).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log::debug!(
                    "file_browser: directory preview entry in {}: {error}",
                    path.display()
                );
                continue;
            }
        };
        if stats.file_count + stats.dir_count >= DIR_PREVIEW_CAP {
            stats.truncated = true;
            break;
        }
        match entry.metadata() {
            Ok(meta) if meta.is_dir() => stats.dir_count += 1,
            Ok(meta) => {
                stats.file_count += 1;
                stats.total_bytes += meta.len();
            }
            Err(error) => {
                stats.file_count += 1;
                log::debug!(
                    "file_browser: directory preview metadata for {}: {error}",
                    entry.path().display()
                );
            }
        }
    }
    Ok(stats)
}
