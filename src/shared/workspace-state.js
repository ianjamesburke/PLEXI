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

const MAX_NODE_GRID_UNITS = 4;
const DEFAULT_PANE_FONT_SIZE = 14;

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

const createLeafLayout = (paneId) => ({
  type: "pane",
  paneId,
});

const createSplitLayout = (axis, first, second) => ({
  type: "split",
  axis,
  first,
  second,
});

const syncPaneSpatialFields = (node, pane) => {
  pane.nodeId = node.id;
  pane.contextIndex = node.contextIndex;
  pane.x = node.x;
  pane.y = node.y;
  pane.splitX = Number.isFinite(pane.splitX) ? pane.splitX : 0;
  pane.splitY = Number.isFinite(pane.splitY) ? pane.splitY : 0;
  pane.splitWidth = Number.isFinite(pane.splitWidth) ? pane.splitWidth : MAX_NODE_GRID_UNITS;
  pane.splitHeight = Number.isFinite(pane.splitHeight) ? pane.splitHeight : MAX_NODE_GRID_UNITS;
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

const chainLayouts = (axis, layouts) => {
  if (layouts.length === 0) {
    return null;
  }

  if (layouts.length === 1) {
    return layouts[0];
  }

  const [first, ...rest] = layouts;
  return createSplitLayout(axis, first, chainLayouts(axis, rest));
};

const buildRowsFromEntries = (entries) => {
  const rowsByY = new Map();

  [...entries]
    .sort((left, right) =>
      (left.rect.y - right.rect.y)
      || (left.rect.x - right.rect.x)
      || left.paneId.localeCompare(right.paneId))
    .forEach((entry) => {
      if (!rowsByY.has(entry.rect.y)) {
        rowsByY.set(entry.rect.y, []);
      }

      rowsByY.get(entry.rect.y).push(entry);
    });

  return [...rowsByY.entries()]
    .sort((left, right) => left[0] - right[0])
    .map(([, rowEntries]) =>
      rowEntries
        .sort((left, right) =>
          (left.rect.x - right.rect.x)
          || left.paneId.localeCompare(right.paneId))
        .map((entry) => entry.paneId))
    .filter((row) => row.length > 0);
};

const buildLayoutFromRows = (rows) => {
  const normalizedRows = rows.filter((row) => row.length > 0);
  if (normalizedRows.length === 0) {
    return null;
  }

  if (normalizedRows.length === 1 && normalizedRows[0].length === 1) {
    return createLeafLayout(normalizedRows[0][0]);
  }

  const rowLayouts = normalizedRows
    .map((row) => chainLayouts("x", row.map((paneId) => createLeafLayout(paneId))))
    .filter(Boolean);

  if (rowLayouts.length === 1) {
    return rowLayouts[0];
  }

  return chainLayouts("y", rowLayouts);
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
    layout: createLeafLayout(pane.id),
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

const computeLayoutEntries = (layout, rect, path = [], entries = [], internalEntries = []) => {
  if (!layout) {
    return { entries, internalEntries };
  }

  if (layout.type === "pane") {
    entries.push({
      paneId: layout.paneId,
      rect,
      path,
    });
    return { entries, internalEntries };
  }

  internalEntries.push({
    path,
    rect,
    layout,
  });

  if (layout.axis === "x") {
    const firstWidth = Math.floor(rect.w / 2);
    const secondWidth = rect.w - firstWidth;
    computeLayoutEntries(layout.first, { ...rect, w: firstWidth }, [...path, "first"], entries, internalEntries);
    computeLayoutEntries(
      layout.second,
      { ...rect, x: rect.x + firstWidth, w: secondWidth },
      [...path, "second"],
      entries,
      internalEntries,
    );
    return { entries, internalEntries };
  }

  const firstHeight = Math.floor(rect.h / 2);
  const secondHeight = rect.h - firstHeight;
  computeLayoutEntries(layout.first, { ...rect, h: firstHeight }, [...path, "first"], entries, internalEntries);
  computeLayoutEntries(
    layout.second,
    { ...rect, y: rect.y + firstHeight, h: secondHeight },
    [...path, "second"],
    entries,
    internalEntries,
  );
  return { entries, internalEntries };
};

const getLayoutSnapshot = (layout) =>
  computeLayoutEntries(
    layout,
    { x: 0, y: 0, w: MAX_NODE_GRID_UNITS, h: MAX_NODE_GRID_UNITS },
  );

const removeUnknownLeaves = (layout, paneIds) => {
  if (!layout) {
    return null;
  }

  if (layout.type === "pane") {
    return paneIds.has(layout.paneId) ? createLeafLayout(layout.paneId) : null;
  }

  const first = removeUnknownLeaves(layout.first, paneIds);
  const second = removeUnknownLeaves(layout.second, paneIds);

  if (!first) {
    return second;
  }

  if (!second) {
    return first;
  }

  return createSplitLayout(layout.axis === "y" ? "y" : "x", first, second);
};

const getStoredNodeEntries = (node) => {
  const paneIds = new Set(node.panes.map((pane) => pane.id));
  const layout = removeUnknownLeaves(node.layout, paneIds);
  const entries = layout
    ? getLayoutSnapshot(layout).entries.filter((entry) => paneIds.has(entry.paneId))
    : [];
  const seenPaneIds = new Set(entries.map((entry) => entry.paneId));

  node.panes.forEach((pane) => {
    if (seenPaneIds.has(pane.id)) {
      return;
    }

    entries.push({
      paneId: pane.id,
      rect: {
        x: Number.isFinite(pane.splitX) ? pane.splitX : 0,
        y: Number.isFinite(pane.splitY) ? pane.splitY : 0,
        w: Number.isFinite(pane.splitWidth) ? pane.splitWidth : MAX_NODE_GRID_UNITS,
        h: Number.isFinite(pane.splitHeight) ? pane.splitHeight : MAX_NODE_GRID_UNITS,
      },
    });
  });

  return entries;
};

const getNodeRows = (node) => buildRowsFromEntries(getStoredNodeEntries(node));

const distributeUnits = (count, totalUnits = MAX_NODE_GRID_UNITS) => {
  if (!Number.isInteger(count) || count <= 0) {
    return [];
  }

  const spans = [];
  let usedUnits = 0;

  for (let index = 0; index < count; index += 1) {
    const remainingItems = count - index;
    const remainingUnits = totalUnits - usedUnits;
    const size = Math.ceil(remainingUnits / remainingItems);
    spans.push(size);
    usedUnits += size;
  }

  return spans;
};

const applyNodeRows = (node, rows) => {
  const normalizedRows = rows
    .map((row) => row.filter(Boolean))
    .filter((row) => row.length > 0);
  const orderedPaneIds = normalizedRows.flat();
  const panesById = new Map(node.panes.map((pane) => [pane.id, pane]));

  node.panes = orderedPaneIds
    .map((paneId) => panesById.get(paneId))
    .filter(Boolean);
  node.layout = buildLayoutFromRows(normalizedRows);
  node.type = node.panes.length <= 1 ? NODE_TYPES.single : NODE_TYPES.splitGroup;
  node.activePaneId = orderedPaneIds.includes(node.activePaneId)
    ? node.activePaneId
    : orderedPaneIds[0] || null;

  if (node.panes.length === 0) {
    return;
  }

  const rowHeights = distributeUnits(normalizedRows.length);
  let currentY = 0;

  normalizedRows.forEach((row, rowIndex) => {
    const rowHeight = rowHeights[rowIndex] ?? 0;
    const columnWidths = distributeUnits(row.length);
    let currentX = 0;

    row.forEach((paneId, columnIndex) => {
      const pane = panesById.get(paneId);
      if (!pane) {
        currentX += columnWidths[columnIndex] ?? 0;
        return;
      }

      pane.splitX = currentX;
      pane.splitY = currentY;
      pane.splitWidth = columnWidths[columnIndex] ?? 0;
      pane.splitHeight = rowHeight;
      syncPaneSpatialFields(node, pane);
      currentX += pane.splitWidth;
    });

    currentY += rowHeight;
  });
};

const normalizeNodeLayout = (node) => {
  const rows = getNodeRows(node);

  if (rows.length === 0 && node.panes[0]) {
    applyNodeRows(node, [[node.panes[0].id]]);
    return;
  }

  applyNodeRows(node, rows);
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
    pane.fontSize = Number.isFinite(pane.fontSize) ? pane.fontSize : DEFAULT_PANE_FONT_SIZE;
    syncPaneSpatialFields(node, pane);
  });

  normalizeNodeLayout(node);
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
    return { minX: 0, maxX: 0, minY: 0, maxY: 0, width: MAX_NODE_GRID_UNITS, height: MAX_NODE_GRID_UNITS };
  }

  const minX = Math.min(...node.panes.map((pane) => pane.splitX));
  const minY = Math.min(...node.panes.map((pane) => pane.splitY));
  const maxX = Math.max(...node.panes.map((pane) => pane.splitX + pane.splitWidth - 1));
  const maxY = Math.max(...node.panes.map((pane) => pane.splitY + pane.splitHeight - 1));

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
      node.contextIndex === contextIndex
      && node.id !== ignoreNodeId
      && node.x === x
      && node.y === y,
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

