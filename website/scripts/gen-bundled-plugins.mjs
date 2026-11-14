#!/usr/bin/env node
// Writes src/lib/bundled-plugins.json from packs/core.toml and each bundled
// plugin's manifest.toml. The site never types this list by hand.
//   node scripts/gen-bundled-plugins.mjs          regenerate
//   node scripts/gen-bundled-plugins.mjs --check  exit 1 if the committed file is stale
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'smol-toml';

const website = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(website, '..');
const outPath = join(website, 'src/lib/bundled-plugins.json');

function readToml(path) {
  let text;
  try {
    text = readFileSync(path, 'utf8');
  } catch (error) {
    throw new Error(`gen-bundled-plugins: cannot read ${path}: ${error.message}`);
  }
  try {
    return parse(text);
  } catch (error) {
    throw new Error(`gen-bundled-plugins: ${path} is not valid TOML: ${error.message}`);
  }
}

function required(value, what, path) {
  if (value === undefined || value === null || value === '') {
    throw new Error(`gen-bundled-plugins: ${path} is missing ${what}`);
  }
  return value;
}

const packPath = join(repo, 'packs/core.toml');
const pack = readToml(packPath);
const entries = required(pack.plugin, '[[plugin]] entries', packPath);

const plugins = entries.map((entry) => {
  const id = required(entry.id, 'plugin.id', packPath);
  const source = required(entry.source, `source for ${id}`, packPath);
  if (!source.startsWith('local:')) {
    throw new Error(`gen-bundled-plugins: ${packPath} plugin ${id} has source "${source}"; only local: sources are bundled`);
  }
  const dir = source.slice('local:'.length);
  const manifestPath = join(repo, 'apps', dir, 'manifest.toml');
  const manifest = readToml(manifestPath);
  const plugin = required(manifest.plugin, '[plugin]', manifestPath);
  return {
    id,
    name: required(plugin.name, 'plugin.name', manifestPath),
    description: required(plugin.description, 'plugin.description', manifestPath),
    capabilities: plugin.capabilities?.capabilities ?? [],
    source: `plugins/${dir}`,
  };
});

const next = `${JSON.stringify(plugins, null, 2)}\n`;

if (process.argv.includes('--check')) {
  let current = '';
  try {
    current = readFileSync(outPath, 'utf8');
  } catch (error) {
    console.error(`gen-bundled-plugins: cannot read ${outPath}: ${error.message}`);
    process.exit(1);
  }
  if (current !== next) {
    console.error(`gen-bundled-plugins: ${outPath} is stale. Run: npm run gen:plugins`);
    process.exit(1);
  }
  console.log(`gen-bundled-plugins: ${plugins.length} plugins, up to date`);
} else {
  writeFileSync(outPath, next);
  console.log(`gen-bundled-plugins: wrote ${plugins.length} plugins to ${outPath}`);
}
