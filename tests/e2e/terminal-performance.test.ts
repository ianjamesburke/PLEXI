import { expect, test } from "@playwright/test";

const MOD = process.platform === "darwin" ? "Meta" : "Control";
const PROMPT_TOKEN = "plexi:";

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
  await page.waitForFunction(
    ({ promptToken }) => {
      const text = document.querySelector(".xterm-rows")?.innerText || "";
      return text.includes(promptToken);
    },
    { promptToken: PROMPT_TOKEN },
    { timeout: 5000 },
  );
}

async function getTerminalText(page: any) {
  return page.locator(".xterm-rows").innerText();
}

async function getTerminalBuffer(page: any) {
  return page.evaluate(() => (window as any).__PLEXI_DEBUG__?.getPanelBuffer?.() || "");
}

function countPrompts(text: string) {
  return (text.match(/plexi:/g) || []).length;
}

async function runCommandAndMeasure(page: any, command: string, expectedText?: string) {
  const before = await getTerminalBuffer(page);
  const beforePrompts = countPrompts(before);
  const start = Date.now();

  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.type(command);
  await page.keyboard.press("Enter");

  await page.waitForFunction(
    ({ beforePrompts, expectedText, promptToken }) => {
      const text = (window as any).__PLEXI_DEBUG__?.getPanelBuffer?.() || "";
      const hasPrompt = (text.match(new RegExp(promptToken, "g")) || []).length > beforePrompts;
      return hasPrompt && (!expectedText || text.includes(expectedText));
    },
    { beforePrompts, expectedText, promptToken: PROMPT_TOKEN },
    { timeout: 5000 },
  );

  return Date.now() - start;
}

test.describe("Terminal performance", () => {
  test("ls returns to a prompt quickly", async ({ page }) => {
    await openApp(page);
    await openTerminal(page);

    const lsRoundTripMs = await runCommandAndMeasure(page, "ls", "README.md");
    const inputReadyMs = await runCommandAndMeasure(page, "echo __READY__", "__READY__");

    console.log(JSON.stringify({ lsRoundTripMs, inputReadyMs }));

    expect(lsRoundTripMs).toBeLessThan(1500);
    expect(inputReadyMs).toBeLessThan(1200);
  });

  test("large output does not leave input sluggish", async ({ page }) => {
    await openApp(page);
    await openTerminal(page);

    const floodRoundTripMs = await runCommandAndMeasure(page, "flood 600");
    const recoveryMs = await runCommandAndMeasure(page, "echo __AFTER_FLOOD__", "__AFTER_FLOOD__");

    console.log(JSON.stringify({ floodRoundTripMs, recoveryMs }));

    expect(floodRoundTripMs).toBeLessThan(2500);
    expect(recoveryMs).toBeLessThan(1500);
  });
});
