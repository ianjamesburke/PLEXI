// Injected at build time from the workspace Cargo.toml [package].version
// (see `readCargoVersion` / Vite `define` in astro.config.mjs). It is a
// compile-time constant — no runtime filesystem read — so it works whether a
// route is prerendered or SSR'd, and can never silently drift from the shipped
// binary the way the old hardcoded `0.0.505` constant did.
declare const __PLEXI_VERSION__: string;

export const CURRENT_VERSION = __PLEXI_VERSION__;
