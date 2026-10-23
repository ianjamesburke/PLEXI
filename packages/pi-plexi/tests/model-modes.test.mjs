import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, symlinkSync } from 'node:fs';
import { tmpdir, userInfo } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { createPiPlexiExtension } from '../extensions/index.ts';
import { DEFAULT_MODEL_MODES, registerModelModes } from '../src/model-modes.ts';
import { migrateSettings, backupProfile, migrateProfile, rollbackProfile, safePath } from '../src/migrate-model-modes.mjs';

const settings = { defaultProvider: 'ian-router', defaultModel: 'auto', compaction: { enabled: true }, packages: ['npm:@alexlikevibe/pi-jev', '/local/pi-plexi'], custom: { retain: 1 } };
function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'pi-modes-')), profile = join(root, 'agent'), backup = join(root, 'backup');
  mkdirSync(join(profile, 'extensions'), { recursive: true }); mkdirSync(join(profile, 'agents'));
  writeFileSync(join(profile, 'settings.json'), JSON.stringify(settings));
  writeFileSync(join(profile, 'extensions/subscription-router.ts'), '// original router\n');
  writeFileSync(join(profile, 'agents/worker.md'), '# Worker\n');
  writeFileSync(join(profile, 'jev.json'), '{"untouched":true}');
  return { root, profile, backup };
}
test('detached registration is lazy; exact targets and supported thinking are enforced', () => {
  const models = [];
  createPiPlexiExtension({ env: {}, homeDir: process.env.HOME })({ registerVirtualModel: model => models.push(model), registerTool() {}, registerCommand() {}, on() {} });
  assert.deepEqual(models.map(m => [m.provider, m.id, m.name]), [['plexi', 'fast', 'FAST'], ['plexi', 'slow', 'SLOW']]);
  for (const [i, alias] of models.entries()) {
    const mode = DEFAULT_MODEL_MODES[i];
    const target = { provider: mode.provider, id: mode.model, reasoning: true };
    const ctx = { modelRegistry: { find(provider, id) { assert.equal(provider, mode.provider); assert.equal(id, mode.model); return target; } } };
    assert.deepEqual(alias.route({}, ctx), { model: target, thinkingLevel: mode.thinking });
    assert.throws(() => alias.route({}, { modelRegistry: { find() {} } }), error => error.message.includes(`${mode.provider}/${mode.model}`) && error.message.includes(mode.thinking));
    target.reasoning = false;
    assert.throws(() => alias.route({}, ctx), /does not support thinking/);
    target.reasoning = true; target.thinkingLevelMap = { [mode.thinking]: null };
    assert.throws(() => alias.route({}, ctx), /does not support thinking/);
  }
});

test('thinking off routes a non-reasoning target', () => {
  const models = [];
  const modes = registerModelModes({ registerVirtualModel: model => models.push(model), registerCommand() {} });
  modes.setModes([
    { id: 'fast', name: 'FAST', provider: 'test', model: 'plain', thinking: 'off' },
    { ...DEFAULT_MODEL_MODES[1] },
  ]);
  const target = { provider: 'test', id: 'plain', reasoning: false };
  assert.deepEqual(models[0].route({}, { modelRegistry: { find: () => target } }), { model: target, thinkingLevel: 'off' });
});

function commandHarness() {
  const commands = new Map();
  const pi = {
    registerVirtualModel() {}, registerTool() {}, on() {},
    registerCommand(name, command) { commands.set(name, command); },
    async setModel(model) { this.models.push(model); return this.setModelResult; },
    setThinkingLevel(level) { this.thinking.push(level); },
    models: [], thinking: [], setModelResult: true,
  };
  createPiPlexiExtension({ env: {}, homeDir: process.env.HOME })(pi);
  const notifications = [];
  const models = new Map(DEFAULT_MODEL_MODES.map((mode) => [mode.id, { provider: 'plexi', id: mode.id }]));
  const ctx = {
    modelRegistry: { find(provider, id) { return provider === 'plexi' ? models.get(id) : undefined; } },
    ui: {
      notify(message, type) { notifications.push([message, type]); },
      async select(title, options) { this.lastSelect = { title, options }; return this.selection; },
    },
  };
  return { commands, pi, ctx, notifications, models };
}

