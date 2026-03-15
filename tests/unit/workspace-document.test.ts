import { describe, expect, test } from "bun:test";
import {
  deserializeWorkspaceDocument,
  serializeWorkspaceDocument,
} from "../../src/shared/workspace-document.js";
import { DIRECTIONS, createContextRecord, createPanelRecord, makeDefaultState } from "../../src/shared/workspace-state.js";

describe("workspace document", () => {
  test("serializes contexts and panels into a durable workspace document", () => {
    const state = makeDefaultState();
    createContextRecord(state, "Main");
    createPanelRecord(state, {
      direction: DIRECTIONS.right,
      cwd: "/tmp/project-a",
      cwdLabel: "~/project-a",
    });
    createContextRecord(state, "Infra");
    createPanelRecord(state, {
      direction: DIRECTIONS.right,
      cwd: "/tmp/project-b",
      cwdLabel: "~/project-b",
    });

    const document = serializeWorkspaceDocument(state);

    expect(document.workspace.contexts).toHaveLength(2);
    expect(document.version).toBe(2);
    expect(document.workspace.contexts[0]?.pinned).toBe(false);
    expect(document.workspace.contexts[0]?.nodes[0]?.type).toBe("single");
    expect(document.workspace.contexts[0]?.panels[0]?.cwdLabel).toBe("~/project-a");
    expect(document.workspace.contexts[1]?.label).toBe("Infra");
    expect(document.workspace.minimapVisible).toBe(true);
  });

  test("restores app state from a workspace document", () => {
    const restored = deserializeWorkspaceDocument({
      version: 2,
      workspace: {
        title: "Plexi Workspace",
        sequence: 3,
        activeContextIndex: 1,
        sidebarVisible: true,
        minimapVisible: false,
        camera: { x: 20, y: -10, zoom: 1.2 },
        contexts: [
          {
            id: "main",
            label: "Main",
            pinned: true,
            activeNodeId: "node-1",
            activePanelId: "panel-1",
            nodes: [
              {
                id: "node-1",
                type: "single",
                x: 0,
                y: 0,
                activePaneId: "panel-1",
                panes: [
                  {
                    id: "panel-1",
                    type: "terminal",
                    title: "Terminal 1",
                    cwd: "/tmp/project-a",
                    cwdLabel: "~/project-a",
                  },
                ],
              },
            ],
            panels: [
              {
                id: "panel-1",
                type: "terminal",
                title: "Terminal 1",
                x: 0,
                y: 0,
                cwd: "/tmp/project-a",
                cwdLabel: "~/project-a",
              },
            ],
          },
          {
            id: "infra",
            label: "Infra",
            activePanelId: null,
            panels: [],
          },
        ],
      },
      terminal: {
        engine: "xterm",
        fontFamily: "Plexi Terminal",
        fontSize: 14,
        cursorStyle: "block",
        theme: "plexi-dark",
      },
      keyboard: {},
    });

    expect(restored.contexts).toHaveLength(2);
    expect(restored.activeContextIndex).toBe(1);
    expect(restored.contexts[0]?.pinned).toBe(true);
    expect(restored.nodes[0]?.id).toBe("node-1");
    expect(restored.panels[0]?.cwdLabel).toBe("~/project-a");
    expect(restored.camera.zoom).toBe(1.2);
    expect(restored.minimapVisible).toBe(false);
  });
});
