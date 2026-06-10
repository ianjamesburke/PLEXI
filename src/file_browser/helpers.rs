use std::cmp::Ordering;
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
    pub created: Option<SystemTime>,
    pub extension: Option<String>,
    pub permissions: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnId {
    Name,
    Kind,
    Size,
    Modified,
    Created,
    Extension,
    Permissions,
    Tags,
}

impl ColumnId {
    pub(crate) const ALL: [Self; 8] = [
        Self::Name,
        Self::Kind,
        Self::Size,
        Self::Modified,
        Self::Created,
        Self::Extension,
        Self::Permissions,
        Self::Tags,
    ];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Kind => "kind",
            Self::Size => "size",
            Self::Modified => "modified",
            Self::Created => "created",
            Self::Extension => "extension",
            Self::Permissions => "permissions",
            Self::Tags => "tags",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Kind => "Kind",
            Self::Size => "Size",
            Self::Modified => "Modified",
            Self::Created => "Created",
            Self::Extension => "Ext",
            Self::Permissions => "Perms",
            Self::Tags => "Tags",
        }
    }

    pub(crate) fn default_width(self) -> f32 {
        match self {
            Self::Name => 260.0,
            Self::Kind => 92.0,
            Self::Size => 96.0,
            Self::Modified => 116.0,
            Self::Created => 116.0,
            Self::Extension => 72.0,
            Self::Permissions => 76.0,
            Self::Tags => 120.0,
        }
    }

    pub(crate) fn min_width(self) -> f32 {
        match self {
            Self::Name => 160.0,
            Self::Kind => 72.0,
            Self::Size => 72.0,
            Self::Modified | Self::Created => 92.0,
            Self::Extension | Self::Permissions => 60.0,
            Self::Tags => 88.0,
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "name" => Some(Self::Name),
            "kind" => Some(Self::Kind),
            "size" => Some(Self::Size),
            "modified" => Some(Self::Modified),
            "created" => Some(Self::Created),
            "extension" => Some(Self::Extension),
            "permissions" => Some(Self::Permissions),
            "tags" => Some(Self::Tags),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "asc" => Some(Self::Asc),
            "desc" => Some(Self::Desc),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SortDescriptor {
    pub column: ColumnId,
    pub direction: SortDirection,
}

impl Default for SortDescriptor {
    fn default() -> Self {
        Self {
            column: ColumnId::Modified,
            direction: SortDirection::Desc,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ColumnConfig {
    pub id: ColumnId,
    pub width: f32,
    pub visible: bool,
}

impl ColumnConfig {
    pub(crate) fn new(id: ColumnId, visible: bool) -> Self {
        Self {
            id,
            width: id.default_width(),
            visible,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ColumnModel {
    pub columns: Vec<ColumnConfig>,
    pub sort: SortDescriptor,
    pub folders_on_top: bool,
}

impl Default for ColumnModel {
    fn default() -> Self {
        Self {
            columns: vec![
                ColumnConfig::new(ColumnId::Name, true),
                ColumnConfig::new(ColumnId::Kind, true),
                ColumnConfig::new(ColumnId::Size, true),
                ColumnConfig::new(ColumnId::Modified, true),
                ColumnConfig::new(ColumnId::Created, false),
                ColumnConfig::new(ColumnId::Extension, false),
                ColumnConfig::new(ColumnId::Permissions, false),
                ColumnConfig::new(ColumnId::Tags, false),
            ],
            sort: SortDescriptor::default(),
            folders_on_top: true,
        }
    }
}

impl ColumnModel {
    pub(crate) fn visible_columns(&self) -> impl Iterator<Item = &ColumnConfig> {
        self.columns.iter().filter(|column| column.visible)
    }

    pub(crate) fn visible_column_count(&self) -> usize {
        self.columns.iter().filter(|column| column.visible).count()
    }

    pub(crate) fn set_column_visible(&mut self, id: ColumnId, visible: bool) {
        if id == ColumnId::Name {
            return;
        }
        if let Some(column) = self.columns.iter_mut().find(|column| column.id == id) {
            column.visible = visible;
        }
        if self.visible_column_count() == 0 {
            if let Some(name) = self
                .columns
                .iter_mut()
                .find(|column| column.id == ColumnId::Name)
            {
                name.visible = true;
            }
        }
    }

    pub(crate) fn resize_column(&mut self, id: ColumnId, width: f32) {
        if let Some(column) = self.columns.iter_mut().find(|column| column.id == id) {
            column.width = width.max(id.min_width());
        }
    }

    pub(crate) fn move_column(&mut self, id: ColumnId, offset: isize) {
        let Some(index) = self.columns.iter().position(|column| column.id == id) else {
            return;
        };
        let new_index = if offset.is_negative() {
            index.saturating_sub(offset.unsigned_abs())
        } else {
            (index + offset as usize).min(self.columns.len().saturating_sub(1))
        };
        if index != new_index {
            self.columns.swap(index, new_index);
        }
    }

    pub(crate) fn toggle_sort(&mut self, column: ColumnId) {
        if self.sort.column == column {
            self.sort.direction = self.sort.direction.toggled();
        } else {
            self.sort = SortDescriptor {
                column,
                direction: default_direction_for(column),
            };
        }
    }

    pub(crate) fn toggle_legacy_sort(&mut self) {
        self.sort = match (self.sort.column, self.sort.direction) {
            (ColumnId::Name, SortDirection::Asc) => SortDescriptor::default(),
            _ => SortDescriptor {
                column: ColumnId::Name,
                direction: SortDirection::Asc,
            },
        };
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "sort": {
                "column": self.sort.column.key(),
                "direction": self.sort.direction.key(),
            },
            "folders_on_top": self.folders_on_top,
            "columns": self.columns.iter().map(|column| {
                serde_json::json!({
                    "id": column.id.key(),
                    "width": column.width,
                    "visible": column.visible,
                })
            }).collect::<Vec<_>>(),
        })
    }

    pub(crate) fn from_json(value: &serde_json::Value) -> Self {
        let mut model = Self::default();
        if let Some(column) = value["sort"]["column"]
            .as_str()
            .and_then(ColumnId::from_key)
        {
            model.sort.column = column;
        }
        if let Some(direction) = value["sort"]["direction"]
            .as_str()
            .and_then(SortDirection::from_key)
        {
            model.sort.direction = direction;
        }
        if let Some(folders_on_top) = value["folders_on_top"].as_bool() {
            model.folders_on_top = folders_on_top;
        }
        if let Some(columns) = value["columns"].as_array() {
            let mut restored = Vec::new();
            for item in columns {
                let Some(id) = item["id"].as_str().and_then(ColumnId::from_key) else {
                    continue;
                };
                if restored.iter().any(|column: &ColumnConfig| column.id == id) {
                    continue;
                }
                restored.push(ColumnConfig {
                    id,
                    width: item["width"]
                        .as_f64()
                        .map(|width| width as f32)
                        .unwrap_or_else(|| id.default_width())
                        .max(id.min_width()),
                    visible: item["visible"].as_bool().unwrap_or(true),
                });
            }
            for id in ColumnId::ALL {
                if !restored.iter().any(|column| column.id == id) {
                    restored.push(ColumnConfig::new(id, id == ColumnId::Name));
                }
            }
            if !restored.iter().any(|column| column.visible) {
                if let Some(name) = restored
                    .iter_mut()
                    .find(|column| column.id == ColumnId::Name)
                {
                    name.visible = true;
                }
            }
            model.columns = restored;
        }
        model
    }
}

#[derive(Clone, Copy)]
pub(crate) struct DirStats {
    pub file_count: usize,
    pub dir_count: usize,
    pub total_bytes: u64,
    pub truncated: bool,
}

fn default_direction_for(column: ColumnId) -> SortDirection {
    match column {
        ColumnId::Name
        | ColumnId::Kind
        | ColumnId::Extension
        | ColumnId::Permissions
        | ColumnId::Tags => SortDirection::Asc,
        ColumnId::Size | ColumnId::Modified | ColumnId::Created => SortDirection::Desc,
    }
}

fn kind_value(entry: &Entry) -> &'static str {
    if entry.is_dir {
        "folder"
    } else if entry.is_image {
        "image"
    } else {
        "file"
    }
}

pub(crate) fn sort_entries(entries: &mut [Entry], sort: SortDescriptor, folders_on_top: bool) {
    entries.sort_by(|a, b| {
        if folders_on_top {
            let folder_order = b.is_dir.cmp(&a.is_dir);
            if folder_order != Ordering::Equal {
                return folder_order;
            }
        }
        let primary = compare_entry_column(a, b, sort.column);
        let directed_primary = match sort.direction {
            SortDirection::Asc => primary,
            SortDirection::Desc => primary.reverse(),
        };
        directed_primary.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn compare_entry_column(a: &Entry, b: &Entry, column: ColumnId) -> Ordering {
    match column {
        ColumnId::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        ColumnId::Kind => kind_value(a).cmp(kind_value(b)),
        ColumnId::Size => a.size_bytes.unwrap_or(0).cmp(&b.size_bytes.unwrap_or(0)),
        ColumnId::Modified => a.modified.cmp(&b.modified),
        ColumnId::Created => a.created.cmp(&b.created),
        ColumnId::Extension => a.extension.cmp(&b.extension),
        ColumnId::Permissions => a.permissions.cmp(&b.permissions),
        ColumnId::Tags => a.tags.join(",").cmp(&b.tags.join(",")),
    }
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

#[cfg(test)]
mod column_model_tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn entry(name: &str, is_dir: bool, size_bytes: Option<u64>, modified_secs: u64) -> Entry {
        let path = PathBuf::from(name);
        Entry {
            name: name.to_string(),
            extension: path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_ascii_lowercase()),
            path,
            is_dir,
            is_image: false,
            size_bytes,
            modified: Some(UNIX_EPOCH + Duration::from_secs(modified_secs)),
            created: None,
            permissions: "rw".to_string(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn sort_entries_uses_descriptor_and_keeps_folders_on_top() {
        let mut entries = vec![
            entry("tiny.txt", false, Some(1), 10),
            entry("z-folder", true, None, 1),
            entry("large.bin", false, Some(900), 20),
        ];
        sort_entries(
            &mut entries,
            SortDescriptor {
                column: ColumnId::Size,
                direction: SortDirection::Desc,
            },
            true,
        );
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["z-folder", "large.bin", "tiny.txt"]);
    }

    #[test]
    fn sort_entries_can_disable_folders_on_top() {
        let mut entries = vec![
            entry("tiny.txt", false, Some(1), 10),
            entry("z-folder", true, None, 1),
            entry("large.bin", false, Some(900), 20),
        ];
        sort_entries(
            &mut entries,
            SortDescriptor {
                column: ColumnId::Name,
                direction: SortDirection::Asc,
            },
            false,
        );
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["large.bin", "tiny.txt", "z-folder"]);
    }

    #[test]
    fn descending_metadata_sort_keeps_name_tiebreaker_ascending() {
        let mut entries = vec![
            entry("zeta.txt", false, Some(10), 20),
            entry("alpha.txt", false, Some(10), 20),
            entry("middle.txt", false, Some(1), 10),
        ];
        sort_entries(
            &mut entries,
            SortDescriptor {
                column: ColumnId::Modified,
                direction: SortDirection::Desc,
            },
            true,
        );
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["alpha.txt", "zeta.txt", "middle.txt"]);
    }

    #[test]
    fn column_model_round_trips_preferences() {
        let mut model = ColumnModel::default();
        model.toggle_sort(ColumnId::Extension);
        model.resize_column(ColumnId::Name, 333.0);
        model.set_column_visible(ColumnId::Created, true);
        model.move_column(ColumnId::Created, -1);
        model.folders_on_top = false;

        let restored = ColumnModel::from_json(&model.to_json());

        assert_eq!(restored.sort.column, ColumnId::Extension);
        assert_eq!(restored.sort.direction, SortDirection::Asc);
        assert!(!restored.folders_on_top);
        assert_eq!(
            restored
                .columns
                .iter()
                .find(|column| column.id == ColumnId::Name)
                .map(|column| column.width),
            Some(333.0)
        );
        assert_eq!(
            restored
                .columns
                .iter()
                .find(|column| column.id == ColumnId::Created)
                .map(|column| column.visible),
            Some(true)
        );
        let created_index = restored
            .columns
            .iter()
            .position(|column| column.id == ColumnId::Created)
            .expect("created column");
        let modified_index = restored
            .columns
            .iter()
            .position(|column| column.id == ColumnId::Modified)
            .expect("modified column");
        assert!(
            created_index < modified_index,
            "column order should survive serialization"
        );
    }
}
