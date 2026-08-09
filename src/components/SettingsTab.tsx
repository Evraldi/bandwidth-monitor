'use client';

import { useState, useEffect } from 'react';
import {
  Settings,
  DEFAULT_SETTINGS,
  getSettings,
  updateSettings,
} from '@/lib/tauri';
import { applyDarkMode } from '@/lib/theme';

type SaveState = 'idle' | 'saving' | 'saved' | 'error';

export default function SettingsTab() {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [isLoading, setIsLoading] = useState(true);
  const [saveState, setSaveState] = useState<SaveState>('idle');

  useEffect(() => {
    getSettings()
      .then((s) => {
        setSettings(s);
        applyDarkMode(s.dark_mode);
      })
      .catch((error) => {
        console.error('Failed to load settings:', error);
      })
      .finally(() => setIsLoading(false));
  }, []);

  const update = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setSettings((prev) => ({ ...prev, [key]: value }));
    if (key === 'dark_mode') {
      applyDarkMode(value as boolean);
    }
  };

  const handleSave = async () => {
    setSaveState('saving');
    try {
      await updateSettings(settings);
      applyDarkMode(settings.dark_mode);
      setSaveState('saved');
      setTimeout(() => setSaveState('idle'), 2500);
    } catch (error) {
      console.error('Failed to save settings:', error);
      setSaveState('error');
      setTimeout(() => setSaveState('idle'), 2500);
    }
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-64">
        <p className="text-gray-500 dark:text-gray-400">Loading settings...</p>
      </div>
    );
  }

  const numberField = (
    key: keyof Settings,
    label: string,
    hint: string,
    min: number,
    max: number,
    unit: string
  ) => (
    <div className="flex items-start justify-between gap-8 py-4">
      <div>
        <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
          {label}
        </div>
        <div className="text-xs text-gray-500 dark:text-gray-400 mt-1">
          {hint}
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <input
          type="number"
          min={min}
          max={max}
          value={settings[key] as number}
          onChange={(e) => update(key, Number(e.target.value) as never)}
          className="w-24 px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100"
        />
        <span className="text-sm text-gray-500 dark:text-gray-400">{unit}</span>
      </div>
    </div>
  );

  const toggleField = (
    key: keyof Settings,
    label: string,
    hint: string
  ) => (
    <div className="flex items-start justify-between gap-8 py-4">
      <div>
        <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
          {label}
        </div>
        <div className="text-xs text-gray-500 dark:text-gray-400 mt-1">
          {hint}
        </div>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={settings[key] as boolean}
        onClick={() => update(key, !(settings[key] as boolean) as never)}
        className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 ${
          settings[key] ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
        }`}
      >
        <span
          className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
            settings[key] ? 'translate-x-6' : 'translate-x-1'
          }`}
        />
      </button>
    </div>
  );

  return (
    <div className="space-y-6">
      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
        <div className="px-6 py-4 bg-gray-50 dark:bg-gray-700 border-b border-gray-200 dark:border-gray-600">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
            Monitoring
          </h3>
        </div>
        <div className="px-6 divide-y divide-gray-200 dark:divide-gray-700">
          {numberField(
            'poll_interval_secs',
            'UI refresh interval',
            'How often the Real-time tab refreshes its data.',
            1,
            60,
            'seconds'
          )}
          {numberField(
            'capture_interval_secs',
            'Capture interval',
            'How often the backend samples network usage per connection.',
            1,
            60,
            'seconds'
          )}
          {numberField(
            'retention_days',
            'Data retention',
            'Snapshots older than this are cleaned up automatically.',
            1,
            365,
            'days'
          )}
        </div>
      </div>

      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
        <div className="px-6 py-4 bg-gray-50 dark:bg-gray-700 border-b border-gray-200 dark:border-gray-600">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
            Alerts
          </h3>
        </div>
        <div className="px-6 divide-y divide-gray-200 dark:divide-gray-700">
          {numberField(
            'spike_threshold_mb',
            'Bandwidth spike threshold',
            'Notify when a single process transfers at least this much in one capture interval.',
            1,
            10240,
            'MB'
          )}
          {toggleField(
            'notifications_enabled',
            'Spike notifications',
            'Show a desktop notification when a process hits the spike threshold.'
          )}
        </div>
      </div>

      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
        <div className="px-6 py-4 bg-gray-50 dark:bg-gray-700 border-b border-gray-200 dark:border-gray-600">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
            Appearance
          </h3>
        </div>
        <div className="px-6 divide-y divide-gray-200 dark:divide-gray-700">
          {toggleField(
            'dark_mode',
            'Dark mode',
            'Use a dark color scheme for the app.'
          )}
        </div>
      </div>

      <div className="flex items-center gap-4">
        <button
          onClick={handleSave}
          disabled={saveState === 'saving'}
          className="px-6 py-2 bg-blue-600 text-white rounded-lg font-medium text-sm hover:bg-blue-700 transition-colors disabled:opacity-50"
        >
          {saveState === 'saving' ? 'Saving...' : 'Save Settings'}
        </button>

        {saveState === 'saved' && (
          <span className="text-sm text-green-600">Settings saved</span>
        )}
        {saveState === 'error' && (
          <span className="text-sm text-red-600">Failed to save settings</span>
        )}
      </div>

      <p className="text-xs text-gray-500 dark:text-gray-400">
        Closing the window hides the app to the system tray — use the tray
        icon to restore it. Quit from the tray menu to fully exit.
      </p>
    </div>
  );
}
