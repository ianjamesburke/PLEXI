export const DIRECTIONS = {
  left: "left",
  right: "right",
  up: "up",
  down: "down",
};

export const PANEL_TYPES = {
  terminal: {
    label: "Terminal",
    summary: "Ghostty-aligned terminal workspace",
  },
  browser: {
    label: "Browser",
    summary: "Browser workspace scaffold",
  },
};

export const clone = (value) => JSON.parse(JSON.stringify(value));

export const makeDefaultState = () => ({
  contexts: [],
  activeContextIndex: 0,
  panels: [],
  activePanelId: null,
  activePanelIdsByContext: {},
  rowFocusPanelIdsByContext: {},
  previousPanelId: null,
  sidebarVisible: true,
  shortcutsVisible: false,
  sequence: 0,
  lastAction: "Ready",
});

export const getVisiblePanels = (state) =>
  state.panels.filter((panel) => panel.contextIndex === state.activeContextIndex);

export const getPanelById = (state, panelId) =>
  state.panels.find((panel) => panel.id === panelId) || null;

export const getActivePanel = (state) => getPanelById(state, state.activePanelId);

const getRowFocusMap = (state, contextIndex = state.activeContextIndex) => {
  if (!state.rowFocusPanelIdsByContext) {
    state.rowFocusPanelIdsByContext = {};
  }

  if (!state.rowFocusPanelIdsByContext[contextIndex]) {
    state.rowFocusPanelIdsByContext[contextIndex] = {};
  }

  return state.rowFocusPanelIdsByContext[contextIndex];
};

const rememberRowFocus = (state, panel) => {
  if (!panel) {
    return;
  }

  getRowFocusMap(state, panel.contextIndex)[panel.y] = panel.id;
};

const forgetRowFocus = (state, panelId, contextIndex = state.activeContextIndex) => {
  if (!state.rowFocusPanelIdsByContext?.[contextIndex]) {
    return;
  }

  const rowFocus = state.rowFocusPanelIdsByContext[contextIndex];
  Object.keys(rowFocus).forEach((row) => {
    if (rowFocus[row] === panelId) {
      delete rowFocus[row];
    }
  });
};

const slugify = (value) =>
  value
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)/g, "");

const ensureUniqueContextId = (state, baseId) => {
  let candidate = baseId;
  let suffix = 1;
  while (state.contexts.some((context) => context.id === candidate)) {
    candidate = `${baseId}-${suffix}`;
    suffix += 1;
  }
  return candidate;
};

export const createContextRecord = (state, label) => {
  const normalized = String(label || "").trim();
  const fallbackId = `context-${state.contexts.length + 1}`;
  const baseId = normalized ? slugify(normalized) : fallbackId;
  const id = ensureUniqueContextId(state, baseId || fallbackId);
  const context = {
    id,
    label: normalized,
  };
  state.contexts.push(context);
  state.activeContextIndex = state.contexts.length - 1;
  ensureActivePanel(state);
  return context;
};

export const renameContextRecord = (state, index, label) => {
  const context = state.contexts[index];
  if (!context) {
    return null;
  }
  context.label = String(label || "").trim();
  return context;
};

export const deleteContextRecord = (state, index) => {
  const context = state.contexts[index];
  if (!context) {
    return null;
  }

  state.panels = state.panels.filter((panel) => panel.contextIndex !== index);

  for (let i = index; i < state.contexts.length - 1; i++) {
    state.contexts[i] = state.contexts[i + 1];
    state.panels.forEach((panel) => {
      if (panel.contextIndex > index) {
        panel.contextIndex--;
      }
    });
  }
  state.contexts.pop();

  if (state.activeContextIndex >= state.contexts.length) {
    state.activeContextIndex = state.contexts.length - 1;
  }
  if (state.activeContextIndex < 0) {
    state.activeContextIndex = 0;
  }

  if (state.activePanelIdsByContext) {
    const newActivePanelIds = {};
    Object.keys(state.activePanelIdsByContext).forEach((keyStr) => {
      const key = Number(keyStr);
      if (key < index) {
        newActivePanelIds[key] = state.activePanelIdsByContext[key];
      } else if (key > index) {
        newActivePanelIds[key - 1] = state.activePanelIdsByContext[key];
      }
    });
    state.activePanelIdsByContext = newActivePanelIds;
  }

  if (state.rowFocusPanelIdsByContext) {
    const newRowFocus = {};
    Object.keys(state.rowFocusPanelIdsByContext).forEach((keyStr) => {
      const key = Number(keyStr);
      if (key < index) {
        newRowFocus[key] = state.rowFocusPanelIdsByContext[key];
      } else if (key > index) {
        newRowFocus[key - 1] = state.rowFocusPanelIdsByContext[key];
      }
    });
    state.rowFocusPanelIdsByContext = newRowFocus;
  }

  ensureActivePanel(state);
  return context;
};

