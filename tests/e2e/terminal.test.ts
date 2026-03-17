import { expect, test } from "@playwright/test";

const MOD = process.platform === "darwin" ? "Meta" : "Control";

async function openApp(page: any) {
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/mainview/");
  await page.waitForFunction(
    () => document.querySelectorAll("#context-list > li").length > 0,
    { timeout: 10000 },
  );
}

async function openTerminal(page: any) {
  await page.keyboard.press(`${MOD}+n`);
  await page.waitForSelector(".xterm", { timeout: 8000 });
  // Wait for the shell prompt to appear
  await page.waitForFunction(
    () => document.querySelector(".xterm-rows")?.innerText?.includes("$"),
    { timeout: 5000 },
  );
}

async function typeCommand(page: any, cmd: string) {
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.type(cmd);
  await page.keyboard.press("Enter");
  // Wait a tick for output to render
  await page.waitForTimeout(300);
}

function getTerminalText(page: any) {
  return page.locator(".xterm-rows").innerText();
}

test.describe("Terminal", () => {
  test("Cmd+N opens a terminal panel", async ({ page }) => {
    await openApp(page);

    // Empty shell visible before any terminal
    await expect(page.locator("#empty-shell")).toBeVisible();

    await page.keyboard.press(`${MOD}+n`);

    // Empty shell hides, terminal mounts
    await expect(page.locator("#empty-shell")).toHaveClass(/is-hidden/);
    await expect(page.locator("[data-panel-terminal-mount]")).toBeVisible();
    await expect(page.locator(".xterm")).toBeVisible();
  });

  test("xterm renders and shows shell prompt", async ({ page }) => {
    await openApp(page);
    await openTerminal(page);

    const text = await getTerminalText(page);
    expect(text).toContain("$");
  });

  test("echo command produces output", async ({ page }) => {
    await openApp(page);
    await openTerminal(page);

    await typeCommand(page, "echo hello");

    const text = await getTerminalText(page);
    expect(text).toContain("echo hello");
    expect(text).toContain("hello");
  });

  test("toolbar path updates after terminal opens", async ({ page }) => {
    await openApp(page);
    await openTerminal(page);

    // Toolbar should show a path once the session reports cwd
    await expect(page.locator("#focus-path")).not.toBeEmpty();
  });

  test("second terminal opens with Cmd+D (split right)", async ({ page }) => {
    await openApp(page);
    await openTerminal(page);

    await page.keyboard.press(`${MOD}+d`);
    await page.waitForTimeout(500);

    // Two terminal mounts should exist
    const mounts = await page.locator("[data-panel-terminal-mount]").count();
    expect(mounts).toBeGreaterThanOrEqual(2);
  });

  test("close terminal with Cmd+W restores empty shell when last panel", async ({ page }) => {
    await openApp(page);
    await openTerminal(page);

    await page.keyboard.press(`${MOD}+w`);
    await page.waitForTimeout(300);

    await expect(page.locator("#empty-shell")).toBeVisible();
  });
});
