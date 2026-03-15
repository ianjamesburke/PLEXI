import { describe, expect, test } from "bun:test";
import {
  DIRECTIONS,
  closePanelRecord,
  createContextRecord,
  createPanelRecord,
  createTopLevelPanelRecord,
  ensureActivePanel,
  focusDirectionalPanel,
  focusPanel,
  getActiveNode,
  getNodeForPanelId,
  getVisibleNodes,
  getVisiblePanels,
  jumpBackPanel,
  makeDefaultState,
  moveContextRecord,
  movePanelRecord,
  setContextIndex,
  toggleContextPinned,
} from "../../src/shared/workspace-state.js";

describe("workspace state helpers", () => {
  test("createPanelRecord splits inside the active node and tracks local pane positions", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });
    const third = createPanelRecord(state, { direction: DIRECTIONS.down });

    expect(first.x).toBe(0);
    expect(first.y).toBe(0);
    expect(first.splitX).toBe(0);
    expect(first.splitY).toBe(0);
    expect(second.x).toBe(0);
    expect(second.y).toBe(0);
    expect(second.splitX).toBe(1);
    expect(second.splitY).toBe(0);
    expect(third.x).toBe(0);
    expect(third.y).toBe(0);
    expect(third.splitX).toBe(1);
    expect(third.splitY).toBe(1);
    expect(getVisibleNodes(state)).toHaveLength(1);
    expect(getNodeForPanelId(state, first.id)?.type).toBe("split-group");
  });

  test("focusDirectionalPanel moves within a split-group before leaving the node", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });
    const third = createPanelRecord(state, { direction: DIRECTIONS.down });

    focusPanel(state, first.id);

    const right = focusDirectionalPanel(state, DIRECTIONS.right);
    expect(right?.id).toBe(second.id);

    const down = focusDirectionalPanel(state, DIRECTIONS.down);
    expect(down?.id).toBe(third.id);
  });

  test("closePanelRecord collapses a split-group back to a single node", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });

    const removed = closePanelRecord(state, second.id);

    expect(removed?.id).toBe(second.id);
    expect(getVisiblePanels(state)).toHaveLength(1);
    expect(getVisibleNodes(state)).toHaveLength(1);
    expect(getNodeForPanelId(state, first.id)?.type).toBe("single");
    expect(state.activePanelId).toBe(first.id);
  });

  test("closePanelRecord compacts local split coordinates after removing a pane", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });
    const third = createPanelRecord(state, { direction: DIRECTIONS.right });

    closePanelRecord(state, second.id);

    expect(first.splitX).toBe(0);
    expect(third.splitX).toBe(1);
    expect(third.splitY).toBe(0);
    expect(getActiveNode(state)?.panes).toHaveLength(2);
  });

  test("split groups enforce a four-pane maximum", () => {
    const state = makeDefaultState();
    createPanelRecord(state, { direction: DIRECTIONS.right });
    createPanelRecord(state, { direction: DIRECTIONS.right });
    createPanelRecord(state, { direction: DIRECTIONS.down });
    createPanelRecord(state, { direction: DIRECTIONS.right });

    const fifth = createPanelRecord(state, { direction: DIRECTIONS.down });

    expect(fifth).toBeNull();
    expect(getVisiblePanels(state)).toHaveLength(4);
  });

  test("movePanelRecord moves the whole top-level node, not only one pane", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });

    movePanelRecord(state, first.id, DIRECTIONS.right);
    expect(first.x).toBe(1);
    expect(first.y).toBe(0);
    expect(second.x).toBe(1);
    expect(second.y).toBe(0);
  });

  test("createTopLevelPanelRecord adds a neighboring node for directional escape", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });

    expect(getVisibleNodes(state)).toHaveLength(2);
    expect(first.x).toBe(0);
    expect(second.x).toBe(1);

    focusPanel(state, first.id);
    expect(focusDirectionalPanel(state, DIRECTIONS.right)?.id).toBe(second.id);
  });

  test("jumpBackPanel returns to the previously focused pane", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });

    focusPanel(state, first.id);
    focusPanel(state, second.id);

    expect(jumpBackPanel(state)?.id).toBe(first.id);
    expect(state.activePanelId).toBe(first.id);
  });

  test("context switching restores the last focused pane per context", () => {
    const state = makeDefaultState();
    createContextRecord(state, "One");
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });

    focusPanel(state, second.id);

    createContextRecord(state, "Two");
    setContextIndex(state, 1);
    expect(state.activePanelId).toBeNull();

    const third = createPanelRecord(state, { direction: DIRECTIONS.right });
    expect(third.contextIndex).toBe(1);

    setContextIndex(state, 0);
    expect(ensureActivePanel(state)?.id).toBe(second.id);

    setContextIndex(state, 1);
    expect(ensureActivePanel(state)?.id).toBe(third.id);
    expect(first.contextIndex).toBe(0);
  });

  test("active node tracking follows active pane selection", () => {
    const state = makeDefaultState();
    createContextRecord(state, "One");
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });

    focusPanel(state, first.id);
    expect(state.activeNodeId).toBe(getNodeForPanelId(state, first.id)?.id);

    focusPanel(state, second.id);
    expect(state.activeNodeId).toBe(getNodeForPanelId(state, second.id)?.id);
  });

  test("pinned contexts stay grouped ahead of unpinned contexts and can reorder within their section", () => {
    const state = makeDefaultState();
    createContextRecord(state, "Alpha");
    createContextRecord(state, "Beta");
    createContextRecord(state, "Gamma");

    toggleContextPinned(state, 2);
    expect(state.contexts.map((context) => [context.label, context.pinned])).toEqual([
      ["Gamma", true],
      ["Alpha", false],
      ["Beta", false],
    ]);

    expect(moveContextRecord(state, 0, 1)).toBe(false);
    expect(moveContextRecord(state, 2, -1)).toBe(true);
    expect(state.contexts.map((context) => context.label)).toEqual([
      "Gamma",
      "Beta",
      "Alpha",
    ]);
  });

  test("new panes inherit cwd from the active pane", () => {
    const state = makeDefaultState();
    createContextRecord(state, "One");
    const first = createPanelRecord(state, {
      direction: DIRECTIONS.right,
      cwd: "/tmp/project-a",
      cwdLabel: "~/project-a",
    });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });

    expect(first.cwd).toBe("/tmp/project-a");
    expect(first.cwdLabel).toBe("~/project-a");
    expect(second.cwd).toBe("/tmp/project-a");
    expect(second.cwdLabel).toBe("~/project-a");
  });
});
