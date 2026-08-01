'use client';

import { useState, useEffect } from 'react';
import { ProcessUsage, getRealtimeUsage, formatBytes, formatBytesPerSecond } from '@/lib/tauri';

type SortKey = 'exe_name' | 'upload_bps' | 'download_bps' | 'total_sent' | 'total_recv';
type SortOrder = 'asc' | 'desc';

export default function RealTimeTab() {
  const [processes, setProcesses] = useState<ProcessUsage[]>([]);
  const [previousData, setPreviousData] = useState<Map<number, ProcessUsage>>(new Map());
  const [sortKey, setSortKey] = useState<SortKey>('exe_name');
  const [sortOrder, setSortOrder] = useState<SortOrder>('asc');
  const [filter, setFilter] = useState('');
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    let intervalId: NodeJS.Timeout;

    const fetchData = async () => {
      try {
        const data = await getRealtimeUsage();
        
        // Calculate bandwidth based on delta from previous poll
        const updatedData = data.map(proc => {
          const prev = previousData.get(proc.pid);
          let upload_bps = 0;
          let download_bps = 0;

          if (prev) {
            // Calculate bytes per second (polling every 2 seconds)
            upload_bps = Math.max(0, (proc.total_sent - prev.total_sent) / 2);
            download_bps = Math.max(0, (proc.total_recv - prev.total_recv) / 2);
          }

          return {
            ...proc,
            upload_bps,
            download_bps,
          };
        });

        setProcesses(updatedData);
        setPreviousData(new Map(updatedData.map(p => [p.pid, p])));
        setIsLoading(false);
      } catch (error) {
        console.error('Failed to fetch realtime usage:', error);
        setIsLoading(false);
      }
    };

    fetchData();
    intervalId = setInterval(fetchData, 2000);

    return () => {
      if (intervalId) clearInterval(intervalId);
    };
  }, [previousData]);

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
        <p className="text-gray-500">Loading network data...</p>
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
          className="px-4 py-2 border border-gray-300 rounded-lg w-96 focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <div className="text-sm text-gray-500">
          Auto-refreshing every 2 seconds • {filteredAndSorted.length} processes
        </div>
      </div>

      <div className="overflow-x-auto rounded-lg border border-gray-200">
        <table className="min-w-full divide-y divide-gray-200">
          <thead className="bg-gray-50">
            <tr>
              <th
                onClick={() => handleSort('exe_name')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100"
              >
                Process Name {sortKey === 'exe_name' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                PID
              </th>
              <th
                onClick={() => handleSort('upload_bps')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100"
              >
                Upload {sortKey === 'upload_bps' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
              <th
                onClick={() => handleSort('download_bps')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100"
              >
                Download {sortKey === 'download_bps' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
              <th
                onClick={() => handleSort('total_sent')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100"
              >
                Total Sent {sortKey === 'total_sent' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
              <th
                onClick={() => handleSort('total_recv')}
                className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100"
              >
                Total Received {sortKey === 'total_recv' && (sortOrder === 'asc' ? '↑' : '↓')}
              </th>
            </tr>
          </thead>
          <tbody className="bg-white divide-y divide-gray-200">
            {filteredAndSorted.map((proc) => (
              <tr key={proc.pid} className="hover:bg-gray-50">
                <td className="px-6 py-4 whitespace-nowrap text-sm font-medium text-gray-900">
                  {proc.exe_name}
                </td>
                <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500">
                  {proc.pid}
                </td>
                <td className={`px-6 py-4 whitespace-nowrap text-sm ${getSpeedColor(proc.upload_bps)}`}>
                  {formatBytesPerSecond(proc.upload_bps)}
                </td>
                <td className={`px-6 py-4 whitespace-nowrap text-sm ${getSpeedColor(proc.download_bps)}`}>
                  {formatBytesPerSecond(proc.download_bps)}
                </td>
                <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500">
                  {formatBytes(proc.total_sent)}
                </td>
                <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500">
                  {formatBytes(proc.total_recv)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {filteredAndSorted.length === 0 && (
        <div className="text-center py-8 text-gray-500">
          No active network connections found
        </div>
      )}
    </div>
  );
}
