const MOCK_HELP = "help  clear  pwd  ls  echo  split-right  split-down  map  focus  close";
const cwdSequence = (cwd) => `\u001b]633;PlexiCwd=${cwd}\u0007`;

function promptFor(session) {
  return `plexi:${session.cwdLabel}$ `;
}

function createSessionRecord(params) {
  const cwd = params.cwd || "/mock/project";
  return {
    panelId: params.panelId,
    cwd,
    cwdLabel: params.cwdLabel || cwd,
    input: "",
  };
}

function emitPrompt(handlers, session) {
  handlers.onOutput?.({
    panelId: session.panelId,
    data: `${cwdSequence(session.cwd)}${promptFor(session)}`,
  });
}

function executeMockCommand(session, command, handlers) {
  const normalized = command.trim();

  if (!normalized) {
    handlers.onOutput?.({
      panelId: session.panelId,
      data: "\r\n",
    });
    emitPrompt(handlers, session);
    return;
  }

  const [verb, ...args] = normalized.split(/\s+/);

  handlers.onOutput?.({
    panelId: session.panelId,
    data: "\r\n",
  });

  switch (verb) {
    case "help":
      handlers.onOutput?.({
        panelId: session.panelId,
        data: `${MOCK_HELP}\r\n`,
      });
      break;
    case "pwd":
      handlers.onOutput?.({
        panelId: session.panelId,
        data: `${session.cwd}\r\n`,
      });
      break;
    case "cd": {
      const target = args[0];

      if (!target || target === "~") {
        session.cwd = "/mock/project";
        session.cwdLabel = "~/project";
      } else if (target.startsWith("/")) {
        session.cwd = target;
        session.cwdLabel = target;
      } else {
        const nextPath = `${session.cwd.replace(/\/+$/, "")}/${target}`.replace(/\/+/g, "/");
        session.cwd = nextPath;
        session.cwdLabel = nextPath;
      }
      break;
    }
    case "ls":
      handlers.onOutput?.({
        panelId: session.panelId,
        data: "README.md  src/  tests/  package.json\r\n",
      });
      break;
    case "echo":
      handlers.onOutput?.({
        panelId: session.panelId,
        data: `${args.join(" ")}\r\n`,
      });
      break;
    case "split-right":
      handlers.onWorkspaceCommand?.("new-terminal-right");
      break;
    case "split-down":
      handlers.onWorkspaceCommand?.("new-terminal-down");
      break;
    case "map":
      handlers.onWorkspaceCommand?.("toggle-overview");
      break;
    case "focus":
      handlers.onWorkspaceCommand?.("toggle-overview");
      handlers.onWorkspaceCommand?.("toggle-overview");
      break;
    case "close":
      handlers.onWorkspaceCommand?.("close-terminal");
      break;
    case "clear":
      handlers.onClear?.(session.panelId);
      break;
    default:
      handlers.onOutput?.({
        panelId: session.panelId,
        data: `Unknown command: ${verb}\r\n`,
      });
      break;
  }

  emitPrompt(handlers, session);
}

export function createMockSessionBridge(handlers = {}) {
  const sessions = new Map();

  return {
    mode: "mock",
    async getBackendInfo() {
      return {
        backend: "mock",
        platform: "browser",
        supported: true,
        shellPath: null,
        shellName: "mock shell",
      };
    },
    async openSession(params) {
      const existing = sessions.get(params.panelId);
      if (existing) {
        return existing.started;
      }

      const session = createSessionRecord(params);
      const started = {
        panelId: session.panelId,
        cwd: session.cwd,
        cwdLabel: session.cwdLabel,
        shellPath: "/mock/shell",
        shellName: "mock shell",
        backend: "mock",
      };

      session.started = started;
      sessions.set(session.panelId, session);

      handlers.onStarted?.(started);
      handlers.onOutput?.({
        panelId: session.panelId,
        data: "Plexi mock shell\r\nType `help` to inspect keyboard-driven workspace commands.\r\n",
      });
      emitPrompt(handlers, session);
      return started;
    },
    async writeToSession({ panelId, data }) {
      const session = sessions.get(panelId);
      if (!session) {
        return;
      }

      for (const char of data) {
        if (char === "\r") {
          executeMockCommand(session, session.input, handlers);
          session.input = "";
          continue;
        }

        if (char === "\u007f") {
          if (session.input.length > 0) {
            session.input = session.input.slice(0, -1);
            handlers.onOutput?.({
              panelId,
              data: "\b \b",
            });
          }
          continue;
        }

        if (char === "\u0003") {
          session.input = "";
          handlers.onOutput?.({
            panelId,
            data: "^C\r\n",
          });
          emitPrompt(handlers, session);
          continue;
        }

        if (/^[\x20-\x7E\n\t]$/.test(char)) {
          session.input += char;
          handlers.onOutput?.({
            panelId,
            data: char,
          });
        }
      }
    },
    async resizeSession(_params) {},
    async closeSession({ panelId }) {
      if (!sessions.has(panelId)) {
        return;
      }

      sessions.delete(panelId);
      handlers.onExit?.({
        panelId,
        exitCode: 0,
      });
    },
    async getWorkspaceStorageInfo() {
      return {
        path: "Browser storage",
        source: "browser",
      };
    },
    async readWorkspaceDocument() {
      return {
        path: "Browser storage",
        document: null,
      };
    },
    async writeWorkspaceDocument(_params) {
      return {
        path: "Browser storage",
        source: "browser",
      };
    },
    async reset() {
      sessions.clear();
    },
  };
}
