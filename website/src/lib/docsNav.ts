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
      { slug: 'quick-note', title: 'Quick Note' },
    ],
  },
  {
    section: 'Concepts',
    items: [
      { slug: 'pgap', title: 'PGAP' },
      { slug: 'panes', title: 'Panes & Pages' },
      { slug: 'apps', title: 'Apps' },
      { slug: 'secrets', title: 'Secrets' },
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
