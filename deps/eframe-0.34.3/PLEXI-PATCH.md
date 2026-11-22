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

The vendored copy also updates renamed lints and obsolete lint expectations for
the repository toolchain; eframe reuses its existing viewport callback alias.

# Live-resize backport (stint 1224)

`WgpuWinitRunning::on_window_event` carries upstream egui #8229 (shipped in
eframe 0.35): `Resized` and, on macOS, `Moved` keep the resize state active.
Before it, every `Resized` ended and restarted the state, so each size step
reconfigured the surface three times and toggled `presentsWithTransaction`.
Drop this hunk when upgrading to eframe 0.35 or later.

Plexi diverges from #8229 in two ways, because upstream can leave the surface
in resize mode after the drag. A zero-size `Resized` (the Windows minimize
signal) ends the state. `run_ui_and_paint` ends it once `LIVE_RESIZE_QUIET`
passes with no real size step, and schedules its own wake, which covers a final
resize followed only by redraws and a same-scale display move. Re-check both
paths against upstream before dropping this patch.
