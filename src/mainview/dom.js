function byId(id) {
  return document.getElementById(id);
}

function byClass(cls) {
  return document.getElementsByClassName(cls)[0];
}

export const dom = {
  appShell: byId("app-shell"),
  sidebar: byId("sidebar"),
  stage: byId("stage"),
  focusShell: byId("focus-shell"),
  focusNodeGrid: byId("focus-node-grid"),
  terminalFrame: byClass("terminal-frame"),
  emptyShell: byId("empty-shell"),
  terminalMount: byId("terminal-mount"),
  minimap: byId("minimap"),
  minimapGrid: byId("minimap-grid"),
  minimapSize: byId("minimap-size"),
  overlayMinimap: byId("minimap-overlay"),
  overlayMinimapGrid: byId("minimap-overlay-grid"),
  shortcutsOverlay: byId("shortcuts-overlay"),
  contextList: byId("context-list"),
  newContextButton: byId("new-context"),
  focusPath: byId("focus-path"),
  focusProcess: byId("focus-process"),
  focusRightSlot: byId("focus-right-slot"),
  focusBottomSlot: byId("focus-bottom-slot"),
  toolbarContext: byId("toolbar-context"),
  toastLayer: byId("toast-layer"),
  contextModal: byId("context-modal"),
  contextForm: byId("context-form"),
  contextNameInput: byId("context-name-input"),
  contextCancelButton: byId("context-cancel"),
  contextCloseButton: byId("context-close"),
  contextDeleteButton: byId("context-delete"),
  contextPinButton: byId("context-pin"),
  contextMoveUpButton: byId("context-move-up"),
  contextMoveDownButton: byId("context-move-down"),
};
