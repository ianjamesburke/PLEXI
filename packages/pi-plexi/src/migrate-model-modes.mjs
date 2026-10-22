import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, readlinkSync, realpathSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import { homedir, userInfo } from 'node:os';
import { dirname, join, resolve, relative, isAbsolute } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createHash, randomUUID } from 'node:crypto';

const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const inside = (path, root) => { const rel = relative(root, path); return !rel || (!rel.startsWith('..') && !isAbsolute(rel)); };
function canonical(path) {
  const absolute = resolve(path);
  if (existsSync(absolute)) return realpathSync(absolute);
  // existsSync is false for a dangling symlink. Resolve its destination too,
  // so a not-yet-created personal profile cannot bypass the refusal.
  try {
    if (lstatSync(absolute).isSymbolicLink()) return canonical(resolve(dirname(absolute), readlinkSync(absolute)));
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  return join(canonical(dirname(absolute)), absolute.slice(dirname(absolute).length + 1));
}
export function safePath(path) {
  if (!path) throw new Error('An explicit --profile and --backup are required');
  const target = canonical(path);
  for (const home of [homedir(), userInfo().homedir]) {
    const forbidden = canonical(join(home, '.pi'));
    if (inside(target, forbidden)) throw new Error(`Refusing personal .pi path: ${path}`);
  }
  return target;
}
function regular(path) {
  if (lstatSync(path).isSymbolicLink() || !lstatSync(path).isFile()) throw new Error(`Expected regular file: ${path}`);
  safePath(path);
}
function file(profile, name) {
  const path = join(profile, name);
  if (!inside(canonical(path), profile)) throw new Error(`Path escapes profile: ${name}`);
  safePath(path);
  return path;
}
export function migrateSettings(settings) {
  const result = structuredClone(settings);
  if (!result || typeof result !== 'object' || Array.isArray(result)) throw new Error('Expected settings object');
  if (result.defaultProvider === 'ian-router') result.defaultProvider = 'plexi';
  if (result.defaultModel === 'auto') result.defaultModel = 'fast';
  result.defaultThinkingLevel = 'medium';
  result.enabledModels = ['plexi/fast', 'plexi/slow'];
  if (result.packages) result.packages = result.packages.map(entry => {
    const source = typeof entry === 'string' ? entry : entry.source;
    if (!/(?:^|\/)pi-jev(?:@[^/]*)?$/.test(source ?? '')) return entry;
    const pkg = typeof entry === 'string' ? { source: entry } : entry;
    // An empty array deliberately disables every extension; preserve that choice.
    if (pkg.extensions?.length === 0) return pkg;
    return { ...pkg, extensions: [...new Set([...(pkg.extensions ?? []), '-extensions/routing.ts'])] };
  });
  return result;
}
function atomic(path, bytes) {
  const temp = `${path}.${randomUUID()}.tmp`;
  writeFileSync(temp, bytes, { flag: 'wx', mode: 0o600 });
  try { renameSync(temp, path); } finally { if (existsSync(temp)) unlinkSync(temp); }
}
function paths(profile) {
  const names = ['settings.json', 'extensions/subscription-router.ts'];
  const agents = file(profile, 'agents');
  if (existsSync(agents)) for (const name of readdirSync(agents).sort()) if (name.endsWith('.md')) names.push(`agents/${name}`);
  return names;
}
export function backupProfile(profileArg, backupArg) {
  const profile = safePath(profileArg), backup = safePath(backupArg);
  if (inside(backup, profile) || inside(profile, backup)) throw new Error('Backup and profile must be separate directories');
  const entries = paths(profile).map(name => {
    const path = file(profile, name);
    if (!existsSync(path)) return { name, sha256: null };
    regular(path);
    return { name, sha256: hash(readFileSync(path)) };
  });
  mkdirSync(backup, { mode: 0o700 }); // Refuse an existing backup, including on repeat calls.
  for (const entry of entries) if (entry.sha256 !== null) {
    const source = file(profile, entry.name), dest = file(backup, entry.name);
    mkdirSync(dirname(dest), { recursive: true, mode: 0o700 });
    writeFileSync(dest, readFileSync(source), { flag: 'wx', mode: 0o600 });
    if (!readFileSync(dest).equals(readFileSync(source)) || hash(readFileSync(dest)) !== entry.sha256) throw new Error(`Backup verification failed: ${entry.name}`);
  }
  writeFileSync(join(backup, 'manifest.json'), JSON.stringify({ profile, entries }, null, 2), { flag: 'wx', mode: 0o600 });
  console.info('[pi-plexi] profile backup verified');
}
function verifiedBackup(profile, backup) {
  const manifestPath = file(backup, 'manifest.json'); regular(manifestPath);
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  if (manifest.profile !== profile) throw new Error('Backup belongs to a different profile');
  if (!Array.isArray(manifest.entries) || !manifest.entries.some(e => e.name === 'settings.json' && typeof e.sha256 === 'string') || !manifest.entries.some(e => e.name === 'extensions/subscription-router.ts') || new Set(manifest.entries.map(e => e.name)).size !== manifest.entries.length) throw new Error('Incomplete backup manifest');
  for (const entry of manifest.entries) {
    if (!/^(settings\.json|extensions\/subscription-router\.ts|agents\/[^/]+\.md)$/.test(entry.name)) throw new Error('Invalid backup entry');
    file(profile, entry.name);
    if (entry.sha256 !== null) {
      const path = file(backup, entry.name); regular(path);
      if (hash(readFileSync(path)) !== entry.sha256) throw new Error(`Backup verification failed: ${entry.name}`);
    }
  }
  return manifest;
}
export function migrateProfile(profileArg, backupArg) {
  const profile = safePath(profileArg), backup = safePath(backupArg);
  const settingsPath = file(profile, 'settings.json'); regular(settingsPath);
  const settings = JSON.parse(readFileSync(settingsPath, 'utf8'));
  const migrated = migrateSettings(settings);
  const router = file(profile, 'extensions/subscription-router.ts');
  if (existsSync(router)) regular(router);
  if (JSON.stringify(settings) === JSON.stringify(migrated) && !existsSync(router)) return false;
  if (!existsSync(backup)) backupProfile(profile, backup);
  const manifest = verifiedBackup(profile, backup);
  // An existing explicit backup is only usable while every source still matches it.
  for (const entry of manifest.entries) {
    const path = file(profile, entry.name);
    if ((existsSync(path) ? hash(readFileSync(path)) : null) !== entry.sha256) throw new Error(`Profile changed since backup: ${entry.name}`);
  }
  atomic(settingsPath, JSON.stringify(migrated, null, 2) + '\n');
  if (existsSync(router)) unlinkSync(router);
  console.info('[pi-plexi] FAST/SLOW migration applied');
  return true;
}
export function rollbackProfile(profileArg, backupArg) {
  const profile = safePath(profileArg), backup = safePath(backupArg);
  const manifest = verifiedBackup(profile, backup);
  for (const entry of manifest.entries) {
    const dest = file(profile, entry.name);
    if (existsSync(dest)) regular(dest);
  }
  for (const entry of manifest.entries) {
    const dest = file(profile, entry.name);
    if (entry.sha256 === null) { if (existsSync(dest)) unlinkSync(dest); continue; }
    mkdirSync(dirname(dest), { recursive: true, mode: 0o700 });
    const bytes = readFileSync(file(backup, entry.name));
    atomic(dest, bytes);
    if (!bytes.equals(readFileSync(dest))) throw new Error(`Rollback verification failed: ${entry.name}`);
  }
  console.info('[pi-plexi] rollback verified');
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const [command, ...args] = process.argv.slice(2);
    if (args.length !== 4 || args[0] !== '--profile' || args[2] !== '--backup') throw new Error('Usage: migrate-model-modes.mjs backup|migrate|rollback --profile <dir> --backup <dir>');
    const action = { backup: backupProfile, migrate: migrateProfile, rollback: rollbackProfile }[command];
    if (!action) throw new Error('Expected backup, migrate, or rollback');
    action(args[1], args[3]);
  } catch (error) { console.error(`[pi-plexi] migration failed: ${error.message}`); process.exitCode = 1; }
}
