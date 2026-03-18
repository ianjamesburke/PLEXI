# Plexi dev recipes

# Dev mode — Tauri serves frontend directly from src/
dev:
    npx tauri dev

# Build production app (DMG + app bundle)
build:
    npx tauri build

# Run Playwright e2e tests (mock session, browser only)
test:
    npx playwright test

# Run Playwright e2e tests with 4x timeouts for slow machines
test-slow:
    SLOW=1 npx playwright test

# Run Tauri backend tests (Rust unit + integration)
test-tauri:
    cd src-tauri && cargo test

# Build debug binary with webdriver support and run binary e2e tests
test-binary:
    cd src-tauri && cargo build --features webdriver
    npx @wdio/cli run wdio.conf.ts

# Run binary e2e tests only (assumes debug binary already built)
test-binary-run:
    npx @wdio/cli run wdio.conf.ts

# Run all tests (Rust + Playwright + binary e2e)
test-all: test-tauri test test-binary

# Open the built app
open:
    open src-tauri/target/release/bundle/macos/plexi.app

# Build and install to /Applications
release-local:
    npx tauri build
    killall Plexi || true
    cp -r src-tauri/target/release/bundle/macos/Plexi.app /Applications/
    open /Applications/Plexi.app
