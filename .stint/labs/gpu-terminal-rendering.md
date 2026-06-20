# GPU-Accelerated Terminal Rendering

Render terminal to offscreen wgpu texture instead of CPU grid-to-egui path. Eliminates the per-cell egui text layout bottleneck for large scrollback and fast output.

tags: terminal, gpu, perf
ref: #2068
