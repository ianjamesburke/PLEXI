import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { lstatSync, readdirSync, readFileSync, readlinkSync, existsSync, mkdtempSync, mkdirSync } from 'node:fs';
import { userInfo, tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

function snapshot(root) {
  const rows = [];
  function walk(path) {
    for (const name of readdirSync(path).sort()) {
      const file = join(path, name), stat = lstatSync(file);
      if (stat.isDirectory()) walk(file);
      else rows.push([file, stat.size, stat.mtimeMs, stat.isSymbolicLink() ? readlinkSync(file) : createHash('sha256').update(readFileSync(file)).digest('hex')]);
    }
  }
  if (existsSync(root)) walk(root);
  const json = JSON.stringify(rows);
  return { files: rows.length, sha256: createHash('sha256').update(json).digest('hex') };
}
const realPi = join(userInfo().homedir, '.pi');
const before = snapshot(realPi);
const scratch = mkdtempSync(join(tmpdir(), 'pi-plexi-tests-'));
mkdirSync(join(scratch, 'agent'));
const result = spawnSync(process.execPath, ['--test', './tests/pi-plexi.test.mjs', './tests/model-modes.test.mjs'], {
  cwd: new URL('..', import.meta.url),
  env: { ...process.env, HOME: scratch, PI_CODING_AGENT_DIR: join(scratch, 'agent') },
  stdio: 'inherit',
});
const after = snapshot(realPi);
console.log('Personal Pi snapshot:', JSON.stringify({ before, after }));
assert.deepEqual(after, before, 'Personal ~/.pi changed during test suite');
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
