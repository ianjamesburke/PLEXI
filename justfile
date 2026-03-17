# Plexi dev recipes

# Normal dev — restores last workspace from localStorage
dev:
    npm run copy-vendor && npx tauri dev

# Fresh dev — clears saved workspace, starts with empty canvas
dev-fresh:
    npm run copy-vendor && npx tauri dev --config '{"build":{"devUrl":"http://localhost:1415/fresh.html"}}'

# Run e2e tests (requires dev server running separately)
test:
    npx playwright test
