import { expect, test } from "@playwright/test";

test("split groups stay inside one top-level node and collapse cleanly on close", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("http://127.0.0.1:4173/build/dev-macos-arm64/plexi-dev.app/Contents/Resources/app/views/mainview/index.html");
  await page.evaluate(() => window.__PLEXI_DEBUG__.reset());

  await page.keyboard.press("Control+N");
  await page.keyboard.press("Control+N");
  await page.keyboard.press("Control+Shift+N");

  const getActivePanelId = () => page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId);
  expect(await getActivePanelId()).toBe("panel-3");

  const topology = await page.evaluate(() => {
    const state = window.__PLEXI_DEBUG__.getState();
    return {
      nodes: (state.nodes || []).length,
      panes: (state.panels || []).map((panel) => ({
        id: panel.id,
        nodeId: panel.nodeId,
        splitX: panel.splitX,
        splitY: panel.splitY,
      })),
    };
  });

  expect(topology.nodes).toBe(1);
  expect(topology.panes).toEqual([
    { id: "panel-1", nodeId: "node-1", splitX: 0, splitY: 0 },
    { id: "panel-2", nodeId: "node-1", splitX: 1, splitY: 0 },
    { id: "panel-3", nodeId: "node-1", splitX: 1, splitY: 1 },
  ]);

  await expect(page.locator(".terminal-frame--split")).toHaveCount(3);
  await expect(page.locator("#minimap-grid .minimap-node")).toHaveCount(1);
  await expect(page.locator("#minimap-grid .minimap-node-count")).toHaveText("3");

  await page.keyboard.press("Control+ArrowUp");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-2");

  await page.keyboard.press("Control+ArrowLeft");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-1");

  await page.keyboard.press("Control+W");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-2");

  const panelPositions = await page.evaluate(() => {
    const state = window.__PLEXI_DEBUG__.getState();
    return state.panels
      .map((panel) => ({ title: panel.title, splitX: panel.splitX, splitY: panel.splitY }))
      .sort((a, b) => a.title.localeCompare(b.title));
  });

  expect(panelPositions).toEqual([
    { title: "Terminal 2", splitX: 0, splitY: 0 },
    { title: "Terminal 3", splitX: 0, splitY: 1 },
  ]);
});

test("narrow windows keep the sidebar beside the workspace without overflowing", async ({ page }) => {
  await page.setViewportSize({ width: 980, height: 720 });
  await page.goto("http://127.0.0.1:4173/build/dev-macos-arm64/plexi-dev.app/Contents/Resources/app/views/mainview/index.html");
  await page.evaluate(() => window.__PLEXI_DEBUG__.reset());

  await page.keyboard.press("Control+N");

  const layout = await page.evaluate(() => {
    const shell = document.querySelector(".app-shell");
    const shellStyle = shell ? getComputedStyle(shell) : null;
    const sidebar = document.querySelector(".sidebar")?.getBoundingClientRect();
    const toolbar = document.querySelector(".workspace-toolbar")?.getBoundingClientRect();
    const path = document.querySelector("#focus-path");
    const process = document.querySelector("#focus-process");

    return {
      gridColumns: shellStyle?.gridTemplateColumns ?? "",
      gridRows: shellStyle?.gridTemplateRows ?? "",
      sidebarWidth: sidebar?.width ?? 0,
      toolbarWidth: toolbar?.width ?? 0,
      pathHidden: path ? getComputedStyle(path).display === "none" : false,
      processHidden: process ? getComputedStyle(process).display === "none" : false,
      scrollWidth: document.body.scrollWidth,
      viewportWidth: window.innerWidth,
    };
  });

  expect(layout.gridColumns).not.toBe("980px");
  expect(layout.gridRows).toBe("720px");
  expect(layout.sidebarWidth).toBeGreaterThan(0);
  expect(layout.toolbarWidth).toBeGreaterThan(0);
  expect(layout.pathHidden).toBe(true);
  expect(layout.processHidden).toBe(true);
  expect(layout.scrollWidth).toBeLessThanOrEqual(layout.viewportWidth);
});

