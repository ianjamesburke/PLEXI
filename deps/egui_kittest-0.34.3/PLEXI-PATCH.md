# Plexi raw input hook patch

Source: crates.io egui_kittest 0.34.3, with the upstream MIT and Apache licenses.

`Harness::_step` calls `eframe::App::raw_input_hook` before running the egui
pass for an eframe app, as eframe's native integration does. Upstream skips the
hook, so an app that routes input in the hook (Plexi's host input router) would
see a different delivery in tests than in production.

Upstream candidate: the one-line hook call in `Harness::_step`.
