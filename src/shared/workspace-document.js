import {
  NODE_TYPES,
  ensureActivePanel,
  makeDefaultState,
  normalizeWorkspaceState,
} from "./workspace-state.js";

const DEFAULT_TERMINAL = {
  engine: "xterm",
  fontFamily: "Plexi Terminal",
  fontSize: 14,
  cursorStyle: "block",
  theme: "plexi-dark",
};

function coerceNumber(value, fallback) {
  return Number.isFinite(value) ? value : fallback;
}

function normalizeContextLabel(label, index) {
  const trimmed = String(label || "").trim();
  return trimmed || `Context ${index + 1}`;
}

function sortByGrid(left, right) {
  if (left.y !== right.y) {
    return left.y - right.y;
  }

  if (left.x !== right.x) {
    return left.x - right.x;
  }

  return left.id.localeCompare(right.id);
}

function sortPanels(panels) {
  return [...panels].sort(sortByGrid);
}

function sortNodes(nodes) {
  return [...nodes].sort(sortByGrid);
}

function serializePane(pane) {
  const { transcript, contextIndex, x, y, nodeId, ...rest } = pane;
  return {
    ...rest,
  };
}

function serializeNode(node) {
  return {
    id: node.id,
    type: node.type || NODE_TYPES.single,
    label: String(node.label || ""),
    x: coerceNumber(node.x, 0),
    y: coerceNumber(node.y, 0),
    activePaneId: node.activePaneId || node.panes[0]?.id || null,
    layout: node.layout ? JSON.parse(JSON.stringify(node.layout)) : null,
    panes: node.panes.map(serializePane),
  };
}

function createPaneFromDocument(panel, fallbackId, index) {
  return {
    id: String(panel?.id || fallbackId),
    type: panel?.type === "browser" ? "browser" : "terminal",
    title: String(panel?.title || `Terminal ${index + 1}`),
    transcript: [],
    hasReceivedInput: panel?.hasReceivedInput === true,
    fontSize: coerceNumber(panel?.fontSize, 14),
    cwd: String(panel?.cwd || "~"),
    cwdLabel: String(panel?.cwdLabel || panel?.cwd || "~"),
    splitX: coerceNumber(panel?.splitX, 0),
    splitY: coerceNumber(panel?.splitY, 0),
    splitWidth: coerceNumber(panel?.splitWidth, 4),
    splitHeight: coerceNumber(panel?.splitHeight, 4),
  };
}

function createNodeFromLegacyPanel(panel, contextIndex, panelIndex) {
  const pane = createPaneFromDocument(panel, `panel-${contextIndex + 1}-${panelIndex + 1}`, panelIndex);
  return {
    id: `node-${pane.id.match(/(\d+)$/)?.[1] || `${contextIndex + 1}-${panelIndex + 1}`}`,
    type: NODE_TYPES.single,
    x: coerceNumber(panel?.x, panelIndex),
    y: coerceNumber(panel?.y, 0),
    contextIndex,
    label: "",
    activePaneId: pane.id,
    panes: [pane],
    layout: {
      type: "pane",
      paneId: pane.id,
    },
  };
}

function createNodeFromDocument(node, contextIndex, nodeIndex) {
  const panes = Array.isArray(node?.panes) ? node.panes : [];
  const normalizedPanes = panes.length > 0
    ? panes.map((pane, paneIndex) =>
      createPaneFromDocument(pane, `panel-${contextIndex + 1}-${nodeIndex + 1}-${paneIndex + 1}`, paneIndex))
    : [createPaneFromDocument(null, `panel-${contextIndex + 1}-${nodeIndex + 1}`, nodeIndex)];

  return {
    id: String(node?.id || `node-${contextIndex + 1}-${nodeIndex + 1}`),
    type: node?.type === NODE_TYPES.splitGroup ? NODE_TYPES.splitGroup : NODE_TYPES.single,
    x: coerceNumber(node?.x, nodeIndex),
    y: coerceNumber(node?.y, 0),
    contextIndex,
    label: String(node?.label || ""),
    activePaneId: String(node?.activePaneId || normalizedPanes[0]?.id || ""),
    panes: normalizedPanes,
    layout: node?.layout ? JSON.parse(JSON.stringify(node.layout)) : null,
  };
}

function getDocumentNodes(context, index) {
  if (Array.isArray(context?.nodes) && context.nodes.length > 0) {
    return context.nodes.map((node, nodeIndex) => createNodeFromDocument(node, index, nodeIndex));
  }

  const contextPanels = Array.isArray(context?.panels) ? context.panels : [];
  return contextPanels.map((panel, panelIndex) => createNodeFromLegacyPanel(panel, index, panelIndex));
}

export function serializeWorkspaceDocument(state) {
  normalizeWorkspaceState(state);

  const contexts = state.contexts.map((context, index) => {
    const nodes = sortNodes(state.nodes.filter((node) => node.contextIndex === index));
    const panels = sortPanels(nodes.flatMap((node) => node.panes));

    return {
      id: String(context.id || `context-${index + 1}`),
      label: String(context.label || "").trim(),
      pinned: context.pinned === true,
      activeNodeId: state.activeNodeIdsByContext?.[index] || null,
      activePanelId: state.activePanelIdsByContext?.[index] || null,
      nodes: nodes.map(serializeNode),
      panels: panels.map((panel) => ({
        ...serializePane(panel),
        x: coerceNumber(panel.x, 0),
        y: coerceNumber(panel.y, 0),
      })),
    };
  });

  return {
    version: 2,
    workspace: {
      title: "Plexi Workspace",
      sequence: coerceNumber(state.sequence, 0),
      activeContextIndex: coerceNumber(state.activeContextIndex, 0),
      sidebarVisible: state.sidebarVisible !== false,
      minimapVisible: state.minimapVisible !== false,
      camera: {
        x: coerceNumber(state.camera?.x, 0),
        y: coerceNumber(state.camera?.y, 0),
        zoom: coerceNumber(state.camera?.zoom, 1),
      },
      contexts,
    },
    terminal: {
      ...DEFAULT_TERMINAL,
    },
    keyboard: {},
  };
}

