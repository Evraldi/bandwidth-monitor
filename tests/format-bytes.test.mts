import assert from 'node:assert/strict';
import test from 'node:test';

import { formatBytes, formatBytesPerSecond, toCSV } from '../src/lib/tauri.ts';

test('formatBytes memakai basis 1024 dan label biner', () => {
  assert.equal(formatBytes(0), '0 B');
  assert.equal(formatBytes(1024), '1.00 KiB');
  assert.equal(formatBytes(1536), '1.50 KiB');
  assert.equal(formatBytes(1024 * 1024), '1.00 MiB');
  assert.equal(formatBytes(1024 * 1024 * 1024), '1.00 GiB');
  assert.equal(formatBytesPerSecond(2048), '2.00 KiB/s');
  assert.ok(!formatBytes(1024).includes('KB'), 'basis 1024 tidak boleh dilabel KB');
});

test('toCSV meng-escape koma, kutip, dan newline', () => {
  assert.equal(
    toCSV([{ name: 'a,b', note: 'baris\nbaru', quote: 'a"b' }]),
    'name,note,quote\n"a,b","baris\nbaru","a""b"'
  );
  assert.equal(toCSV([]), '');
});
