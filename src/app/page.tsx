'use client';

import { useState, useEffect } from 'react';
import RealTimeTab from '@/components/RealTimeTab';
import HistoryTab from '@/components/HistoryTab';
import MonthlyTab from '@/components/MonthlyTab';
import SettingsTab from '@/components/SettingsTab';
import { getSettings, isAdmin } from '@/lib/tauri';
import { applyDarkMode, readDarkModeFromCache } from '@/lib/theme';

type TabType = 'realtime' | 'history' | 'monthly' | 'settings';

export default function Home() {
  const [activeTab, setActiveTab] = useState<TabType>('realtime');
  const [adminStatus, setAdminStatus] = useState<boolean | null>(null);

  useEffect(() => {
    // Apply cached theme immediately to avoid a flash, then let the
    // persisted setting be the source of truth.
    const cached = readDarkModeFromCache();
    if (cached !== null) applyDarkMode(cached);
    getSettings()
      .then((s) => applyDarkMode(s.dark_mode))
      .catch(() => {});

    isAdmin()
      .then(setAdminStatus)
      .catch(() => setAdminStatus(false));
  }, []);

  const tabs = [
    { id: 'realtime' as TabType, label: 'Real-time', icon: '📊' },
    { id: 'history' as TabType, label: 'Daily History', icon: '📅' },
    { id: 'monthly' as TabType, label: 'Monthly Summary', icon: '📈' },
    { id: 'settings' as TabType, label: 'Settings', icon: '⚙️' },
  ];

  const renderAdminBanner = () => {
    if (adminStatus === null) {
      return (
        <div className="bg-yellow-50 border border-yellow-200 rounded-lg p-4 text-sm text-yellow-800 dark:bg-yellow-900/30 dark:border-yellow-700 dark:text-yellow-200">
          <p className="font-medium mb-1">Checking administrator privileges...</p>
        </div>
      );
    }
    if (adminStatus) {
      return (
        <div className="bg-green-50 border border-green-200 rounded-lg p-4 text-sm text-green-800 dark:bg-green-900/30 dark:border-green-700 dark:text-green-200">
          <p className="font-medium mb-1">✓ Running as administrator</p>
          <p className="text-green-700 dark:text-green-300">
            Full network monitoring is active. You can close this window — the
            app keeps running in the system tray.
          </p>
        </div>
      );
    }
    return (
      <div className="bg-red-50 border border-red-200 rounded-lg p-4 text-sm text-red-800 dark:bg-red-900/30 dark:border-red-700 dark:text-red-200">
        <p className="font-medium mb-1">✗ Not running as administrator</p>
        <p className="text-red-700 dark:text-red-300">
          Network usage will not be accurate. Restart the app as administrator.
        </p>
      </div>
    );
  };

  return (
    <div className="min-h-screen bg-gradient-to-br from-gray-50 to-gray-100 dark:from-gray-900 dark:to-gray-950">
      <div className="container mx-auto px-4 py-8">
        {/* Header */}
        <div className="mb-8">
          <h1 className="text-4xl font-bold text-gray-900 dark:text-white mb-2">
            Bandwidth Monitor
          </h1>
          <p className="text-gray-600 dark:text-gray-400">
            Track your network usage per application in real-time
          </p>
        </div>

        {/* Tab Navigation */}
        <div className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 mb-6">
          <div className="flex border-b border-gray-200 dark:border-gray-700 overflow-x-auto">
            {tabs.map((tab) => (
              <button
                key={tab.id}
                onClick={() => setActiveTab(tab.id)}
                className={`
                  flex items-center gap-2 px-6 py-4 font-medium text-sm transition-colors whitespace-nowrap
                  ${
                    activeTab === tab.id
                      ? 'text-blue-600 border-b-2 border-blue-600 bg-blue-50/50 dark:text-blue-400 dark:bg-blue-900/30'
                      : 'text-gray-600 hover:text-gray-900 hover:bg-gray-50 dark:text-gray-400 dark:hover:text-gray-100 dark:hover:bg-gray-700'
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
            {activeTab === 'settings' && <SettingsTab />}
          </div>
        </div>

        {/* Admin Status */}
        {renderAdminBanner()}
      </div>
    </div>
  );
}
