import type { Dispatch, ReactNode, SetStateAction } from 'react';
import type { NotificationChannel, NotificationEvent, NotificationLog } from '../../../api/notifications';
import { SettingsTabLayout } from './SettingsTabLayout';

type ChannelConfigField = { key: string; label: string; type: string; placeholder: string };

type NotificationsTabProps = {
  notificationChannels: NotificationChannel[];
  notificationEvents: NotificationEvent[];
  notificationLogs: NotificationLog[];
  notificationLogsTotal: number;
  notificationLoading: boolean;
  testingChannel: string | null;
  testDestination: Record<string, string>;
  editingChannel: string | null;
  channelConfig: Record<string, Record<string, string>>;
  setTestDestination: Dispatch<SetStateAction<Record<string, string>>>;
  setEditingChannel: Dispatch<SetStateAction<string | null>>;
  setChannelConfig: Dispatch<SetStateAction<Record<string, Record<string, string>>>>;
  onToggleChannel: (channelType: string, enabled: boolean) => void;
  onToggleEvent: (eventType: string, enabled: boolean) => void;
  onUpdateEventSeverity: (eventType: string, severity: string) => void;
  onTestChannel: (channelType: string) => void;
  onSaveChannelConfig: (channelType: string) => void;
  getChannelIcon: (channelType: string) => ReactNode;
  getChannelConfigFields: (channelType: string) => ChannelConfigField[];
  formatAuditAction: (action: string) => string;
};

