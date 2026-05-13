use super::super::*;

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
