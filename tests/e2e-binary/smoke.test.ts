async function getPanelBuffer(): Promise<string> {
  return browser.execute(() => {
    return (window as any).__PLEXI_DEBUG__?.getPanelBuffer?.() || "";
  });
}

async function sendInput(text: string): Promise<void> {
  await browser.execute((input: string) => {
    const state = (window as any).__PLEXI_DEBUG__?.getState?.();
    const panelId = state?.activePanelId;
    if (!panelId) throw new Error("No active panel");
    const invoke = (window as any).__TAURI__?.core?.invoke
      ?? (window as any).__TAURI_INTERNALS__?.invoke;
    if (invoke) {
      invoke("write_session", { panelId, data: input });
    }
  }, text);
}

async function getState(): Promise<any> {
  return browser.execute(() => {
    return (window as any).__PLEXI_DEBUG__?.getState?.() || {};
  });
}

describe("Plexi binary smoke test", () => {
  before(async () => {
    // Wait for app init
    await browser.waitUntil(
      async () => {
        const items = await $$("#context-list > li");
        return items.length > 0;
      },
      { timeout: 15000, timeoutMsg: "App did not initialize" },
    );

    // Close all existing panels from saved workspace to start clean
    await browser.execute(() => {
      const debug = (window as any).__PLEXI_DEBUG__;
      const state = debug?.getState?.();
      const panelCount = state?.panels?.length ?? 0;
      for (let i = 0; i < panelCount; i++) {
        debug?.runCommand("close-terminal");
      }
    });
    await browser.pause(1000);
  });

  it("app launches and shows the title", async () => {
    const title = await browser.getTitle();
    expect(title).toBe("Plexi");
  });

  it("app shell renders with sidebar and workspace", async () => {
    expect(await (await $(".app-shell")).isDisplayed()).toBe(true);
    expect(await (await $(".sidebar")).isDisplayed()).toBe(true);
    expect(await (await $(".workspace-shell")).isDisplayed()).toBe(true);
  });

  it("starts with empty shell (no panels after reset)", async () => {
    const state = await getState();
    expect(state.panels?.length ?? 0).toBe(0);
  });

  it("opening a terminal produces a shell prompt", async () => {
    await browser.execute(() => {
      (window as any).__PLEXI_DEBUG__?.runCommand("new-node-right");
    });

    const xterm = await $(".xterm");
    await xterm.waitForDisplayed({ timeout: 8000 });

    await browser.waitUntil(
      async () => (await getPanelBuffer()).length > 10,
      { timeout: 10000, timeoutMsg: "Shell session did not produce output" },
    );
  });

  it("running a command produces output in the PTY buffer", async () => {
    await sendInput("echo __E2E_SMOKE__\r");

    await browser.waitUntil(
      async () => (await getPanelBuffer()).includes("__E2E_SMOKE__"),
      { timeout: 5000, timeoutMsg: "Command output did not appear" },
    );
  });

  it("closing the last terminal restores empty shell", async () => {
    await browser.execute(() => {
      (window as any).__PLEXI_DEBUG__?.runCommand("close-terminal");
    });

    await browser.waitUntil(
      async () => {
        const state = await getState();
        return (state.panels?.length ?? 1) === 0;
      },
      { timeout: 5000, timeoutMsg: "Panel was not removed" },
    );
  });
});