export function deserializeWorkspaceDocument(document) {
  const nextState = makeDefaultState();
  const workspace = document?.workspace || {};
  const contexts = Array.isArray(workspace.contexts) ? workspace.contexts : [];
  const activePanelIdsByContext = {};
  const activeNodeIdsByContext = {};

  nextState.contexts = contexts.map((context, index) => ({
    id: String(context?.id || `context-${index + 1}`),
    label: String(context?.label || "").trim(),
    pinned: context?.pinned === true,
  }));

  nextState.nodes = contexts.flatMap((context, index) => {
    activePanelIdsByContext[index] = context?.activePanelId || null;
    activeNodeIdsByContext[index] = context?.activeNodeId || null;
    return getDocumentNodes(context, index);
  });

  nextState.sequence = Math.max(
    coerceNumber(workspace.sequence, 0),
    ...nextState.nodes.flatMap((node) => [
      Number(node.id.match(/(\d+)$/)?.[1] || 0),
      ...node.panes.map((pane) => Number(pane.id.match(/(\d+)$/)?.[1] || 0)),
    ]),
  );
  nextState.activeContextIndex = Math.max(
    0,
    Math.min(
      coerceNumber(workspace.activeContextIndex, 0),
      Math.max(0, nextState.contexts.length - 1),
    ),
  );
  nextState.activeNodeIdsByContext = activeNodeIdsByContext;
  nextState.activePanelIdsByContext = activePanelIdsByContext;
  nextState.camera = {
    x: coerceNumber(workspace.camera?.x, 0),
    y: coerceNumber(workspace.camera?.y, 0),
    zoom: coerceNumber(workspace.camera?.zoom, 1),
  };
  nextState.sidebarVisible = workspace.sidebarVisible !== false;
  nextState.minimapVisible = workspace.minimapVisible !== false;
  nextState.shortcutsVisible = false;
  nextState.mode = "focus";
  nextState.lastAction = "Workspace restored";

  normalizeWorkspaceState(nextState);
  nextState.activeNodeId = activeNodeIdsByContext[nextState.activeContextIndex] || null;
  nextState.activePanelId = activePanelIdsByContext[nextState.activeContextIndex] || null;
  ensureActivePanel(nextState);
  return nextState;
}

export function formatWorkspaceDocumentJson(state) {
  return JSON.stringify(serializeWorkspaceDocument(state), null, 2);
}

export function migrateLegacyWorkspaceState(parsed) {
  const nextState = makeDefaultState();
  const parsedPanels = Array.isArray(parsed?.panels)
    ? parsed.panels.map((panel, index) => ({
      ...panel,
      id: String(panel.id || `panel-${index + 1}`),
      title: String(panel.title || `Terminal ${index + 1}`),
      contextIndex: coerceNumber(panel.contextIndex, 0),
      cwd: panel.cwd || "~",
      cwdLabel: panel.cwdLabel || panel.cwd || "~",
      transcript: [],
      hasReceivedInput: panel.hasReceivedInput === true,
    }))
    : [];
  const parsedContexts = Array.isArray(parsed?.contexts)
    ? parsed.contexts.map((context, index) => ({
      id: String(context.id || `context-${index + 1}`),
      label: String(context.label || "").trim(),
      pinned: context.pinned === true,
    }))
    : [];
  const inferredContextCount = parsedContexts.length === 0 && parsedPanels.length > 0
    ? Math.max(...parsedPanels.map((panel) => panel.contextIndex || 0)) + 1
    : 0;

  nextState.contexts = parsedContexts.length > 0
    ? parsedContexts
    : Array.from({ length: inferredContextCount }, (_value, index) => ({
      id: `context-${index + 1}`,
      label: "",
      pinned: false,
    }));
  nextState.nodes = parsedPanels.map((panel, index) =>
    createNodeFromLegacyPanel(panel, panel.contextIndex, index));
  nextState.sequence = coerceNumber(parsed?.sequence, parsedPanels.length);
  nextState.activeContextIndex = coerceNumber(parsed?.activeContextIndex, 0);
  nextState.activeNodeIdsByContext = parsed?.activeNodeIdsByContext || {};
  nextState.activePanelIdsByContext = parsed?.activePanelIdsByContext || {};
  nextState.activeNodeId = parsed?.activeNodeId || null;
  nextState.activePanelId = parsed?.activePanelId || null;
  nextState.previousPanelId = parsed?.previousPanelId || null;
  nextState.camera = {
    x: coerceNumber(parsed?.camera?.x, 0),
    y: coerceNumber(parsed?.camera?.y, 0),
    zoom: coerceNumber(parsed?.camera?.zoom, 1),
  };
  nextState.sidebarVisible = parsed?.sidebarVisible !== false;
  nextState.minimapVisible = parsed?.minimapVisible !== false;
  nextState.shortcutsVisible = false;
  nextState.mode = "focus";
  nextState.lastAction = "Workspace restored";

  normalizeWorkspaceState(nextState);
  ensureActivePanel(nextState);
  return nextState;
}

export function getDisplayContextLabel(label, index) {
  return normalizeContextLabel(label, index);
}
