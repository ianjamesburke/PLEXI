const TERMINAL_FONT_FAMILY = [
  '"Plexi Terminal"',
  '"JetBrainsMono Nerd Font Mono"',
  '"JetBrains Mono"',
  '"Symbols Nerd Font Mono"',
  '"MesloLGM Nerd Font Mono"',
  '"MesloLGSDZ Nerd Font Mono"',
  '"Hack Nerd Font Mono"',
  '"0xProto Nerd Font Mono"',
  '"Menlo"',
  '"Monaco"',
  "monospace",
].join(", ");

export const STORAGE_KEY = "plexi.workspace.v2";

// The workspace file loaded on startup. Maps to ~/.plexi/workspaces/default.json.
// Multiple named workspaces can exist alongside it, but this is always the one
// Plexi opens automatically. Switching workspaces at runtime isn't supported yet.
export const DEFAULT_WORKSPACE_NAME = "default";
export const MAX_BUFFER_CHARS = 120000;
export const platformName = navigator.userAgentData?.platform || navigator.platform || navigator.userAgent;
export const isMacOS = /\bMac/i.test(platformName);

export const ASSET_CANDIDATES = {
  xtermCss: [
    "./vendor/xterm/xterm.css",
  ],
  xtermJs: [
    "./vendor/xterm/xterm.js",
  ],
  fitJs: [
    "./vendor/xterm/addon-fit.js",
  ],
  webLinksJs: [
    "./vendor/xterm/addon-web-links.js",
  ],
  webGlJs: [
    "./vendor/xterm/addon-webgl.js",
  ],
  unicode11Js: [
    "./vendor/xterm/addon-unicode11.js",
  ],
};

export const TERMINAL_PROFILE = {
  cursorBlink: true,
  convertEol: false,
  fontFamily: TERMINAL_FONT_FAMILY,
  fontSize: 14,
  fontWeight: "400",
  fontWeightBold: "600",
  letterSpacing: 0,
  lineHeight: 1,
  drawBoldTextInBrightColors: false,
  allowTransparency: false,
  theme: {
    background: "#0d0f13",
    foreground: "#f3f5f7",
    cursor: "#d57936",
    selectionBackground: "rgba(213, 121, 54, 0.3)",
    black: "#0d0f13",
    brightBlack: "#66707b",
    red: "#ef8b7b",
    brightRed: "#f0a79c",
    green: "#91c27a",
    brightGreen: "#acd494",
    yellow: "#d7b36d",
    brightYellow: "#ebca8d",
    blue: "#7da3d8",
    brightBlue: "#9db8e4",
    magenta: "#bc8ed8",
    brightMagenta: "#cea8e4",
    cyan: "#6cb8bd",
    brightCyan: "#8cccd0",
    white: "#d8dde3",
    brightWhite: "#ffffff",
  },
};

export function applyPlatformClasses() {
  document.documentElement.style.setProperty("--plexi-font-mono", TERMINAL_FONT_FAMILY);
  document.body.classList.toggle("platform-macos", isMacOS);
}