const getNextBottomRowPosition = (state, options = {}) => {
  const contextIndex = Number.isFinite(options.contextIndex)
    ? options.contextIndex
    : state.activeContextIndex;
  const contextNodes = state.nodes.filter((node) =>
    node.contextIndex === contextIndex && node.id !== (options.ignoreNodeId || null));

  if (contextNodes.length === 0) {
    return { x: 0, y: 0 };
  }

  return {
    x: getBounds(contextNodes).minX,
    y: getBounds(contextNodes).maxY + 1,
  };
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
  fontSize: Number.isFinite(fallbackPanel?.fontSize) ? fallbackPanel.fontSize : DEFAULT_PANE_FONT_SIZE,
  cwd: cwd || fallbackPanel?.cwd || "~",
  cwdLabel: cwdLabel || fallbackPanel?.cwdLabel || cwd || fallbackPanel?.cwd || "~",
  splitX: 0,
  splitY: 0,
  splitWidth: MAX_NODE_GRID_UNITS,
  splitHeight: MAX_NODE_GRID_UNITS,
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
    : direction === DIRECTIONS.down
      ? getNextBottomRowPosition(state)
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

const getNodeLayoutEntries = (node) =>
  [...node.panes]
    .sort((left, right) =>
      (left.splitY - right.splitY)
      || (left.splitX - right.splitX)
      || left.id.localeCompare(right.id))
    .map((pane) => ({
      paneId: pane.id,
      rect: {
        x: pane.splitX,
        y: pane.splitY,
        w: pane.splitWidth,
        h: pane.splitHeight,
      },
    }));

const insertPaneIntoNode = (node, activePaneId, newPane, direction) => {
  const rows = getNodeRows(node);
  const rowIndex = rows.findIndex((row) => row.includes(activePaneId));
  if (rowIndex === -1) {
    return false;
  }

  node.panes.push(newPane);
  if (direction === DIRECTIONS.down) {
    if (rows.length >= MAX_NODE_GRID_UNITS) {
      node.panes = node.panes.filter((pane) => pane.id !== newPane.id);
      return false;
    }

    rows.splice(rowIndex + 1, 0, [newPane.id]);
    applyNodeRows(node, rows);
    return true;
  }

  const columnIndex = rows[rowIndex].indexOf(activePaneId);
  if (columnIndex === -1 || rows[rowIndex].length >= MAX_NODE_GRID_UNITS) {
    node.panes = node.panes.filter((pane) => pane.id !== newPane.id);
    return false;
  }

  rows[rowIndex].splice(columnIndex + 1, 0, newPane.id);
  applyNodeRows(node, rows);
  return true;
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

  const panel = buildPanelRecord(state, type, cwd, cwdLabel, activePanel);
  if (!insertPaneIntoNode(activeNode, activePanel.id, panel, direction)) {
    return null;
  }

  syncLegacyPanels(state);
  focusPanel(state, panel.id);
  return panel;
};

const getClosestPaneForRect = (node, rect, ignorePaneId = null) => {
  const entries = getNodeLayoutEntries(node).filter((entry) => entry.paneId !== ignorePaneId);
  const overlapsVertically = (entry) =>
    entry.rect.y < rect.y + rect.h && rect.y < entry.rect.y + entry.rect.h;
  const overlapsHorizontally = (entry) =>
    entry.rect.x < rect.x + rect.w && rect.x < entry.rect.x + entry.rect.w;
  const centerDistance = (entry) => {
    const centerX = entry.rect.x + (entry.rect.w / 2);
    const centerY = entry.rect.y + (entry.rect.h / 2);
    const rectCenterX = rect.x + (rect.w / 2);
    const rectCenterY = rect.y + (rect.h / 2);
    return Math.abs(centerX - rectCenterX) + Math.abs(centerY - rectCenterY);
  };

  const scored = entries
    .map((entry) => {
      const entryRight = entry.rect.x + entry.rect.w;
      const entryBottom = entry.rect.y + entry.rect.h;
      const removedRight = rect.x + rect.w;
      const removedBottom = rect.y + rect.h;
      const verticalOverlap = overlapsVertically(entry);
      const horizontalOverlap = overlapsHorizontally(entry);

      let lane = 4;
      let primary = centerDistance(entry);
      let secondary = 0;

      if (verticalOverlap && entryRight <= rect.x) {
        lane = 0;
        primary = rect.x - entryRight;
        secondary = Math.abs(entry.rect.y - rect.y);
      } else if (verticalOverlap && entry.rect.x >= removedRight) {
        lane = 1;
        primary = entry.rect.x - removedRight;
        secondary = Math.abs(entry.rect.y - rect.y);
      } else if (horizontalOverlap && entryBottom <= rect.y) {
        lane = 2;
        primary = rect.y - entryBottom;
        secondary = Math.abs(entry.rect.x - rect.x);
      } else if (horizontalOverlap && entry.rect.y >= removedBottom) {
        lane = 3;
        primary = entry.rect.y - removedBottom;
        secondary = Math.abs(entry.rect.x - rect.x);
      }

      return {
        paneId: entry.paneId,
        lane,
        primary,
        secondary,
        distance: centerDistance(entry),
      };
    })
    .sort((left, right) =>
      (left.lane - right.lane)
      || (left.primary - right.primary)
      || (left.secondary - right.secondary)
      || (left.distance - right.distance)
      || left.paneId.localeCompare(right.paneId));

  return scored[0]?.paneId || null;
};

const getFallbackNodeAfterClose = (nodes, removedNode) => {
  if (nodes.length === 0) {
    return null;
  }

  const sameRow = nodes
    .filter((node) => node.y === removedNode.y)
    .sort((left, right) => (left.x - right.x) || left.id.localeCompare(right.id));

  const previousInRow = sameRow
    .filter((node) => node.x < removedNode.x)
    .sort((left, right) => (right.x - left.x) || left.id.localeCompare(right.id))[0];
  if (previousInRow) {
    return previousInRow;
  }

  const nextInRow = sameRow.find((node) => node.x >= removedNode.x);
  if (nextInRow) {
    return nextInRow;
  }

  return nodes
    .map((node) => ({
      node,
      rowDistance: Math.abs(node.y - removedNode.y),
      columnDistance: Math.abs(node.x - removedNode.x),
    }))
    .sort((left, right) =>
      (left.rowDistance - right.rowDistance)
      || (left.columnDistance - right.columnDistance)
      || left.node.id.localeCompare(right.node.id))[0]?.node || null;
};

export const closePanelRecord = (state, panelId) => {
  normalizeWorkspaceState(state);

  const node = getNodeForPanelId(state, panelId);
  const panel = getPanelById(state, panelId);
  if (!node || !panel) {
    return null;
  }

  const removedRect = {
    x: panel.splitX,
    y: panel.splitY,
    w: panel.splitWidth,
    h: panel.splitHeight,
  };
  forgetRowFocus(state, panel.id, panel.contextIndex);

  if (node.panes.length > 1) {
    const fallbackPanelId = getClosestPaneForRect(node, removedRect, panelId);
    const rows = getNodeRows(node)
      .map((row) => row.filter((paneId) => paneId !== panelId))
      .filter((row) => row.length > 0);
    node.panes = node.panes.filter((pane) => pane.id !== panelId);
    applyNodeRows(node, rows);
    syncLegacyPanels(state);
    const nextPanelId = fallbackPanelId || getPanelFocusFallback(node)?.id || null;
    if (nextPanelId) {
      focusPanel(state, nextPanelId);
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
  const fallbackNode = getFallbackNodeAfterClose(sameContextNodes, node);
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
    (direction === DIRECTIONS.left && dx >= 0)
    || (direction === DIRECTIONS.right && dx <= 0)
    || (direction === DIRECTIONS.up && dy >= 0)
    || (direction === DIRECTIONS.down && dy <= 0)
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

const getRectHorizontalOverlap = (left, right) =>
  Math.max(0, Math.min(left.x + left.w, right.x + right.w) - Math.max(left.x, right.x));

const getRectVerticalOverlap = (left, right) =>
  Math.max(0, Math.min(left.y + left.h, right.y + right.h) - Math.max(left.y, right.y));

const getIntraNodeDirectionalNeighbor = (node, activePane, direction) => {
  if (!node || node.panes.length <= 1) {
    return null;
  }

  const rows = getNodeRows(node);
  const rowIndex = rows.findIndex((row) => row.includes(activePane.id));
  if (rowIndex === -1) {
    return null;
  }

  const columnIndex = rows[rowIndex].indexOf(activePane.id);
  if (direction === DIRECTIONS.left) {
    const previousPaneId = rows[rowIndex][columnIndex - 1];
    return node.panes.find((pane) => pane.id === previousPaneId) || null;
  }

  if (direction === DIRECTIONS.right) {
    const nextPaneId = rows[rowIndex][columnIndex + 1];
    return node.panes.find((pane) => pane.id === nextPaneId) || null;
  }

  const entries = getNodeLayoutEntries(node);
  const activeEntry = entries.find((entry) => entry.paneId === activePane.id);
  if (!activeEntry) {
    return null;
  }

  const targetRowIndex = direction === DIRECTIONS.up ? rowIndex - 1 : rowIndex + 1;
  const targetRow = rows[targetRowIndex];
  if (!targetRow) {
    return null;
  }

  const candidates = targetRow
    .map((paneId) => {
      const entry = entries.find((item) => item.paneId === paneId) || null;
      const pane = node.panes.find((item) => item.id === paneId) || null;
      if (!entry || !pane) {
        return null;
      }

      const overlap = direction === DIRECTIONS.up || direction === DIRECTIONS.down
        ? getRectHorizontalOverlap(activeEntry.rect, entry.rect)
        : getRectVerticalOverlap(activeEntry.rect, entry.rect);
      const score = direction === DIRECTIONS.up || direction === DIRECTIONS.down
        ? Math.abs((entry.rect.x + (entry.rect.w / 2)) - (activeEntry.rect.x + (activeEntry.rect.w / 2)))
        : Math.abs((entry.rect.y + (entry.rect.h / 2)) - (activeEntry.rect.y + (activeEntry.rect.h / 2)));

      return {
        pane,
        overlap,
        score,
      };
    })
    .filter(Boolean)
    .sort((left, right) =>
      (right.overlap - left.overlap)
      || (left.score - right.score)
      || left.pane.id.localeCompare(right.pane.id));

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
    const xAlignedNode = rowNodes.reduce((best, node) =>
      !best || Math.abs(node.x - activeNode.x) < Math.abs(best.x - activeNode.x) ? node : best, null);

    return rememberedNode || xAlignedNode || null;
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
