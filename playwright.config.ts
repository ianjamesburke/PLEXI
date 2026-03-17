import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  timeout: 30000,
  webServer: {
    command: "npm run copy-vendor && python3 -m http.server 1415 --bind 127.0.0.1 --directory src",
    url: 'http://127.0.0.1:1415/mainview/',
    reuseExistingServer: true,
  },
  use: {
    baseURL: 'http://localhost:1415',
    headless: true,
  },
});
