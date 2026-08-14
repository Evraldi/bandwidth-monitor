import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';

export interface ProcessUsage {
  exe_name: string;
  pid: number;
  upload_bps: number;
  download_bps: number;
  total_sent: number;
  total_recv: number;
}

export interface DailySummary {
  exe_name: string;
  total_sent: number;
  total_recv: number;
}

export interface MonthlySummary {
  exe_name: string;
  total_sent: number;
  total_recv: number;
}

export interface DailyDataPoint {
  date: string;
  total_sent: number;
  total_recv: number;
}

export interface Settings {
  poll_interval_secs: number;
  capture_interval_secs: number;
  retention_days: number;
  dark_mode: boolean;
  notifications_enabled: boolean;
  spike_threshold_mb: number;
}

export const DEFAULT_SETTINGS: Settings = {
  poll_interval_secs: 2,
  capture_interval_secs: 5,
  retention_days: 7,
  dark_mode: false,
  notifications_enabled: true,
  spike_threshold_mb: 100,
};

export async function getRealtimeUsage(): Promise<ProcessUsage[]> {
  return invoke<ProcessUsage[]>('get_realtime_usage');
}

export async function getDailySummary(date: string): Promise<DailySummary[]> {
  return invoke<DailySummary[]>('get_daily_summary', { date });
}

export async function getMonthlySummary(
  year: number,
  month: number
): Promise<MonthlySummary[]> {
  return invoke<MonthlySummary[]>('get_monthly_summary', { year, month });
}

export async function getHistoryChart(
  exeName: string,
  days: number
): Promise<DailyDataPoint[]> {
  return invoke<DailyDataPoint[]>('get_history_chart', {
    exeName,
    days,
  });
}

export async function getDailySummaryRange(
  start: string,
  end: string
): Promise<DailySummary[]> {
  return invoke<DailySummary[]>('get_daily_summary_range', { start, end });
}

export async function getMonthlyDailyTotals(
  year: number,
  month: number
): Promise<DailyDataPoint[]> {
  return invoke<DailyDataPoint[]>('get_monthly_daily_totals', { year, month });
}

export async function aggregateToday(): Promise<void> {
  return invoke<void>('aggregate_today');
}

export async function getSettings(): Promise<Settings> {
  return invoke<Settings>('get_settings');
}

export async function updateSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>('update_settings', { settings });
}

export async function isAdmin(): Promise<boolean> {
  return invoke<boolean>('is_admin');
}

export function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1);
  return `${(bytes / Math.pow(k, i)).toFixed(2)} ${sizes[i]}`;
}

export function formatBytesPerSecond(bps: number): string {
  return `${formatBytes(bps)}/s`;
}

export function toCSV(rows: Record<string, string | number>[]): string {
  const headers = rows.length > 0 ? Object.keys(rows[0]) : [];
  const escape = (value: string | number) => {
    const s = String(value);
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  const lines = [headers.map(escape).join(',')];
  for (const row of rows) {
    lines.push(headers.map((h) => escape(row[h])).join(','));
  }
  return lines.join('\n');
}

export async function exportData(
  rows: Record<string, string | number>[],
  defaultName: string,
  kind: 'csv' | 'json'
): Promise<boolean> {
  if (rows.length === 0) return false;
  const contents = kind === 'json' ? JSON.stringify(rows, null, 2) : toCSV(rows);
  const path = await save({
    defaultPath: defaultName,
    filters: [{ name: kind.toUpperCase(), extensions: [kind] }],
  });
  if (!path) return false;
  await invoke('write_export_file', { path, contents });
  return true;
}
