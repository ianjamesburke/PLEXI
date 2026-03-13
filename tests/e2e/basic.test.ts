import { test, expect } from '@playwright/test';
import { join } from 'path';

test('Plexi UI renders correctly', async ({ page }) => {
  // Load the HTML file directly
  const htmlPath = join(__dirname, '../../src/mainview/index.html');
  await page.goto(`file://${htmlPath}`);
  
  // Verify title
  await expect(page).toHaveTitle('Plexi');
  
  // Verify main heading
  const heading = page.locator('h1');
  await expect(heading).toHaveText('Plexi');
  
  // Verify subtitle
  const subtitle = page.locator('.subtitle');
  await expect(subtitle).toContainText('Infinite 2D Canvas');
  
  // Verify canvas placeholder
  const canvas = page.locator('#canvas');
  await expect(canvas).toBeVisible();
  
  // Screenshot for verification
  await page.screenshot({ path: 'tests/e2e/screenshot.png', fullPage: true });
  
  console.log('Screenshot saved to tests/e2e/screenshot.png');
});
