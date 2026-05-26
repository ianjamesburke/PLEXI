#!/usr/bin/env python3
import plexi

emit = plexi.Emitter()
emit.info("startup-message demo: initialized")

for frame in plexi.frames():
    with frame.canvas() as canvas:
        canvas.text(10, 10, "Startup Message Demo", size=18)
        canvas.text(10, 40, "Check the companion terminal — it should show:", size=13)
        canvas.text(10, 60, "  Starting Startup Message Demo…", size=13)
        canvas.text(10, 90, "This message was written by the host on launch", size=11)
        canvas.text(10, 108, "via [launch] startup_message in manifest.toml.", size=11)
