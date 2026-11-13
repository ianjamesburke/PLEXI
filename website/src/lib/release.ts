/**
 * Release facts shown across the site. Change them here only.
 * Source: https://github.com/ianjamesburke/PLEXI/releases
 */
const REPO = 'https://github.com/ianjamesburke/PLEXI';

export const release = {
  repo: REPO,
  releases: `${REPO}/releases`,
  beta: {
    version: '0.5.0',
    name: '0.5 Groundwork',
    label: '0.5 Groundwork (0.5.0)',
    tag: 'v0.5.0-beta.1',
    url: `${REPO}/releases/tag/v0.5.0-beta.1`,
    platform: 'macOS on Apple Silicon',
  },
  stable: {
    tag: 'v0.3.5',
    url: `${REPO}/releases/tag/v0.3.5`,
  },
  next: '0.6 Palette',
  installBeta: 'curl -fsSL https://plexiapp.com/install | bash -s -- --channel beta',
  unsignedGuide: `${REPO}/blob/alpha/docs/unsigned-install.md`,
} as const;