export const getActiveContext = (state) => state.contexts[state.activeContextIndex] || null;

const isOccupied = (panels, x, y, contextIndex, ignoreId = null) =>
  panels.some(
    (panel) =>
      panel.contextIndex === contextIndex &&
      panel.id !== ignoreId &&
      panel.x === x &&
      panel.y === y,
  );

const compactRowColumnsLeft = (panels, anchorX = null) => {
  if (panels.length === 0) {
    return;
  }

  const minX = anchorX ?? getBounds(panels).minX;
  const rows = [...new Set(panels.map((panel) => panel.y))].sort((a, b) => a - b);
  rows.forEach((rowY) => {
    const rowPanels = panels
      .filter((panel) => panel.y === rowY)
      .sort((left, right) => (left.x - right.x) || left.id.localeCompare(right.id));
    rowPanels.forEach((panel, index) => {
      panel.x = minX + index;
    });
  });
};

export const getNextOpenPosition = (state, origin, direction) => {
  const step =
    direction === DIRECTIONS.left
      ? { x: -1, y: 0 }
      : direction === DIRECTIONS.up
        ? { x: 0, y: -1 }
        : direction === DIRECTIONS.down
          ? { x: 0, y: 1 }
          : { x: 1, y: 0 };

  let x = origin.x + step.x;
  let y = origin.y + step.y;

  while (isOccupied(state.panels, x, y, state.activeContextIndex)) {
    x += step.x;
    y += step.y;
  }

  return { x, y };
};

export const ensureActivePanel = (state) => {
  const visiblePanels = getVisiblePanels(state);
  const storedActivePanelId = state.activePanelIdsByContext?.[state.activeContextIndex] || null;

  if (visiblePanels.length === 0) {
    state.activePanelId = null;
    if (state.activePanelIdsByContext) {
      state.activePanelIdsByContext[state.activeContextIndex] = null;
    }
    state.previousPanelId = null;
    return null;
  }

  const activePanel = getActivePanel(state);

  if (activePanel && activePanel.contextIndex === state.activeContextIndex) {
    if (state.activePanelIdsByContext) {
      state.activePanelIdsByContext[state.activeContextIndex] = activePanel.id;
    }
    rememberRowFocus(state, activePanel);
    return activePanel;
  }

  const storedActivePanel = visiblePanels.find((panel) => panel.id === storedActivePanelId);
  state.activePanelId = (storedActivePanel || visiblePanels[0]).id;
  if (state.activePanelIdsByContext) {
    state.activePanelIdsByContext[state.activeContextIndex] = state.activePanelId;
  }
  state.previousPanelId = null;
  const nextActivePanel = getPanelById(state, state.activePanelId);
  rememberRowFocus(state, nextActivePanel);
  return nextActivePanel;
};

export const focusPanel = (state, panelId) => {
  const panel = getPanelById(state, panelId);

  if (!panel || panel.contextIndex !== state.activeContextIndex) {
    return null;
  }

  if (panelId === state.activePanelId) {
    rememberRowFocus(state, panel);
    return panel;
  }

  state.previousPanelId = state.activePanelId;
  state.activePanelId = panelId;
  if (state.activePanelIdsByContext) {
    state.activePanelIdsByContext[state.activeContextIndex] = panelId;
  }
  rememberRowFocus(state, panel);
  return panel;
};

