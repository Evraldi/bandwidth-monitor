import { DEFAULT_SETTINGS, type Settings } from './tauri.ts';

export type NumericSetting =
  | 'poll_interval_secs'
  | 'capture_interval_secs'
  | 'retention_days'
  | 'spike_threshold_mib';

/** Sumber kebenaran untuk batas input. Backend memakai rentang yang sama. */
export const SETTING_BOUNDS: Record<NumericSetting, { min: number; max: number }> = {
  poll_interval_secs: { min: 1, max: 60 },
  capture_interval_secs: { min: 1, max: 60 },
  retention_days: { min: 1, max: 365 },
  spike_threshold_mib: { min: 1, max: 10240 },
};

/**
 * Nilai dari input type=number bisa berupa '' (kosong), NaN, Infinity, atau
 * di luar rentang walaupun atribut min/max HTML sudah ada. Semua dibersihkan
 * di sini, bukan hanya di markup.
 */
export function clampNumericSetting(
  key: NumericSetting,
  raw: unknown,
  fallback: number
): number {
  if (typeof raw === 'string' && raw.trim() === '') return fallback;
  const value = typeof raw === 'number' ? raw : Number(raw);
  if (!Number.isFinite(value)) return fallback;
  const { min, max } = SETTING_BOUNDS[key];
  return Math.min(max, Math.max(min, Math.round(value)));
}

/** Dipakai saat menerima settings dari backend (termasuk DB versi lama). */
export function sanitizeSettings(input: Partial<Settings>): Settings {
  const legacy = input as Partial<Settings> & { spike_threshold_mb?: number };
  const result: Settings = { ...DEFAULT_SETTINGS };

  for (const key of Object.keys(SETTING_BOUNDS) as NumericSetting[]) {
    const legacyValue = key === 'spike_threshold_mib' ? legacy.spike_threshold_mb : undefined;
    const source = input[key] ?? legacyValue ?? DEFAULT_SETTINGS[key];
    result[key] = clampNumericSetting(key, source, DEFAULT_SETTINGS[key]);
  }

  if (typeof input.dark_mode === 'boolean') result.dark_mode = input.dark_mode;
  if (typeof input.notifications_enabled === 'boolean') {
    result.notifications_enabled = input.notifications_enabled;
  }
  return result;
}
