import { expect, test } from "@playwright/test";

const MOD = process.platform === "darwin" ? "Meta" : "Control";

test.describe("Tauri Plexi", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 960 });
    await page.goto("/mainview/");

    // Wait for app to initialize (async init)
    await page.waitForSelector("#app-shell", { timeout: 10000 });
    await page.waitForFunction(
      () => document.querySelectorAll("#context-list > li").length > 0,
      { timeout: 10000 },
    );
  });

  test("page loads and shows title", async ({ page }) => {
    await expect(page).toHaveTitle("Plexi");
    await expect(page.locator(".app-title")).toHaveText("Plexi");
  });

  test("app shell renders with sidebar and workspace", async ({ page }) => {
    await expect(page.locator(".app-shell")).toBeVisible();
    await expect(page.locator(".sidebar")).toBeVisible();
    await expect(page.locator(".workspace-shell")).toBeVisible();
    await expect(page.locator(".workspace-toolbar")).toBeVisible();
  });

  test("empty shell shows when no panels open", async ({ page }) => {
    await expect(page.locator("#empty-shell")).toBeVisible();
    await expect(page.locator(".app-title")).toHaveText("Plexi");
  });

  test("context list shows at least one context", async ({ page }) => {
    const contextItems = await page.locator("#context-list > li").count();
    expect(contextItems).toBeGreaterThanOrEqual(1);
  });

  test("toolbar shows context name", async ({ page }) => {
    const contextLabel = page.locator("#toolbar-context");
    await expect(contextLabel).toBeVisible();
    const text = await contextLabel.textContent();
    expect(text).toBeTruthy();
  });

  test("sidebar can be toggled", async ({ page }) => {
    const appShell = page.locator(".app-shell");

    // Initially visible
    await expect(page.locator(".sidebar")).toBeVisible();

    // Toggle off (Cmd+B)
    await page.keyboard.press(`${MOD}+b`);
    await expect(appShell).toHaveClass(/sidebar-hidden/);

    // Toggle on
    await page.keyboard.press(`${MOD}+b`);
    await expect(appShell).not.toHaveClass(/sidebar-hidden/);
  });

  test("minimap is visible in sidebar", async ({ page }) => {
    const minimap = page.locator("#minimap");
    await expect(minimap).toBeVisible();

    const minimapSize = page.locator("#minimap-size");
    await expect(minimapSize).toBeVisible();
  });

  test("keyboard shortcuts overlay shows", async ({ page }) => {
    const overlay = page.locator("#shortcuts-overlay");

    // Initially hidden
    await expect(overlay).toHaveClass(/is-hidden/);

    // Show shortcuts (Cmd+/)
    await page.keyboard.press(`${MOD}+/`);
    await expect(overlay).not.toHaveClass(/is-hidden/);

    // Hide shortcuts
    await page.keyboard.press(`${MOD}+/`);
    await expect(overlay).toHaveClass(/is-hidden/);
  });

  test("layout is responsive to viewport resize", async ({ page }) => {
    // Narrow viewport
    await page.setViewportSize({ width: 800, height: 600 });
    await page.waitForTimeout(500);

    await expect(page.locator(".app-shell")).toBeVisible();
    await expect(page.locator(".workspace-shell")).toBeVisible();

    // Wide viewport
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.waitForTimeout(500);

    await expect(page.locator(".app-shell")).toBeVisible();
    await expect(page.locator(".workspace-shell")).toBeVisible();
  });

  test("page has no console errors", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => {
      errors.push(error.message);
    });

    await page.waitForTimeout(2000);

    // Filter out expected/non-critical errors
    const criticalErrors = errors.filter(
      (e) =>
        !e.includes("Failed to initialize"),
    );

    expect(criticalErrors).toEqual([]);
  });
});
