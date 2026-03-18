import { defineConfig } from '@playwright/test';

const slow = !!process.env.SLOW;
const timeoutMultiplier = slow ? 4 : 1;

export default defineConfig({
  testDir: './tests/e2e',
  timeout: 30000 * timeoutMultiplier,
  expect: {
    timeout: 5000 * timeoutMultiplier,
  },
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