export const createPanelRecord = (
  state,
  { type = "terminal", direction = DIRECTIONS.right, cwd = null, cwdLabel = null } = {},
) => {
  if (state.contexts.length === 0) {
    createContextRecord(state, "");
  }

  state.sequence += 1;

  const visiblePanels = getVisiblePanels(state);
  const activePanel = ensureActivePanel(state);
  const origin = activePanel
    ? { x: activePanel.x, y: activePanel.y }
    : { x: 0, y: 0 };
  const { x, y } = visiblePanels.length === 0
    ? { x: 0, y: 0 }
    : direction === DIRECTIONS.down
      ? (() => {
        const bounds = getBounds(visiblePanels);
        return { x: bounds.minX, y: bounds.maxY + 1 };
      })()
      : getNextOpenPosition(state, origin, direction);
  const id = `panel-${state.sequence}`;

  const panel = {
    id,
    type,
    title: `${PANEL_TYPES[type].label} ${state.sequence}`,
    x,
    y,
    contextIndex: state.activeContextIndex,
    transcript: [],
    hasReceivedInput: false,
    cwd: cwd || activePanel?.cwd || "~",
    cwdLabel: cwdLabel || activePanel?.cwdLabel || cwd || activePanel?.cwd || "~",
  };

  state.panels.push(panel);
  focusPanel(state, id);

  return panel;
};

export const closePanelRecord = (state, panelId) => {
  const index = state.panels.findIndex((panel) => panel.id === panelId);

  if (index === -1) {
    return null;
  }

  const contextPanelsBeforeClose = state.panels.filter((panel) => panel.contextIndex === state.panels[index].contextIndex);
  const minXBeforeClose = getBounds(contextPanelsBeforeClose).minX;
  const [removed] = state.panels.splice(index, 1);
  forgetRowFocus(state, removed.id, removed.contextIndex);
  const contextPanelsAfterClose = state.panels.filter((panel) => panel.contextIndex === removed.contextIndex);
  compactRowColumnsLeft(contextPanelsAfterClose, minXBeforeClose);

  const visiblePanels = getVisiblePanels(state);

  if (visiblePanels.length === 0) {
    state.activePanelId = null;
    if (state.activePanelIdsByContext) {
      state.activePanelIdsByContext[removed.contextIndex] = null;
    }
    state.previousPanelId = null;
    return removed;
  }

  const rowPanels = contextPanelsAfterClose
    .filter((panel) => panel.y === removed.y)
    .sort((left, right) => (left.x - right.x) || left.id.localeCompare(right.id));
  const shiftedPanel = rowPanels.find((panel) => panel.x === removed.x);
  const sameRowFallback = rowPanels[rowPanels.length - 1] || null;
  const rowBelow = contextPanelsAfterClose
    .filter((panel) => panel.y > removed.y)
    .sort((left, right) => (left.y - right.y) || (left.x - right.x) || left.id.localeCompare(right.id))[0] || null;
  const rowAbove = contextPanelsAfterClose
    .filter((panel) => panel.y < removed.y)
    .sort((left, right) => (right.y - left.y) || (left.x - right.x) || left.id.localeCompare(right.id))[0] || null;
  const previousVisible = visiblePanels.find((panel) => panel.id === state.previousPanelId);
  const fallback = shiftedPanel || sameRowFallback || rowBelow || rowAbove || previousVisible || visiblePanels[0];
  state.activePanelId = fallback.id;
  if (state.activePanelIdsByContext) {
    state.activePanelIdsByContext[removed.contextIndex] = fallback.id;
  }
  state.previousPanelId = null;
  rememberRowFocus(state, fallback);
  return removed;
};

export const movePanelRecord = (state, panelId, direction) => {
  const panel = getPanelById(state, panelId);

  if (!panel) {
    return null;
  }

  forgetRowFocus(state, panel.id, panel.contextIndex);
  const nextPosition = getNextOpenPosition(
    state,
    { x: panel.x, y: panel.y },
    direction,
  );

  panel.x = nextPosition.x;
  panel.y = nextPosition.y;
  if (panel.id === state.activePanelId) {
    rememberRowFocus(state, panel);
  }
  return panel;
};