test('FAST/SLOW commands register and switch virtual models', async () => {
  const { commands, pi, ctx, notifications } = commandHarness();
  assert.deepEqual([...commands.keys()].filter(name => ['fast', 'slow', 'models', 'model'].includes(name)), ['fast', 'slow', 'models']);
  await commands.get('fast').handler('', ctx);
  await commands.get('slow').handler('', ctx);
  assert.deepEqual(pi.models, [{ provider: 'plexi', id: 'fast' }, { provider: 'plexi', id: 'slow' }]);
  assert.deepEqual(pi.thinking, ['low', 'high']);
  assert.deepEqual(notifications, [['Model: FAST (openrouter/deepseek/deepseek-v4.1-flash, thinking low)', undefined], ['Model: SLOW (openrouter/openai/gpt-6-astra, thinking high)', undefined]]);
});

test('/models handles arguments, picker state, cancel, and completions', async () => {
  const { commands, pi, ctx, notifications } = commandHarness();
  const models = commands.get('models');
  assert.deepEqual(models.getArgumentCompletions(''), [{ value: 'fast', label: 'fast' }, { value: 'slow', label: 'slow' }]);
  assert.deepEqual(models.getArgumentCompletions('S'), [{ value: 'slow', label: 'slow' }]);
  await models.handler('FAST', ctx);
  await models.handler('other', ctx);
  ctx.model = { provider: 'plexi', id: 'fast' };
  ctx.ui.selection = 'SLOW — openrouter/openai/gpt-6-astra (thinking high)';
  await models.handler('', ctx);
  assert.deepEqual(ctx.ui.lastSelect, {
    title: 'Model',
    options: [
      'FAST — openrouter/deepseek/deepseek-v4.1-flash (thinking low) (current)',
      'SLOW — openrouter/openai/gpt-6-astra (thinking high)',
    ],
  });
  const countBeforeCancel = pi.models.length;
  ctx.ui.selection = undefined;
  await models.handler('', ctx);
  assert.deepEqual(pi.models, [{ provider: 'plexi', id: 'fast' }, { provider: 'plexi', id: 'slow' }]);
  assert.equal(pi.models.length, countBeforeCancel);
  assert.deepEqual(notifications, [
    ['Model: FAST (openrouter/deepseek/deepseek-v4.1-flash, thinking low)', undefined],
    ['Unknown model. Choose: fast, slow.', 'error'],
    ['Model: SLOW (openrouter/openai/gpt-6-astra, thinking high)', undefined],
  ]);
});

