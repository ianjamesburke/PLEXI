# Plexi screenshot patch

Source: crates.io eframe 0.34.3, with the upstream MIT and Apache licenses.

`native::wgpu_integration` treats a queued screenshot as a request for a fresh
UI pass even when the viewport is occluded. The original visibility observations
remain in raw input; screenshot rendering does not deliver queued hidden-window
input or invoke the first-frame window-show hook. The renderer captures without
presenting, using the companion egui-wgpu patch. Ordinary hidden passes remain
logic-only.

The `screenshot_tests` regression exercises the production paint predicate.
Run from the workspace with `cargo test -p eframe --no-default-features
--features wgpu,default_fonts screenshot_tests` through the repository Cargo lease.