test("short windows do not trigger a separate compact layout mode", async ({ page }) => {
  await page.setViewportSize({ width: 980, height: 640 });
  await page.goto("http://127.0.0.1:4173/build/dev-macos-arm64/plexi-dev.app/Contents/Resources/app/views/mainview/index.html");
  await page.evaluate(() => window.__PLEXI_DEBUG__.reset());

  const layout = await page.evaluate(() => {
    const toolbar = document.querySelector(".workspace-toolbar");
    const sidebarHeader = document.querySelector(".sidebar-header");
    const sidebarSection = document.querySelector(".sidebar-section");

    return {
      toolbarPaddingTop: toolbar ? getComputedStyle(toolbar).paddingTop : "",
      toolbarPaddingBottom: toolbar ? getComputedStyle(toolbar).paddingBottom : "",
      sidebarHeaderPaddingTop: sidebarHeader ? getComputedStyle(sidebarHeader).paddingTop : "",
      sidebarSectionGap: sidebarSection ? getComputedStyle(sidebarSection).gap : "",
    };
  });

  expect(layout.toolbarPaddingTop).not.toBe("9px");
  expect(layout.toolbarPaddingBottom).not.toBe("9px");
  expect(layout.sidebarHeaderPaddingTop).not.toBe("9px");
  expect(layout.sidebarSectionGap).not.toBe("6px");
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
  expect(pageErrors).toEqual([]);

  await page.keyboard.press("Control+N");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-1");
  await expect(page.locator("#toolbar-context")).toHaveText("Context 1");
  await expect(page.locator("#focus-right-slot")).toBeVisible();
  await expect(page.locator("#focus-bottom-slot")).toBeVisible();
  await expect(page.locator("#focus-path")).toHaveText("~");
  await expect(page.locator(".xterm")).toBeVisible();

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
  await expect(page.locator("#toast-layer")).toContainText("Terminal 2 split below");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-2");
  

  await page.keyboard.press("Control+N");
  await expect(page.locator("#toast-layer")).toContainText("Terminal 3 split right");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-3");
  await expect(page.locator(".terminal-frame--split")).toHaveCount(3);
  await expect(page.locator(".pane-preview")).toHaveCount(2);
  await page.locator('[data-command="new-node-right"]').click();
  await expect(page.locator("#toast-layer")).toContainText("Terminal 4 opened in a node to the right");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-4");
  expect(await page.evaluate(() => (window.__PLEXI_DEBUG__.getState().nodes || []).length)).toBe(2);

  await page.keyboard.press("Control+ArrowLeft");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-3");

  await page.locator("#minimap-grid .minimap-node").nth(1).click();
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-4");

  await page.keyboard.press("Control+S");
  await page.reload();
  expect(await page.evaluate(() => (window.__PLEXI_DEBUG__.getState().nodes || []).length)).toBe(2);
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-4");
  

  await page.locator('[data-rename-context-index="0"]').click();
  await expect(page.locator("#context-modal")).toBeVisible();
  await page.locator("#context-name-input").fill("Project Alpha");
  await page.locator("#context-form").evaluate((form) => form.requestSubmit());
  await expect(page.locator("#toolbar-context")).toHaveText("Project Alpha");
  await page.locator('[data-rename-context-index="0"]').click();
  await page.locator("#context-pin").click();
  await expect(page.locator("#context-list")).toContainText("★ Project Alpha");
  await page.locator("#context-close").click();

  await page.locator("#new-context").click();
  await expect(page.locator("#context-modal")).toBeVisible();
  await page.locator("#context-name-input").fill("<b>Context 2</b>");
  await page.locator("#context-form").evaluate((form) => form.requestSubmit());
  await page.keyboard.press("Control+2");
  await expect(page.locator("#toolbar-context")).toHaveText("<b>Context 2</b>");
  await expect(page.locator("#context-list")).toContainText("<b>Context 2</b>");
  expect(await page.locator("#context-list b").count()).toBe(0);
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBeNull();
  await expect(page.locator("#focus-path")).toBeHidden();
  

  await page.keyboard.press("Control+1");
  await expect(page.locator("#toolbar-context")).toHaveText("Project Alpha");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBe("panel-4");

  await page.locator('[data-rename-context-index="1"]').click();
  await expect(page.locator("#context-modal")).toBeVisible();
  await expect(page.locator("#context-delete")).toBeVisible();

  // Double click to confirm delete
  await page.locator("#context-delete").click();
  await page.locator("#context-delete").click();
  
  await expect(page.locator("#toast-layer")).toContainText("Context <b>Context 2</b> deleted");
  expect(await page.locator("#toast-layer b").count()).toBe(0);
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().contexts)).toHaveLength(1);

  await page.keyboard.press("Control+B");
  await expect(page.locator("#app-shell")).toHaveClass(/app-shell--sidebar-hidden/);

  await page.keyboard.press("Control+B");
  await expect(page.locator("#app-shell")).not.toHaveClass(/app-shell--sidebar-hidden/);
  await expect(page.locator(".sidebar-header")).toHaveClass(/electrobun-webkit-app-region-drag/);
  await expect(page.locator(".workspace-toolbar")).toHaveClass(/electrobun-webkit-app-region-drag/);

  await page.keyboard.press("Control+M");
  await expect(page.locator("#minimap")).not.toHaveClass(/is-hidden/);
  await expect(page.locator("#minimap-overlay")).toHaveClass(/is-hidden/);
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().minimapVisible)).toBe(false);

  await page.keyboard.press("Control+M");
  await expect(page.locator("#minimap-overlay")).not.toHaveClass(/is-hidden/);
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().minimapVisible)).toBe(true);

  await page.setViewportSize({ width: 1040, height: 720 });

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
  await expect(page.locator("#toast-layer")).toContainText("Terminal 4 closed");
  await expect(page.locator("#minimap-size")).toHaveText("1 node · 3 panes");

  await page.keyboard.press("Control+W");
  await page.keyboard.press("Control+W");
  await page.keyboard.press("Control+W");
  expect(await page.evaluate(() => window.__PLEXI_DEBUG__.getState().activePanelId)).toBeNull();
  await expect(page.locator("#toolbar-context")).toHaveText("Project Alpha");
  await expect(page.locator("#empty-shell")).toBeVisible();

  await page.screenshot({ path: "tests/e2e/screenshot.png", fullPage: true });
});
