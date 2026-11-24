import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const conf = JSON.parse(readFileSync(join(root, 'src-tauri', 'tauri.conf.json'), 'utf8'));
const caps = JSON.parse(
  readFileSync(join(root, 'src-tauri', 'capabilities', 'default.json'), 'utf8')
);

test('CSP aktif dan tidak membuka source berbahaya', () => {
  const csp = conf.app?.security?.csp;
  assert.ok(csp && typeof csp === 'object', 'CSP tidak boleh null');
  assert.match(csp['default-src'], /'self'/);
  assert.match(csp['connect-src'], /ipc:/);
  assert.match(csp['script-src'], /'self'/);

  for (const directive of Object.values(csp) as string[]) {
    assert.ok(!directive.includes("'unsafe-eval'"), `unsafe-eval di ${directive}`);
    assert.ok(!directive.includes('*'), `wildcard di ${directive}`);
  }
});

test('ACL hanya memberi permission yang benar-benar dipakai UI', () => {
  assert.ok(conf.app.security.capabilities.includes('default'));
  assert.deepEqual(new Set(caps.permissions), new Set([
    'dialog:allow-save',
    'autostart:allow-enable',
    'autostart:allow-disable',
    'autostart:allow-is-enabled',
  ]));
});
