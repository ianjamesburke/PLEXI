import { expect, test } from "@playwright/test";

test("keyboard layout flow keeps down-splits left and compacts rows on close", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("http://127.0.0.1:4173/build/dev-macos-arm64/plexi-dev.app/Contents/Resources/app/views/mainview/index.html");
  await page.evaluate(() => window.__PLEXI_DEBUG__.reset());

  await page.keyboard.press("Control+N"); // Terminal 1 at (0, 0)
  await page.keyboard.press("Control+N"); // Terminal 2 at (1, 0)
  await page.keyboard.press("Control+N"); // Terminal 3 at (2, 0)
  await page.keyboard.press("Control+Shift+N"); // Terminal 4 should be at (0, 1)

  const getActivePanelId = () => page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId);
  expect(await getActivePanelId()).toBe("panel-4");
  

  await page.keyboard.press("Control+ArrowUp"); // back to row 0 (remembered: Terminal 3)
  await page.keyboard.press("Control+ArrowLeft"); // Terminal 2
  await page.keyboard.press("Control+ArrowLeft"); // Terminal 1
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-1");

  await page.keyboard.press("Control+W"); // close Terminal 1, row should compact left
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-2");
  

  const panelPositions = await page.evaluate(() => {
    const state = window.__PLEXI_DEBUG__.getState();
    return state.panels
      .map((panel) => ({ title: panel.title, x: panel.x, y: panel.y }))
      .sort((a, b) => a.title.localeCompare(b.title));
  });

  expect(panelPositions).toEqual([
    { title: "Terminal 2", x: 0, y: 0 },
    { title: "Terminal 3", x: 1, y: 0 },
    { title: "Terminal 4", x: 0, y: 1 },
  ]);
});

test("Plexi keyboard-first terminal workspace flows correctly", async ({ page }) => {
  const pageErrors = [];
  page.on("pageerror", (error) => {
    pageErrors.push(error.message);
  });

  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("http://127.0.0.1:4173/build/dev-macos-arm64/plexi-dev.app/Contents/Resources/app/views/mainview/index.html");
  await page.evaluate(() => window.__PLEXI_DEBUG__.reset());

  await expect(page).toHaveTitle("Plexi");
  await expect(page.locator(".app-title")).toHaveText("Plexi");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBeNull();
  await expect(page.locator("#focus-path")).toBeHidden();
  
  await expect(page.locator("#toolbar-context")).toHaveText("Context 1");
  await expect(page.locator("#empty-shell .empty-tagline")).toHaveText("Open a terminal, cd into a project and then open one beside or below it.");
  await expect(page.locator("#empty-shell")).toContainText("Open your first terminal here");
  await expect(page.locator("#empty-shell")).toContainText("Contexts live here.");
  await expect(page.locator("#workspace-storage-label")).toHaveText("Browser");
  expect(pageErrors).toEqual([]);

  await page.keyboard.press("Control+N");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-1");
  await expect(page.locator("#toolbar-context")).toHaveText("Context 1");
  await expect(page.locator("#focus-right-slot")).toBeVisible();
  await expect(page.locator("#focus-bottom-slot")).toBeVisible();
  await expect(page.locator("#focus-path")).toHaveText("~");
  await expect(page.locator(".xterm")).toBeVisible();
  await expect(page.locator("#engine-label")).toContainText("xterm.js ready");

  const terminalProfile = await page.evaluate(() => window.__PLEXI_DEBUG__.getTerminalProfile());
  expect(terminalProfile.fontFamily).toContain("Plexi Terminal");
  expect(terminalProfile.convertEol).toBe(false);
  expect(terminalProfile.letterSpacing).toBe(0);
  expect(terminalProfile.lineHeight).toBe(1);
  await expect.poll(() => page.evaluate(() => document.fonts.check('14px "Plexi Terminal"'))).toBe(true);

  await page.keyboard.type("help");
  await page.keyboard.press("Enter");
  await expect(page.locator("#terminal-mount")).toContainText("split-right");

  await page.keyboard.press("Control+Shift+N");
  await expect(page.locator("#toast-layer")).toContainText("Terminal 2 created below");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-2");
  

  await page.keyboard.press("Control+N");
  await expect(page.locator("#toast-layer")).toContainText("Terminal 3 created to the right");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-3");
  

  await page.locator('[data-rename-context-index="0"]').click();
  await expect(page.locator("#context-modal")).toBeVisible();
  await page.locator("#context-name-input").fill("Project Alpha");
  await page.locator("#context-form").evaluate((form) => form.requestSubmit());
  await expect(page.locator("#toolbar-context")).toHaveText("Project Alpha");

  await page.locator("#new-context").click();
  await expect(page.locator("#context-modal")).toBeVisible();
  await page.locator("#context-name-input").fill("Context 2");
  await page.locator("#context-form").evaluate((form) => form.requestSubmit());
  await page.keyboard.press("Control+2");
  await expect(page.locator("#toolbar-context")).toHaveText("Context 2");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBeNull();
  await expect(page.locator("#focus-path")).toBeHidden();
  

  await page.keyboard.press("Control+1");
  await expect(page.locator("#toolbar-context")).toHaveText("Project Alpha");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-3");

  await page.locator('[data-rename-context-index="1"]').click();
  await expect(page.locator("#context-modal")).toBeVisible();
  await expect(page.locator("#context-delete")).toBeVisible();

  // Double click to confirm delete
  await page.locator("#context-delete").click();
  await page.locator("#context-delete").click();
  
  await expect(page.locator("#toast-layer")).toContainText("Context Context 2 deleted");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().contexts)).toHaveLength(1);

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
  await expect(page.locator("#minimap-size")).toHaveText("2 terminals");

  await page.keyboard.press("Control+W");
  await page.keyboard.press("Control+W");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBeNull();
  await expect(page.locator("#toolbar-context")).toHaveText("Project Alpha");
  await expect(page.locator("#empty-shell")).toBeVisible();

  await page.screenshot({ path: "tests/e2e/screenshot.png", fullPage: true });
});
