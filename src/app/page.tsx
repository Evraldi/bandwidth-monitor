'use client';

import { useState } from 'react';
import RealTimeTab from '@/components/RealTimeTab';
import HistoryTab from '@/components/HistoryTab';
import MonthlyTab from '@/components/MonthlyTab';

type TabType = 'realtime' | 'history' | 'monthly';

export default function Home() {
  const [activeTab, setActiveTab] = useState<TabType>('realtime');

  const tabs = [
    { id: 'realtime' as TabType, label: 'Real-time', icon: '📊' },
    { id: 'history' as TabType, label: 'Daily History', icon: '📅' },
    { id: 'monthly' as TabType, label: 'Monthly Summary', icon: '📈' },
  ];

  return (
    <div className="min-h-screen bg-gradient-to-br from-gray-50 to-gray-100">
      <div className="container mx-auto px-4 py-8">
        {/* Header */}
        <div className="mb-8">
          <h1 className="text-4xl font-bold text-gray-900 mb-2">
            Bandwidth Monitor
          </h1>
          <p className="text-gray-600">
            Track your network usage per application in real-time
          </p>
        </div>

        {/* Tab Navigation */}
        <div className="bg-white rounded-lg shadow-sm border border-gray-200 mb-6">
          <div className="flex border-b border-gray-200">
            {tabs.map((tab) => (
              <button
                key={tab.id}
                onClick={() => setActiveTab(tab.id)}
                className={`
                  flex items-center gap-2 px-6 py-4 font-medium text-sm transition-colors
                  ${
                    activeTab === tab.id
                      ? 'text-blue-600 border-b-2 border-blue-600 bg-blue-50/50'
                      : 'text-gray-600 hover:text-gray-900 hover:bg-gray-50'
                  }
                `}
              >
                <span className="text-lg">{tab.icon}</span>
                {tab.label}
              </button>
            ))}
          </div>

          {/* Tab Content */}
          <div className="p-6">
            {activeTab === 'realtime' && <RealTimeTab />}
            {activeTab === 'history' && <HistoryTab />}
            {activeTab === 'monthly' && <MonthlyTab />}
          </div>
        </div>

        {/* Footer Info */}
        <div className="bg-yellow-50 border border-yellow-200 rounded-lg p-4 text-sm text-yellow-800">
          <p className="font-medium mb-1">⚠️ Administrator Privileges Required</p>
          <p className="text-yellow-700">
            This application requires administrator privileges to monitor network connections. 
            Make sure to run as administrator for accurate data collection.
          </p>
        </div>
      </div>
    </div>
  );
}
