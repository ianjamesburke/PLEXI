use std::time::SystemTime;

#[derive(Clone)]
pub(crate) struct Entry {
    pub name: String,
    pub path: std::path::PathBuf,
    pub is_dir: bool,
    pub is_image: bool,
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
