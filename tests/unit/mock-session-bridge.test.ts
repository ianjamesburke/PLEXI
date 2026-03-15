import { describe, expect, test } from "bun:test";
import { createMockSessionBridge } from "../../src/mainview/mock-session-bridge.js";

describe("mock session bridge clipboard", () => {
  test("stores and reads clipboard text through the bridge", async () => {
    const bridge = createMockSessionBridge();

    await bridge.writeClipboardText("hello world");

    expect(await bridge.readClipboardText()).toBe("hello world");
  });

  test("reset clears clipboard text", async () => {
    const bridge = createMockSessionBridge();

    await bridge.writeClipboardText("hello world");
    await bridge.reset();

    expect(await bridge.readClipboardText()).toBe("");
  });
});
