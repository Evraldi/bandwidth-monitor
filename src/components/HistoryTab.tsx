'use client';

import { useState, useEffect } from 'react';
import { DailySummary, getDailySummary, formatBytes, aggregateToday } from '@/lib/tauri';
import { BarChart, Bar, XAxis, YAxis, CartesianGrid, Tooltip, Legend, ResponsiveContainer } from 'recharts';

function formatLocalDate(d: Date): string {
  const year = d.getFullYear();
  const month = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

export default function HistoryTab() {
  const [date, setDate] = useState(() => formatLocalDate(new Date()));
  const [summaries, setSummaries] = useState<DailySummary[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    const fetchData = async () => {
      setIsLoading(true);
      try {
        // Re-aggregate today's snapshots so History is fresh without waiting
        // for the midnight summarizer. aggregate_daily is idempotent, so this
        // is safe to run on every visit.
        if (date === formatLocalDate(new Date())) {
          await aggregateToday();
        }
        const data = await getDailySummary(date);
        setSummaries(data);
      } catch (error) {
        console.error('Failed to fetch daily summary:', error);
      } finally {
        setIsLoading(false);
      }
    };

    fetchData();
  }, [date]);

  const chartData = summaries.slice(0, 10).map(proc => ({
    name: proc.exe_name.length > 20 
      ? proc.exe_name.substring(0, 20) + '...' 
      : proc.exe_name,
    sent: proc.total_sent / (1024 * 1024), // Convert to MB
    recv: proc.total_recv / (1024 * 1024),
    total: (proc.total_sent + proc.total_recv) / (1024 * 1024),
  }));

  const totalSent = summaries.reduce((acc, proc) => acc + proc.total_sent, 0);
  const totalRecv = summaries.reduce((acc, proc) => acc + proc.total_recv, 0);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <label className="text-sm font-medium text-gray-700 dark:text-gray-300">
            Select Date:
          </label>
          <input
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
            max={formatLocalDate(new Date())}
            className="px-4 py-2 border border-gray-300 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
        </div>
        
        {!isLoading && summaries.length > 0 && (
          <div className="text-sm text-gray-600">
            <span className="font-medium">Total:</span>{' '}
            <span className="text-blue-600">{formatBytes(totalSent)}</span> sent •{' '}
            <span className="text-green-600">{formatBytes(totalRecv)}</span> received
          </div>
        )}
      </div>

      {isLoading ? (
        <div className="flex items-center justify-center h-64">
          <p className="text-gray-500 dark:text-gray-400">Loading daily summary...</p>
        </div>
      ) : summaries.length === 0 ? (
        <div className="flex items-center justify-center h-64 bg-gray-50 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-700">
          <p className="text-gray-500 dark:text-gray-400">No data available for this date</p>
        </div>
      ) : (
        <>
          <div className="bg-white dark:bg-gray-800 p-6 rounded-lg border border-gray-200 dark:border-gray-700">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">
              Top 10 Applications by Data Usage
            </h3>
            <ResponsiveContainer width="100%" height={300}>
              <BarChart
                data={chartData}
                layout="vertical"
                margin={{ top: 5, right: 30, left: 120, bottom: 5 }}
              >
                <CartesianGrid strokeDasharray="3 3" />
                <XAxis type="number" label={{ value: 'MB', position: 'insideBottom', offset: -5 }} />
                <YAxis type="category" dataKey="name" width={110} />
                <Tooltip 
                  formatter={(value) => typeof value === 'number' ? `${value.toFixed(2)} MB` : ''}
                  labelStyle={{ color: '#000' }}
                />
                <Legend />
                <Bar dataKey="sent" fill="#3b82f6" name="Sent" />
                <Bar dataKey="recv" fill="#10b981" name="Received" />
              </BarChart>
            </ResponsiveContainer>
          </div>

          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
            <div className="px-6 py-4 bg-gray-50 dark:bg-gray-700 border-b border-gray-200 dark:border-gray-700">
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
                Detailed Breakdown
              </h3>
            </div>
            <div className="overflow-x-auto">
              <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                <thead className="bg-gray-50 dark:bg-gray-700">
                  <tr>
                    <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      #
                    </th>
                    <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      Application
                    </th>
                    <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      Sent
                    </th>
                    <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      Received
                    </th>
                    <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      Total
                    </th>
                  </tr>
                </thead>
                <tbody className="bg-white dark:bg-gray-800 divide-y divide-gray-200 dark:divide-gray-700">
                  {summaries.map((proc, index) => (
                    <tr key={proc.exe_name} className="hover:bg-gray-50 dark:hover:bg-gray-700 dark:bg-gray-700">
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400">
                        {index + 1}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm font-medium text-gray-900 dark:text-gray-100">
                        {proc.exe_name}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-right text-blue-600">
                        {formatBytes(proc.total_sent)}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-right text-green-600">
                        {formatBytes(proc.total_recv)}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-right font-medium text-gray-900 dark:text-gray-100">
                        {formatBytes(proc.total_sent + proc.total_recv)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
