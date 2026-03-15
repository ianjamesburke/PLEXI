export const DIRECTIONS = {
  left: "left",
  right: "right",
  up: "up",
  down: "down",
};

export const NODE_TYPES = {
  single: "single",
  splitGroup: "split-group",
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

const MAX_PANES_PER_NODE = 4;

export const clone = (value) => JSON.parse(JSON.stringify(value));

export const makeDefaultState = () => ({
  contexts: [],
  activeContextIndex: 0,
  nodes: [],
  panels: [],
  activeNodeId: null,
  activePanelId: null,
  activeNodeIdsByContext: {},
  activePanelIdsByContext: {},
  rowFocusPanelIdsByContext: {},
  previousPanelId: null,
  sidebarVisible: true,
  minimapVisible: true,
  shortcutsVisible: false,
  sequence: 0,
  lastAction: "Ready",
});

const ensureCollections = (state) => {
  if (!Array.isArray(state.nodes)) {
    state.nodes = [];
  }

  if (!Array.isArray(state.panels)) {
    state.panels = [];
  }

  if (!state.activeNodeIdsByContext) {
    state.activeNodeIdsByContext = {};
  }

  if (!state.activePanelIdsByContext) {
    state.activePanelIdsByContext = {};
  }

  if (!state.rowFocusPanelIdsByContext) {
    state.rowFocusPanelIdsByContext = {};
  }
};

const sortByGrid = (left, right) => {
  if (left.y !== right.y) {
    return left.y - right.y;
  }

  if (left.x !== right.x) {
    return left.x - right.x;
  }

  return left.id.localeCompare(right.id);
};

const syncPaneSpatialFields = (node, pane) => {
  pane.nodeId = node.id;
  pane.contextIndex = node.contextIndex;
  pane.x = node.x;
  pane.y = node.y;
  pane.splitX = Number.isFinite(pane.splitX) ? pane.splitX : 0;
  pane.splitY = Number.isFinite(pane.splitY) ? pane.splitY : 0;
  return pane;
};

const flattenNodePanes = (nodes) =>
  nodes
    .flatMap((node) => node.panes.map((pane) => syncPaneSpatialFields(node, pane)))
    .sort(sortByGrid);

const syncLegacyPanels = (state) => {
  ensureCollections(state);
  state.panels = flattenNodePanes(state.nodes);
  return state.panels;
};

const createSingleNodeRecord = (state, pane, position = {}) => {
  const suffixMatch = String(pane.id || "").match(/(\d+)$/);
  const suffix = suffixMatch ? suffixMatch[1] : `${state.sequence + 1}`;
  const node = {
    id: `node-${suffix}`,
    type: NODE_TYPES.single,
    x: Number.isFinite(position.x) ? position.x : Number.isFinite(pane.x) ? pane.x : 0,
    y: Number.isFinite(position.y) ? position.y : Number.isFinite(pane.y) ? pane.y : 0,
    contextIndex: Number.isFinite(position.contextIndex)
      ? position.contextIndex
      : Number.isFinite(pane.contextIndex)
        ? pane.contextIndex
        : state.activeContextIndex,
    label: String(position.label || ""),
    activePaneId: pane.id,
    panes: [syncPaneSpatialFields(
      {
        id: `node-${suffix}`,
        contextIndex: Number.isFinite(position.contextIndex)
          ? position.contextIndex
          : Number.isFinite(pane.contextIndex)
            ? pane.contextIndex
            : state.activeContextIndex,
        x: Number.isFinite(position.x) ? position.x : Number.isFinite(pane.x) ? pane.x : 0,
        y: Number.isFinite(position.y) ? position.y : Number.isFinite(pane.y) ? pane.y : 0,
      },
      pane,
    )],
  };

  node.panes.forEach((item) => syncPaneSpatialFields(node, item));
  return node;
};

const hydrateNodesFromPanels = (state) => {
  ensureCollections(state);

  if (state.nodes.length > 0) {
    syncLegacyPanels(state);
    return;
  }

  const panels = Array.isArray(state.panels) ? state.panels : [];
  state.nodes = panels.map((panel) => createSingleNodeRecord(state, panel));
  syncLegacyPanels(state);
};

const normalizeNodePaneGrid = (node) => {
  const columns = [...new Set(node.panes.map((pane) => pane.splitX))].sort((a, b) => a - b);
  const rows = [...new Set(node.panes.map((pane) => pane.splitY))].sort((a, b) => a - b);
  const columnMap = new Map(columns.map((value, index) => [value, index]));
  const rowMap = new Map(rows.map((value, index) => [value, index]));

  node.panes.forEach((pane) => {
    pane.splitX = columnMap.get(pane.splitX) ?? 0;
    pane.splitY = rowMap.get(pane.splitY) ?? 0;
    syncPaneSpatialFields(node, pane);
  });
};

const normalizeNode = (node) => {
  node.type = node.type === NODE_TYPES.splitGroup ? NODE_TYPES.splitGroup : NODE_TYPES.single;
  node.label = String(node.label || "");
  node.contextIndex = Number.isFinite(node.contextIndex) ? node.contextIndex : 0;
  node.x = Number.isFinite(node.x) ? node.x : 0;
  node.y = Number.isFinite(node.y) ? node.y : 0;
  node.panes = Array.isArray(node.panes) ? node.panes : [];

  node.panes.forEach((pane) => {
    pane.type = pane.type === "browser" ? "browser" : "terminal";
    pane.title = String(pane.title || PANEL_TYPES[pane.type].label);
    pane.cwd = String(pane.cwd || "~");
    pane.cwdLabel = String(pane.cwdLabel || pane.cwd || "~");
    pane.transcript = Array.isArray(pane.transcript) ? pane.transcript : [];
    pane.hasReceivedInput = pane.hasReceivedInput === true;
    syncPaneSpatialFields(node, pane);
  });

  if (node.panes.length <= 1) {
    node.type = NODE_TYPES.single;
    node.panes.forEach((pane) => {
      pane.splitX = 0;
      pane.splitY = 0;
      syncPaneSpatialFields(node, pane);
    });
  } else {
    normalizeNodePaneGrid(node);
  }

  node.activePaneId = node.panes.some((pane) => pane.id === node.activePaneId)
    ? node.activePaneId
    : node.panes[0]?.id || null;
};

export const normalizeWorkspaceState = (state) => {
  ensureCollections(state);
  hydrateNodesFromPanels(state);
  state.contexts = Array.isArray(state.contexts) ? state.contexts : [];
  state.contexts.forEach((context) => {
    context.id = String(context.id || "");
    context.label = String(context.label || "");
    context.pinned = context.pinned === true;
  });
  state.nodes.forEach(normalizeNode);
  syncLegacyPanels(state);
  return state;
};

export const getVisibleNodes = (state) => {
  normalizeWorkspaceState(state);
  return state.nodes.filter((node) => node.contextIndex === state.activeContextIndex);
};

export const getNodeById = (state, nodeId) => {
  normalizeWorkspaceState(state);
  return state.nodes.find((node) => node.id === nodeId) || null;
};

export const getNodeForPanelId = (state, panelId) => {
  normalizeWorkspaceState(state);
  return state.nodes.find((node) => node.panes.some((pane) => pane.id === panelId)) || null;
};

export const getVisiblePanels = (state) => {
  normalizeWorkspaceState(state);
  return state.panels.filter((panel) => panel.contextIndex === state.activeContextIndex);
};

export const getPanelById = (state, panelId) => {
  normalizeWorkspaceState(state);
  return state.panels.find((panel) => panel.id === panelId) || null;
};

export const getActivePanel = (state) => getPanelById(state, state.activePanelId);

export const getActiveNode = (state) => getNodeById(state, state.activeNodeId);

export const getNodePaneBounds = (node) => {
  if (!node || node.panes.length === 0) {
    return { minX: 0, maxX: 0, minY: 0, maxY: 0, width: 1, height: 1 };
  }

  const xs = node.panes.map((pane) => pane.splitX);
  const ys = node.panes.map((pane) => pane.splitY);
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

const getRowFocusMap = (state, contextIndex = state.activeContextIndex) => {
  ensureCollections(state);

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
  const rowFocus = state.rowFocusPanelIdsByContext?.[contextIndex];
  if (!rowFocus) {
    return;
  }

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
  normalizeWorkspaceState(state);

  const normalized = String(label || "").trim();
  const fallbackId = `context-${state.contexts.length + 1}`;
  const baseId = normalized ? slugify(normalized) : fallbackId;
  const id = ensureUniqueContextId(state, baseId || fallbackId);
  const context = {
    id,
    label: normalized,
    pinned: false,
  };
  state.contexts.push(context);
  state.activeContextIndex = state.contexts.length - 1;
  ensureActivePanel(state);
  return context;
};

export const renameContextRecord = (state, index, label) => {
  normalizeWorkspaceState(state);

  const context = state.contexts[index];
  if (!context) {
    return null;
  }

  context.label = String(label || "").trim();
  return context;
};

const remapContextIndexes = (state, orderedContextEntries) => {
  const oldToNew = new Map(orderedContextEntries.map((entry, newIndex) => [entry.oldIndex, newIndex]));

  state.contexts = orderedContextEntries.map((entry) => entry.context);

  state.nodes.forEach((node) => {
    const nextIndex = oldToNew.get(node.contextIndex);
    node.contextIndex = Number.isInteger(nextIndex) ? nextIndex : node.contextIndex;
    node.panes.forEach((pane) => {
      pane.contextIndex = node.contextIndex;
    });
  });

  const remapRecord = (source) => {
    const next = {};
    Object.entries(source || {}).forEach(([key, value]) => {
      const nextIndex = oldToNew.get(Number(key));
      if (Number.isInteger(nextIndex)) {
        next[nextIndex] = value;
      }
    });
    return next;
  };

  state.activeNodeIdsByContext = remapRecord(state.activeNodeIdsByContext);
  state.activePanelIdsByContext = remapRecord(state.activePanelIdsByContext);
  state.rowFocusPanelIdsByContext = remapRecord(state.rowFocusPanelIdsByContext);
  state.activeContextIndex = oldToNew.get(state.activeContextIndex) ?? state.activeContextIndex;
  syncLegacyPanels(state);
  ensureActivePanel(state);
};

export const moveContextRecord = (state, index, offset) => {
  normalizeWorkspaceState(state);

  if (!Number.isInteger(index) || !Number.isInteger(offset) || offset === 0) {
    return false;
  }

  const contextsWithIndex = state.contexts.map((context, oldIndex) => ({ context, oldIndex }));
  const current = contextsWithIndex[index];
  if (!current) {
    return false;
  }

  const targetIndex = index + offset;
  if (targetIndex < 0 || targetIndex >= contextsWithIndex.length) {
    return false;
  }

  const target = contextsWithIndex[targetIndex];
  if (!target || Boolean(target.context.pinned) !== Boolean(current.context.pinned)) {
    return false;
  }

  contextsWithIndex.splice(index, 1);
  contextsWithIndex.splice(targetIndex, 0, current);
  remapContextIndexes(state, contextsWithIndex);
  return true;
};

export const toggleContextPinned = (state, index) => {
  normalizeWorkspaceState(state);

  const contextsWithIndex = state.contexts.map((context, oldIndex) => ({ context, oldIndex }));
  const entry = contextsWithIndex[index];
  if (!entry) {
    return false;
  }

  entry.context.pinned = !entry.context.pinned;

  const pinned = contextsWithIndex.filter((item) => item.context.pinned);
  const unpinned = contextsWithIndex.filter((item) => !item.context.pinned);
  remapContextIndexes(state, [...pinned, ...unpinned]);
  return entry.context.pinned;
};

export const deleteContextRecord = (state, index) => {
  normalizeWorkspaceState(state);

  const context = state.contexts[index];
  if (!context) {
    return null;
  }

  state.nodes = state.nodes.filter((node) => node.contextIndex !== index);
  state.nodes.forEach((node) => {
    if (node.contextIndex > index) {
      node.contextIndex -= 1;
      node.panes.forEach((pane) => {
        pane.contextIndex = node.contextIndex;
      });
    }
  });

  state.contexts.splice(index, 1);

  if (state.activeContextIndex >= state.contexts.length) {
    state.activeContextIndex = state.contexts.length - 1;
  }
  if (state.activeContextIndex < 0) {
    state.activeContextIndex = 0;
  }

  const remapByContext = (source) => {
    const next = {};
    Object.keys(source || {}).forEach((keyStr) => {
      const key = Number(keyStr);
      if (key < index) {
        next[key] = source[key];
      } else if (key > index) {
        next[key - 1] = source[key];
      }
    });
    return next;
  };

  state.activeNodeIdsByContext = remapByContext(state.activeNodeIdsByContext);
  state.activePanelIdsByContext = remapByContext(state.activePanelIdsByContext);
  state.rowFocusPanelIdsByContext = remapByContext(state.rowFocusPanelIdsByContext);

  syncLegacyPanels(state);
  ensureActivePanel(state);
  return context;
};

export const getActiveContext = (state) => state.contexts[state.activeContextIndex] || null;

const isOccupied = (nodes, x, y, contextIndex, ignoreNodeId = null) =>
  nodes.some(
    (node) =>
      node.contextIndex === contextIndex &&
      node.id !== ignoreNodeId &&
      node.x === x &&
      node.y === y,
  );

export const getNextOpenPosition = (state, origin, direction, options = {}) => {
  normalizeWorkspaceState(state);

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
  const contextIndex = Number.isFinite(options.contextIndex)
    ? options.contextIndex
    : state.activeContextIndex;

  while (isOccupied(state.nodes, x, y, contextIndex, options.ignoreNodeId || null)) {
    x += step.x;
    y += step.y;
  }

  return { x, y };
};

const getPanelFocusFallback = (node) =>
  node?.panes.find((pane) => pane.id === node.activePaneId) || node?.panes[0] || null;

export const ensureActivePanel = (state) => {
  normalizeWorkspaceState(state);

  const visibleNodes = getVisibleNodes(state);
  const visiblePanels = getVisiblePanels(state);
  const storedActiveNodeId = state.activeNodeIdsByContext?.[state.activeContextIndex] || null;
  const storedActivePanelId = state.activePanelIdsByContext?.[state.activeContextIndex] || null;

  if (visiblePanels.length === 0) {
    state.activeNodeId = null;
    state.activePanelId = null;
    state.activeNodeIdsByContext[state.activeContextIndex] = null;
    state.activePanelIdsByContext[state.activeContextIndex] = null;
    state.previousPanelId = null;
    return null;
  }

  const activePanel = getActivePanel(state);
  if (activePanel && activePanel.contextIndex === state.activeContextIndex) {
    const activeNode = getNodeForPanelId(state, activePanel.id);
    state.activeNodeId = activeNode?.id || null;
    state.activePanelId = activePanel.id;
    state.activeNodeIdsByContext[state.activeContextIndex] = state.activeNodeId;
    state.activePanelIdsByContext[state.activeContextIndex] = state.activePanelId;
    if (activeNode) {
      activeNode.activePaneId = activePanel.id;
    }
    rememberRowFocus(state, activePanel);
    return activePanel;
  }

  const storedNode = visibleNodes.find((node) => node.id === storedActiveNodeId) || null;
  const storedPanel = visiblePanels.find((panel) => panel.id === storedActivePanelId) || null;
  const fallbackPanel = storedPanel || getPanelFocusFallback(storedNode) || getPanelFocusFallback(visibleNodes[0]);

  state.activePanelId = fallbackPanel.id;
  state.activeNodeId = fallbackPanel.nodeId || getNodeForPanelId(state, fallbackPanel.id)?.id || null;
  state.activeNodeIdsByContext[state.activeContextIndex] = state.activeNodeId;
  state.activePanelIdsByContext[state.activeContextIndex] = state.activePanelId;
  if (state.activeNodeId) {
    const node = getNodeById(state, state.activeNodeId);
    if (node) {
      node.activePaneId = fallbackPanel.id;
    }
  }
  state.previousPanelId = null;
  rememberRowFocus(state, fallbackPanel);
  return fallbackPanel;
};

export const focusPanel = (state, panelId) => {
  normalizeWorkspaceState(state);

  const panel = getPanelById(state, panelId);
  if (!panel || panel.contextIndex !== state.activeContextIndex) {
    return null;
  }

  const node = getNodeForPanelId(state, panelId);
  if (!node) {
    return null;
  }

  if (panelId !== state.activePanelId) {
    state.previousPanelId = state.activePanelId;
  }

  state.activeNodeId = node.id;
  state.activePanelId = panel.id;
  state.activeNodeIdsByContext[state.activeContextIndex] = node.id;
  state.activePanelIdsByContext[state.activeContextIndex] = panel.id;
  node.activePaneId = panel.id;
  rememberRowFocus(state, panel);
  return panel;
};

const buildPanelRecord = (state, type, cwd = null, cwdLabel = null, fallbackPanel = null) => ({
  id: `panel-${state.sequence}`,
  type,
  title: `${PANEL_TYPES[type].label} ${state.sequence}`,
  transcript: [],
  hasReceivedInput: false,
  cwd: cwd || fallbackPanel?.cwd || "~",
  cwdLabel: cwdLabel || fallbackPanel?.cwdLabel || cwd || fallbackPanel?.cwd || "~",
  splitX: 0,
  splitY: 0,
});

export const createTopLevelPanelRecord = (
  state,
  { type = "terminal", direction = DIRECTIONS.right, cwd = null, cwdLabel = null } = {},
) => {
  normalizeWorkspaceState(state);

  if (state.contexts.length === 0) {
    createContextRecord(state, "");
  }

  state.sequence += 1;
  const activeNode = getActiveNode(state);
  const activePanel = ensureActivePanel(state);
  const origin = activeNode
    ? { x: activeNode.x, y: activeNode.y }
    : { x: 0, y: 0 };
  const position = state.nodes.length === 0
    ? { x: 0, y: 0 }
    : getNextOpenPosition(state, origin, direction);
  const panel = buildPanelRecord(state, type, cwd, cwdLabel, activePanel);
  const node = createSingleNodeRecord(state, panel, {
    contextIndex: state.activeContextIndex,
    x: position.x,
    y: position.y,
  });
  state.nodes.push(node);
  syncLegacyPanels(state);
  focusPanel(state, panel.id);
  return panel;
};

const compactTopLevelNodesLeft = (nodes, anchorX = null) => {
  if (nodes.length === 0) {
    return;
  }

  const minX = anchorX ?? getBounds(nodes).minX;
  const rows = [...new Set(nodes.map((node) => node.y))].sort((a, b) => a - b);
  rows.forEach((rowY) => {
    const rowNodes = nodes
      .filter((node) => node.y === rowY)
      .sort((left, right) => (left.x - right.x) || left.id.localeCompare(right.id));
    rowNodes.forEach((node, index) => {
      node.x = minX + index;
      node.panes.forEach((pane) => syncPaneSpatialFields(node, pane));
    });
  });
};

const insertPaneIntoNode = (node, activePane, newPane, direction) => {
  if (direction === DIRECTIONS.down) {
    newPane.splitX = activePane.splitX;
    newPane.splitY = activePane.splitY + 1;
    node.panes.forEach((pane) => {
      if (pane.id !== activePane.id && pane.splitY >= newPane.splitY) {
        pane.splitY += 1;
      }
    });
  } else {
    newPane.splitX = activePane.splitX + 1;
    newPane.splitY = activePane.splitY;
    node.panes.forEach((pane) => {
      if (pane.id !== activePane.id && pane.splitY === newPane.splitY && pane.splitX >= newPane.splitX) {
        pane.splitX += 1;
      }
    });
  }

  node.panes.push(newPane);
  node.type = node.panes.length > 1 ? NODE_TYPES.splitGroup : NODE_TYPES.single;
  node.activePaneId = newPane.id;
  normalizeNodePaneGrid(node);
};

export const createPanelRecord = (
  state,
  { type = "terminal", direction = DIRECTIONS.right, cwd = null, cwdLabel = null } = {},
) => {
  normalizeWorkspaceState(state);

  if (state.contexts.length === 0) {
    createContextRecord(state, "");
  }

  state.sequence += 1;
  const activePanel = ensureActivePanel(state);
  const activeNode = getActiveNode(state);

  if (!activeNode || !activePanel) {
    const position = state.nodes.length === 0
      ? { x: 0, y: 0 }
      : getNextOpenPosition(state, { x: 0, y: 0 }, DIRECTIONS.right);
    const panel = buildPanelRecord(state, type, cwd, cwdLabel);
    const node = createSingleNodeRecord(state, panel, {
      contextIndex: state.activeContextIndex,
      x: position.x,
      y: position.y,
    });
    state.nodes.push(node);
    syncLegacyPanels(state);
    focusPanel(state, panel.id);
    return panel;
  }

  if (activeNode.panes.length >= MAX_PANES_PER_NODE) {
    return null;
  }

  const panel = buildPanelRecord(state, type, cwd, cwdLabel, activePanel);
  insertPaneIntoNode(activeNode, activePanel, panel, direction);
  syncLegacyPanels(state);
  focusPanel(state, panel.id);
  return panel;
};

export const closePanelRecord = (state, panelId) => {
  normalizeWorkspaceState(state);

  const node = getNodeForPanelId(state, panelId);
  const panel = getPanelById(state, panelId);
  if (!node || !panel) {
    return null;
  }

  forgetRowFocus(state, panel.id, panel.contextIndex);

  if (node.panes.length > 1) {
    node.panes = node.panes.filter((pane) => pane.id !== panelId);
    normalizeNode(node);
    syncLegacyPanels(state);
    const fallback = getPanelFocusFallback(node);
    if (fallback) {
      focusPanel(state, fallback.id);
    }
    return panel;
  }

  const contextNodesBeforeClose = state.nodes.filter((item) => item.contextIndex === node.contextIndex);
  const minXBeforeClose = getBounds(contextNodesBeforeClose).minX;
  state.nodes = state.nodes.filter((item) => item.id !== node.id);
  const contextNodesAfterClose = state.nodes.filter((item) => item.contextIndex === node.contextIndex);
  compactTopLevelNodesLeft(contextNodesAfterClose, minXBeforeClose);
  syncLegacyPanels(state);

  const visiblePanels = getVisiblePanels(state);
  if (visiblePanels.length === 0) {
    state.activeNodeId = null;
    state.activePanelId = null;
    state.activeNodeIdsByContext[node.contextIndex] = null;
    state.activePanelIdsByContext[node.contextIndex] = null;
    state.previousPanelId = null;
    return panel;
  }

  const sameContextNodes = getVisibleNodes(state);
  const fallbackNode = sameContextNodes.find((item) => item.y === node.y && item.x === node.x)
    || sameContextNodes.find((item) => item.y === node.y)
    || sameContextNodes.find((item) => item.y > node.y)
    || sameContextNodes.find((item) => item.y < node.y)
    || sameContextNodes[0];
  const fallbackPanel = getPanelFocusFallback(fallbackNode);
  if (fallbackPanel) {
    focusPanel(state, fallbackPanel.id);
  }
  return panel;
};

export const movePanelRecord = (state, panelId, direction) => {
  normalizeWorkspaceState(state);

  const node = getNodeForPanelId(state, panelId);
  if (!node) {
    return null;
  }

  const panel = getPanelById(state, panelId);
  const nextPosition = getNextOpenPosition(
    state,
    { x: node.x, y: node.y },
    direction,
    { contextIndex: node.contextIndex, ignoreNodeId: node.id },
  );

  node.x = nextPosition.x;
  node.y = nextPosition.y;
  node.panes.forEach((pane) => syncPaneSpatialFields(node, pane));
  syncLegacyPanels(state);
  if (panel && panel.id === state.activePanelId) {
    rememberRowFocus(state, panel);
  }
  return panel;
};

export const setContextIndex = (state, index) => {
  normalizeWorkspaceState(state);

  const maxIndex = Math.max(0, state.contexts.length - 1);
  state.activeContextIndex = Math.max(0, Math.min(index, maxIndex));
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

export const jumpBackPanel = (state) => {
  normalizeWorkspaceState(state);

  const previousPanel = getPanelById(state, state.previousPanelId);
  if (!previousPanel || previousPanel.contextIndex !== state.activeContextIndex) {
    return null;
  }

  return focusPanel(state, previousPanel.id);
};

const scoreDirectionalCandidate = (origin, candidate, direction) => {
  const dx = candidate.x - origin.x;
  const dy = candidate.y - origin.y;

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

const getIntraNodeDirectionalNeighbor = (node, activePane, direction) => {
  if (!node || node.panes.length <= 1) {
    return null;
  }

  const candidates = node.panes
    .filter((pane) => pane.id !== activePane.id)
    .map((pane) => ({
      pane,
      score: scoreDirectionalCandidate(
        { x: activePane.splitX, y: activePane.splitY },
        { x: pane.splitX, y: pane.splitY },
        direction,
      ),
    }))
    .filter((candidate) => Number.isFinite(candidate.score))
    .sort((left, right) => left.score - right.score);

  return candidates[0]?.pane || null;
};

const getTopLevelDirectionalNeighbor = (state, activeNode, direction) => {
  const visibleNodes = getVisibleNodes(state).filter((node) => node.id !== activeNode.id);
  if (visibleNodes.length === 0) {
    return null;
  }

  if (direction === DIRECTIONS.up || direction === DIRECTIONS.down) {
    const rowCandidates = visibleNodes.filter((node) =>
      direction === DIRECTIONS.up ? node.y < activeNode.y : node.y > activeNode.y);

    if (rowCandidates.length === 0) {
      return null;
    }

    const targetRow = rowCandidates.reduce((closestRow, node) => {
      if (closestRow === null) {
        return node.y;
      }

      const currentDistance = Math.abs(node.y - activeNode.y);
      const closestDistance = Math.abs(closestRow - activeNode.y);
      return currentDistance < closestDistance ? node.y : closestRow;
    }, null);

    const rowNodes = rowCandidates
      .filter((node) => node.y === targetRow)
      .sort((left, right) => (left.x - right.x) || left.id.localeCompare(right.id));
    const rememberedPanelId = getRowFocusMap(state, activeNode.contextIndex)[targetRow];
    const rememberedNode = rowNodes.find((node) => node.panes.some((pane) => pane.id === rememberedPanelId));

    return rememberedNode || rowNodes[0] || null;
  }

  const candidates = visibleNodes
    .map((node) => ({
      node,
      score: scoreDirectionalCandidate(activeNode, node, direction),
    }))
    .filter((candidate) => Number.isFinite(candidate.score))
    .sort((left, right) => left.score - right.score);

  return candidates[0]?.node || null;
};

export const getDirectionalNeighbor = (state, direction) => {
  const activePanel = ensureActivePanel(state);
  const activeNode = getActiveNode(state);

  if (!activePanel || !activeNode) {
    return null;
  }

  const localNeighbor = getIntraNodeDirectionalNeighbor(activeNode, activePanel, direction);
  if (localNeighbor) {
    return localNeighbor;
  }

  const topLevelNeighbor = getTopLevelDirectionalNeighbor(state, activeNode, direction);
  return getPanelFocusFallback(topLevelNeighbor);
};

export const focusDirectionalPanel = (state, direction) => {
  const panel = getDirectionalNeighbor(state, direction);
  if (!panel) {
    return null;
  }

  return focusPanel(state, panel.id);
};

export const getBounds = (items) => {
  if (items.length === 0) {
    return {
      minX: 0,
      maxX: 0,
      minY: 0,
      maxY: 0,
      width: 1,
      height: 1,
    };
  }

  const xs = items.map((item) => item.x);
  const ys = items.map((item) => item.y);
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
