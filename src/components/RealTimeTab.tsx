'use client';

import { useState, useEffect, useRef } from 'react';
import { ProcessUsage, getRealtimeUsage, getSettings, formatBytes, formatBytesPerSecond } from '@/lib/tauri';

type SortKey = 'exe_name' | 'upload_bps' | 'download_bps' | 'total_sent' | 'total_recv';
type SortOrder = 'asc' | 'desc';

export default function RealTimeTab() {
  const [processes, setProcesses] = useState<ProcessUsage[]>([]);
  const previousDataRef = useRef<Map<number, ProcessUsage>>(new Map());
  const [sortKey, setSortKey] = useState<SortKey>('exe_name');
  const [sortOrder, setSortOrder] = useState<SortOrder>('asc');
  const [filter, setFilter] = useState('');
  const [isLoading, setIsLoading] = useState(true);
  const [pollIntervalMs, setPollIntervalMs] = useState(2000);

  useEffect(() => {
    getSettings()
      .then((s) => setPollIntervalMs(Math.max(1, s.poll_interval_secs) * 1000))
      .catch(() => {});
  }, []);

  useEffect(() => {
    let lastFetchTime = 0;

    const fetchData = async () => {
      try {
        const data = await getRealtimeUsage();
        const now = Date.now();
        const elapsedSec = lastFetchTime === 0 ? 0 : (now - lastFetchTime) / 1000;
        lastFetchTime = now;

        const prevData = previousDataRef.current;

        // Calculate bandwidth based on delta from previous poll
        const updatedData = data.map(proc => {
          const prev = prevData.get(proc.pid);
          let upload_bps = 0;
          let download_bps = 0;

          if (prev && elapsedSec > 0) {
            // Calculate bytes per second based on real elapsed time
            upload_bps = Math.max(0, (proc.total_sent - prev.total_sent) / elapsedSec);
            download_bps = Math.max(0, (proc.total_recv - prev.total_recv) / elapsedSec);
          }

          return {
            ...proc,
            upload_bps,
            download_bps,
          };
        });

        setProcesses(updatedData);
        previousDataRef.current = new Map(updatedData.map(p => [p.pid, p]));
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
  }, [pollIntervalMs]);

  const handleSort = (key: SortKey) => {
    if (sortKey === key) {
      setSortOrder(sortOrder === 'asc' ? 'desc' : 'asc');
    } else {
      setSortKey(key);
      setSortOrder('desc');
    }
  };

  const getSpeedColor = (bps: number): string => {
    if (bps > 10 * 1024 * 1024) return 'text-red-600 font-semibold'; // >10MB/s
    if (bps > 1 * 1024 * 1024) return 'text-yellow-600 font-medium'; // >1MB/s
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

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-64">
        <p className="text-gray-500 dark:text-gray-400">Loading network data...</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <input
          type="text"
          placeholder="Filter by process name or PID..."
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          className="px-4 py-2 border border-gray-300 rounded-lg w-96 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100"
        />
        <div className="text-sm text-gray-500 dark:text-gray-400">
          Auto-refreshing every {pollIntervalMs / 1000} seconds • {filteredAndSorted.length} processes
        </div>
      </div>

      <div className="overflow-x-auto rounded-lg border border-gray-200 dark:border-gray-700">
        <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
          <thead className="bg-gray-50 dark:bg-gray-700">
            <tr>
              <th
                onClick={() => handleSort('exe_name')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-600"
              >
                Process Name {sortKey === 'exe_name' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider dark:text-gray-300">
                PID
              </th>
              <th
                onClick={() => handleSort('upload_bps')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-600"
              >
                Upload {sortKey === 'upload_bps' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
              <th
                onClick={() => handleSort('download_bps')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-600"
              >
                Download {sortKey === 'download_bps' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
              <th
                onClick={() => handleSort('total_sent')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-600"
              >
                Total Sent {sortKey === 'total_sent' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
              <th
                onClick={() => handleSort('total_recv')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-600"
              >
                Total Received {sortKey === 'total_recv' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
            </tr>
          </thead>
          <tbody className="bg-white divide-y divide-gray-200 dark:bg-gray-800 dark:divide-gray-700">
            {filteredAndSorted.map((proc) => (
              <tr key={proc.pid} className="hover:bg-gray-50 dark:hover:bg-gray-700">
                <td className="px-6 py-4 whitespace-nowrap text-sm font-medium text-gray-900 dark:text-gray-100">
                  {proc.exe_name}
                </td>
                <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400">
                  {proc.pid}
                </td>
                <td className={`px-6 py-4 whitespace-nowrap text-sm ${getSpeedColor(proc.upload_bps)}`}>
                  {formatBytesPerSecond(proc.upload_bps)}
                </td>
                <td className={`px-6 py-4 whitespace-nowrap text-sm ${getSpeedColor(proc.download_bps)}`}>
                  {formatBytesPerSecond(proc.download_bps)}
                </td>
                <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400">
                  {formatBytes(proc.total_sent)}
                </td>
                <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400">
                  {formatBytes(proc.total_recv)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {filteredAndSorted.length === 0 && (
        <div className="text-center py-8 text-gray-500 dark:text-gray-400">
          No active network connections found
        </div>
      )}
    </div>
  );
}
