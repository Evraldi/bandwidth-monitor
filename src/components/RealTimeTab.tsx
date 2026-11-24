'use client';

import { useState, useEffect } from 'react';
import {
  CaptureHealth,
  ProcessUsage,
  getCaptureHealth,
  getRealtimeUsage,
  getSettings,
  formatBytes,
  formatBytesPerSecond,
} from '@/lib/tauri';
import { LineChart, Line, CartesianGrid, XAxis, YAxis, Tooltip, Legend, ResponsiveContainer } from 'recharts';

type SortKey = 'exe_name' | 'upload_bps' | 'download_bps' | 'total_sent' | 'total_recv';
type SortOrder = 'asc' | 'desc';

interface LivePoint {
  t: number;
  up: number;
  down: number;
}

const CHART_WINDOW_MS = 60_000;

export default function RealTimeTab({ active = true }: { active?: boolean }) {
  const [processes, setProcesses] = useState<ProcessUsage[]>([]);
  const [sortKey, setSortKey] = useState<SortKey>('exe_name');
  const [sortOrder, setSortOrder] = useState<SortOrder>('asc');
  const [filter, setFilter] = useState('');
  const [isLoading, setIsLoading] = useState(true);
  const [pollIntervalMs, setPollIntervalMs] = useState(2000);
  const [history, setHistory] = useState<LivePoint[]>([]);
  const [captureHealth, setCaptureHealth] = useState<CaptureHealth | null>(null);

  useEffect(() => {
    getSettings()
      .then((s) => setPollIntervalMs(Math.max(1, s.poll_interval_secs) * 1000))
      .catch(() => {});
  }, []);

  useEffect(() => {
    // Pause polling while the tab is hidden (kept-alive). Backend tetap
    // mencapture dan menyimpan baseline, jadi tidak ada baseline UI yang
    // perlu di-reset saat tab aktif lagi.
    if (!active) return;

    const fetchData = async () => {
      try {
        const [data, health] = await Promise.all([
          getRealtimeUsage(),
          getCaptureHealth().catch(() => null),
        ]);
        setCaptureHealth(health);
        const now = Date.now();

        // Rate byte/detik sudah dihitung backend dari delta capture. UI hanya
        // menampilkan; tidak lagi menebak-nebak dari selisih total antar poll.
        setProcesses(data);

        // Append aggregate totals to the rolling bandwidth chart window
        const totalUp = data.reduce((acc, p) => acc + p.upload_bps, 0);
        const totalDown = data.reduce((acc, p) => acc + p.download_bps, 0);
        setHistory((prev) => {
          const cutoff = now - CHART_WINDOW_MS;
          const pruned = prev.filter((p) => p.t >= cutoff);
          pruned.push({ t: now, up: totalUp, down: totalDown });
          return pruned;
        });

        setIsLoading(false);
      } catch (error) {
        console.error('Failed to fetch realtime usage:', error);
        setIsLoading(false);
      }
    };

    fetchData();
    const intervalId = setInterval(fetchData, pollIntervalMs);

    return () => {
      clearInterval(intervalId);
    };
  }, [pollIntervalMs, active]);

  const handleSort = (key: SortKey) => {
    if (sortKey === key) {
      setSortOrder(sortOrder === 'asc' ? 'desc' : 'asc');
    } else {
      setSortKey(key);
      setSortOrder('desc');
    }
  };

  const getSpeedColor = (bps: number): string => {
    if (bps > 10 * 1024 * 1024) return 'text-red-600 font-semibold'; // >10 MiB/s
    if (bps > 1 * 1024 * 1024) return 'text-yellow-600 font-medium'; // >1 MiB/s
    return 'text-green-600';
  };

  const filteredAndSorted = processes
    .filter(proc => 
      proc.exe_name.toLowerCase().includes(filter.toLowerCase()) ||
      proc.pid.toString().includes(filter)
    )
    .sort((a, b) => {
      const multiplier = sortOrder === 'asc' ? 1 : -1;
      if (sortKey === 'exe_name') {
        return multiplier * a.exe_name.localeCompare(b.exe_name);
      }
      return multiplier * ((a[sortKey] as number) - (b[sortKey] as number));
    });

  const captureStatus = !captureHealth
    ? 'Capture: status tidak tersedia'
    : !captureHealth.etwActive
      ? 'Capture: fallback TCP (ETW tidak aktif)'
      : !captureHealth.sessionRunning
        ? 'Capture: ETW berhenti, data mungkin tidak lengkap'
        : captureHealth.eventsLost + captureHealth.buffersLost > 0
          ? `Capture: ETW aktif • ${captureHealth.eventsLost + captureHealth.buffersLost} event/buffer hilang`
          : 'Capture: ETW aktif';

  const captureStatusClass = !captureHealth
    ? 'text-gray-500 dark:text-gray-400'
    : !captureHealth.etwActive ||
        captureHealth.eventsLost + captureHealth.buffersLost > 0
      ? 'text-amber-600 dark:text-amber-400'
      : captureHealth.sessionRunning
        ? 'text-green-600 dark:text-green-400'
        : 'text-red-600 dark:text-red-400';

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-64">
        <p className="text-gray-500 dark:text-gray-400">Loading network data...</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-4 flex-1 min-w-0">
          <label className="text-sm font-medium text-gray-700 dark:text-gray-300 whitespace-nowrap">
            Filter:
          </label>
          <input
            type="text"
            placeholder="By process name or PID..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="flex-1 min-w-0 px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100"
          />
        </div>
        <div className="text-sm text-gray-600 dark:text-gray-400 shrink-0">
          <div className="whitespace-nowrap">
            Auto-refreshing every {pollIntervalMs / 1000} seconds • {filteredAndSorted.length} processes
          </div>
          <div className={`whitespace-nowrap ${captureStatusClass}`}>{captureStatus}</div>
        </div>
      </div>

      <div className="bg-white dark:bg-gray-800 p-6 rounded-lg border border-gray-200 dark:border-gray-700">
        <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">
          Live Bandwidth
        </h3>
        <ResponsiveContainer width="100%" height={180}>
          <LineChart data={history} margin={{ top: 5, right: 20, left: 0, bottom: 5 }}>
            <CartesianGrid strokeDasharray="3 3" />
            <XAxis
              dataKey="t"
              type="number"
              domain={['dataMin', 'dataMax']}
              tickFormatter={(v: number) => new Date(v).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}
              stroke="#9ca3af"
            />
            <YAxis tickFormatter={(v: number) => formatBytes(v)} width={70} stroke="#9ca3af" />
            <Tooltip
              labelFormatter={(label) => typeof label === 'number' ? new Date(label).toLocaleTimeString() : String(label)}
              formatter={(value) => typeof value === 'number' ? formatBytesPerSecond(value) : String(value)}
            />
            <Legend />
            <Line type="monotone" dataKey="up" name="Upload" stroke="#3b82f6" strokeWidth={2} dot={false} isAnimationActive={false} />
            <Line type="monotone" dataKey="down" name="Download" stroke="#10b981" strokeWidth={2} dot={false} isAnimationActive={false} />
          </LineChart>
        </ResponsiveContainer>
      </div>

      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
        <div className="px-6 py-4 bg-gray-50 dark:bg-gray-700 border-b border-gray-200 dark:border-gray-700">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
            Live Network Activity
          </h3>
        </div>
        <div className="overflow-x-auto">
          <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
            <thead className="bg-gray-50 dark:bg-gray-700">
              <tr>
                <th
                  onClick={() => handleSort('exe_name')}
                  className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-600"
                >
                  Process Name {sortKey === 'exe_name' && (sortOrder === 'asc' ? '↑' : '↓')}
                </th>
                <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                  PID
                </th>
                <th
                  onClick={() => handleSort('upload_bps')}
                  className="px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-600"
                >
                  Upload {sortKey === 'upload_bps' && (sortOrder === 'asc' ? '↑' : '↓')}
                </th>
                <th
                  onClick={() => handleSort('download_bps')}
                  className="px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-600"
                >
                  Download {sortKey === 'download_bps' && (sortOrder === 'asc' ? '↑' : '↓')}
                </th>
                <th
                  onClick={() => handleSort('total_sent')}
                  className="px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-600"
                >
                  Total Sent {sortKey === 'total_sent' && (sortOrder === 'asc' ? '↑' : '↓')}
                </th>
                <th
                  onClick={() => handleSort('total_recv')}
                  className="px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-600"
                >
                  Total Received {sortKey === 'total_recv' && (sortOrder === 'asc' ? '↑' : '↓')}
                </th>
              </tr>
            </thead>
            <tbody className="bg-white dark:bg-gray-800 divide-y divide-gray-200 dark:divide-gray-700">
              {filteredAndSorted.map((proc) => (
                <tr key={proc.pid} className="hover:bg-gray-50 dark:hover:bg-gray-700">
                  <td className="px-6 py-4 whitespace-nowrap text-sm font-medium text-gray-900 dark:text-gray-100">
                    {proc.exe_name}
                  </td>
                  <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400">
                    {proc.pid}
                  </td>
                  <td className={`px-6 py-4 whitespace-nowrap text-sm text-right ${getSpeedColor(proc.upload_bps)}`}>
                    {formatBytesPerSecond(proc.upload_bps)}
                  </td>
                  <td className={`px-6 py-4 whitespace-nowrap text-sm text-right ${getSpeedColor(proc.download_bps)}`}>
                    {formatBytesPerSecond(proc.download_bps)}
                  </td>
                  <td className="px-6 py-4 whitespace-nowrap text-sm text-right text-gray-500 dark:text-gray-400">
                    {formatBytes(proc.total_sent)}
                  </td>
                  <td className="px-6 py-4 whitespace-nowrap text-sm text-right text-gray-500 dark:text-gray-400">
                    {formatBytes(proc.total_recv)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {filteredAndSorted.length === 0 && (
        <div className="flex items-center justify-center h-64 bg-gray-50 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-700">
          <p className="text-gray-500 dark:text-gray-400">No active network connections found</p>
        </div>
      )}
    </div>
  );
}
