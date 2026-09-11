//! Value formatters shared by every surface that shows the same quantity to a
//! human — the CLI, the file browser, and the host UI.

/// A byte count as a human-readable size: `512 B`, `2.0 KB`, `3.0 MB`, `1.4 GB`.
/// Thresholds are 1024-based and every scaled unit carries one decimal.
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::human_size;

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
