import type { ElectrobunRPCSchema } from "electrobun/bun";

export type SessionBackendInfo = {
  backend: "bun-pty" | "mock";
  platform: string;
  supported: boolean;
  shellPath: string | null;
  shellName: string;
};

export type OpenSessionParams = {
  panelId: string;
  cwd?: string | null;
  cols?: number;
  rows?: number;
};

export type SessionStartedMessage = {
  panelId: string;
  cwd: string;
  cwdLabel: string;
  shellPath: string;
  shellName: string;
  backend: "bun-pty" | "mock";
};

export type SessionOutputMessage = {
  panelId: string;
  data: string;
};

export type SessionExitMessage = {
  panelId: string;
  exitCode: number;
};

export type SessionErrorMessage = {
  panelId: string;
  message: string;
};

export type PlexiRPCSchema = ElectrobunRPCSchema & {
  bun: {
    requests: {
      getBackendInfo: {
        params: void;
        response: SessionBackendInfo;
      };
      openSession: {
        params: OpenSessionParams;
        response: SessionStartedMessage;
      };
      writeToSession: {
        params: {
          panelId: string;
          data: string;
        };
        response: void;
      };
      resizeSession: {
        params: {
          panelId: string;
          cols: number;
          rows: number;
        };
        response: void;
      };
      closeSession: {
        params: {
          panelId: string;
        };
        response: void;
      };
    };
    messages: Record<never, never>;
  };
  webview: {
    requests: Record<never, never>;
    messages: {
      sessionStarted: SessionStartedMessage;
      sessionOutput: SessionOutputMessage;
      sessionExit: SessionExitMessage;
      sessionError: SessionErrorMessage;
    };
  };
};
