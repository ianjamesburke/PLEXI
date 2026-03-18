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

/** Read text directly from the ghostty-web WASM terminal buffer (rendered content). */
async function getTerminalContent(panelId?: string): Promise<string> {
  if (panelId) {
    return browser.execute((id: string) => {
      return (window as any).__PLEXI_DEBUG__?.getTerminalContent?.(id) || "";
    }, panelId);
  }
  return browser.execute(() => {
    return (window as any).__PLEXI_DEBUG__?.getTerminalContent?.() || "";
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
    async () => (await getActiveContextPanelCount()) === count,
    { timeout: 8000, timeoutMsg: msg },
  );
}

/** Wait for the active panel's PTY to have real output (shell prompt). */
async function waitForPtyReady(): Promise<void> {
  await browser.waitUntil(
    async () => (await getPanelBuffer()).length > 20,
    { timeout: 15000, timeoutMsg: "PTY session did not produce output in panel buffer" },
  );
}

/**
 * Wait for the ghostty-web terminal to actually render content from the WASM buffer.
 * This catches issues where data reaches the JS buffer but never makes it to the terminal.
 */
async function waitForTerminalReady(): Promise<void> {
  await browser.waitUntil(
    async () => (await getTerminalContent()).length > 5,
    { timeout: 15000, timeoutMsg: "Ghostty terminal WASM buffer has no rendered content" },
  );
}

/** Get the cwd reported by the active panel (from OSC 7 tracking). */
async function getActivePanelCwd(): Promise<string> {
  return browser.execute(() => {
    const state = (window as any).__PLEXI_DEBUG__?.getState?.();
    const panel = state?.panels?.find((p: any) => p.id === state.activePanelId);
    return panel?.cwd || "";
  });
}

/** Count panels in the currently active context. */
async function getActiveContextPanelCount(): Promise<number> {
  return browser.execute(() => {
    const state = (window as any).__PLEXI_DEBUG__?.getState?.();
    if (!state) return 0;
    const idx = state.activeContextIndex ?? 0;
    return (state.panels ?? []).filter((p: any) => p.contextIndex === idx).length;
  });
}

async function closeAllPanels(): Promise<void> {
  const panels = await getActiveContextPanelCount();
  for (let i = 0; i < panels; i++) {
    await runCommand("close-terminal");
    await browser.pause(500);
  }
  if (panels > 0) {
    await browser.waitUntil(
      async () => (await getActiveContextPanelCount()) === 0,
      { timeout: 8000, timeoutMsg: "Could not close all panels in active context" },
    );
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

  it("starts with no panels in active context (clean state)", async () => {
    expect(await getActiveContextPanelCount()).toBe(0);
  });

  // --- Terminal lifecycle ---

  it("opens a terminal with a real PTY session", async () => {
    await runCommand("new-node-right");
    await waitForPanelCount(1, "Terminal did not open");

    await browser.waitUntil(
      async () => {
        const exists = await browser.execute(() =>
          document.querySelector(".terminal-mount canvas") !== null
        );
        return exists;
      },
      { timeout: 8000, timeoutMsg: "Terminal canvas did not appear" },
    );

    await waitForPtyReady();
  });

  it("ghostty-web terminal renders PTY output (not just buffer)", async () => {
    // Verify that the ghostty-web WASM terminal actually has content,
    // not just the JS panelBuffer.
    await waitForTerminalReady();

    const termContent = await getTerminalContent();
    const bufContent = await getPanelBuffer();

    expect(termContent.length).toBeGreaterThan(0);
    expect(bufContent.length).toBeGreaterThan(0);
  });

  it("executes a command and output appears in BOTH buffer and terminal", async () => {
    await waitForPtyReady();
    await waitForTerminalReady();
    await browser.pause(1000);

    const marker = `__E2E_ECHO_${Date.now()}__`;
    await sendInput(`echo ${marker}\r`);

    // Wait for it in the panel buffer (JS-side)
    await browser.waitUntil(
      async () => (await getPanelBuffer()).includes(marker),
      { timeout: 8000, timeoutMsg: "echo marker not found in panel buffer" },
    );

    // Wait for it in the actual ghostty-web terminal (WASM-side)
    await browser.pause(500);
    await browser.waitUntil(
      async () => (await getTerminalContent()).includes(marker),
      {
        timeout: 8000,
        timeoutMsg: "echo marker in panel buffer but NOT in ghostty-web terminal — " +
          "data received from PTY but terminal.write() not reaching WASM buffer",
      },
    );
  });

  it("executes multiple commands and all appear in terminal", async () => {
    const markers = [
      `__MULTI_A_${Date.now()}__`,
      `__MULTI_B_${Date.now()}__`,
      `__MULTI_C_${Date.now()}__`,
    ];

    for (const marker of markers) {
      await sendInput(`echo ${marker}\r`);
      await browser.pause(300);
    }

    // Verify all markers appear in both buffer and terminal
    for (const marker of markers) {
      await browser.waitUntil(
        async () => (await getPanelBuffer()).includes(marker),
        { timeout: 8000, timeoutMsg: `Marker ${marker} not in panel buffer` },
      );
    }

    await browser.pause(1000);

    const termContent = await getTerminalContent();
    for (const marker of markers) {
      expect(termContent).toContain(marker);
    }
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

  // --- Ephemeral directory: cwd propagation across splits ---

  const TEMP_DIR_NAME = `__plexi_e2e_${Date.now()}`;
  const TEMP_DIR_PATH = `~/.plexi/${TEMP_DIR_NAME}`;

  it("setup: create ephemeral temp directory", async () => {
    // Close everything first to start fresh
    await closeAllPanels();

    await runCommand("new-node-right");
    await waitForPanelCount(1, "Terminal did not open");
    await waitForPtyReady();
    await waitForTerminalReady();
    await browser.pause(1000);

    // Create the temp dir and cd into it
    await sendInput(`mkdir -p ${TEMP_DIR_PATH} && cd ${TEMP_DIR_PATH}\r`);
    await browser.pause(2000);

    // Verify we're in the temp dir (OSC 7 cwd tracking)
    await browser.waitUntil(
      async () => (await getActivePanelCwd()).includes(TEMP_DIR_NAME),
      { timeout: 8000, timeoutMsg: "cwd did not update to temp dir" },
    );
  });

  it("split inherits cwd from the active terminal", async () => {
    await runCommand("new-terminal-right");
    await waitForPanelCount(2, "Split did not create second panel");
    await waitForPtyReady();
    await browser.pause(2000);

    // The new split pane should have started in the same directory
    await browser.waitUntil(
      async () => (await getActivePanelCwd()).includes(TEMP_DIR_NAME),
      { timeout: 8000, timeoutMsg: "Split pane did not inherit cwd" },
    );
  });

  it("can create a file in the temp dir and verify from the other pane", async () => {
    // Both panes are cd'd into the temp dir.
    // Write a file from the split pane (currently active).
    await sendInput("echo PLEXI_E2E > testfile.txt\r");
    await browser.pause(1000);

    // Switch to original pane and verify the file exists there too
    await runCommand("pane-1");
    await browser.pause(500);
    await sendInput("cat testfile.txt\r");

    await browser.waitUntil(
      async () => (await getPanelBuffer()).includes("PLEXI_E2E"),
      { timeout: 5000, timeoutMsg: "File content not visible from other pane" },
    );
  });

  it("teardown: remove ephemeral temp directory", async () => {
    // Clean up the temp dir from whichever pane we're in
    await sendInput(`rm -rf ${TEMP_DIR_PATH}\r`);
    await browser.pause(1000);

    // Verify it's gone
    await sendInput(`test -d ${TEMP_DIR_PATH} && echo EXISTS || echo GONE\r`);
    await browser.waitUntil(
      async () => (await getPanelBuffer()).includes("GONE"),
      { timeout: 5000, timeoutMsg: "Temp directory was not removed" },
    );
  });

  // --- Context management ---
  // TODO: new-context opens a modal requiring user input (name).
  // Need to either: (a) add a programmatic createContext API, or
  // (b) automate the modal interaction via WebDriver.
  // Also: no keyboard shortcut for new-context exists yet — add one.

  // --- Final cleanup ---

  it("final cleanup: close all panels", async () => {
    await closeAllPanels();
    expect(await getActiveContextPanelCount()).toBe(0);
  });

  // Safety net: clean up temp dir even if tests fail mid-way
  after(async () => {
    try {
      // If any panels are still open, use one to clean up
      const state = await getState();
      if ((state.panels?.length ?? 0) === 0) {
        await runCommand("new-node-right");
        await waitForPanelCount(1, "Could not open cleanup terminal");
        await waitForPtyReady();
        await browser.pause(1000);
      }
      await sendInput(`rm -rf ~/.plexi/__plexi_e2e_*\r`);
      await browser.pause(1000);
      await closeAllPanels();
    } catch {
      // Best-effort cleanup — don't fail the suite on cleanup errors
    }
  });
});