test('model selection reports missing or rejected virtual models', async () => {
  const { commands, pi, ctx, notifications, models } = commandHarness();
  pi.setModelResult = false;
  await commands.get('fast').handler('', ctx);
  models.delete('slow');
  await commands.get('slow').handler('', ctx);
  assert.deepEqual(pi.thinking, []);
  assert.deepEqual(notifications, [
    ['Unable to switch to plexi/fast.', 'error'],
    ['Unable to switch to plexi/slow: model not found.', 'error'],
  ]);
});
test('pure settings migration preserves unrelated fields and package filters', () => {
  const output = migrateSettings(settings);
  assert.equal(settings.defaultProvider, 'ian-router');
  assert.equal(output.defaultProvider, 'plexi'); assert.equal(output.defaultModel, 'fast'); assert.equal(output.defaultThinkingLevel, 'low');
  assert.deepEqual(output.enabledModels, ['plexi/fast', 'plexi/slow']);
  assert.deepEqual(output.compaction, settings.compaction); assert.deepEqual(output.custom, settings.custom);
  assert.deepEqual(output.packages[0], { source: settings.packages[0], extensions: ['-extensions/routing.ts'] });
  assert.deepEqual(migrateSettings(output), output);
  const filtered = { ...settings, packages: [{ source: 'npm:@alexlikevibe/pi-jev@0.2.1', extensions: ['+extensions/routing.ts', 'extensions/jev.ts', 'extensions/compaction.ts'], skills: [] }] };
  assert.deepEqual(migrateSettings(filtered).packages[0].extensions, [...filtered.packages[0].extensions, '-extensions/routing.ts']);
  assert.deepEqual(migrateSettings({ ...settings, packages: [{ source: settings.packages[0], extensions: [] }] }).packages[0].extensions, []);
});
test('verified backup, atomic migration, idempotence, byte-exact rollback and reapply', () => {
  const { profile, backup } = fixture();
  const original = readFileSync(join(profile, 'settings.json'));
  backupProfile(profile, backup);
  assert.throws(() => backupProfile(profile, backup), /EEXIST/);
  assert.equal(migrateProfile(profile, backup), true);
  assert.equal(existsSync(join(profile, 'extensions/subscription-router.ts')), false);
  assert.equal(migrateProfile(profile, backup), false);
  assert.equal(readFileSync(join(profile, 'jev.json'), 'utf8'), '{"untouched":true}');
  rollbackProfile(profile, backup);
  for (const name of ['settings.json', 'extensions/subscription-router.ts', 'agents/worker.md']) assert.deepEqual(readFileSync(join(profile, name)), readFileSync(join(backup, name)));
  assert.deepEqual(readFileSync(join(profile, 'settings.json')), original);
  assert.equal(migrateProfile(profile, backup), true);
});
test('refuses real and HOME .pi paths and symlink escapes before writing', () => {
  assert.throws(() => safePath(join(userInfo().homedir, '.pi/agent')), /Refusing/);
  assert.throws(() => safePath(join(process.env.HOME, '.pi/agent')), /Refusing/);
  const { root, profile, backup } = fixture();
  symlinkSync(join(process.env.HOME, '.pi'), join(root, 'missing-home-pi'));
  assert.throws(() => safePath(join(root, 'missing-home-pi/agent')), /Refusing/);
  symlinkSync(join(userInfo().homedir, '.pi'), join(root, 'personal'));
  assert.throws(() => safePath(join(root, 'personal/agent')), /Refusing/);
  symlinkSync(join(userInfo().homedir, '.pi'), join(profile, 'escape'));
  assert.throws(() => backupProfile(profile, join(profile, 'escape/backup')), /Refusing/);
  const result = spawnSync(process.execPath, ['src/migrate-model-modes.mjs', 'migrate'], { cwd: new URL('..', import.meta.url), env: process.env, encoding: 'utf8' });
  assert.equal(result.status, 1); assert.match(result.stderr, /Usage/);
  const refused = spawnSync(process.execPath, ['src/migrate-model-modes.mjs', 'migrate', '--profile', join(userInfo().homedir, '.pi/agent'), '--backup', backup], { cwd: new URL('..', import.meta.url), env: process.env, encoding: 'utf8' });
  assert.equal(refused.status, 1); assert.match(refused.stderr, /Refusing/);
});
test('tampered backup and changed source abort without modifying settings', () => {
  const { profile, backup } = fixture(); backupProfile(profile, backup);
  const original = readFileSync(join(profile, 'settings.json'));
  writeFileSync(join(backup, 'agents/worker.md'), 'tampered');
  assert.throws(() => migrateProfile(profile, backup), /verification failed/);
  assert.deepEqual(readFileSync(join(profile, 'settings.json')), original);
  assert.throws(() => rollbackProfile(profile, backup), /verification failed/);
});

test('CLI backs up automatically, rolls back, and refuses stale backup reuse', () => {
  const { profile, backup } = fixture();
  const original = readFileSync(join(profile, 'settings.json'));
  function run(command) {
    return spawnSync(process.execPath, ['src/migrate-model-modes.mjs', command, '--profile', profile, '--backup', backup], { cwd: new URL('..', import.meta.url), env: process.env, encoding: 'utf8' });
  }
  const migrated = run('migrate'); assert.equal(migrated.status, 0, migrated.stderr);
  assert.deepEqual(readFileSync(join(backup, 'settings.json')), original);
  const rolledBack = run('rollback'); assert.equal(rolledBack.status, 0, rolledBack.stderr);
  assert.deepEqual(readFileSync(join(profile, 'settings.json')), original);
  writeFileSync(join(profile, 'agents/worker.md'), '# Updated worker\n');
  const refused = run('migrate'); assert.equal(refused.status, 1); assert.match(refused.stderr, /Profile changed since backup/);
  assert.deepEqual(readFileSync(join(profile, 'settings.json')), original);
});
