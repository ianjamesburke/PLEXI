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
    expect(second.splitX).toBe(2);
    expect(second.splitY).toBe(0);
    expect(third.x).toBe(0);
    expect(third.y).toBe(0);
    expect(third.splitX).toBe(0);
    expect(third.splitY).toBe(2);
    expect(third.splitWidth).toBe(4);
    expect(third.splitHeight).toBe(2);
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
    expect(third.splitX).toBe(2);
    expect(third.splitY).toBe(0);
    expect(third.splitWidth).toBe(2);
    expect(getActiveNode(state)?.panes).toHaveLength(2);
  });

  test("closing a pane prefers the previous column in the same split row", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });
    const third = createPanelRecord(state, { direction: DIRECTIONS.right });

    focusPanel(state, third.id);
    closePanelRecord(state, third.id);

    expect(state.activePanelId).toBe(second.id);
    expect(first.splitX).toBe(0);
    expect(second.splitX).toBe(2);
  });

  test("split groups create full-width bands when splitting below a side-by-side row", () => {
    const state = makeDefaultState();
    createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });
    const third = createPanelRecord(state, { direction: DIRECTIONS.down });

    expect(second.splitWidth).toBe(2);
    expect(third.splitX).toBe(0);
    expect(third.splitY).toBe(2);
    expect(third.splitWidth).toBe(4);
    expect(third.splitHeight).toBe(2);
  });

  test("split groups can grow beyond four panes while space remains in the 4x4 layout", () => {
    const state = makeDefaultState();
    createPanelRecord(state, { direction: DIRECTIONS.right });
    createPanelRecord(state, { direction: DIRECTIONS.right });
    createPanelRecord(state, { direction: DIRECTIONS.down });
    createPanelRecord(state, { direction: DIRECTIONS.right });

    const fifth = createPanelRecord(state, { direction: DIRECTIONS.down });

    expect(fifth).not.toBeNull();
    expect(getVisiblePanels(state)).toHaveLength(5);
  });

  test("focusDirectionalPanel reaches the bottom-right pane inside a 2x2 split-group", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });
    const third = createPanelRecord(state, { direction: DIRECTIONS.down });
    const fourth = createPanelRecord(state, { direction: DIRECTIONS.right });

    focusPanel(state, third.id);

    expect(focusDirectionalPanel(state, DIRECTIONS.right)?.id).toBe(fourth.id);

    focusPanel(state, second.id);
    expect(focusDirectionalPanel(state, DIRECTIONS.down)?.id).toBe(fourth.id);

    focusPanel(state, first.id);
    expect(focusDirectionalPanel(state, DIRECTIONS.down)?.id).toBe(third.id);
  });

  test("splitting right after creating a new row keeps the new panes on that row", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.down });
    const third = createPanelRecord(state, { direction: DIRECTIONS.right });

    expect(first.splitX).toBe(0);
    expect(first.splitY).toBe(0);
    expect(first.splitWidth).toBe(4);
    expect(first.splitHeight).toBe(2);
    expect(second.splitX).toBe(0);
    expect(second.splitY).toBe(2);
    expect(second.splitWidth).toBe(2);
    expect(second.splitHeight).toBe(2);
    expect(third.splitX).toBe(2);
    expect(third.splitY).toBe(2);
    expect(third.splitWidth).toBe(2);
    expect(third.splitHeight).toBe(2);
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

  test("focusDirectionalPanel uses remembered row targets in a 2x2 top-level grid", () => {
    const state = makeDefaultState();
    // Build a 2x2 grid: top-left(0,0), top-right(1,0), bottom-left(0,1), bottom-right(1,1)
    const topLeft = createPanelRecord(state, { direction: DIRECTIONS.right });
    focusPanel(state, topLeft.id);
    const topRight = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });
    focusPanel(state, topLeft.id);
    const bottomLeft = createTopLevelPanelRecord(state, { direction: DIRECTIONS.down });
    focusPanel(state, bottomLeft.id);
    const bottomRight = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });

    expect(getVisibleNodes(state)).toHaveLength(4);
    expect(getVisibleNodes(state).map((n) => `(${n.x},${n.y})`).sort()).toEqual(["(0,0)", "(0,1)", "(1,0)", "(1,1)"]);

    // down from top-right should land on the remembered bottom-row node
    focusPanel(state, topRight.id);
    expect(focusDirectionalPanel(state, DIRECTIONS.down)?.id).toBe(bottomRight.id);

    // right from bottom-left should land on bottom-right
    focusPanel(state, bottomLeft.id);
    expect(focusDirectionalPanel(state, DIRECTIONS.right)?.id).toBe(bottomRight.id);

    // down from top-left should also land on the remembered bottom-row node
    focusPanel(state, topLeft.id);
    expect(focusDirectionalPanel(state, DIRECTIONS.down)?.id).toBe(bottomRight.id);
  });

  test("focusDirectionalPanel remembers the last focused node in each top-level row", () => {
    const state = makeDefaultState();
    const topLeft = createPanelRecord(state, { direction: DIRECTIONS.right });
    focusPanel(state, topLeft.id);
    const topRight = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });
    focusPanel(state, topLeft.id);
    const bottomLeft = createTopLevelPanelRecord(state, { direction: DIRECTIONS.down });
    focusPanel(state, bottomLeft.id);
    const bottomRight = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });

    focusPanel(state, topRight.id);
    expect(focusDirectionalPanel(state, DIRECTIONS.down)?.id).toBe(bottomRight.id);

    focusPanel(state, bottomRight.id);
    expect(focusDirectionalPanel(state, DIRECTIONS.up)?.id).toBe(topRight.id);
    expect(focusDirectionalPanel(state, DIRECTIONS.down)?.id).toBe(bottomRight.id);
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

  test("createTopLevelPanelRecord places new nodes below on a fresh bottom row", () => {
    const state = makeDefaultState();
    createPanelRecord(state, { direction: DIRECTIONS.right });
    createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });
    createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });
    const fourth = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });

    focusPanel(state, fourth.id);
    const below = createTopLevelPanelRecord(state, { direction: DIRECTIONS.down });

    expect(getNodeForPanelId(state, below.id)?.x).toBe(0);
    expect(getNodeForPanelId(state, below.id)?.y).toBe(1);
  });

  test("createTopLevelPanelRecord always appends downward nodes to the workspace bottom", () => {
    const state = makeDefaultState();
    const topLeft = createPanelRecord(state, { direction: DIRECTIONS.right });
    const topRight = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });
    focusPanel(state, topLeft.id);
    createTopLevelPanelRecord(state, { direction: DIRECTIONS.down });
    focusPanel(state, topRight.id);

    const below = createTopLevelPanelRecord(state, { direction: DIRECTIONS.down });

    expect(getNodeForPanelId(state, below.id)?.x).toBe(0);
    expect(getNodeForPanelId(state, below.id)?.y).toBe(2);
  });

  test("closing a top-level node prefers the previous column in the same row", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });
    const third = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });

    focusPanel(state, third.id);
    closePanelRecord(state, third.id);

    expect(state.activePanelId).toBe(second.id);
    expect(getVisibleNodes(state).map((node) => node.x)).toEqual([0, 1]);
    expect(first.x).toBe(0);
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

  test("new panes inherit font size from the active pane", () => {
    const state = makeDefaultState();
    createContextRecord(state, "One");
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    first.fontSize = 18;

    const second = createPanelRecord(state, { direction: DIRECTIONS.down });
    const third = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });

    expect(second.fontSize).toBe(18);
    expect(third.fontSize).toBe(18);
  });
});
