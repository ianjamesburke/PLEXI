use super::super::*;

#[test]
fn image_cache_handle_capability_denied() {
    let mut cache = image_cache::ImageCache::new();
    let handle = "test-handle-cap-denied";
    // net_http_granted = false → immediate error completion
    cache.request_by_handle(handle, "https://example.com/img.png", false);

    let completions = cache.poll(&egui::Context::default());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].0, handle);
    assert!(
        completions[0].1.is_err(),
        "expected error for denied capability"
    );
    assert!(matches!(
        cache.state(handle),
        Some(image_cache::CachedImage::Error(_))
    ));
}

#[test]
fn image_cache_handle_poll_returns_completed() {
    // Write a 1×1 red PNG — reuse the file-fetch path to avoid needing a real HTTP server.
    // We'll use request() and verify that handle-based requests appear in poll() return.
    // Since request_by_handle only does HTTP, test the state machine via capability-denied path.
    let mut cache = image_cache::ImageCache::new();
    let handle = "aaaaaaaa-0000-0000-0000-000000000001";
    cache.request_by_handle(handle, "https://example.com/any.png", false);

    // No sleep needed — immediate_completions populated synchronously.
    let completions = cache.poll(&egui::Context::default());
    assert!(
        completions.iter().any(|(h, _)| h == handle),
        "poll() must include handle completions"
    );
}

#[test]
fn image_cache_loads_png() {
    // Write a 1×1 red PNG to a temp dir using the `image` crate (avoids
    // embedding raw bytes that could be subtly invalid).
    let dir = tempfile::tempdir().expect("tempdir");
    let png_path = dir.path().join("test.png");
    let mut img = image::RgbaImage::new(1, 1);
    img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    img.save(&png_path).expect("save png");

    let mut cache = image_cache::ImageCache::new();
    cache.request("test.png", dir.path());

    // Give the background thread time to load.
    std::thread::sleep(std::time::Duration::from_millis(200));
    cache.poll(&egui::Context::default());

    assert!(
        cache.get("test.png").is_some(),
        "expected image to be loaded"
    );
}

#[test]
fn image_cache_missing_file_is_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cache = image_cache::ImageCache::new();
    cache.request("nonexistent.png", dir.path());

    std::thread::sleep(std::time::Duration::from_millis(200));
    cache.poll(&egui::Context::default());

    assert!(
        matches!(
            cache.state("nonexistent.png"),
            Some(image_cache::CachedImage::Error(_))
        ),
        "expected Error state for missing file"
    );
}
