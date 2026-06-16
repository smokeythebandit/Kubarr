import type { Dispatch, ReactNode, SetStateAction } from 'react';
import { AlertTriangle, Trash2 } from 'lucide-react';
import type { AuditLog, AuditLogQuery, AuditStats } from '../../../api/audit';
import { SettingsTabLayout } from './SettingsTabLayout';

type AuditTabProps = {
  auditLogs: AuditLog[];
  auditStats: AuditStats | null;
  auditLoading: boolean;
  auditPage: number;
  auditTotalPages: number;
  auditTotal: number;
  auditFilter: AuditLogQuery;
  clearingLogs: boolean;
  onClearOldLogs: () => void;
  onAuditFilterChange: (key: keyof AuditLogQuery, value: string | boolean | undefined) => void;
  setAuditPage: Dispatch<SetStateAction<number>>;
  formatAuditAction: (action: string) => string;
  getActionIcon: (action: string, success: boolean) => ReactNode;
};

export function AuditTab({
  auditLogs,
  auditStats,
  auditLoading,
  auditPage,
  auditTotalPages,
  auditTotal,
  auditFilter,
  clearingLogs,
  onClearOldLogs,
  onAuditFilterChange,
  setAuditPage,
  formatAuditAction,
  getActionIcon,
}: AuditTabProps) {
  return (
    <SettingsTabLayout
      title="Audit Logs"
      description="Monitor system activity and security events."
      actions={(
        <button
          onClick={onClearOldLogs}
          disabled={clearingLogs}
          className="flex items-center gap-2 px-4 py-2 bg-red-600 hover:bg-red-700 disabled:bg-gray-600 disabled:cursor-not-allowed rounded-md font-medium text-white transition-colors"
        >
          <Trash2 size={16} />
          {clearingLogs ? 'Clearing...' : 'Clear Old Logs'}
        </button>
      )}
    >

      {auditStats && (
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
            <div className="text-sm text-gray-500 dark:text-gray-400">Total Events</div>
            <div className="text-2xl font-bold text-gray-900 dark:text-white">{auditStats.total_events.toLocaleString()}</div>
          </div>
          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
            <div className="text-sm text-gray-500 dark:text-gray-400">Today</div>
            <div className="text-2xl font-bold text-blue-600 dark:text-blue-400">{auditStats.events_today.toLocaleString()}</div>
          </div>
          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
            <div className="text-sm text-gray-500 dark:text-gray-400">Successful</div>
            <div className="text-2xl font-bold text-green-600 dark:text-green-400">{auditStats.successful_events.toLocaleString()}</div>
          </div>
          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
            <div className="text-sm text-gray-500 dark:text-gray-400">Failed</div>
            <div className="text-2xl font-bold text-red-600 dark:text-red-400">{auditStats.failed_events.toLocaleString()}</div>
          </div>
        </div>
      )}

      {auditStats && auditStats.recent_failures.length > 0 && (
        <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg p-4">
          <div className="flex items-center gap-2 text-red-800 dark:text-red-200 font-medium mb-2">
            <AlertTriangle size={16} />
            Recent Failed Events
          </div>
          <div className="space-y-1 text-sm text-red-700 dark:text-red-300">
            {auditStats.recent_failures.slice(0, 3).map((log) => (
              <div key={log.id} className="flex items-center gap-2">
                <span className="font-mono text-xs">{new Date(log.timestamp).toLocaleString()}</span>
                <span>{formatAuditAction(log.action)}</span>
                {log.username && <span className="text-red-600 dark:text-red-400">by {log.username}</span>}
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
        <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Search</label>
            <input
              type="text"
              value={auditFilter.search || ''}
              onChange={(e) => onAuditFilterChange('search', e.target.value)}
              placeholder="Search logs..."
              className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Action</label>
            <select
              value={auditFilter.action || ''}
              onChange={(e) => onAuditFilterChange('action', e.target.value)}
              className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              <option value="">All Actions</option>
              <option value="login">Login</option>
              <option value="login_failed">Login Failed</option>
              <option value="logout">Logout</option>
              <option value="2fa_verified">2FA Verified</option>
              <option value="2fa_failed">2FA Failed</option>
              <option value="user_created">User Created</option>
              <option value="user_updated">User Updated</option>
              <option value="user_deleted">User Deleted</option>
              <option value="role_assigned">Role Assigned</option>
              <option value="app_installed">App Installed</option>
              <option value="app_started">App Started</option>
              <option value="app_stopped">App Stopped</option>
            </select>
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Resource Type</label>
            <select
              value={auditFilter.resource_type || ''}
              onChange={(e) => onAuditFilterChange('resource_type', e.target.value)}
              className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              <option value="">All Types</option>
              <option value="user">User</option>
              <option value="role">Role</option>
              <option value="app">App</option>
              <option value="session">Session</option>
              <option value="system">System</option>
              <option value="invite">Invite</option>
            </select>
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Status</label>
            <select
              value={auditFilter.success === undefined ? '' : auditFilter.success.toString()}
              onChange={(e) => onAuditFilterChange('success', e.target.value === '' ? undefined : e.target.value === 'true')}
              className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              <option value="">All</option>
              <option value="true">Successful</option>
              <option value="false">Failed</option>
            </select>
          </div>
        </div>
      </div>

      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
        {auditLoading ? (
          <div className="flex justify-center items-center py-12">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
          </div>
        ) : auditLogs.length === 0 ? (
          <div className="text-center py-12 text-gray-500 dark:text-gray-400">
            No audit logs found.
          </div>
        ) : (
          <>
            <div className="overflow-x-auto">
              <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                <thead className="bg-gray-50 dark:bg-gray-700">
                  <tr>
                    <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Time</th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">User</th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Action</th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider hidden md:table-cell">Resource</th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Status</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                  {auditLogs.map((log) => (
                    <tr key={log.id} className={!log.success ? 'bg-red-50 dark:bg-red-900/10' : ''}>
                      <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">
                        <div>{new Date(log.timestamp).toLocaleDateString()}</div>
                        <div className="text-xs text-gray-500">{new Date(log.timestamp).toLocaleTimeString()}</div>
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-900 dark:text-white">
                        {log.username || <span className="text-gray-500">System</span>}
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap">
                        <div className="flex items-center gap-2">
                          {getActionIcon(log.action, log.success)}
                          <span className="text-sm text-gray-900 dark:text-white">{formatAuditAction(log.action)}</span>
                        </div>
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300 hidden md:table-cell">
                        <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-gray-100 dark:bg-gray-700 text-gray-800 dark:text-gray-200">
                          {log.resource_type}
                        </span>
                        {log.resource_id && <span className="ml-2 text-gray-500">#{log.resource_id}</span>}
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap">
                        {log.success ? (
                          <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-green-100 dark:bg-green-900 text-green-800 dark:text-green-200">
                            Success
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

            <div className="flex items-center justify-between px-4 py-3 border-t border-gray-200 dark:border-gray-700">
              <div className="text-sm text-gray-500 dark:text-gray-400">
                Showing {((auditPage - 1) * 20) + 1} to {Math.min(auditPage * 20, auditTotal)} of {auditTotal.toLocaleString()} entries
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => setAuditPage(p => Math.max(1, p - 1))}
                  disabled={auditPage === 1}
                  className="px-3 py-1 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 disabled:opacity-50 disabled:cursor-not-allowed rounded text-sm text-gray-700 dark:text-gray-300"
                >
                  Previous
                </button>
                <span className="text-sm text-gray-700 dark:text-gray-300">
                  Page {auditPage} of {auditTotalPages}
                </span>
                <button
                  onClick={() => setAuditPage(p => Math.min(auditTotalPages, p + 1))}
                  disabled={auditPage === auditTotalPages}
                  className="px-3 py-1 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 disabled:opacity-50 disabled:cursor-not-allowed rounded text-sm text-gray-700 dark:text-gray-300"
                >
                  Next
                </button>
              </div>
            </div>
          </>
        )}
      </div>
    </SettingsTabLayout>
  );
}
