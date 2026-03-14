import { makeDefaultState, ensureActivePanel } from "./workspace-state.js";

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

function sortPanels(panels) {
  return [...panels].sort((left, right) => {
    if (left.y !== right.y) {
      return left.y - right.y;
    }

    if (left.x !== right.x) {
      return left.x - right.x;
    }

    return left.id.localeCompare(right.id);
  });
}

export function serializeWorkspaceDocument(state) {
  const contexts = state.contexts.map((context, index) => {
    const panels = sortPanels(
      state.panels
        .filter((panel) => panel.contextIndex === index)
        .map(({ transcript, contextIndex, ...panel }) => ({
          ...panel,
        })),
    );

    return {
      id: String(context.id || `context-${index + 1}`),
      label: String(context.label || "").trim(),
      activePanelId: state.activePanelIdsByContext?.[index] || null,
      panels,
    };
  });

  return {
    version: 1,
    workspace: {
      title: "Plexi Workspace",
      sequence: coerceNumber(state.sequence, 0),
      activeContextIndex: coerceNumber(state.activeContextIndex, 0),
      sidebarVisible: state.sidebarVisible !== false,
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
  const panels = [];
  const activePanelIdsByContext = {};

  nextState.contexts = contexts.map((context, index) => ({
    id: String(context?.id || `context-${index + 1}`),
    label: String(context?.label || "").trim(),
  }));

  contexts.forEach((context, index) => {
    activePanelIdsByContext[index] = context?.activePanelId || null;

    const contextPanels = Array.isArray(context?.panels) ? context.panels : [];
    contextPanels.forEach((panel, panelIndex) => {
      panels.push({
        id: String(panel?.id || `panel-${index + 1}-${panelIndex + 1}`),
        type: panel?.type === "browser" ? "browser" : "terminal",
        title: String(panel?.title || `Terminal ${panels.length + 1}`),
        x: coerceNumber(panel?.x, panelIndex),
        y: coerceNumber(panel?.y, 0),
        contextIndex: index,
        transcript: [],
        cwd: String(panel?.cwd || "~"),
        cwdLabel: String(panel?.cwdLabel || panel?.cwd || "~"),
      });
    });
  });

  nextState.panels = panels;
  nextState.sequence = Math.max(
    coerceNumber(workspace.sequence, 0),
    ...panels.map((panel) => {
      const match = panel.id.match(/(\d+)$/);
      return match ? Number(match[1]) : 0;
    }),
  );
  nextState.activeContextIndex = Math.max(
    0,
    Math.min(
      coerceNumber(workspace.activeContextIndex, 0),
      Math.max(0, nextState.contexts.length - 1),
    ),
  );
  nextState.activePanelIdsByContext = activePanelIdsByContext;
  nextState.camera = {
    x: coerceNumber(workspace.camera?.x, 0),
    y: coerceNumber(workspace.camera?.y, 0),
    zoom: coerceNumber(workspace.camera?.zoom, 1),
  };
  nextState.sidebarVisible = workspace.sidebarVisible !== false;
  nextState.shortcutsVisible = false;
  nextState.mode = "focus";
  nextState.lastAction = "Workspace restored";

  if (nextState.contexts.length === 0) {
    nextState.contexts = [
      {
        id: "main",
        label: "Main",
      },
    ];
  }

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
    }))
    : [];
  const parsedContexts = Array.isArray(parsed?.contexts)
    ? parsed.contexts.map((context, index) => ({
      id: String(context.id || `context-${index + 1}`),
      label: String(context.label || "").trim(),
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
    }));
  nextState.panels = parsedPanels;
  nextState.sequence = coerceNumber(parsed?.sequence, parsedPanels.length);
  nextState.activeContextIndex = coerceNumber(parsed?.activeContextIndex, 0);
  nextState.activePanelIdsByContext = parsed?.activePanelIdsByContext || {};
  nextState.activePanelId = parsed?.activePanelId || null;
  nextState.previousPanelId = parsed?.previousPanelId || null;
  nextState.camera = {
    x: coerceNumber(parsed?.camera?.x, 0),
    y: coerceNumber(parsed?.camera?.y, 0),
    zoom: coerceNumber(parsed?.camera?.zoom, 1),
  };
  nextState.sidebarVisible = parsed?.sidebarVisible !== false;
  nextState.shortcutsVisible = false;
  nextState.mode = "focus";
  nextState.lastAction = "Workspace restored";

  if (nextState.contexts.length === 0) {
    nextState.contexts = [
      {
        id: "main",
        label: "Main",
      },
    ];
  }

  ensureActivePanel(nextState);
  return nextState;
}

export function getDisplayContextLabel(label, index) {
  return normalizeContextLabel(label, index);
}
