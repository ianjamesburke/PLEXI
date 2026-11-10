# Plexi screenshot patch

Source: crates.io egui-wgpu 0.34.3, with the upstream MIT and Apache licenses.

`Painter::paint_and_update_textures_with_presentation` allows the existing paint
path to target a capture texture without acquiring or presenting a window
drawable. It uses the live renderer, window backing dimensions, texture format,
MSAA/depth attachments, texture updates and paint callbacks. `CaptureState`
can copy only to the CPU readback buffer, omitting the surface copy.

The original presentation method delegates with presentation enabled. Plexi's
`screenshot_gpu_readback_without_compositor_drawable` regression checks real GPU
readback without a surface, row padding, and freshness when reusing the capture
texture. Live host validation remains a separate requirement.
