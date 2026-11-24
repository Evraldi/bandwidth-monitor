import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')) as {
  scripts: Record<string, string>;
};
const workflow = readFileSync(join(root, '.github/workflows/ci.yml'), 'utf8');

test('script test mengutip glob supaya Node yang men-expand di Windows', () => {
  // cmd/PowerShell tidak meng-expand `*.test.mts`. Tanpa tanda kutip, Node
  // menerima literal `tests\*.test.mts` dan gagal di runner Windows.
  assert.match(pkg.scripts.test, /node --test "tests\/\*\.test\.mts"/);
  assert.doesNotMatch(pkg.scripts.test, /(?<!")tests\/\*\.test\.mts(?!")/);
});

test('CI memakai Node yang mendukung type stripping .mts', () => {
  const version = workflow.match(/node-version:\s*['"]?(\d+)/);
  assert.ok(version, 'node-version harus didefinisikan');
  assert.ok(
    Number(version[1]) >= 22,
    '.mts butuh Node >= 22.18 (type stripping), lebih aman 24'
  );
});
