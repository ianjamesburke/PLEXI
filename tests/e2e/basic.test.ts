import { test, expect } from '@playwright/test';
import { join } from 'path';

test('Plexi UI renders correctly', async ({ page }) => {
  // Set viewport for consistent screenshots
  await page.setViewportSize({ width: 1400, height: 900 });
  
  // Load the HTML file directly
  const htmlPath = join(__dirname, '../../src/mainview/index.html');
  await page.goto(`file://${htmlPath}`);
  
  // Wait for fonts to load
  await page.waitForTimeout(500);
  
  // Verify title
  await expect(page).toHaveTitle('Plexi');
  
  // Verify logo
  const logo = page.locator('.logo');
  await expect(logo).toHaveText('Plexi');
  
  // Verify tagline
  const tagline = page.locator('.tagline');
  await expect(tagline).toContainText('Infinite canvas');
  
  // Verify canvas area
  const canvas = page.locator('#canvas');
  await expect(canvas).toBeVisible();
  
  // Verify sidebar contexts
  const sidebar = page.locator('.sidebar');
  await expect(sidebar).toBeVisible();
  
  // Verify status bar
  const statusBar = page.locator('.status-bar');
  await expect(statusBar).toBeVisible();
  
  // Screenshot for verification
  await page.screenshot({ path: 'tests/e2e/screenshot.png', fullPage: true });
  
  console.log('Screenshot saved to tests/e2e/screenshot.png');
});
