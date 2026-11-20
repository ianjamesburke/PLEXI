# Plexi raw input hook patch

Source: crates.io egui_kittest 0.34.3, with the upstream MIT and Apache licenses.

eframe calls `App::raw_input_hook` before every egui pass
(`EpiIntegration::update` in `deps/eframe-0.34.3/src/native/epi_integration.rs`).
Upstream kittest skips it, so an eframe app that routes input in the hook sees a
different delivery in tests than in production. This copy calls the hook for an
eframe app before each pass: the constructor's bootstrap pass and every
`Harness::_step`.

The only other change types the cursor stroke width as `1.0_f32`: as a path
dependency the crate builds under the repository's `-D warnings` (no registry
`--cap-lints`), where `float_literal_f32_fallback` is an error.

Witness: `kittest_passes_go_through_raw_input_hook` in Plexi's `src/ui_tests`.
Upstream candidate: both hook calls in `src/lib.rs`.