export const setContextIndex = (state, index) => {
  const maxIndex = Math.max(0, state.contexts.length - 1);
  const nextIndex = Math.max(0, Math.min(index, maxIndex));
  state.activeContextIndex = nextIndex;
  ensureActivePanel(state);
};

export const cycleVisiblePanel = (state, direction) => {
  const visiblePanels = getVisiblePanels(state);

  if (visiblePanels.length === 0) {
    return null;
  }

  const currentIndex = visiblePanels.findIndex((panel) => panel.id === state.activePanelId);
  const nextIndex = currentIndex === -1
    ? 0
    : (currentIndex + direction + visiblePanels.length) % visiblePanels.length;
  return focusPanel(state, visiblePanels[nextIndex].id);
};

const scoreDirectionalCandidate = (activePanel, candidate, direction) => {
  const dx = candidate.x - activePanel.x;
  const dy = candidate.y - activePanel.y;

  if (
    (direction === DIRECTIONS.left && dx >= 0) ||
    (direction === DIRECTIONS.right && dx <= 0) ||
    (direction === DIRECTIONS.up && dy >= 0) ||
    (direction === DIRECTIONS.down && dy <= 0)
  ) {
    return Number.POSITIVE_INFINITY;
  }

  const primary = direction === DIRECTIONS.left || direction === DIRECTIONS.right
    ? Math.abs(dx)
    : Math.abs(dy);
  const secondary = direction === DIRECTIONS.left || direction === DIRECTIONS.right
    ? Math.abs(dy)
    : Math.abs(dx);

  return primary * 10 + secondary;
};

const getRowVisitTarget = (state, activePanel, direction) => {
  const visiblePanels = getVisiblePanels(state).filter((panel) => panel.id !== activePanel.id);
  const rowCandidates = visiblePanels.filter((panel) =>
    direction === DIRECTIONS.up ? panel.y < activePanel.y : panel.y > activePanel.y
  );

  if (rowCandidates.length === 0) {
    return null;
  }

  const targetRow = rowCandidates.reduce((closestRow, panel) => {
    if (closestRow === null) {
      return panel.y;
    }

    const currentDistance = Math.abs(panel.y - activePanel.y);
    const closestDistance = Math.abs(closestRow - activePanel.y);
    return currentDistance < closestDistance ? panel.y : closestRow;
  }, null);

  const rowPanels = rowCandidates
    .filter((panel) => panel.y === targetRow)
    .sort((left, right) => (left.x - right.x) || left.id.localeCompare(right.id));
  const rememberedPanelId = getRowFocusMap(state, activePanel.contextIndex)[targetRow];
  const rememberedPanel = rowPanels.find((panel) => panel.id === rememberedPanelId);

  return rememberedPanel || rowPanels[0] || null;
};

export const getDirectionalNeighbor = (state, direction) => {
  const activePanel = ensureActivePanel(state);

  if (!activePanel) {
    return null;
  }

  if (direction === DIRECTIONS.up || direction === DIRECTIONS.down) {
    return getRowVisitTarget(state, activePanel, direction);
  }

  const visiblePanels = getVisiblePanels(state).filter((panel) => panel.id !== activePanel.id);

  if (visiblePanels.length === 0) {
    return null;
  }

  const scored = visiblePanels
    .map((panel) => ({
      panel,
      score: scoreDirectionalCandidate(activePanel, panel, direction),
    }))
    .filter((candidate) => Number.isFinite(candidate.score))
    .sort((a, b) => a.score - b.score);

  return scored[0]?.panel || null;
};

export const focusDirectionalPanel = (state, direction) => {
  const panel = getDirectionalNeighbor(state, direction);

  if (!panel) {
    return null;
  }

  return focusPanel(state, panel.id);
};

export const getBounds = (panels) => {
  if (panels.length === 0) {
    return {
      minX: 0,
      maxX: 0,
      minY: 0,
      maxY: 0,
      width: 1,
      height: 1,
    };
  }

  const xs = panels.map((panel) => panel.x);
  const ys = panels.map((panel) => panel.y);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);

  return {
    minX,
    maxX,
    minY,
    maxY,
    width: maxX - minX + 1,
    height: maxY - minY + 1,
  };
};
