import { expect, test } from "@playwright/test";

test("Plexi keyboard-first terminal workspace flows correctly", async ({ page }) => {
  const pageErrors = [];
  page.on("pageerror", (error) => {
    pageErrors.push(error.message);
  });

  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("http://127.0.0.1:4173/build/dev-macos-arm64/plexi-dev.app/Contents/Resources/app/views/mainview/index.html");

  await expect(page).toHaveTitle("Plexi");
  await expect(page.locator(".app-title")).toHaveText("Plexi");
  await expect(page.locator("#status-panels")).toHaveText("1 terminal");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 1");
  await expect(page.locator(".xterm")).toBeVisible();
  await expect(page.locator("#engine-label")).toContainText("xterm.js ready");
  expect(pageErrors).toEqual([]);

  const terminalProfile = await page.evaluate(() => window.__PLEXI_DEBUG__.getTerminalProfile());
  expect(terminalProfile.fontFamily).toContain("Plexi Terminal");
  expect(terminalProfile.convertEol).toBe(false);
  expect(terminalProfile.letterSpacing).toBe(0);
  expect(terminalProfile.lineHeight).toBe(1);
  await expect.poll(() => page.evaluate(() => document.fonts.check('14px "Plexi Terminal"'))).toBe(true);

  await page.keyboard.type("stuck");
  await page.keyboard.press("Control+C");
  await expect(page.locator("#terminal-mount")).toContainText("^C");

  await page.keyboard.type("help");
  await page.keyboard.press("Enter");
  await expect(page.locator("#terminal-mount")).toContainText("split-right");

  await page.keyboard.press("Control+N");
  await expect(page.locator("#status-ready")).toContainText("Terminal 2 created to the right");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 2");
  await expect(page.locator("#status-position")).toHaveText("1, 0");

  await page.keyboard.press("ArrowLeft");
  await expect(page.locator("#status-ready")).toContainText("Terminal 2 created to the right");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 2");

  await page.keyboard.press("Control+ArrowLeft");
  await expect(page.locator("#status-ready")).toContainText("Focused Terminal 1");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 1");

  await page.keyboard.press("Control+Shift+N");
  await expect(page.locator("#status-ready")).toContainText("created below");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 3");
  await expect(page.locator("#status-position")).toHaveText("0, 1");

  await page.keyboard.press("Control+2");
  await expect(page.locator("#status-context")).toHaveText("Context: Frontend Dev");
  await expect(page.locator("#focus-title")).toHaveText("No active terminal");

  await page.keyboard.press("Control+1");
  await expect(page.locator("#status-context")).toHaveText("Context: Default");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 3");

  await page.keyboard.press("Control+Shift+O");
  await expect(page.locator("#mode-label")).toHaveText("Overview");
  await expect(page.locator(".overview-node")).toHaveCount(3);

  await page.keyboard.press("Control+Shift+O");
  await expect(page.locator("#mode-label")).toHaveText("Focus");

  await page.keyboard.press("Control+B");
  await expect(page.locator("#app-shell")).toHaveClass(/app-shell--sidebar-hidden/);

  await page.keyboard.press("Control+B");
  await expect(page.locator("#app-shell")).not.toHaveClass(/app-shell--sidebar-hidden/);
  await expect(page.locator(".sidebar-header")).toHaveClass(/electrobun-webkit-app-region-drag/);
  await expect(page.locator(".workspace-toolbar")).toHaveClass(/electrobun-webkit-app-region-drag/);

  await page.setViewportSize({ width: 1040, height: 720 });
  await expect(page.locator(".status-bar")).toBeVisible();
  await expect(page.locator("#engine-label")).toBeVisible();

  const viewportFit = await page.evaluate(() => {
    const active = document.getElementById("active-label")?.getBoundingClientRect();
    const status = document.querySelector(".status-bar")?.getBoundingClientRect();

    return {
      activeBottom: active?.bottom ?? 0,
      statusBottom: status?.bottom ?? 0,
      viewportHeight: window.innerHeight,
    };
  });

  const viewportTolerance = 4;
  expect(viewportFit.activeBottom).toBeLessThanOrEqual(viewportFit.viewportHeight + viewportTolerance);
  expect(viewportFit.statusBottom).toBeLessThanOrEqual(viewportFit.viewportHeight + viewportTolerance);

  await page.keyboard.press("Control+W");
  await expect(page.locator("#status-ready")).toContainText("Terminal 3 closed");
  await expect(page.locator("#status-panels")).toHaveText("2 terminals");

  await page.screenshot({ path: "tests/e2e/screenshot.png", fullPage: true });
});
