import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  timeout: 30000,
  webServer: {
    command: 'python3 -m http.server 4173 --directory .',
    port: 4173,
    reuseExistingServer: true,
  },
  use: {
    headless: true,
  },
});
