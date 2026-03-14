function byId(id) {
  return document.getElementById(id);
}

export const dom = {
  appShell: byId("app-shell"),
  sidebar: byId("sidebar"),
  stage: byId("stage"),
  focusShell: byId("focus-shell"),
  emptyShell: byId("empty-shell"),
  terminalMount: byId("terminal-mount"),
  minimap: byId("minimap"),
  minimapGrid: byId("minimap-grid"),
  minimapSize: byId("minimap-size"),
  shortcutsOverlay: byId("shortcuts-overlay"),
  contextList: byId("context-list"),
  newContextButton: byId("new-context"),
  focusPath: byId("focus-path"),
  focusProcess: byId("focus-process"),
  focusRightSlot: byId("focus-right-slot"),
  focusBottomSlot: byId("focus-bottom-slot"),
  toolbarContext: byId("toolbar-context"),
  workspaceStorageLabel: byId("workspace-storage-label"),
  engineLabel: byId("engine-label"),
  toastLayer: byId("toast-layer"),
  contextModal: byId("context-modal"),
  contextForm: byId("context-form"),
  contextNameInput: byId("context-name-input"),
  contextCancelButton: byId("context-cancel"),
  contextCloseButton: byId("context-close"),
};
