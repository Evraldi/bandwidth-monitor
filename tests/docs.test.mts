import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const planning = readFileSync(join(root, 'bandwidth-monitor-planning.md'), 'utf8');

test('planning tidak lagi menyarankan ferrisetw', () => {
  assert.doesNotMatch(planning, /ferrisetw\s*=\s*"/);
  assert.doesNotMatch(planning, /Integrasi ETW via `ferrisetw`/);
  assert.match(planning, /Win32 ETW API/);
});

test('planning mencantumkan manifest build.rs, bukan UAC tauri.conf', () => {
  assert.match(planning, /build\.rs/);
  assert.match(planning, /requireAdministrator/);
  assert.match(planning, /asInvoker/);
  assert.doesNotMatch(planning, /UAC config di `tauri\.conf\.json`/);
});

test('perintah build memakai npm run tauri', () => {
  assert.match(planning, /npm run tauri dev/);
  assert.match(planning, /npm run tauri build/);
  assert.doesNotMatch(planning, /cargo tauri dev/);
});
