import { Activity, AlertTriangle, Clock, FileText, Link, Lock, Shield, TrendingUp, Users, XCircle } from 'lucide-react';
import type { Invite } from '../../../api/users';
import type { Role } from '../../../api/roles';
import type { AuditStats } from '../../../api/audit';
import { SettingsTabLayout } from './SettingsTabLayout';

type DashboardTabProps = {
  usersCount: number;
  pendingUsersCount: number;
  invites: Invite[];
  roles: Role[];
  auditStats: AuditStats | null;
  isExpired: (expiresAt: string | null) => boolean;
  formatAuditAction: (action: string) => string;
  onSelectSection: (section: 'users' | 'invites' | 'permissions' | 'audit') => void;
};

export function DashboardTab({
  usersCount,
  pendingUsersCount,
  invites,
  roles,
  auditStats,
  isExpired,
  formatAuditAction,
  onSelectSection,
}: DashboardTabProps) {
  const activeInvitesCount = invites.filter(i => !i.is_used && !isExpired(i.expires_at)).length;

  return (
    <SettingsTabLayout
      title="System Dashboard"
      description="Overview of system activity and health."
    >

      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-blue-100 dark:bg-blue-900/30 rounded-lg">
              <Users className="w-5 h-5 text-blue-600 dark:text-blue-400" />
            </div>
            <div>
              <div className="text-2xl font-bold text-gray-900 dark:text-white">{usersCount}</div>
              <div className="text-sm text-gray-500 dark:text-gray-400">Total Users</div>
            </div>
          </div>
        </div>
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-yellow-100 dark:bg-yellow-900/30 rounded-lg">
              <Clock className="w-5 h-5 text-yellow-600 dark:text-yellow-400" />
            </div>
            <div>
              <div className="text-2xl font-bold text-gray-900 dark:text-white">{pendingUsersCount}</div>
              <div className="text-sm text-gray-500 dark:text-gray-400">Pending Approval</div>
            </div>
          </div>
        </div>
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-green-100 dark:bg-green-900/30 rounded-lg">
              <Link className="w-5 h-5 text-green-600 dark:text-green-400" />
            </div>
            <div>
              <div className="text-2xl font-bold text-gray-900 dark:text-white">{activeInvitesCount}</div>
              <div className="text-sm text-gray-500 dark:text-gray-400">Active Invites</div>
            </div>
          </div>
        </div>
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-purple-100 dark:bg-purple-900/30 rounded-lg">
              <Shield className="w-5 h-5 text-purple-600 dark:text-purple-400" />
            </div>
            <div>
              <div className="text-2xl font-bold text-gray-900 dark:text-white">{roles.length}</div>
              <div className="text-sm text-gray-500 dark:text-gray-400">Roles</div>
            </div>
          </div>
        </div>
      </div>

      {auditStats && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
            <div className="flex items-center gap-2 mb-4">
              <Activity className="w-5 h-5 text-gray-500" />
              <h4 className="text-lg font-medium text-gray-900 dark:text-white">Activity Overview</h4>
            </div>
            <div className="space-y-4">
              <div className="flex justify-between items-center">
                <span className="text-gray-600 dark:text-gray-400">Events Today</span>
                <span className="text-xl font-semibold text-blue-600 dark:text-blue-400">{auditStats.events_today.toLocaleString()}</span>
              </div>
              <div className="flex justify-between items-center">
                <span className="text-gray-600 dark:text-gray-400">Events This Week</span>
                <span className="text-xl font-semibold text-gray-900 dark:text-white">{auditStats.events_this_week.toLocaleString()}</span>
              </div>
              <div className="flex justify-between items-center">
                <span className="text-gray-600 dark:text-gray-400">Total Events</span>
                <span className="text-xl font-semibold text-gray-900 dark:text-white">{auditStats.total_events.toLocaleString()}</span>
              </div>
              <div className="pt-4 border-t border-gray-200 dark:border-gray-700">
                <div className="flex justify-between items-center mb-2">
                  <span className="text-sm text-gray-500 dark:text-gray-400">Success Rate</span>
                  <span className="text-sm font-medium text-green-600 dark:text-green-400">
                    {auditStats.total_events > 0 ? ((auditStats.successful_events / auditStats.total_events) * 100).toFixed(1) : 0}%
                  </span>
                </div>
                <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2">
                  <div
                    className="bg-green-600 h-2 rounded-full transition-all duration-300"
                    style={{ width: `${auditStats.total_events > 0 ? (auditStats.successful_events / auditStats.total_events) * 100 : 0}%` }}
                  />
                </div>
              </div>
            </div>
          </div>

          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
            <div className="flex items-center gap-2 mb-4">
              <TrendingUp className="w-5 h-5 text-gray-500" />
              <h4 className="text-lg font-medium text-gray-900 dark:text-white">Top Actions</h4>
            </div>
            {auditStats.top_actions.length === 0 ? (
              <p className="text-gray-500 dark:text-gray-400 text-sm">No activity recorded yet.</p>
            ) : (
              <div className="space-y-3">
                {auditStats.top_actions.slice(0, 5).map((action, index) => (
                  <div key={action.action} className="flex items-center gap-3">
                    <span className="text-sm font-medium text-gray-400 w-4">{index + 1}</span>
                    <div className="flex-1">
                      <div className="flex justify-between items-center mb-1">
                        <span className="text-sm text-gray-900 dark:text-white">{formatAuditAction(action.action)}</span>
                        <span className="text-sm text-gray-500">{action.count.toLocaleString()}</span>
                      </div>
                      <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
                        <div
                          className="bg-blue-600 h-1.5 rounded-full transition-all duration-300"
                          style={{ width: `${(action.count / auditStats.top_actions[0].count) * 100}%` }}
                        />
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}

      {auditStats && auditStats.recent_failures.length > 0 && (
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
          <div className="flex items-center gap-2 mb-4">
            <AlertTriangle className="w-5 h-5 text-red-500" />
            <h4 className="text-lg font-medium text-gray-900 dark:text-white">Recent Failed Events</h4>
          </div>
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
              <thead>
                <tr>
                  <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">Time</th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">User</th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">Action</th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase hidden md:table-cell">Error</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                {auditStats.recent_failures.slice(0, 5).map((log) => (
                  <tr key={log.id}>
                    <td className="px-4 py-2 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">
                      {new Date(log.timestamp).toLocaleString()}
                    </td>
                    <td className="px-4 py-2 whitespace-nowrap text-sm text-gray-900 dark:text-white">
                      {log.username || 'Unknown'}
                    </td>
                    <td className="px-4 py-2 whitespace-nowrap">
                      <span className="inline-flex items-center gap-1 text-sm text-red-600 dark:text-red-400">
                        <XCircle className="w-4 h-4" />
                        {formatAuditAction(log.action)}
                      </span>
                    </td>
                    <td className="px-4 py-2 text-sm text-gray-500 dark:text-gray-400 hidden md:table-cell max-w-xs truncate" title={log.error_message || ''}>
                      {log.error_message || '-'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
            <button
              onClick={() => onSelectSection('audit')}
              className="text-sm text-blue-600 dark:text-blue-400 hover:underline"
            >
              View all audit logs →
            </button>
          </div>
        </div>
      )}

      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
        <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-4">Quick Actions</h4>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <button
            onClick={() => onSelectSection('users')}
            className="flex flex-col items-center gap-2 p-4 rounded-lg border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors"
          >
            <Users className="w-6 h-6 text-blue-600 dark:text-blue-400" />
            <span className="text-sm text-gray-700 dark:text-gray-300">Manage Users</span>
          </button>
          <button
            onClick={() => onSelectSection('invites')}
            className="flex flex-col items-center gap-2 p-4 rounded-lg border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors"
          >
            <Link className="w-6 h-6 text-green-600 dark:text-green-400" />
            <span className="text-sm text-gray-700 dark:text-gray-300">Create Invite</span>
          </button>
          <button
            onClick={() => onSelectSection('permissions')}
            className="flex flex-col items-center gap-2 p-4 rounded-lg border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors"
          >
            <Lock className="w-6 h-6 text-purple-600 dark:text-purple-400" />
            <span className="text-sm text-gray-700 dark:text-gray-300">Permissions</span>
          </button>
          <button
            onClick={() => onSelectSection('audit')}
            className="flex flex-col items-center gap-2 p-4 rounded-lg border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors"
          >
            <FileText className="w-6 h-6 text-orange-600 dark:text-orange-400" />
            <span className="text-sm text-gray-700 dark:text-gray-300">Audit Logs</span>
          </button>
        </div>
      </div>
    </SettingsTabLayout>
  );
}
