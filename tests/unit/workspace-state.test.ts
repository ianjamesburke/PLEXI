import { describe, expect, test } from "bun:test";
import {
  DIRECTIONS,
  adjustZoom,
  closePanelRecord,
  createPanelRecord,
  ensureActivePanel,
  focusDirectionalPanel,
  getVisiblePanels,
  makeDefaultState,
  movePanelRecord,
  setContextIndex,
} from "../../src/shared/workspace-state.js";

describe("workspace state helpers", () => {
  test("createPanelRecord places terminals to the right and below the active one", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });
    const third = createPanelRecord(state, { direction: DIRECTIONS.down });

    expect(first.x).toBe(0);
    expect(first.y).toBe(0);
    expect(second.x).toBe(1);
    expect(second.y).toBe(0);
    expect(third.x).toBe(1);
    expect(third.y).toBe(1);
  });

  test("focusDirectionalPanel follows nearest spatial neighbor", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });
    const third = createPanelRecord(state, { direction: DIRECTIONS.down });

    state.activePanelId = first.id;

    const right = focusDirectionalPanel(state, DIRECTIONS.right);
    expect(right?.id).toBe(second.id);

    const down = focusDirectionalPanel(state, DIRECTIONS.down);
    expect(down?.id).toBe(third.id);
  });

  test("closePanelRecord falls back to another visible terminal", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });

    state.previousPanelId = first.id;
    const removed = closePanelRecord(state, second.id);

    expect(removed?.id).toBe(second.id);
    expect(state.activePanelId).toBe(first.id);
    expect(getVisiblePanels(state)).toHaveLength(1);
  });

  test("movePanelRecord finds the next open slot in the requested direction", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });

    movePanelRecord(state, first.id, DIRECTIONS.right);
    expect(first.x).toBe(2);
    expect(first.y).toBe(0);
    expect(second.x).toBe(1);
  });

  test("adjustZoom stays within supported overview range", () => {
    const state = makeDefaultState();
    expect(adjustZoom(state, -2)).toBe(0.45);
    expect(adjustZoom(state, 3)).toBe(2);
  });

  test("context switching restores the last focused panel per context", () => {
    const state = makeDefaultState();
    const first = createPanelRecord(state, { direction: DIRECTIONS.right });
    const second = createPanelRecord(state, { direction: DIRECTIONS.right });

    state.activePanelId = second.id;
    state.activePanelIdsByContext[state.activeContextIndex] = second.id;

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
});
