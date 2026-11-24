import assert from 'node:assert/strict';
import test from 'node:test';

import {
  clampNumericSetting,
  sanitizeSettings,
  SETTING_BOUNDS,
} from '../src/lib/settings.ts';

test('input kosong dan NaN tidak merusak settings', () => {
  assert.equal(clampNumericSetting('capture_interval_secs', '', 5), 5);
  assert.equal(clampNumericSetting('capture_interval_secs', Number.NaN, 5), 5);
  assert.equal(clampNumericSetting('capture_interval_secs', Infinity, 5), 5);
  assert.equal(clampNumericSetting('retention_days', 'abc', 7), 7);
});

test('nilai dibatasi min/max backend', () => {
  assert.equal(clampNumericSetting('capture_interval_secs', 0, 5), SETTING_BOUNDS.capture_interval_secs.min);
  assert.equal(clampNumericSetting('capture_interval_secs', 999, 5), SETTING_BOUNDS.capture_interval_secs.max);
  assert.equal(clampNumericSetting('spike_threshold_mib', 12.7, 100), 13);
});

test('sanitize menerima field lama spike_threshold_mb', () => {
  const clean = sanitizeSettings({
    spike_threshold_mb: 250,
    retention_days: 0,
    dark_mode: true,
  } as never);
  assert.equal(clean.spike_threshold_mib, 250);
  assert.equal(clean.retention_days, 1);
  assert.equal(clean.dark_mode, true);
  assert.equal(clean.notifications_enabled, true);
});
