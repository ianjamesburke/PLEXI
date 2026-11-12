/**
 * Reads the repo-root CHANGELOG.md (written by git-cliff, never by hand) into versions.
 * The format is line-oriented and owned by cliff.toml:
 *   ## [1.2.3] — 2026-01-31
 *   ### Group
 *   - one change (#123)
 * Any other line shape after the first version heading throws, so a template
 * change in cliff.toml fails the build here instead of rendering garbage.
 */
import raw from '../../../CHANGELOG.md?raw';

export interface ChangeItem { text: string; pr?: number }
export interface ChangeGroup { name: string; items: ChangeItem[] }
export interface ChangeVersion { version: string; date: string; groups: ChangeGroup[] }

const VERSION_PREFIX = '## [';
const GROUP_PREFIX = '### ';
const ITEM_PREFIX = '- ';
const PR_SUFFIX = /\s*\(#(\d+)\)$/;

function parseVersionHeading(line: string, lineNo: number): ChangeVersion {
  const close = line.indexOf(']');
  const version = line.slice(VERSION_PREFIX.length, close);
  const date = line.slice(close + 1).replace('—', '').trim();
  if (close < 0 || !version || !date) {
    throw new Error(`changelog: CHANGELOG.md line ${lineNo} is not "## [version] — date": ${line}`);
  }
  return { version, date, groups: [] };
}

export function parseChangelog(text: string): { versions: ChangeVersion[]; footnote: string } {
  const versions: ChangeVersion[] = [];
  const footnote: string[] = [];
  let inOlder = false;
  text.split('\n').forEach((line, index) => {
    const lineNo = index + 1;
    if (line.startsWith(VERSION_PREFIX)) {
      versions.push(parseVersionHeading(line, lineNo));
      return;
    }
    if (line === '## Older Releases') { inOlder = true; return; }
    if (inOlder) { if (line.trim()) footnote.push(line.trim()); return; }
    if (versions.length === 0 || line.trim() === '') return; // preamble and blank lines
    const current = versions[versions.length - 1];
    if (line.startsWith(GROUP_PREFIX)) {
      current.groups.push({ name: line.slice(GROUP_PREFIX.length).trim(), items: [] });
      return;
    }
    if (line.startsWith(ITEM_PREFIX)) {
      const group = current.groups[current.groups.length - 1];
      if (!group) throw new Error(`changelog: CHANGELOG.md line ${lineNo} is an item before any "### " group`);
      const body = line.slice(ITEM_PREFIX.length).trim();
      const pr = body.match(PR_SUFFIX);
      group.items.push(pr ? { text: body.replace(PR_SUFFIX, ''), pr: Number(pr[1]) } : { text: body });
      return;
    }
    throw new Error(`changelog: CHANGELOG.md line ${lineNo} has an unexpected shape: ${line}`);
  });
  if (versions.length === 0) throw new Error('changelog: CHANGELOG.md has no "## [version]" headings');
  return { versions, footnote: footnote.join(' ') };
}

export const changelog = parseChangelog(raw);
