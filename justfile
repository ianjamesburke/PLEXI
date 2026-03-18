# Plexi dev recipes

# Dev mode — Tauri serves frontend directly from src/
dev:
    npx tauri dev

# Build production app (DMG + app bundle)
build:
    npx tauri build

# Run e2e tests (requires dev server running separately)
test:
    npx playwright test

# Open the built app
open:
    open src-tauri/target/release/bundle/macos/plexi.app

# Build and install to /Applications
release-local:
    npx tauri build
    killall Plexi || true
    cp -r src-tauri/target/release/bundle/macos/Plexi.app /Applications/
    open /Applications/Plexi.app
