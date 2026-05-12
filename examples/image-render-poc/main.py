#!/usr/bin/env python3
"""POC for RenderCommand::Image (#1144).

Shows two images side by side:
  - Left:  logo.png (real image, must exist next to this script)
  - Right: missing.png (non-existent — tests the placeholder / error state)

Run via `plexi-alpha open image-render-poc` from inside the examples dir,
or register the example dir as a Plexi app dir and open it from there.
"""

from plexi_sdk import context

BG = "#1a1a24"

with context() as ctx:
    ctx.clear(BG)

    # Left: real image
    ctx.image("logo.png", 20, 20, 200, 200)
    ctx.text("logo.png (contain)", 20, 230, size=12, color="#888888")

    # Right: missing image — should show placeholder with error text
    ctx.image("missing.png", 240, 20, 200, 200)
    ctx.text("missing.png (placeholder)", 240, 230, size=12, color="#888888")
