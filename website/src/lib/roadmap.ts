/**
 * The public roadmap at /roadmap. The page renders only from this file.
 *
 * To edit: change a release's `status` or an item's `status`, add or remove
 * items, or add a release. Order in the list is the order on the page.
 * `slug` is the link anchor (plexiapp.com/roadmap#palette), so keep it stable
 * once people have the link. No dates or time estimates.
 * The current beta's name and tag come from release.ts; change them there.
 */
import { release } from './release';

export type ReleaseStatus = 'shipped' | 'in progress' | 'next' | 'planned' | 'after 1.0';
export type ItemStatus = 'shipped' | 'in progress' | 'planned';

export interface RoadmapItem {
  text: string;
  status: ItemStatus;
}

export interface RoadmapRelease {
  slug: string;
  /** Number and name together, e.g. "0.6 Palette". A number is never shown without its name. */
  title: string;
  status: ReleaseStatus;
  /** One sentence. */
  purpose: string;
  items: RoadmapItem[];
  /** Optional: what this release changes for someone using Plexi. */
  forYou?: string;
  /** Optional: the published build, when there is one. */
  build?: { label: string; url: string };
}

/** The release a visitor can install today. The page marks it "You are here". */
export const currentSlug = 'groundwork';

export const roadmap: RoadmapRelease[] = [
  {
    slug: 'groundwork',
    title: release.beta.name,
    status: 'shipped',
    purpose: 'Sets up the foundation the next releases build on.',
    build: { label: release.beta.tag, url: release.beta.url },
    items: [
      { text: 'Apps are now plugins. What a plugin draws in a pane is a view.', status: 'shipped' },
      { text: 'Plexi checks for new builds and updates itself.', status: 'shipped' },
      { text: 'Nightly alpha builds for anyone who wants the newest work.', status: 'shipped' },
      { text: 'plexi plugin check points out layout problems before you share a plugin.', status: 'shipped' },
      { text: 'Groundwork for the permission system hardening that continues in the next releases.', status: 'shipped' },
      { text: 'Patch releases carry fixes as they are finished.', status: 'in progress' },
    ],
    forYou: 'You can install it today. It is an early beta for following along, so install only plugins you wrote or trust.',
  },
  {
    slug: 'palette',
    title: release.next,
    status: 'next',
    purpose: 'One picker, built into Plexi, finds files, commands, notes and agents.',
    items: [
      { text: 'The design of the shared picker.', status: 'in progress' },
      { text: 'Cmd+P opens any file by name, with no Explorer open.', status: 'planned' },
      { text: 'Tab switches the palette between views: everything, files, commands, notes and pinned lists.', status: 'planned' },
      { text: 'Commands that need a name or a folder ask for it right in the palette.', status: 'planned' },
      { text: 'The Explorer runs on the same picker, opens beside your work and remembers your settings.', status: 'planned' },
      { text: 'Commands that plugins add show up in the palette, labelled with their plugin.', status: 'planned' },
      { text: 'Agents you define appear by name. Pick one and it picks up its last chat.', status: 'planned' },
      { text: 'Permission system hardening continues.', status: 'planned' },
    ],
    forYou: 'The picker is part of Plexi itself in this release. Plugins get to build on it in 0.7 Workbench. Still an early beta.',
  },
  {
    slug: 'workbench',
    title: '0.7 Workbench',
    status: 'planned',
    purpose: 'The first release meant for people who do not know me.',
    items: [
      { text: 'Views you keep open sit in a split beside your work. Quick ones open in a small window on top.', status: 'planned' },
      { text: 'Notes becomes a plugin, built on the same tools any plugin author gets.', status: 'planned' },
      { text: 'A Plugins manager for turning plugins on and off.', status: 'planned' },
      { text: 'Spellcheck for the file you have open.', status: 'planned' },
      { text: 'Plugins can use the picker and add their own views to the palette.', status: 'planned' },
      { text: 'Each plugin gets one defined place to keep its saved data.', status: 'planned' },
      { text: 'One permission broker: every request for access goes through the same place.', status: 'planned' },
      { text: 'Code identity: Plexi and the plugins you approve are signed and identified, so a permission stays with the code you gave it to.', status: 'planned' },
      { text: 'A walkthrough of twelve everyday steps passes on a fresh install.', status: 'planned' },
    ],
    forYou: 'When you upgrade, Plexi asks you once to approve your plugins again.',
  },
  {
    slug: 'writing-desk',
    title: '0.8 Writing Desk',
    status: 'planned',
    purpose: 'A text editor built into Plexi that plugins can extend.',
    items: [
      { text: 'A text editor in Plexi itself, with a place for plugins to add to it.', status: 'planned' },
      { text: 'Tabs for working across several files.', status: 'planned' },
      { text: 'Undo that works across Plexi.', status: 'planned' },
      { text: 'Images attached to notes show in the notes preview.', status: 'planned' },
    ],
  },
  {
    slug: '1-0',
    title: '1.0',
    status: 'planned',
    purpose: 'Stable.',
    items: [
      { text: 'Install plugins other people wrote.', status: 'planned' },
      { text: 'The Explorer rebuilt as a plugin that ships with Plexi.', status: 'planned' },
      { text: 'Builds signed and notarized by Apple.', status: 'planned' },
      { text: 'A published licence for the code.', status: 'planned' },
    ],
  },
  {
    slug: 'after-1-0',
    title: 'After 1.0',
    status: 'after 1.0',
    purpose: 'Work that waits until 1.0 is out.',
    items: [
      { text: 'The Assistant, an agent that can work across all of Plexi.', status: 'planned' },
      { text: 'Windows and Linux.', status: 'planned' },
      { text: 'A sandbox for the agents you run.', status: 'planned' },
    ],
  },
];

/** Where "Tell me what you think" points. GitHub Discussions is off on the repo, so this is a new issue. */
export const feedbackUrl = `${release.repo}/issues/new`;
