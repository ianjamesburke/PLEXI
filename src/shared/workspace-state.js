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

export const clampZoom = (zoom) => Math.min(2, Math.max(0.45, Number(zoom.toFixed(2))));

export const makeDefaultState = () => ({
  camera: { x: 0, y: 0, zoom: 1 },
  contexts: [],
  activeContextIndex: 0,
  panels: [],
  activePanelId: null,
  activePanelIdsByContext: {},
  previousPanelId: null,
  mode: "focus",
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

export const getActiveContext = (state) => state.contexts[state.activeContextIndex] || null;

const isOccupied = (panels, x, y, contextIndex, ignoreId = null) =>
  panels.some(
    (panel) =>
      panel.contextIndex === contextIndex &&
      panel.id !== ignoreId &&
      panel.x === x &&
      panel.y === y,
  );

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
    return activePanel;
  }

  const storedActivePanel = visiblePanels.find((panel) => panel.id === storedActivePanelId);
  state.activePanelId = (storedActivePanel || visiblePanels[0]).id;
  if (state.activePanelIdsByContext) {
    state.activePanelIdsByContext[state.activeContextIndex] = state.activePanelId;
  }
  state.previousPanelId = null;
  return getPanelById(state, state.activePanelId);
};

export const focusPanel = (state, panelId) => {
  if (panelId === state.activePanelId) {
    return getPanelById(state, panelId);
  }

  const panel = getPanelById(state, panelId);

  if (!panel || panel.contextIndex !== state.activeContextIndex) {
    return null;
  }

  state.previousPanelId = state.activePanelId;
  state.activePanelId = panelId;
  if (state.activePanelIdsByContext) {
    state.activePanelIdsByContext[state.activeContextIndex] = panelId;
  }
  return panel;
};

export const createPanelRecord = (state, { type = "terminal", direction = DIRECTIONS.right } = {}) => {
  state.sequence += 1;

  const visiblePanels = getVisiblePanels(state);
  const activePanel = ensureActivePanel(state);
  const origin = activePanel
    ? { x: activePanel.x, y: activePanel.y }
    : { x: 0, y: 0 };
  const { x, y } = visiblePanels.length === 0
    ? { x: 0, y: 0 }
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
    cwd: activePanel?.cwd || "~",
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

  const [removed] = state.panels.splice(index, 1);
  const visiblePanels = getVisiblePanels(state);

  if (visiblePanels.length === 0) {
    state.activePanelId = null;
    if (state.activePanelIdsByContext) {
      state.activePanelIdsByContext[removed.contextIndex] = null;
    }
    state.previousPanelId = null;
    return removed;
  }

  const previousVisible = visiblePanels.find((panel) => panel.id === state.previousPanelId);
  const fallback = previousVisible || visiblePanels[0];
  state.activePanelId = fallback.id;
  if (state.activePanelIdsByContext) {
    state.activePanelIdsByContext[removed.contextIndex] = fallback.id;
  }
  state.previousPanelId = null;
  return removed;
};

export const movePanelRecord = (state, panelId, direction) => {
  const panel = getPanelById(state, panelId);

  if (!panel) {
    return null;
  }

  const nextPosition = getNextOpenPosition(
    state,
    { x: panel.x, y: panel.y },
    direction,
  );

  panel.x = nextPosition.x;
  panel.y = nextPosition.y;
  return panel;
};

export const toggleMode = (state) => {
  state.mode = state.mode === "focus" ? "overview" : "focus";
  return state.mode;
};

export const setContextIndex = (state, index) => {
  state.activeContextIndex = index;
  ensureActivePanel(state);
};

export const panCamera = (state, dx, dy) => {
  state.camera.x += dx;
  state.camera.y += dy;
  return state.camera;
};

export const adjustZoom = (state, delta) => {
  state.camera.zoom = clampZoom(state.camera.zoom + delta);
  return state.camera.zoom;
};

export const resetViewport = (state) => {
  state.camera = { x: 0, y: 0, zoom: 1 };
  return state.camera;
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

export const getDirectionalNeighbor = (state, direction) => {
  const activePanel = ensureActivePanel(state);

  if (!activePanel) {
    return null;
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
