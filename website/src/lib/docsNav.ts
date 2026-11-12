export interface DocNavItem {
  slug: string;
  title: string;
  /** One line shown on the docs start page and under the article title. */
  summary: string;
  /** Set for pages written by a generator; names the source it is built from. */
  generatedFrom?: string;
}

export interface DocNavSection {
  section: string;
  /** One line shown above the group on the docs start page. */
  blurb: string;
  items: DocNavItem[];
}

export const docsNav: DocNavSection[] = [
  {
    section: 'Start',
    blurb: 'Install Plexi, run an agent in it, then build something for it.',
    items: [
      { slug: 'getting-started', title: 'Install', summary: 'Install on an Apple Silicon Mac, open an unsigned build, learn the first five shortcuts.' },
      { slug: 'first-agent', title: 'Run your first agent', summary: 'Hook up Claude Code, Codex or Pi so Plexi knows when an agent is working, blocked or done.' },
      { slug: 'first-plugin', title: 'Build your first plugin', summary: 'Scaffold a Python plugin, check it, and open its view in a pane.' },
    ],
  },
  {
    section: 'Concepts',
    blurb: 'The few ideas the rest of the docs assume.',
    items: [
      { slug: 'plugins-and-views', title: 'Plugins and views', summary: 'A plugin is a package. A view is what it shows in a pane. Capabilities decide what it may touch.' },
      { slug: 'panes', title: 'Panes', summary: 'Terminal panes and view panes, splitting, focus, and working-directory inheritance.' },
      { slug: 'secrets', title: 'Secrets', summary: 'Store API keys per workspace, and control which terminals, agents and plugins can read them.' },
    ],
  },
  {
    section: 'Built in',
    blurb: 'Tools that ship with the host.',
    items: [
      { slug: 'quick-note', title: 'Quick Note', summary: 'Capture a note from anywhere with one shortcut.' },
      { slug: 'file-explorer', title: 'File Explorer', summary: 'Browse, preview and manage files from the keyboard.' },
    ],
  },
  {
    section: 'Build plugins',
    blurb: 'Authoring guide first, then the references it points into.',
    items: [
      { slug: 'plugins', title: 'Plugin guide', summary: 'The init, update, view pattern, the sandbox a plugin runs in, capabilities and logging.' },
      { slug: 'plugin-manifest', title: 'Plugin manifest', summary: 'Every manifest field and the capability registry.', generatedFrom: 'tools/gen_manifest_docs.py' },
      { slug: 'sdk', title: 'Python SDK', summary: 'Every effect, event and UI component, with signatures.', generatedFrom: 'the SDK docstrings (tools/gen_sdk_docs.py)' },
      { slug: 'pgap', title: 'PGAP protocol', summary: 'The wire protocol between a plugin and the host, message by message.', generatedFrom: 'the protocol schema (tools/gen_capability_docs.py)' },
    ],
  },
  {
    section: 'Reference',
    blurb: 'Generated from the source code, never written by hand.',
    items: [
      { slug: 'cli', title: 'CLI', summary: 'Every plexi command and flag.', generatedFrom: 'the command definitions (gen_cli_docs)' },
      { slug: 'config', title: 'Configuration', summary: 'Every config.toml key, its type and default.', generatedFrom: 'the config structs (gen_config_docs)' },
    ],
  },
];

export interface DocNavEntry extends DocNavItem {
  section: string;
}

export const docsFlat: DocNavEntry[] = docsNav.flatMap(group =>
  group.items.map(item => ({ ...item, section: group.section })),
);

export function docEntry(slug: string | undefined): DocNavEntry | undefined {
  return docsFlat.find(item => item.slug === slug);
}

/** Previous and next page in reading order, for the article footer. */
export function docNeighbors(slug: string | undefined): { prev?: DocNavEntry; next?: DocNavEntry } {
  const index = docsFlat.findIndex(item => item.slug === slug);
  if (index === -1) return {};
  return { prev: docsFlat[index - 1], next: docsFlat[index + 1] };
}
