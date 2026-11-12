export interface DocNavItem {
  slug: string;
  title: string;
}

export interface DocNavSection {
  section: string;
  items: DocNavItem[];
}

export const docsNav: DocNavSection[] = [
  {
    section: 'Getting Started',
    items: [
      { slug: 'getting-started', title: 'Installation & Setup' },
      { slug: 'config', title: 'Configuration' },
      { slug: 'quick-note', title: 'Quick Note' },
      { slug: 'file-explorer', title: 'File Explorer' },
    ],
  },
  {
    section: 'Concepts',
    items: [
      { slug: 'pgap', title: 'PGAP' },
      { slug: 'panes', title: 'Panes' },
      { slug: 'plugins', title: 'Plugins' },
      { slug: 'plugin-manifest', title: 'Plugin manifest' },
      { slug: 'secrets', title: 'Secrets' },
    ],
  },
  {
    section: 'SDK Reference',
    items: [
      { slug: 'sdk', title: 'Overview' },
    ],
  },
  {
    section: 'CLI Reference',
    items: [
      { slug: 'cli', title: 'CLI Overview' },
    ],
  },
];

export function allDocSlugs(): string[] {
  return docsNav.flatMap(s => s.items.map(i => i.slug));
}