export function NotificationsTab({
  notificationChannels,
  notificationEvents,
  notificationLogs,
  notificationLogsTotal,
  notificationLoading,
  testingChannel,
  testDestination,
  editingChannel,
  channelConfig,
  setTestDestination,
  setEditingChannel,
  setChannelConfig,
  onToggleChannel,
  onToggleEvent,
  onUpdateEventSeverity,
  onTestChannel,
  onSaveChannelConfig,
  getChannelIcon,
  getChannelConfigFields,
  formatAuditAction,
}: NotificationsTabProps) {
  return (
    <SettingsTabLayout
      title="Notifications"
      description="Configure notification channels and event triggers."
    >

      {notificationLoading ? (
        <div className="flex justify-center items-center py-12">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
        </div>
      ) : (
        <>
          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
            <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-4">Notification Channels</h4>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
              Configure and enable notification channels. Users can set their own preferences for each enabled channel.
            </p>
            <div className="space-y-4">
              {notificationChannels.map((channel) => (
                <div key={channel.channel_type} className="border border-gray-200 dark:border-gray-700 rounded-lg p-4">
                  <div className="flex items-center justify-between mb-4">
                    <div className="flex items-center gap-3">
                      <div className={`p-2 rounded-lg ${channel.enabled ? 'bg-blue-100 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400' : 'bg-gray-100 dark:bg-gray-700 text-gray-500'}`}>
                        {getChannelIcon(channel.channel_type)}
                      </div>
                      <div>
                        <div className="font-medium text-gray-900 dark:text-white capitalize">{channel.channel_type}</div>
                        <div className="text-sm text-gray-500 dark:text-gray-400">
                          {channel.enabled ? 'Enabled' : 'Disabled'}
                        </div>
                      </div>
                    </div>
                    <button
                      onClick={() => onToggleChannel(channel.channel_type, !channel.enabled)}
                      className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 ${
                        channel.enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
                      }`}
                    >
                      <span
                        className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                          channel.enabled ? 'translate-x-6' : 'translate-x-1'
                        }`}
                      />
                    </button>
                  </div>

                  {editingChannel === channel.channel_type ? (
                    <div className="mt-4 space-y-3 border-t border-gray-200 dark:border-gray-700 pt-4">
                      {getChannelConfigFields(channel.channel_type).map((field) => (
                        <div key={field.key}>
                          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                            {field.label}
                          </label>
                          <input
                            type={field.type}
                            placeholder={field.placeholder}
                            value={channelConfig[channel.channel_type]?.[field.key] || ''}
                            onChange={(e) =>
                              setChannelConfig((prev) => ({
                                ...prev,
                                [channel.channel_type]: {
                                  ...prev[channel.channel_type],
                                  [field.key]: e.target.value,
                                },
                              }))
                            }
                            className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                          />
                        </div>
                      ))}
                      <div className="flex gap-2 mt-4">
                        <button
                          onClick={() => onSaveChannelConfig(channel.channel_type)}
                          className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-md font-medium transition-colors"
                        >
                          Save
                        </button>
                        <button
                          onClick={() => setEditingChannel(null)}
                          className="px-4 py-2 bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300 rounded-md font-medium transition-colors"
                        >
                          Cancel
                        </button>
                      </div>
                    </div>
                  ) : (
                    <div className="mt-4 flex items-center gap-2 border-t border-gray-200 dark:border-gray-700 pt-4">
                      <button
                        onClick={() => {
                          setEditingChannel(channel.channel_type);
                          setChannelConfig((prev) => ({
                            ...prev,
                            [channel.channel_type]: channel.config as Record<string, string> || {},
                          }));
                        }}
                        className="px-3 py-1.5 text-sm bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300 rounded-md transition-colors"
                      >
                        Configure
                      </button>
                      {channel.enabled && (
                        <div className="flex items-center gap-2">
                          <input
                            type="text"
                            placeholder={channel.channel_type === 'email' ? 'test@example.com' : channel.channel_type === 'telegram' ? 'Chat ID' : 'Phone number'}
                            value={testDestination[channel.channel_type] || ''}
                            onChange={(e) =>
                              setTestDestination((prev) => ({
                                ...prev,
                                [channel.channel_type]: e.target.value,
                              }))
                            }
                            className="px-3 py-1.5 text-sm bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                          />
                          <button
                            onClick={() => onTestChannel(channel.channel_type)}
                            disabled={testingChannel === channel.channel_type}
                            className="px-3 py-1.5 text-sm bg-green-600 hover:bg-green-700 disabled:bg-gray-600 text-white rounded-md transition-colors"
                          >
                            {testingChannel === channel.channel_type ? 'Testing...' : 'Test'}
                          </button>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </div>

          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
            <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-4">Event Triggers</h4>
            <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
              Choose which events trigger notifications.
            </p>
            <div className="overflow-x-auto">
              <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                <thead className="bg-gray-50 dark:bg-gray-700">
                  <tr>
                    <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Event</th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Severity</th>
                    <th className="px-4 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Enabled</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                  {notificationEvents.map((event) => (
                    <tr key={event.event_type}>
                      <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-900 dark:text-white">
                        {formatAuditAction(event.event_type)}
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap">
                        <select
                          value={event.severity}
                          onChange={(e) => onUpdateEventSeverity(event.event_type, e.target.value)}
                          className="px-2 py-1 text-sm bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
                        >
                          <option value="info">Info</option>
                          <option value="warning">Warning</option>
                          <option value="critical">Critical</option>
                        </select>
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap text-right">
                        <button
                          onClick={() => onToggleEvent(event.event_type, !event.enabled)}
                          className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 ${
                            event.enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
                          }`}
                        >
                          <span
                            className={`inline-block h-3 w-3 transform rounded-full bg-white transition-transform ${
                              event.enabled ? 'translate-x-5' : 'translate-x-1'
                            }`}
                          />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>

          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
            <div className="flex items-center justify-between mb-4">
              <h4 className="text-lg font-medium text-gray-900 dark:text-white">Delivery Logs</h4>
              <span className="text-sm text-gray-500 dark:text-gray-400">{notificationLogsTotal} total</span>
            </div>
            {notificationLogs.length === 0 ? (
              <div className="text-center py-8 text-gray-500 dark:text-gray-400">
                No notification logs yet.
              </div>
            ) : (
              <div className="overflow-x-auto">
                <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                  <thead className="bg-gray-50 dark:bg-gray-700">
                    <tr>
                      <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Time</th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Channel</th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Event</th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Status</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                    {notificationLogs.slice(0, 20).map((log) => (
                      <tr key={log.id} className={log.status === 'failed' ? 'bg-red-50 dark:bg-red-900/10' : ''}>
                        <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">
                          {new Date(log.created_at).toLocaleString()}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap">
                          <span className="inline-flex items-center gap-1.5 text-sm text-gray-900 dark:text-white capitalize">
                            {getChannelIcon(log.channel_type)}
                            {log.channel_type}
                          </span>
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">
                          {formatAuditAction(log.event_type)}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap">
                          {log.status === 'sent' ? (
                            <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-green-100 dark:bg-green-900 text-green-800 dark:text-green-200">
                              Sent
                            </span>
                          ) : (
                            <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-red-100 dark:bg-red-900 text-red-800 dark:text-red-200" title={log.error_message || ''}>
                              Failed
                            </span>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        </>
      )}
    </SettingsTabLayout>
  );
}
