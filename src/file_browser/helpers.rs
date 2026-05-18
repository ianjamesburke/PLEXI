use std::path::Path;
use std::time::SystemTime;

/// Routing classification for a file the user wants to open. Drives the
/// GUI↔terminal media bridge (#79): video/audio files spawn the in-app
/// player examples; everything else falls through to the system default
/// (`open` / `xdg-open`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Video,
    Audio,
    Other,
}

impl MediaKind {
    /// Classify a path by extension. Case-insensitive. Paths with no
    /// extension or unrecognised extensions return `Other`.
    pub(crate) fn for_path(path: &Path) -> Self {
        let Some(ext) = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
        else {
            return Self::Other;
        };
        match ext.as_str() {
            // Common video container formats. AVFoundation handles the
            // big three on macOS (#346); the rest still route to the
            // in-app player so it can surface a decoder error rather
            // than the user double-clicking into QuickTime.
            "mp4" | "mov" | "m4v" | "mkv" | "webm" | "avi" => Self::Video,
            // Common audio formats. v3.4 audio-player is a
            // command-bridge POC — actual decode is shelled to `ffplay`
            // via `RunInLinkedTerminal`. All recognised extensions route
            // identically.
            "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" | "opus" => Self::Audio,
            _ => Self::Other,
        }
    }

    /// Manifest id of the in-app player for this media kind. `None`
    /// means "fall through to the system default opener".
    pub(crate) fn player_app_id(self) -> Option<&'static str> {
        match self {
            Self::Video => Some("video-player"),
            Self::Audio => Some("audio-player"),
            Self::Other => None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Entry {
    pub name: String,
    pub path: std::path::PathBuf,
    pub is_dir: bool,
    pub is_image: bool,
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortMode {
    RecentlyTouched,
    Name,
}

#[derive(Clone, Copy)]
pub(crate) struct DirStats {
    pub file_count: usize,
    pub dir_count: usize,
    pub total_bytes: u64,
    pub truncated: bool,
}

pub(crate) fn format_size(bytes: Option<u64>) -> String {
    match bytes {
        None => "\u{2014}".to_string(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        Some(b) if b < 1024 * 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        Some(b) => format!("{:.1} GB", b as f64 / (1024.0 * 1024.0 * 1024.0)),
    }
}

pub(crate) fn format_modified(modified: Option<SystemTime>) -> String {
    let Some(modified) = modified else {
        return "\u{2014}".to_string();
    };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
        return "\u{2014}".to_string();
    };
    let secs = elapsed.as_secs();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 86400 * 7 {
        format!("{}d ago", secs / 86400)
    } else if secs < 86400 * 30 {
        format!("{}w ago", secs / (86400 * 7))
    } else {
        format!("{}mo ago", secs / (86400 * 30))
    }
}

#[cfg(test)]
mod media_bridge_tests {
    //! GUI↔Terminal media bridge routing (#79). The file browser opens
    //! recognised media in the in-app players instead of shelling out to
    //! the system default — these tests pin the classifier so a renamed
    //! manifest id or a dropped extension surfaces here, not in a smoke
    //! test halfway through release.
    use super::MediaKind;
    use std::path::PathBuf;

    #[test]
    fn open_video_file_routes_to_video_player_app() {
        for ext in &["mp4", "MP4", "mov", "mkv", "webm", "m4v", "avi"] {
            let p = PathBuf::from(format!("/tmp/clip.{ext}"));
            assert_eq!(
                MediaKind::for_path(&p),
                MediaKind::Video,
                "extension {ext} should classify as Video"
            );
            assert_eq!(
                MediaKind::for_path(&p).player_app_id(),
                Some("video-player"),
                "video routes to video-player app"
            );
        }
    }

    #[test]
    fn open_audio_file_routes_to_audio_player_app() {
        for ext in &["mp3", "MP3", "wav", "flac", "m4a", "aac", "ogg", "opus"] {
            let p = PathBuf::from(format!("/tmp/song.{ext}"));
            assert_eq!(
                MediaKind::for_path(&p),
                MediaKind::Audio,
                "extension {ext} should classify as Audio"
            );
            assert_eq!(
                MediaKind::for_path(&p).player_app_id(),
                Some("audio-player"),
                "audio routes to audio-player app"
            );
        }
    }

    #[test]
    fn unrecognized_extension_falls_back_to_system_open() {
        for path in &[
            "/tmp/notes.txt",
            "/tmp/photo.tiff",
            "/tmp/archive.zip",
            "/tmp/Makefile",
            "/tmp/.dotfile",
            "/tmp/no-extension",
        ] {
            let p = PathBuf::from(path);
            assert_eq!(
                MediaKind::for_path(&p),
                MediaKind::Other,
                "{path} should fall through to system default"
            );
            assert_eq!(
                MediaKind::for_path(&p).player_app_id(),
                None,
                "{path} should not route to an in-app player"
            );
        }
    }
}
