async function getPanelBuffer(panelId?: string): Promise<string> {
  // Don't pass panelId through if undefined — browser.execute serializes
  // undefined as null, which bypasses JS default parameter assignment.
  if (panelId) {
    return browser.execute((id: string) => {
      return (window as any).__PLEXI_DEBUG__?.getPanelBuffer?.(id) || "";
    }, panelId);
  }
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

async function runCommand(cmd: string): Promise<void> {
  await browser.execute((c: string) => {
    (window as any).__PLEXI_DEBUG__?.runCommand(c);
  }, cmd);
}

async function waitForPanelCount(count: number, msg: string): Promise<void> {
  await browser.waitUntil(
    async () => (await getState()).panels?.length === count,
    { timeout: 8000, timeoutMsg: msg },
  );
}

/** Wait for the active panel's PTY to have real output (shell prompt). */
async function waitForPtyReady(): Promise<void> {
  await browser.waitUntil(
    async () => (await getPanelBuffer()).length > 20,
    { timeout: 15000, timeoutMsg: "PTY session did not produce output" },
  );
}

async function closeAllPanels(): Promise<void> {
  const panels = (await getState()).panels?.length ?? 0;
  for (let i = 0; i < panels; i++) {
    await runCommand("close-terminal");
    await browser.pause(500);
  }
  if (panels > 0) {
    await waitForPanelCount(0, "Could not close all panels");
    await browser.pause(1000);
  }
}

/**
 * Sequential E2E tests against the real Tauri binary.
 *
 * Tests build on each other — each starts from the state left by the
 * previous one. Generous pauses let PTY sessions fully establish.
 */
describe("Plexi binary E2E", () => {
  before(async () => {
    await browser.waitUntil(
      async () => {
        const items = await $$("#context-list > li");
        return items.length > 0;
      },
      { timeout: 15000, timeoutMsg: "App did not initialize" },
    );

    // Close any panels from saved workspace
    await closeAllPanels();
  });

  // --- App shell ---

  it("shows the correct title", async () => {
    expect(await browser.getTitle()).toBe("Plexi");
  });

  it("renders sidebar and workspace", async () => {
    expect(await (await $(".app-shell")).isDisplayed()).toBe(true);
    expect(await (await $(".sidebar")).isDisplayed()).toBe(true);
    expect(await (await $(".workspace-shell")).isDisplayed()).toBe(true);
  });

  it("shows at least one context in the sidebar", async () => {
    const items = await $$("#context-list > li");
    expect(items.length).toBeGreaterThanOrEqual(1);
  });

  it("starts with no panels (clean state)", async () => {
    expect((await getState()).panels?.length ?? 0).toBe(0);
  });

  // --- Terminal lifecycle ---

  it("opens a terminal with a real PTY session", async () => {
    await runCommand("new-node-right");
    await waitForPanelCount(1, "Terminal did not open");

    const xterm = await $(".xterm");
    await xterm.waitForDisplayed({ timeout: 8000 });

    await waitForPtyReady();
  });

  it("executes a command and receives output", async () => {
    // Wait for shell to be fully initialized before sending input
    await waitForPtyReady();
    await browser.pause(1000);

    await sendInput("echo __CMD_TEST__\r");

    await browser.waitUntil(
      async () => (await getPanelBuffer()).includes("__CMD_TEST__"),
      { timeout: 8000, timeoutMsg: "Command output not in PTY buffer" },
    );
  });

  // --- Split panes (within an existing node) ---

  it("split right creates a second panel in the same node", async () => {
    await runCommand("new-terminal-right");
    await waitForPanelCount(2, "Split right did not create second panel");
    await browser.pause(1000);
  });

  it("closing the split pane keeps the original", async () => {
    await runCommand("close-terminal");
    await waitForPanelCount(1, "Close did not remove split panel");
    await browser.pause(500);

    // Original panel should still have PTY output from the echo test
    const buf = await getPanelBuffer();
    expect(buf.length).toBeGreaterThan(10);
  });

  it("split down creates a second panel in the same node", async () => {
    await runCommand("new-terminal-down");
    await waitForPanelCount(2, "Split down did not create second panel");
    await browser.pause(1000);
  });

  // Clean up splits before node tests
  it("closing all terminals restores empty shell", async () => {
    await closeAllPanels();
    expect((await getState()).panels?.length ?? 0).toBe(0);
  });

  // --- Top-level nodes (new independent terminal groups) ---

  it("new-node-right creates a top-level terminal to the right", async () => {
    await runCommand("new-node-right");
    await waitForPanelCount(1, "First node did not open");
    await waitForPtyReady();

    await runCommand("new-node-right");
    await waitForPanelCount(2, "Second node-right did not open");
    await browser.pause(1000);

    const state = await getState();
    expect(state.panels.length).toBe(2);
  });

  it("new-node-down creates a top-level terminal below", async () => {
    await runCommand("new-node-down");
    await waitForPanelCount(3, "Node-down did not open");
    await browser.pause(1000);

    const state = await getState();
    expect(state.panels.length).toBe(3);
  });

  // --- Context management ---
  // TODO: new-context opens a modal requiring user input (name).
  // Need to either: (a) add a programmatic createContext API, or
  // (b) automate the modal interaction via WebDriver.
  // Also: no keyboard shortcut for new-context exists yet — add one.

  // --- Final cleanup ---

  it("final cleanup: close all panels", async () => {
    await closeAllPanels();
    expect((await getState()).panels?.length ?? 0).toBe(0);
  });
});
