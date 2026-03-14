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
  await expect(page.locator("#focus-title")).toHaveText("Terminal 1");
  await expect(page.locator("#toolbar-context")).toHaveText("Main");
  await expect(page.locator(".xterm")).toBeVisible();
  await expect(page.locator("#engine-label")).toContainText("xterm.js ready");
  await expect(page.locator("#workspace-storage-label")).toHaveText("Browser");
  await expect(page.locator("#workspace-path")).toHaveText("Browser storage");
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

  await page.keyboard.type("cd /mock/project/nested");
  await page.keyboard.press("Enter");
  await expect(page.locator("#focus-path")).toHaveText("/mock/project/nested");

  await page.keyboard.press("Control+N");
  await expect(page.locator("#toast-layer")).toContainText("Terminal 2 created to the right");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 2");
  await expect(page.locator("#focus-position")).toHaveText("1, 0");
  await expect(page.locator("#focus-path")).toHaveText("/mock/project/nested");

  await page.keyboard.press("Control+N");
  await expect(page.locator("#toast-layer")).toContainText("Terminal 3 created to the right");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 3");
  await expect(page.locator("#focus-position")).toHaveText("2, 0");

  await page.keyboard.press("ArrowLeft");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 3");
  await expect(page.locator("#terminal-mount")).not.toContainText("[D");

  await page.keyboard.press("Control+ArrowLeft");
  await expect(page.locator("#toast-layer")).toContainText("Focused Terminal 2");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 2");
  await expect(page.locator("#terminal-mount")).not.toContainText("[D");

  await page.keyboard.press("Control+ArrowLeft");
  await expect(page.locator("#toast-layer")).toContainText("Focused Terminal 1");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 1");
  await expect(page.locator("#terminal-mount")).not.toContainText("[D");

  await page.keyboard.press("Control+Shift+N");
  await expect(page.locator("#toast-layer")).toContainText("created below");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 4");
  await expect(page.locator("#focus-position")).toHaveText("0, 1");

  await page.keyboard.press("Control+ArrowRight");
  await page.keyboard.press("Control+ArrowRight");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 3");
  await expect(page.locator("#focus-position")).toHaveText("2, 0");

  await page.keyboard.press("Control+ArrowDown");
  await expect(page.locator("#toast-layer")).toContainText("Focused Terminal 4");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 4");
  await expect(page.locator("#focus-position")).toHaveText("0, 1");

  await page.keyboard.press("Control+ArrowUp");
  await expect(page.locator("#toast-layer")).toContainText("Focused Terminal 3");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 3");
  await expect(page.locator("#focus-position")).toHaveText("2, 0");

  await page.locator("#new-context").click();
  await expect(page.locator("#context-modal")).toBeVisible();
  await page.locator("#context-name-input").fill("Frontend Dev");
  await page.locator("#context-form").evaluate((form) => form.requestSubmit());
  await page.keyboard.press("Control+2");
  await expect(page.locator("#toolbar-context")).toHaveText("Frontend Dev");
  await expect(page.locator("#focus-title")).toHaveText("No active terminal");

  await page.keyboard.press("Control+1");
  await expect(page.locator("#toolbar-context")).toHaveText("Main");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 3");

  await page.locator("#toggle-workspace-json").click();
  await expect(page.locator("#workspace-json-shell")).toBeVisible();
  await expect(page.locator("#workspace-json")).toHaveValue(/"contexts"/);
  await expect(page.locator("#workspace-json")).toHaveValue(/"label": "Main"/);

  await page.keyboard.press("Control+Shift+O");
  await expect(page.locator("#mode-label")).toHaveText("Overview");
  await expect(page.locator(".overview-node")).toHaveCount(4);

  await page.keyboard.press("Control+Shift+O");
  await expect(page.locator("#mode-label")).toHaveText("Focus");

  await page.keyboard.press("Control+B");
  await expect(page.locator("#app-shell")).toHaveClass(/app-shell--sidebar-hidden/);

  await page.keyboard.press("Control+B");
  await expect(page.locator("#app-shell")).not.toHaveClass(/app-shell--sidebar-hidden/);
  await expect(page.locator(".sidebar-header")).toHaveClass(/electrobun-webkit-app-region-drag/);
  await expect(page.locator(".workspace-toolbar")).toHaveClass(/electrobun-webkit-app-region-drag/);

  await page.setViewportSize({ width: 1040, height: 720 });
  await expect(page.locator("#engine-label")).toBeVisible();

  const viewportFit = await page.evaluate(() => {
    const header = document.querySelector(".workspace-toolbar")?.getBoundingClientRect();
    const toast = document.getElementById("toast-layer")?.getBoundingClientRect();

    return {
      headerBottom: header?.bottom ?? 0,
      toastBottom: toast?.bottom ?? 0,
      viewportHeight: window.innerHeight,
    };
  });

  const viewportTolerance = 4;
  expect(viewportFit.headerBottom).toBeLessThanOrEqual(viewportFit.viewportHeight + viewportTolerance);
  expect(viewportFit.toastBottom).toBeLessThanOrEqual(viewportFit.viewportHeight + viewportTolerance);

  await page.keyboard.press("Control+W");
  await expect(page.locator("#toast-layer")).toContainText("Terminal 3 closed");
  await expect(page.locator("#minimap-size")).toHaveText("3 terminals");

  await page.keyboard.press("Control+W");
  await page.keyboard.press("Control+W");
  await page.keyboard.press("Control+W");
  await expect(page.locator("#focus-title")).toHaveText("Terminal 5");
  await expect(page.locator("#terminal-mount")).not.toContainText("%");

  await page.screenshot({ path: "tests/e2e/screenshot.png", fullPage: true });
});
