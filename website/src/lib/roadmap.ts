/**
 * The public roadmap at /roadmap. The data lives in roadmap.json; this file
 * checks it and gives it types. The page renders only from it.
 *
 * To edit roadmap.json: change a release's `status` or an item's `status`, add
 * or remove items, or add a release. Order in the list is the order on the page.
 * `slug` is the link anchor (plexiapp.com/roadmap#palette), so keep it stable
 * once people have the link. No dates or time estimates.
 * A release with `"release": "beta"` or `"release": "next"` takes its title
 * (and, for beta, its build link) from release.ts; change them there. Any
 * other release has its own `title`.
 * `stints` lists the stint ids that deliver an item. They are not shown on the
 * page; `just check-roadmap` compares them with the stints and reports drift.
 */
import data from './roadmap.json';
import { release } from './release';

const RELEASE_STATUSES = ['shipped', 'in progress', 'next', 'planned', 'after 1.0'] as const;
const ITEM_STATUSES = ['shipped', 'in progress', 'planned'] as const;

export type ReleaseStatus = (typeof RELEASE_STATUSES)[number];
export type ItemStatus = (typeof ITEM_STATUSES)[number];

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

function oneOf<T extends string>(allowed: readonly T[], value: string, where: string): T {
  if (!(allowed as readonly string[]).includes(value)) {
    throw new Error(`roadmap.json: ${where} has status "${value}"; expected one of ${allowed.join(', ')}`);
  }
  return value as T;
}

function toRelease(r: (typeof data.releases)[number]): RoadmapRelease {
  const bound = 'release' in r ? r.release : undefined;
  const ownTitle = 'title' in r ? r.title : undefined;
  if ((bound === undefined) === (ownTitle === undefined)) {
    throw new Error(`roadmap.json: release "${r.slug}" needs exactly one of "release" or "title"`);
  }
  if (bound !== undefined && bound !== 'beta' && bound !== 'next') {
    throw new Error(`roadmap.json: release "${r.slug}" has "release": "${bound}"; expected "beta" or "next"`);
  }
  return {
    slug: r.slug,
    title: bound === 'beta' ? release.beta.name : bound === 'next' ? release.next : ownTitle!,
    status: oneOf(RELEASE_STATUSES, r.status, `release "${r.slug}"`),
    purpose: r.purpose,
    items: r.items.map(i => ({ text: i.text, status: oneOf(ITEM_STATUSES, i.status, `item "${i.text}"`) })),
    ...('forYou' in r && r.forYou ? { forYou: r.forYou } : {}),
    ...(bound === 'beta' ? { build: { label: release.beta.tag, url: release.beta.url } } : {}),
  };
}

export const roadmap: RoadmapRelease[] = data.releases.map(toRelease);

/** The release a visitor can install today. The page marks it "You are here". */
export const currentSlug: string = data.currentSlug;
if (!roadmap.some(r => r.slug === currentSlug)) {
  throw new Error(`roadmap.json: currentSlug "${currentSlug}" names no release`);
}

/** Where "Tell me what you think" points. GitHub Discussions is off on the repo, so this is a new issue. */
export const feedbackUrl = `${release.repo}/issues/new`;
