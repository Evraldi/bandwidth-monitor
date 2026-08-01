'use client';

import { useState, useEffect } from 'react';
import { DailySummary, getDailySummary, formatBytes } from '@/lib/tauri';
import { BarChart, Bar, XAxis, YAxis, CartesianGrid, Tooltip, Legend, ResponsiveContainer } from 'recharts';

export default function HistoryTab() {
  const [date, setDate] = useState(() => {
    const today = new Date();
    return today.toISOString().split('T')[0];
  });
  const [summaries, setSummaries] = useState<DailySummary[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    const fetchData = async () => {
      setIsLoading(true);
      try {
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
          <label className="text-sm font-medium text-gray-700">
            Select Date:
          </label>
          <input
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
            max={new Date().toISOString().split('T')[0]}
            className="px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
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
          <p className="text-gray-500">Loading daily summary...</p>
        </div>
      ) : summaries.length === 0 ? (
        <div className="flex items-center justify-center h-64 bg-gray-50 rounded-lg border border-gray-200">
          <p className="text-gray-500">No data available for this date</p>
        </div>
      ) : (
        <>
          <div className="bg-white p-6 rounded-lg border border-gray-200">
            <h3 className="text-lg font-semibold text-gray-900 mb-4">
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

          <div className="bg-white rounded-lg border border-gray-200 overflow-hidden">
            <div className="px-6 py-4 bg-gray-50 border-b border-gray-200">
              <h3 className="text-lg font-semibold text-gray-900">
                Detailed Breakdown
              </h3>
            </div>
            <div className="overflow-x-auto">
              <table className="min-w-full divide-y divide-gray-200">
                <thead className="bg-gray-50">
                  <tr>
                    <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                      #
                    </th>
                    <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                      Application
                    </th>
                    <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider">
                      Sent
                    </th>
                    <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider">
                      Received
                    </th>
                    <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider">
                      Total
                    </th>
                  </tr>
                </thead>
                <tbody className="bg-white divide-y divide-gray-200">
                  {summaries.map((proc, index) => (
                    <tr key={proc.exe_name} className="hover:bg-gray-50">
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500">
                        {index + 1}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm font-medium text-gray-900">
                        {proc.exe_name}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-right text-blue-600">
                        {formatBytes(proc.total_sent)}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-right text-green-600">
                        {formatBytes(proc.total_recv)}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-right font-medium text-gray-900">
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
